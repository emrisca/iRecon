use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use image::{ImageFormat, RgbaImage};
use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::matcher;
use crate::AppState;

const PREVIEW_WIDTH: u32 = 1280;
const SETTING_PROJECT: &str = "label.project";
const SETTING_SOURCE: &str = "label.source";
const SETTING_SHORTCUT: &str = "label.grab_shortcut";
const DEFAULT_GRAB_SHORTCUT: &str = "Ctrl+Alt+G";
const PREVIEW_QUALITY: u8 = 70;
pub(crate) const PALETTE: [&str; 8] = [
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
    /// Cached thumbnail file. The webview loads it through the asset protocol,
    /// which keeps a frame list from carrying base64 for every row.
    pub thumb_path: String,
    pub created_at: String,
    /// Number of saved examples on this frame, across every class.
    pub examples: i64,
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
pub struct CleanupReport {
    pub removed_classes: usize,
    pub removed_examples: usize,
    pub merged_classes: usize,
    pub moved_examples: usize,
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
    /// Crop file on disk, loaded through the asset protocol beside `path`.
    pub thumb_path: String,
    pub bbox: String,
    pub role: String,
    pub created_at: String,
}

pub(crate) fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

pub(crate) fn png_data_url(image: &RgbaImage) -> String {
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

pub(crate) fn jpeg_data_url(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Downsamples to `target_width` and encodes JPEG bytes. Preview payloads are
/// JPEG, not PNG: a 1280-wide PNG of a game frame is over a megabyte of base64
/// each. The bytes are also what the library caches as a thumbnail, so opening
/// a project does not decode every full-size frame again.
pub(crate) fn jpeg_bytes(image: &RgbaImage, target_width: u32, quality: u8) -> Vec<u8> {
    let width = image.width();
    let height = ((image.height() as f32 * target_width as f32 / width.max(1) as f32)
        .round() as u32)
        .max(1);
    let small = if width <= target_width {
        image.clone()
    } else {
        image::imageops::resize(
            image,
            target_width,
            height,
            image::imageops::FilterType::Triangle,
        )
    };

    let rgb = image::RgbImage::from_fn(small.width(), small.height(), |x, y| {
        let [r, g, b, _] = small.get_pixel(x, y).0;
        image::Rgb([r, g, b])
    });
    let mut buffer = Vec::new();
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, quality);
    if encoder.encode_image(&rgb).is_err() {
        return Vec::new();
    }
    buffer
}

/// Grid thumbnails stay small because the Label stage loads the original
/// frame file; only listings use this size.
pub(crate) const THUMB_WIDTH: u32 = 320;
pub(crate) const THUMB_QUALITY: u8 = 70;

pub(crate) fn preview(image: &RgbaImage) -> String {
    jpeg_data_url(&jpeg_bytes(image, PREVIEW_WIDTH, PREVIEW_QUALITY))
}

pub(crate) fn project_folder(
    app: &AppHandle,
    project_id: i64,
) -> Result<(String, PathBuf), String> {
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabSettings {
    pub shortcut: Option<String>,
    pub project_id: Option<i64>,
    pub source_id: Option<i64>,
}

fn read_setting(app: &AppHandle, key: &str) -> Option<String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().ok()?;
    conn.query_row(
        "select value from setting where key = ?1",
        params![key],
        |row| row.get(0),
    )
    .ok()
}

fn write_setting(app: &AppHandle, key: &str, value: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "insert into setting (key, value) values (?1, ?2)
         on conflict(key) do update set value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn grab_settings(app: AppHandle) -> Result<GrabSettings, String> {
    let shortcut = match read_setting(&app, SETTING_SHORTCUT) {
        Some(value) if value.trim().eq_ignore_ascii_case("none") => None,
        Some(value) if value.trim().is_empty() => Some(DEFAULT_GRAB_SHORTCUT.to_string()),
        Some(value) => Some(value),
        None => Some(DEFAULT_GRAB_SHORTCUT.to_string()),
    };
    let project = read_setting(&app, SETTING_PROJECT).and_then(|value| value.parse::<i64>().ok());
    let source = read_setting(&app, SETTING_SOURCE).and_then(|value| value.parse::<i64>().ok());

    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let project_id = project.filter(|id| {
        conn.query_row("select 1 from project where id = ?1", params![id], |_| Ok(()))
            .is_ok()
    });
    let source_id = match (project_id, source) {
        (Some(project_id), Some(source_id)) if conn
            .query_row(
                "select 1 from source where id = ?1 and project_id = ?2",
                params![source_id, project_id],
                |_| Ok(()),
            )
            .is_ok() =>
        {
            Some(source_id)
        }
        _ => None,
    };

    Ok(GrabSettings {
        shortcut,
        project_id,
        source_id,
    })
}

#[tauri::command]
pub fn grab_target_set(
    app: AppHandle,
    project_id: i64,
    source_id: Option<i64>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "select 1 from project where id = ?1",
        params![project_id],
        |_| Ok(()),
    )
    .map_err(|_| "That project no longer exists".to_string())?;
    if let Some(source_id) = source_id {
        conn.query_row(
            "select 1 from source where id = ?1 and project_id = ?2",
            params![source_id, project_id],
            |_| Ok(()),
        )
        .map_err(|_| "That source belongs to another project".to_string())?;
    }
    drop(conn);

    write_setting(&app, SETTING_PROJECT, &project_id.to_string())?;
    write_setting(
        &app,
        SETTING_SOURCE,
        &source_id.map(|id| id.to_string()).unwrap_or_default(),
    )?;
    Ok(())
}

#[tauri::command]
pub fn grab_shortcut_set(app: AppHandle, shortcut: Option<String>) -> Result<(), String> {
    let shortcut = shortcut
        .map(|value| normalize_global_shortcut(&value))
        .filter(|value| !value.is_empty());
    register_grab_shortcut(&app, shortcut.as_deref())?;
    write_setting(
        &app,
        SETTING_SHORTCUT,
        shortcut.as_deref().unwrap_or("none"),
    )?;
    Ok(())
}

/// A bare letter registered with the desktop would be swallowed in every app,
/// so single keys become Ctrl+Alt+key. Function keys are safe on their own.
fn normalize_global_shortcut(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains('+') || is_function_key(trimmed) {
        trimmed.to_string()
    } else {
        format!("Ctrl+Alt+{trimmed}")
    }
}

fn is_function_key(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_uppercase().as_str(),
        "F1" | "F2" | "F3" | "F4" | "F5" | "F6" | "F7" | "F8" | "F9" | "F10" | "F11" | "F12"
    )
}

fn parse_shortcut(value: &str) -> Option<(Option<Modifiers>, Code)> {
    let mut parts: Vec<&str> = value
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let key = parts.pop()?;
    let mut modifiers = Modifiers::empty();
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" | "option" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "meta" | "super" | "cmd" | "command" => modifiers |= Modifiers::META,
            _ => return None,
        }
    }
    let code = key_code(key)?;
    Some((if modifiers.is_empty() { None } else { Some(modifiers) }, code))
}

