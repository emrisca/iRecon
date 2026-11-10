//! The capture library: every frame a project can label.
//!
//! A grabbed frame and an imported image are the same row, so the Label page does
//! not care which way a frame arrived. Imports are copied into the project's
//! `frames/` folder, which keeps a project a folder a user can move or delete.

use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::label::{self, FrameView};
use crate::source::is_image;
use crate::AppState;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// Thumbnails live beside the frames. Without them every library or evaluation
/// page decodes each full-size PNG again, which takes over a second per frame.
fn frame_row_to_view(
    id: i64,
    path: String,
    width: u32,
    height: u32,
    created_at: String,
    examples: i64,
    thumbs: &Path,
) -> FrameView {
    let thumb = thumbs.join(format!("{id}.jpg"));
    let preview = if let Ok(bytes) = std::fs::read(&thumb) {
        label::jpeg_data_url(&bytes)
    } else {
        match image::open(&path) {
            Ok(image) => {
                let bytes = label::jpeg_bytes(&image.to_rgba8(), 1280, 70);
                if !bytes.is_empty() {
                    let _ = std::fs::write(&thumb, &bytes);
                }
                label::jpeg_data_url(&bytes)
            }
            Err(_) => String::new(),
        }
    };
    FrameView {
        id,
        path,
        width,
        height,
        preview,
        created_at,
        examples,
    }
}

#[tauri::command]
pub fn library_list(app: AppHandle, project_id: i64) -> Result<Vec<FrameView>, String> {
    let state = app.state::<AppState>();
    let rows: Vec<(i64, String, u32, u32, String, i64)> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare(
                "select f.id, f.path, f.width, f.height, f.created_at,
                        (select count(*) from example e where e.frame_id = f.id)
                 from frame f
                 where f.project_id = ?1 order by f.id desc",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut frames = Vec::new();
        for row in rows {
            frames.push(row.map_err(|e| e.to_string())?);
        }
        frames
    };

    let (_slug, root) = label::project_folder(&app, project_id)?;
    let thumbs = root.join("thumbs");
    std::fs::create_dir_all(&thumbs).map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(id, path, width, height, created_at, examples)| {
            frame_row_to_view(id, path, width, height, created_at, examples, &thumbs)
        })
        .collect())
}

#[tauri::command]
pub fn library_frame(app: AppHandle, frame_id: i64) -> Result<FrameView, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let (id, path, width, height, created_at, project_id, examples) = conn
        .query_row(
            "select f.id, f.path, f.width, f.height, f.created_at, f.project_id,
                    (select count(*) from example e where e.frame_id = f.id)
             from frame f where f.id = ?1",
            params![frame_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|_| "That frame is gone".to_string())?;
    drop(conn);
    let (_slug, root) = label::project_folder(&app, project_id)?;
    let thumbs = root.join("thumbs");
    std::fs::create_dir_all(&thumbs).map_err(|e| e.to_string())?;
    Ok(frame_row_to_view(
        id,
        path,
        width,
        height,
        created_at,
        examples,
        &thumbs,
    ))
}

fn import_paths(
    app: &AppHandle,
    project_id: i64,
    paths: Vec<PathBuf>,
) -> Result<ImportReport, String> {
    let (_slug, root) = label::project_folder(app, project_id)?;
    let frames = root.join("frames");
    std::fs::create_dir_all(&frames).map_err(|e| e.to_string())?;

    let mut report = ImportReport {
        imported: 0,
        skipped: 0,
        errors: Vec::new(),
    };

    for (index, path) in paths.iter().enumerate() {
        if !path.is_file() || !is_image(path) {
            report.skipped += 1;
            report
                .errors
                .push(format!("{} is not a PNG or JPEG image", path.display()));
            continue;
        }
        let image = match image::open(path) {
            Ok(image) => image.to_rgba8(),
            Err(error) => {
                report.skipped += 1;
                report
                    .errors
                    .push(format!("{} could not be read: {error}", path.display()));
                continue;
            }
        };
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let destination = frames.join(format!("import-{}-{index}.{extension}", label::now_millis()));
        if let Err(error) = std::fs::copy(path, &destination) {
            report.skipped += 1;
            report.errors.push(format!(
                "{} could not be copied into the project: {error}",
                path.display()
            ));
            continue;
        }

        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "insert into frame (project_id, path, width, height, created_at)
             values (?1, ?2, ?3, ?4, datetime('now'))",
            params![
                project_id,
                destination.to_string_lossy().to_string(),
                image.width() as i64,
                image.height() as i64
            ],
        )
        .map_err(|e| e.to_string())?;
        report.imported += 1;
    }

    Ok(report)
}

#[tauri::command]
pub fn library_import(
    app: AppHandle,
    project_id: i64,
    paths: Vec<String>,
) -> Result<ImportReport, String> {
    if paths.is_empty() {
        return Err("Give the library at least one image path".into());
    }
    import_paths(
        &app,
        project_id,
        paths.into_iter().map(PathBuf::from).collect(),
    )
}

#[tauri::command]
pub fn library_import_folder(
    app: AppHandle,
    project_id: i64,
    folder: String,
) -> Result<ImportReport, String> {
    let entries = std::fs::read_dir(&folder)
        .map_err(|error| format!("Could not read {folder}: {error}"))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_image(path))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(format!("{folder} has no PNG or JPEG images"));
    }
    import_paths(&app, project_id, paths)
}
