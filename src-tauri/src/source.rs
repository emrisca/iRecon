//! Where a project's frames come from.
//!
//! A source is a display, a window, an image file or a folder of images. The row
//! keeps the picker target (a display or window id, or a path) and an optional
//! crop rectangle. The crop is stored normalized, like every other box in the
//! app, so it means the same thing at any resolution.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use xcap::{Monitor, Window};

use crate::capture::FramePayload;
use crate::label;
use crate::AppState;

const PREVIEW_WIDTH: u32 = 1280;
pub(crate) const IMAGE_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// One thing the picker can offer.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTarget {
    pub target: String,
    pub label: String,
    pub detail: String,
}

/// The part of a source the project cares about, normalized to 0..1.
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub id: i64,
    pub project_id: i64,
    pub kind: String,
    pub label: String,
    pub target: String,
    pub crop: Option<Crop>,
}

pub(crate) fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| IMAGE_EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn crop_is_valid(crop: &Crop) -> bool {
    crop.w > 0.0
        && crop.h > 0.0
        && crop.x >= 0.0
        && crop.y >= 0.0
        && crop.x + crop.w <= 1.0001
        && crop.y + crop.h <= 1.0001
}

fn row_to_source(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceView> {
    let x: Option<f32> = row.get(5)?;
    let y: Option<f32> = row.get(6)?;
    let w: Option<f32> = row.get(7)?;
    let h: Option<f32> = row.get(8)?;
    let crop = match (x, y, w, h) {
        (Some(x), Some(y), Some(w), Some(h)) => Some(Crop { x, y, w, h }),
        _ => None,
    };
    Ok(SourceView {
        id: row.get(0)?,
        project_id: row.get(1)?,
        kind: row.get(2)?,
        label: row.get(3)?,
        target: row.get(4)?,
        crop,
    })
}

pub(crate) fn source_row(app: &AppHandle, id: i64) -> Result<SourceView, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.query_row(
        "select id, project_id, kind, label, target,
                crop_x, crop_y, crop_w, crop_h
         from source where id = ?1",
        params![id],
        row_to_source,
    )
    .map_err(|_| "That source no longer exists".to_string())
}

fn display_targets() -> Result<Vec<CaptureTarget>, String> {
    let monitors = Monitor::all().map_err(|e| format!("Could not list displays: {e}"))?;
    Ok(monitors
        .iter()
        .map(|monitor| {
            let primary = monitor.is_primary().unwrap_or(false);
            CaptureTarget {
                target: monitor.id().map(|id| id.to_string()).unwrap_or_default(),
                label: monitor
                    .name()
                    .unwrap_or_else(|_| "Display".to_string()),
                detail: format!(
                    "{} by {}{}",
                    monitor.width().unwrap_or(0),
                    monitor.height().unwrap_or(0),
                    if primary { ", primary" } else { "" }
                ),
            }
        })
        .collect())
}

fn window_targets() -> Result<Vec<CaptureTarget>, String> {
    let windows = Window::all().map_err(|e| format!("Could not list windows: {e}"))?;
    let mut targets: Vec<CaptureTarget> = windows
        .iter()
        .filter_map(|window| {
            let title = window.title().unwrap_or_default();
            if title.trim().is_empty() {
                return None;
            }
            Some(CaptureTarget {
                target: window.id().ok()?.to_string(),
                label: title,
                detail: format!(
                    "{}, {} by {}",
                    window
                        .app_name()
                        .unwrap_or_else(|_| "Window".to_string()),
                    window.width().unwrap_or(0),
                    window.height().unwrap_or(0)
                ),
            })
        })
        .collect();
    targets.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(targets)
}

#[tauri::command]
pub fn displays_list() -> Result<Vec<CaptureTarget>, String> {
    display_targets()
}

#[tauri::command]
pub fn windows_list() -> Result<Vec<CaptureTarget>, String> {
    window_targets()
}

#[tauri::command]
pub fn sources_list(app: AppHandle, project_id: i64) -> Result<Vec<SourceView>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select id, project_id, kind, label, target,
                    crop_x, crop_y, crop_w, crop_h
             from source where project_id = ?1 order by id",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id], row_to_source)
        .map_err(|e| e.to_string())?;
    let mut sources = Vec::new();
    for row in rows {
        sources.push(row.map_err(|e| e.to_string())?);
    }
    Ok(sources)
}