/// Registers the grab shortcut with the desktop, so it still fires when iRecon
/// is behind another window. `None` removes the global binding.
pub(crate) fn register_grab_shortcut(
    app: &AppHandle,
    shortcut: Option<&str>,
) -> Result<(), String> {
    let label = shortcut
        .map(normalize_global_shortcut)
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let parsed = if label.is_empty() {
        None
    } else {
        let (modifiers, code) = parse_shortcut(&label)
            .ok_or_else(|| format!("{label} cannot be used as a global shortcut"))?;
        Some(Shortcut::new(modifiers, code))
    };

    let state = app.state::<AppState>();
    let mut current = state.grab_shortcut.lock().map_err(|e| e.to_string())?;
    if current
        .as_ref()
        .map(|(saved, _)| saved.as_str())
        .unwrap_or_default()
        == label
    {
        return Ok(());
    }
    if let Some((_, old)) = current.take() {
        let _ = app.global_shortcut().unregister(old);
    }

    let Some(shortcut) = parsed else {
        return Ok(());
    };
    app.global_shortcut()
        .on_shortcut(shortcut.clone(), move |app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let handle = app.clone();
            std::thread::spawn(move || match grab_from_settings(&handle) {
                Ok(frame) => {
                    let _ = handle.emit("spike:frame-grabbed", frame);
                }
                Err(error) => {
                    let _ = handle.emit("spike:grab-error", error);
                }
            });
        })
        .map_err(|error| format!("Could not register {label}: {error}"))?;
    *current = Some((label, shortcut));
    Ok(())
}

