use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use image::{ImageFormat, RgbaImage};
use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::AppState;

const PREVIEW_WIDTH: u32 = 1280;
const PALETTE: [&str; 8] = [
    "oklch(0.62 0.11 250)",
    "oklch(0.62 0.11 160)",
    "oklch(0.62 0.11 25)",
    "oklch(0.62 0.11 300)",
    "oklch(0.62 0.11 200)",
    "oklch(0.62 0.11 90)",
    "oklch(0.62 0.11 340)",
    "oklch(0.62 0.11 45)",
];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameView {
    pub id: i64,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub preview: String,
    pub created_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassView {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub threshold: f32,
    pub examples: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleView {
    pub id: i64,
    pub class_id: i64,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub preview: String,
    pub bbox: String,
    pub created_at: String,
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

fn data_url(image: &RgbaImage) -> String {
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    if image.write_to(&mut cursor, ImageFormat::Png).is_err() {
        return String::new();
    }
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(buffer)
    )
}

fn preview(image: &RgbaImage) -> String {
    let width = image.width();
    if width <= PREVIEW_WIDTH {
        return data_url(image);
    }
    let scale = PREVIEW_WIDTH as f32 / width as f32;
    let height = ((image.height() as f32 * scale).round() as u32).max(1);
    let small = image::imageops::resize(
        image,
        PREVIEW_WIDTH,
        height,
        image::imageops::FilterType::Triangle,
    );
    data_url(&small)
}

fn project_folder(app: &AppHandle, project_id: i64) -> Result<(String, PathBuf), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let slug: String = conn
        .query_row(
            "select slug from project where id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|_| "That project no longer exists".to_string())?;
    drop(conn);

    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("projects")
        .join(&slug);
    std::fs::create_dir_all(root.join("frames")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root.join("examples")).map_err(|e| e.to_string())?;
    Ok((slug, root))
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "class".to_string()
    } else {
        out
    }
}

/// Hides every iRecon window just long enough to grab the screen underneath,
/// then restores them even when the capture fails. Each window is hidden by
/// handle rather than by guessed label, and the capture waits for the window
/// server to report them hidden instead of trusting a fixed delay.
fn capture_without_window(app: &AppHandle) -> Result<RgbaImage, String> {
    let monitor = crate::capture::pick_monitor()?;
    // Remember which windows were on screen, so a hidden overlay stays hidden
    // and the main window is the one that gets focus back.
    let windows: Vec<(tauri::WebviewWindow, bool)> = app
        .webview_windows()
        .into_values()
        .map(|window| {
            let was_visible = window.is_visible().unwrap_or(false);
            (window, was_visible)
        })
        .collect();

    let shown: Vec<&tauri::WebviewWindow> = windows
        .iter()
        .filter(|(_, was_visible)| *was_visible)
        .map(|(window, _)| window)
        .collect();

    for window in &shown {
        if let Err(error) = window.hide() {
            return Err(format!("Could not hide the window for the grab: {error}"));
        }
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1000);
    while std::time::Instant::now() < deadline {
        if shown
            .iter()
            .all(|window| !window.is_visible().unwrap_or(true))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // One more compositor frame so the hidden window is really gone from the
    // display before it is read back.
    std::thread::sleep(std::time::Duration::from_millis(150));

    let captured = monitor.capture_image();

    for window in &shown {
        let _ = window.show();
    }
    if let Some((window, _)) = windows.iter().find(|(window, _)| window.label() == "main") {
        let _ = window.set_focus();
    }

    captured.map_err(|e| format!("Could not capture the screen: {e}"))
}

#[tauri::command]
pub fn frame_grab(app: AppHandle, project_id: i64) -> Result<FrameView, String> {
    let (_slug, folder) = project_folder(&app, project_id)?;
    let image = capture_without_window(&app)?;

    let stamp = now_millis();
    let path = folder.join("frames").join(format!("{stamp}.png"));
    image
        .save_with_format(&path, ImageFormat::Png)
        .map_err(|e| format!("Could not save the frame: {e}"))?;

    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "insert into frame (project_id, path, width, height, created_at)
         values (?1, ?2, ?3, ?4, datetime('now'))",
        params![
            project_id,
            path.to_string_lossy().to_string(),
            image.width() as i64,
            image.height() as i64
        ],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row("select created_at from frame where id = ?1", params![id], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?;
    drop(conn);

    Ok(FrameView {
        id,
        path: path.to_string_lossy().to_string(),
        width: image.width(),
        height: image.height(),
        preview: preview(&image),
        created_at,
    })
}

#[tauri::command]
pub fn classes_list(app: AppHandle, project_id: i64) -> Result<Vec<ClassView>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select c.id, c.name, c.color, c.threshold,
                    (select count(*) from example e where e.class_id = c.id)
             from class c
             where c.project_id = ?1
             order by c.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok(ClassView {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                threshold: row.get(3)?,
                examples: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut classes = Vec::new();
    for row in rows {
        classes.push(row.map_err(|e| e.to_string())?);
    }
    Ok(classes)
}

#[tauri::command]
pub fn class_create(app: AppHandle, project_id: i64, name: String) -> Result<ClassView, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the class a name".into());
    }
    if name.chars().count() > 40 {
        return Err("Keep the class name under 40 characters".into());
    }

    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    if let Ok(existing) = conn.query_row(
        "select id, name, color, threshold,
                (select count(*) from example e where e.class_id = class.id)
         from class where project_id = ?1 and lower(name) = lower(?2)",
        params![project_id, name],
        |row| {
            Ok(ClassView {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                threshold: row.get(3)?,
                examples: row.get(4)?,
            })
        },
    ) {
        return Ok(existing);
    }

    let count: i64 = conn
        .query_row(
            "select count(*) from class where project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let color = PALETTE[(count as usize) % PALETTE.len()].to_string();

    conn.execute(
        "insert into class (project_id, name, color, threshold) values (?1, ?2, ?3, 0.55)",
        params![project_id, name, color],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();

    Ok(ClassView {
        id,
        name,
        color,
        threshold: 0.55,
        examples: 0,
    })
}

#[tauri::command]
pub fn class_delete(app: AppHandle, id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let folder: Option<(i64, String)> = conn
        .query_row(
            "select project_id, name from class where id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    conn.execute("delete from class where id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    drop(conn);

    if let Some((project_id, name)) = folder {
        if let Ok((_slug, root)) = project_folder(&app, project_id) {
            let directory = root.join("examples").join(slugify(&name));
            if directory.exists() {
                let _ = std::fs::remove_dir_all(directory);
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn example_create(
    app: AppHandle,
    class_id: i64,
    frame_id: i64,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Result<ExampleView, String> {
    let state = app.state::<AppState>();
    let (project_id, frame_path, class_name): (i64, String, String) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let project_id: i64 = conn
            .query_row(
                "select project_id from frame where id = ?1",
                params![frame_id],
                |row| row.get(0),
            )
            .map_err(|_| "That frame is gone".to_string())?;
        let frame_path: String = conn
            .query_row("select path from frame where id = ?1", params![frame_id], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        let class_name: String = conn
            .query_row(
                "select name from class where id = ?1",
                params![class_id],
                |row| row.get(0),
            )
            .map_err(|_| "That class is gone".to_string())?;
        (project_id, frame_path, class_name)
    };

    let (_slug, root) = project_folder(&app, project_id)?;
    let source = image::open(&frame_path).map_err(|e| format!("Could not open the frame: {e}"))?;
    let source = source.to_rgba8();
    let (width, height) = (source.width(), source.height());

    let x0 = ((x.clamp(0.0, 1.0) * width as f32).round() as u32).min(width.saturating_sub(1));
    let y0 = ((y.clamp(0.0, 1.0) * height as f32).round() as u32).min(height.saturating_sub(1));
    let x1 = (((x + w).clamp(0.0, 1.0) * width as f32).round() as u32).min(width);
    let y1 = (((y + h).clamp(0.0, 1.0) * height as f32).round() as u32).min(height);
    if x1 <= x0 || y1 <= y0 {
        return Err("Drag a box that has some area to it".into());
    }

    let crop = image::imageops::crop_imm(&source, x0, y0, x1 - x0, y1 - y0).to_image();
    let directory = root.join("examples").join(slugify(&class_name));
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!("{}.png", now_millis()));
    crop.save_with_format(&path, ImageFormat::Png)
        .map_err(|e| format!("Could not save the crop: {e}"))?;

    let bbox = format!(
        "{{\"x\":{:.5},\"y\":{:.5},\"w\":{:.5},\"h\":{:.5}}}",
        x, y, w, h
    );

    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "insert into example (class_id, path, role, origin, bbox, created_at)
         values (?1, ?2, 'positive', 'draw', ?3, datetime('now'))",
        params![class_id, path.to_string_lossy().to_string(), bbox],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row(
            "select created_at from example where id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    drop(conn);

    Ok(ExampleView {
        id,
        class_id,
        path: path.to_string_lossy().to_string(),
        width: crop.width(),
        height: crop.height(),
        preview: data_url(&crop),
        bbox,
        created_at,
    })
}

#[tauri::command]
pub fn examples_list(app: AppHandle, class_id: i64) -> Result<Vec<ExampleView>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select id, class_id, path, bbox, created_at from example
             where class_id = ?1 order by id desc",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![class_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut examples = Vec::new();
    for row in rows {
        let (id, class_id, path, bbox, created_at) = row.map_err(|e| e.to_string())?;
        let (width, height, preview) = match image::open(&path) {
            Ok(image) => {
                let image = image.to_rgba8();
                (image.width(), image.height(), data_url(&image))
            }
            Err(_) => (0, 0, String::new()),
        };
        examples.push(ExampleView {
            id,
            class_id,
            path,
            width,
            height,
            preview,
            bbox,
            created_at,
        });
    }
    Ok(examples)
}
