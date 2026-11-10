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
    let mut preview = String::new();
    if !thumb.is_file() {
        if let Ok(image) = image::open(&path) {
            let bytes = label::jpeg_bytes(
                &image.to_rgba8(),
                label::THUMB_WIDTH,
                label::THUMB_QUALITY,
            );
            if !bytes.is_empty() {
                let _ = std::fs::write(&thumb, &bytes);
            }
            if !thumb.is_file() {
                // The write failed, so this row has to carry its own pixels.
                preview = label::jpeg_data_url(&bytes);
            }
        }
    }
    FrameView {
        id,
        path,
        width,
        height,
        preview,
        thumb_path: if thumb.is_file() {
            thumb.to_string_lossy().to_string()
        } else {
            String::new()
        },
        created_at,
        examples,
    }
}

type FrameRow = (i64, String, u32, u32, String, i64);

/// Newest first, optionally only frames newer than `after_id`. The limit keeps
/// a large import from turning one list call into tens of megabytes of base64.
fn frame_rows(
    app: &AppHandle,
    project_id: i64,
    after_id: i64,
    limit: i64,
) -> Result<Vec<FrameView>, String> {
    let state = app.state::<AppState>();
    let rows: Vec<FrameRow> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare(
                "select f.id, f.path, f.width, f.height, f.created_at,
                        (select count(*) from example e where e.frame_id = f.id)
                 from frame f
                 where f.project_id = ?1 and f.id > ?2
                 order by f.id desc limit ?3",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id, after_id, limit], |row| {
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

    let (_slug, root) = label::project_folder(app, project_id)?;
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
pub fn library_list(
    app: AppHandle,
    project_id: i64,
    limit: Option<u32>,
) -> Result<Vec<FrameView>, String> {
    frame_rows(&app, project_id, 0, limit.map(i64::from).unwrap_or(-1))
}

/// Only frames newer than `after_id`, for a grid that follows an import without
/// rebuilding every preview on each tick.
#[tauri::command]
pub fn library_list_since(
    app: AppHandle,
    project_id: i64,
    after_id: i64,
    limit: Option<u32>,
) -> Result<Vec<FrameView>, String> {
    let limit = limit.unwrap_or(60).min(200);
    frame_rows(&app, project_id, after_id, i64::from(limit))
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

/// Deletes the original screenshot for every frame that already has at least
/// one saved example. The examples are crops with their own files, so they stay.
#[tauri::command]
pub fn library_delete_labeled(app: AppHandle, project_id: i64) -> Result<usize, String> {
    let frames: Vec<(i64, String)> = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare(
                "select f.id, f.path from frame f
                 where f.project_id = ?1
                   and exists (select 1 from example e where e.frame_id = f.id)
                 order by f.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut frames = Vec::new();
        for row in rows {
            frames.push(row.map_err(|e| e.to_string())?);
        }
        frames
    };
    if frames.is_empty() {
        return Ok(0);
    }

    let (_slug, root) = label::project_folder(&app, project_id)?;
    let thumbs = root.join("thumbs");
    {
        let state = app.state::<AppState>();
        let mut conn = state.db.lock().map_err(|e| e.to_string())?;
        let transaction = conn.transaction().map_err(|e| e.to_string())?;
        for (id, _) in &frames {
            // The frame's pixels are gone after this, so leave the example
            // rows with their crops and clear the dangling frame reference.
            transaction
                .execute(
                    "update example set frame_id = null where frame_id = ?1",
                    params![id],
                )
                .map_err(|e| e.to_string())?;
            transaction
                .execute("delete from frame_truth where frame_id = ?1", params![id])
                .map_err(|e| e.to_string())?;
            transaction
                .execute("delete from frame where id = ?1", params![id])
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
    }

    for (id, path) in &frames {
        if Path::new(path).starts_with(&root) {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_file(thumbs.join(format!("{id}.jpg")));
    }
    Ok(frames.len())
}

/// Removes one screenshot that has not been labeled yet. The Label page uses
/// this as its "do not label this image" choice.
#[tauri::command]
pub fn library_frame_delete(app: AppHandle, frame_id: i64) -> Result<(), String> {
    let (project_id, path) = {
        let state = app.state::<AppState>();
        let mut conn = state.db.lock().map_err(|e| e.to_string())?;
        let transaction = conn.transaction().map_err(|e| e.to_string())?;
        let (project_id, path, examples): (i64, String, i64) = transaction
            .query_row(
                "select f.project_id, f.path,
                        (select count(*) from example e where e.frame_id = f.id)
                 from frame f where f.id = ?1",
                params![frame_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "That frame is gone".to_string())?;
        if examples > 0 {
            return Err(
                "This screenshot has saved labels. Delete labeled screenshots from Library."
                    .into(),
            );
        }
        transaction
            .execute("delete from frame_truth where frame_id = ?1", params![frame_id])
            .map_err(|e| e.to_string())?;
        transaction
            .execute("delete from frame where id = ?1", params![frame_id])
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        (project_id, path)
    };

    let (_slug, root) = label::project_folder(&app, project_id)?;
    if Path::new(&path).starts_with(&root) {
        let _ = std::fs::remove_file(&path);
    }
    let _ = std::fs::remove_file(root.join("thumbs").join(format!("{frame_id}.jpg")));
    Ok(())
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

/// Projects imported before the smaller thumbnail size still hold 640 px
/// files. Re-encode them once, after the window opens, so the migration costs
/// one background pass instead of blocking startup. The marker keeps later
/// launches from rescanning every file.
pub fn migrate_thumbnails(app: &AppHandle) {
    let Ok(data) = app.path().app_data_dir() else {
        return;
    };
    let projects = data.join("projects");
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let Ok(entries) = std::fs::read_dir(&projects) else {
            return;
        };
        for entry in entries.flatten() {
            let root = entry.path();
            let marker = root.join(".thumbs-320");
            if marker.is_file() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(root.join("thumbs")) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|value| value.to_str()) != Some("jpg") {
                    continue;
                }
                if path
                    .file_name()
                    .map(|name| name.to_string_lossy().starts_with("candidate-"))
                    .unwrap_or(false)
                {
                    continue;
                }
                let Ok((width, _)) = image::image_dimensions(&path) else {
                    continue;
                };
                if width <= label::THUMB_WIDTH {
                    continue;
                }
                let Ok(image) = image::open(&path) else {
                    continue;
                };
                let bytes = label::jpeg_bytes(
                    &image.to_rgba8(),
                    label::THUMB_WIDTH,
                    label::THUMB_QUALITY,
                );
                if !bytes.is_empty() {
                    let _ = std::fs::write(&path, bytes);
                }
            }
            let _ = std::fs::write(&marker, b"320\n");
        }
    });
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