pub(crate) fn register_saved_grab_shortcut(app: &AppHandle) {
    let saved = match read_setting(app, SETTING_SHORTCUT) {
        Some(value) if value.trim().eq_ignore_ascii_case("none") => return,
        Some(value) if value.trim().is_empty() => {
            let _ = write_setting(app, SETTING_SHORTCUT, DEFAULT_GRAB_SHORTCUT);
            DEFAULT_GRAB_SHORTCUT.to_string()
        }
        Some(value) => value,
        None => {
            let _ = write_setting(app, SETTING_SHORTCUT, DEFAULT_GRAB_SHORTCUT);
            DEFAULT_GRAB_SHORTCUT.to_string()
        }
    };
    let shortcut = normalize_global_shortcut(&saved);
    if shortcut != saved {
        let _ = write_setting(app, SETTING_SHORTCUT, &shortcut);
    }
    if let Err(error) = register_grab_shortcut(app, Some(&shortcut)) {
        eprintln!("iRecon could not register the saved grab shortcut {shortcut}: {error}");
    }
}

fn grab_from_settings(app: &AppHandle) -> Result<FrameView, String> {
    let settings = grab_settings(app.clone())?;
    let project_id = settings.project_id.ok_or_else(|| {
        "Choose a project on the Label page before using the global grab shortcut".to_string()
    })?;
    frame_grab(app.clone(), project_id, settings.source_id)
}

fn key_code(label: &str) -> Option<Code> {
    let upper = label.trim().to_ascii_uppercase();
    match upper.as_str() {
        "SPACE" => return Some(Code::Space),
        "ARROWUP" | "UP" => return Some(Code::ArrowUp),
        "ARROWDOWN" | "DOWN" => return Some(Code::ArrowDown),
        "ARROWLEFT" | "LEFT" => return Some(Code::ArrowLeft),
        "ARROWRIGHT" | "RIGHT" => return Some(Code::ArrowRight),
        "ENTER" | "RETURN" => return Some(Code::Enter),
        "TAB" => return Some(Code::Tab),
        "BACKSPACE" => return Some(Code::Backspace),
        "DELETE" => return Some(Code::Delete),
        "INSERT" => return Some(Code::Insert),
        "HOME" => return Some(Code::Home),
        "END" => return Some(Code::End),
        "PAGEUP" => return Some(Code::PageUp),
        "PAGEDOWN" => return Some(Code::PageDown),
        "ESCAPE" => return Some(Code::Escape),
        "F1" => return Some(Code::F1),
        "F2" => return Some(Code::F2),
        "F3" => return Some(Code::F3),
        "F4" => return Some(Code::F4),
        "F5" => return Some(Code::F5),
        "F6" => return Some(Code::F6),
        "F7" => return Some(Code::F7),
        "F8" => return Some(Code::F8),
        "F9" => return Some(Code::F9),
        "F10" => return Some(Code::F10),
        "F11" => return Some(Code::F11),
        "F12" => return Some(Code::F12),
        _ => {}
    }

    let mut chars = upper.chars();
    let character = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(match character {
        'A' => Code::KeyA,
        'B' => Code::KeyB,
        'C' => Code::KeyC,
        'D' => Code::KeyD,
        'E' => Code::KeyE,
        'F' => Code::KeyF,
        'G' => Code::KeyG,
        'H' => Code::KeyH,
        'I' => Code::KeyI,
        'J' => Code::KeyJ,
        'K' => Code::KeyK,
        'L' => Code::KeyL,
        'M' => Code::KeyM,
        'N' => Code::KeyN,
        'O' => Code::KeyO,
        'P' => Code::KeyP,
        'Q' => Code::KeyQ,
        'R' => Code::KeyR,
        'S' => Code::KeyS,
        'T' => Code::KeyT,
        'U' => Code::KeyU,
        'V' => Code::KeyV,
        'W' => Code::KeyW,
        'X' => Code::KeyX,
        'Y' => Code::KeyY,
        'Z' => Code::KeyZ,
        '0' => Code::Digit0,
        '1' => Code::Digit1,
        '2' => Code::Digit2,
        '3' => Code::Digit3,
        '4' => Code::Digit4,
        '5' => Code::Digit5,
        '6' => Code::Digit6,
        '7' => Code::Digit7,
        '8' => Code::Digit8,
        '9' => Code::Digit9,
        '-' => Code::Minus,
        '=' => Code::Equal,
        '[' => Code::BracketLeft,
        ']' => Code::BracketRight,
        ';' => Code::Semicolon,
        '\'' => Code::Quote,
        ',' => Code::Comma,
        '.' => Code::Period,
        '/' => Code::Slash,
        '\\' => Code::Backslash,
        '`' => Code::Backquote,
        _ => return None,
    })
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

#[tauri::command]
pub fn frame_grab(
    app: AppHandle,
    project_id: i64,
    source_id: Option<i64>,
) -> Result<FrameView, String> {
    let (_slug, folder) = project_folder(&app, project_id)?;
    let image = match source_id {
        Some(id) => {
            let source = crate::source::source_row(&app, id)?;
            if source.project_id != project_id {
                return Err("That source belongs to another project".into());
            }
            if source.kind == "display" {
                crate::grab::grab_display(&app, &source.target)?
            } else {
                crate::source::load_frame(&source)?
            }
        }
        None => crate::grab::grab(&app)?,
    };

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
        thumb_path: String::new(),
        created_at,
        examples: 0,
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
        "insert into class (project_id, name, color, threshold) values (?1, ?2, ?3, 0.45)",
        params![project_id, name, color],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();

    Ok(ClassView {
        id,
        name,
        color,
        threshold: 0.45,
        examples: 0,
    })
}

fn valid_color(value: &str) -> bool {
    match value.strip_prefix('#') {
        Some(hex) => hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()),
        None => value.starts_with("oklch(") && value.ends_with(')'),
    }
}