#[tauri::command]
pub fn source_add(
    app: AppHandle,
    project_id: i64,
    kind: String,
    target: String,
    label: Option<String>,
) -> Result<SourceView, String> {
    let kind = kind.trim().to_lowercase();
    if !matches!(kind.as_str(), "display" | "window" | "file" | "folder") {
        return Err("A source is a display, a window, a file or a folder".into());
    }
    let target = target.trim().to_string();
    if target.is_empty() {
        return Err("Pick something to read frames from".into());
    }

    if kind == "file" {
        let path = Path::new(&target);
        if !path.is_file() {
            return Err(format!("There is no file at {target}"));
        }
        if !is_image(path) {
            return Err("Only PNG and JPEG images can be read".into());
        }
    }
    if kind == "folder" {
        let path = Path::new(&target);
        if !path.is_dir() {
            return Err(format!("There is no folder at {target}"));
        }
    }

    let mut resolved = label.unwrap_or_default().trim().to_string();
    if resolved.is_empty() {
        resolved = match kind.as_str() {
            "file" | "folder" => Path::new(&target)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| target.clone()),
            "display" => display_targets()?
                .into_iter()
                .find(|entry| entry.target == target)
                .map(|entry| entry.label)
                .ok_or_else(|| "That display is no longer connected".to_string())?,
            _ => window_targets()?
                .into_iter()
                .find(|entry| entry.target == target)
                .map(|entry| entry.label)
                .ok_or_else(|| "That window is gone. Pick it again.".to_string())?,
        };
    }
    if resolved.chars().count() > 80 {
        return Err("Keep the source name under 80 characters".into());
    }

    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    if let Ok(existing) = conn.query_row(
        "select id, project_id, kind, label, target,
                crop_x, crop_y, crop_w, crop_h
         from source where project_id = ?1 and kind = ?2 and target = ?3",
        params![project_id, kind, target],
        row_to_source,
    ) {
        return Ok(existing);
    }

    conn.execute(
        "insert into source (project_id, kind, label, target, created_at)
         values (?1, ?2, ?3, ?4, datetime('now'))",
        params![project_id, kind, resolved, target],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    drop(conn);

    source_row(&app, id)
}