fn read_class(conn: &rusqlite::Connection, id: i64) -> Result<ClassView, String> {
    conn.query_row(
        "select c.id, c.name, c.color, c.threshold,
                (select count(*) from example e where e.class_id = c.id)
         from class c where c.id = ?1",
        params![id],
        |row| {
            Ok(ClassView {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                threshold: row.get(3)?,
                examples: row.get(4)?,
            })
        },
    )
    .map_err(|_| "That class no longer exists".to_string())
}

/// Renames a class or changes its colour, or both. Names stay unique per
/// project because the class name is how a crop is filed on disk.
#[tauri::command]
pub fn class_update(
    app: AppHandle,
    id: i64,
    name: Option<String>,
    color: Option<String>,
) -> Result<ClassView, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let project_id: i64 = conn
        .query_row(
            "select project_id from class where id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|_| "That class no longer exists".to_string())?;

    if let Some(name) = name {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("Give the class a name".into());
        }
        if name.chars().count() > 40 {
            return Err("Keep the class name under 40 characters".into());
        }
        let clash: Option<i64> = conn
            .query_row(
                "select id from class
                 where project_id = ?1 and lower(name) = lower(?2) and id != ?3",
                params![project_id, name, id],
                |row| row.get(0),
            )
            .ok();
        if clash.is_some() {
            return Err("A class with that name already exists".into());
        }
        conn.execute(
            "update class set name = ?1 where id = ?2",
            params![name, id],
        )
        .map_err(|e| e.to_string())?;
    }

    if let Some(color) = color {
        let color = color.trim().to_string();
        if !valid_color(&color) {
            return Err("Pick a colour from the palette".into());
        }
        conn.execute(
            "update class set color = ?1 where id = ?2",
            params![color, id],
        )
        .map_err(|e| e.to_string())?;
    }

    read_class(&conn, id)
}

#[tauri::command]
pub fn class_threshold(app: AppHandle, id: i64, threshold: f32) -> Result<ClassView, String> {
    if !(0.0..=1.0).contains(&threshold) {
        return Err("A threshold goes from 0 to 1".into());
    }
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "update class set threshold = ?1 where id = ?2",
        params![threshold, id],
    )
    .map_err(|e| e.to_string())?;
    conn.query_row(
        "select c.id, c.name, c.color, c.threshold,
                (select count(*) from example e where e.class_id = c.id)
         from class c where c.id = ?1",
        params![id],
        |row| {
            Ok(ClassView {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                threshold: row.get(3)?,
                examples: row.get(4)?,
            })
        },
    )
    .map_err(|_| "That class no longer exists".to_string())
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

/// A name made only of punctuation or whitespace is an export placeholder, not
/// a real class. Matching is case and whitespace insensitive for duplicates.
fn class_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[tauri::command]
pub fn classes_cleanup(app: AppHandle, project_id: i64) -> Result<CleanupReport, String> {
    let state = app.state::<AppState>();
    let mut conn = state.db.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut statement = tx
        .prepare("select id, name from class where project_id = ?1 order by id")
        .map_err(|e| e.to_string())?;
    let classes = statement
        .query_map(params![project_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(statement);

    let mut report = CleanupReport {
        removed_classes: 0,
        removed_examples: 0,
        merged_classes: 0,
        moved_examples: 0,
    };
    let mut placeholder_folders = Vec::new();
    for (id, name) in &classes {
        if name.chars().any(|ch| ch.is_alphanumeric()) {
            continue;
        }
        report.removed_examples += tx
            .execute("delete from example where class_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.execute("delete from candidate where class_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.execute("delete from frame_truth where class_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.execute("delete from class where id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        report.removed_classes += 1;
        placeholder_folders.push(name.clone());
    }

    let mut groups: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for (id, name) in classes {
        if !name.chars().any(|ch| ch.is_alphanumeric()) {
            continue;
        }
        groups.entry(class_key(&name)).or_default().push((id, name));
    }
    for members in groups.values() {
        if members.len() < 2 {
            continue;
        }
        let mut ranked = Vec::with_capacity(members.len());
        for (id, name) in members {
            let count: i64 = tx
                .query_row(
                    "select count(*) from example where class_id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            ranked.push((count, *id, name.clone()));
        }
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let target = ranked[0].1;
        for (_, source, _) in ranked.iter().skip(1) {
            report.moved_examples += tx
                .execute(
                    "update example set class_id = ?1 where class_id = ?2",
                    params![target, source],
                )
                .map_err(|e| e.to_string())?;
            tx.execute(
                "update candidate set class_id = ?1 where class_id = ?2",
                params![target, source],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "insert or ignore into frame_truth (frame_id, class_id)
                 select frame_id, ?1 from frame_truth where class_id = ?2",
                params![target, source],
            )
            .map_err(|e| e.to_string())?;
            tx.execute("delete from frame_truth where class_id = ?1", params![source])
                .map_err(|e| e.to_string())?;
            tx.execute("delete from class where id = ?1", params![source])
                .map_err(|e| e.to_string())?;
            report.merged_classes += 1;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    drop(conn);

    if let Ok((_slug, root)) = project_folder(&app, project_id) {
        for name in placeholder_folders {
            let directory = root.join("examples").join(slugify(&name));
            if directory.exists() {
                let _ = std::fs::remove_dir_all(directory);
            }
        }
    }
    Ok(report)
}

#[tauri::command]
pub fn classes_merge(
    app: AppHandle,
    source_id: i64,
    target_id: i64,
) -> Result<ClassView, String> {
    if source_id == target_id {
        return Err("Pick a different class to merge into".into());
    }
    let state = app.state::<AppState>();
    let mut conn = state.db.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let source_project: i64 = tx
        .query_row(
            "select project_id from class where id = ?1",
            params![source_id],
            |row| row.get(0),
        )
        .map_err(|_| "That class no longer exists".to_string())?;
    let target_project: i64 = tx
        .query_row(
            "select project_id from class where id = ?1",
            params![target_id],
            |row| row.get(0),
        )
        .map_err(|_| "That class no longer exists".to_string())?;
    if source_project != target_project {
        return Err("Those classes are in different projects".into());
    }

    tx.execute(
        "update example set class_id = ?1 where class_id = ?2",
        params![target_id, source_id],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "update candidate set class_id = ?1 where class_id = ?2",
        params![target_id, source_id],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "insert or ignore into frame_truth (frame_id, class_id)
         select frame_id, ?1 from frame_truth where class_id = ?2",
        params![target_id, source_id],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "delete from frame_truth where class_id = ?1",
        params![source_id],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("delete from class where id = ?1", params![source_id])
        .map_err(|e| e.to_string())?;
    let target = read_class(&tx, target_id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(target)
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
    role: Option<String>,
) -> Result<ExampleView, String> {
    let role = role.unwrap_or_else(|| "positive".to_string());
    if role != "positive" && role != "negative" {
        return Err("An example is a positive or a negative".into());
    }
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

    let embedding = matcher::embed(&crop);
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "insert into example
           (class_id, frame_id, path, role, origin, bbox, embedding, embedding_model, created_at)
         values (?1, ?2, ?3, ?7, 'draw', ?4, ?5, ?6, datetime('now'))",
        params![
            class_id,
            frame_id,
            path.to_string_lossy().to_string(),
            bbox,
            matcher::to_blob(&embedding),
            matcher::EMBEDDING_MODEL,
            role
        ],
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
        preview: String::new(),
        thumb_path: path.to_string_lossy().to_string(),
        bbox,
        role,
        created_at,
    })
}

#[tauri::command]
pub fn examples_list(
    app: AppHandle,
    class_id: i64,
    before_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<ExampleView>, String> {
    let limit = i64::from(limit.unwrap_or(400).min(400));
    let before = before_id.unwrap_or(i64::MAX);
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select id, class_id, path, coalesce(bbox, ''), role, created_at
             from example where class_id = ?1 and id < ?2 order by id desc limit ?3",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![class_id, before, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut examples = Vec::new();
    for row in rows {
        let (id, class_id, path, bbox, role, created_at) = row.map_err(|e| e.to_string())?;
        // Only the header is read. The webview loads the crop itself, so a class
        // with thousands of examples no longer base64s every PNG here.
        let (width, height) = image::image_dimensions(&path).unwrap_or((0, 0));
        examples.push(ExampleView {
            id,
            class_id,
            path: path.clone(),
            width,
            height,
            preview: String::new(),
            thumb_path: path,
            bbox,
            role,
            created_at,
        });
    }
    Ok(examples)
}

#[tauri::command]
pub fn example_delete(app: AppHandle, id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let path: Option<String> = conn
        .query_row("select path from example where id = ?1", params![id], |row| {
            row.get(0)
        })
        .ok();
    let Some(path) = path else {
        return Ok(());
    };
    conn.execute("delete from example where id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    drop(conn);
    if !path.is_empty() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// Ranks the project's classes against one box in a stored frame.
#[tauri::command]
pub fn match_region(
    app: AppHandle,
    project_id: i64,
    frame_id: i64,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> Result<Vec<matcher::ClassMatch>, String> {
    let state = app.state::<AppState>();
    let (frame_path, names, entries) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let frame_path: String = conn
            .query_row(
                "select path from frame where id = ?1 and project_id = ?2",
                params![frame_id, project_id],
                |row| row.get(0),
            )
            .map_err(|_| "That frame is gone".to_string())?;

        let mut names = HashMap::new();
        {
            let mut statement = conn
                .prepare("select id, name from class where project_id = ?1")
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map(params![project_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (id, name) = row.map_err(|e| e.to_string())?;
                names.insert(id, name);
            }
        }

        let mut entries = Vec::new();
        {
            let mut statement = conn
                .prepare(
                    "select e.id, e.class_id, e.role, e.embedding
                     from example e join class c on c.id = e.class_id
                     where c.project_id = ?1 and e.embedding is not null
                       and e.embedding_model = ?2",
                )
                .map_err(|e| e.to_string())?;
            let rows = statement
                .query_map(params![project_id, matcher::EMBEDDING_MODEL], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (example_id, class_id, role, blob) = row.map_err(|e| e.to_string())?;
                if let Some(embedding) = matcher::from_blob(&blob) {
                    entries.push(matcher::PoolEntry {
                        example_id,
                        class_id,
                        role,
                        embedding,
                    });
                }
            }
        }
        (frame_path, names, entries)
    };

    if entries.is_empty() {
        return Ok(Vec::new());
    }

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
    let candidate = matcher::embed(&crop);
    Ok(matcher::rank(&candidate, &entries, &names))
}

/// Fills in fingerprints for examples saved before the matcher existed, or by
/// an older fingerprint version. Returns how many were written.
#[tauri::command]
pub fn examples_embed(app: AppHandle, project_id: i64) -> Result<usize, String> {
    let state = app.state::<AppState>();
    let pending: Vec<(i64, String)> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare(
                "select e.id, e.path from example e join class c on c.id = e.class_id
                 where c.project_id = ?1
                   and (e.embedding is null or e.embedding_model is not ?2)",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id, matcher::EMBEDDING_MODEL], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut pending = Vec::new();
        for row in rows {
            pending.push(row.map_err(|e| e.to_string())?);
        }
        pending
    };

    let mut embedded = 0usize;
    for (id, path) in pending {
        let Ok(image) = image::open(&path) else {
            continue;
        };
        let embedding = matcher::embed(&image.to_rgba8());
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "update example set embedding = ?1, embedding_model = ?2 where id = ?3",
            params![matcher::to_blob(&embedding), matcher::EMBEDDING_MODEL, id],
        )
        .map_err(|e| e.to_string())?;
        embedded += 1;
    }
    Ok(embedded)
}