#[tauri::command]
pub fn source_crop(app: AppHandle, id: i64, crop: Option<Crop>) -> Result<SourceView, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    match crop {
        Some(crop) => {
            if !crop_is_valid(&crop) {
                return Err("The crop has to sit inside the frame".into());
            }
            conn.execute(
                "update source set crop_x = ?1, crop_y = ?2, crop_w = ?3, crop_h = ?4
                 where id = ?5",
                params![crop.x, crop.y, crop.w, crop.h, id],
            )
            .map_err(|e| e.to_string())?;
        }
        None => {
            conn.execute(
                "update source set crop_x = null, crop_y = null,
                        crop_w = null, crop_h = null
                 where id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    drop(conn);
    source_row(&app, id)
}

#[tauri::command]
pub fn source_delete(app: AppHandle, id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("delete from source where id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn source_preview(app: AppHandle, id: i64) -> Result<FramePayload, String> {
    let source = source_row(&app, id)?;
    let image = load_frame(&source)?;
    let (data, width, height) = preview(&image);
    Ok(FramePayload {
        data,
        width,
        height,
    })
}

/// Saves one full-resolution frame from a source into the project's library.
/// A window source is read through its own handle, so a covered game can be
/// labelled or evaluated without bringing it to the front.
#[tauri::command]
pub fn source_frame(app: AppHandle, project_id: i64, id: i64) -> Result<label::FrameView, String> {
    let source = source_row(&app, id)?;
    if source.project_id != project_id {
        return Err("That source belongs to another project".into());
    }
    let image = load_frame(&source)?;
    let (_slug, folder) = label::project_folder(&app, project_id)?;
    let path = folder
        .join("frames")
        .join(format!("{}.png", label::now_millis()));
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| format!("Could not save the frame: {error}"))?;

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
    let frame_id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row(
            "select created_at from frame where id = ?1",
            params![frame_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    Ok(label::FrameView {
        id: frame_id,
        path: path.to_string_lossy().to_string(),
        width: image.width(),
        height: image.height(),
        preview: label::preview(&image),
        created_at,
    })
}

fn first_image(folder: &Path) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(folder)
        .map_err(|error| format!("Could not read {}: {error}", folder.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_image(path))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .next()
        .ok_or_else(|| format!("{} has no PNG or JPEG images", folder.display()))
}

/// Every image a file or folder source can replay, in a stable order. The
/// sequence player uses this so a folder behaves like a recorded clip.
pub(crate) fn sequence_paths(kind: &str, target: &str) -> Result<Vec<PathBuf>, String> {
    let path = Path::new(target);
    if kind == "file" {
        if !path.is_file() {
            return Err(format!("There is no file at {target}"));
        }
        return Ok(vec![path.to_path_buf()]);
    }
    let entries =
        std::fs::read_dir(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|entry| entry.is_file() && is_image(entry))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{} has no PNG or JPEG images", path.display()));
    }
    Ok(paths)
}

pub(crate) fn load_frame(source: &SourceView) -> Result<RgbaImage, String> {
    match source.kind.as_str() {
        "display" => {
            let id: u32 = source
                .target
                .parse()
                .map_err(|_| "That display is no longer connected".to_string())?;
            let monitors = Monitor::all().map_err(|e| format!("Could not list displays: {e}"))?;
            let monitor = monitors
                .iter()
                .find(|monitor| monitor.id().map(|value| value == id).unwrap_or(false))
                .ok_or_else(|| "That display is no longer connected".to_string())?;
            monitor
                .capture_image()
                .map_err(|e| format!("Could not capture the display: {e}"))
        }
        "window" => {
            let id: u32 = source
                .target
                .parse()
                .map_err(|_| "That window is gone. Pick it again.".to_string())?;
            let windows = Window::all().map_err(|e| format!("Could not list windows: {e}"))?;
            let window = windows
                .iter()
                .find(|window| window.id().map(|value| value == id).unwrap_or(false))
                .ok_or_else(|| "That window is gone. Pick it again.".to_string())?;
            if window.is_minimized().unwrap_or(false) {
                return Err("That window is minimised, so there is nothing to read".into());
            }
            window
                .capture_image()
                .map_err(|e| format!("Could not capture the window: {e}"))
        }
        "file" => image::open(&source.target)
            .map(|image| image.to_rgba8())
            .map_err(|error| format!("Could not open that image: {error}")),
        _ => {
            let path = first_image(Path::new(&source.target))?;
            image::open(&path)
                .map(|image| image.to_rgba8())
                .map_err(|error| format!("Could not open {}: {error}", path.display()))
        }
    }
}

fn preview(image: &RgbaImage) -> (String, u32, u32) {
    if image.width() <= PREVIEW_WIDTH {
        return (label::png_data_url(image), image.width(), image.height());
    }
    let width = PREVIEW_WIDTH;
    let height = ((image.height() as f32 * width as f32 / image.width() as f32).round() as u32).max(1);
    let small = image::imageops::resize(
        image,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    (label::png_data_url(&small), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crop(x: f32, y: f32, w: f32, h: f32) -> Crop {
        Crop { x, y, w, h }
    }

    #[test]
    fn the_whole_frame_is_a_valid_crop() {
        assert!(crop_is_valid(&crop(0.0, 0.0, 1.0, 1.0)));
    }

    #[test]
    fn a_flipped_or_outside_crop_is_refused() {
        assert!(!crop_is_valid(&crop(0.2, 0.2, 0.0, 0.5)));
        assert!(!crop_is_valid(&crop(0.8, 0.2, 0.4, 0.5)));
        assert!(!crop_is_valid(&crop(-0.1, 0.0, 0.5, 0.5)));
    }

    #[test]
    fn only_png_and_jpeg_count_as_images() {
        assert!(is_image(Path::new("frame.PNG")));
        assert!(is_image(Path::new("frame.jpeg")));
        assert!(!is_image(Path::new("frame.webp")));
        assert!(!is_image(Path::new("frame")));
    }
}
