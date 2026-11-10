//! Candidate review: the uncertain proposals the capture loop saved.
//!
//! A candidate is a crop the matcher was unsure about, plus the class it nearly
//! matched and the score it got. Accepting writes a positive example, rejecting
//! writes a negative, and both take effect on the next pool reload.

use std::path::Path;

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::label::{self, ExampleView};
use crate::matcher;
use crate::AppState;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateView {
    pub id: i64,
    pub project_id: i64,
    pub class_id: i64,
    pub class_name: String,
    pub color: String,
    pub path: String,
    pub thumb: String,
    pub thumb_path: String,
    pub score: f32,
    pub state: String,
    pub created_at: String,
}

/// Candidate images are immutable once harvested, so their small queue thumb
/// and larger detail preview can be cached beside the project. The queue only
/// decodes an image the first time it is listed.
fn cached_image(root: &Path, id: i64, path: &str, kind: &str, width: u32, quality: u8) -> String {
    let thumbs = root.join("thumbs");
    let _ = std::fs::create_dir_all(&thumbs);
    let cache = thumbs.join(format!("candidate-{id}-{kind}.jpg"));
    if let Ok(bytes) = std::fs::read(&cache) {
        return label::jpeg_data_url(&bytes);
    }
    let bytes = image::open(path)
        .map(|image| label::jpeg_bytes(&image.to_rgba8(), width, quality))
        .unwrap_or_default();
    if !bytes.is_empty() {
        let _ = std::fs::write(&cache, &bytes);
    }
    label::jpeg_data_url(&bytes)
}

/// A saved candidate thumb if one exists, otherwise the crop itself. Both are
/// served through the asset protocol, so listing the queue never decodes an
/// image and a cold queue stays fast.
fn cached_thumb_path(root: &Path, id: i64, path: &str) -> String {
    let cache = root.join("thumbs").join(format!("candidate-{id}-thumb.jpg"));
    if cache.is_file() {
        cache.to_string_lossy().to_string()
    } else {
        path.to_string()
    }
}

#[tauri::command]
pub fn candidate_preview(app: AppHandle, id: i64) -> Result<String, String> {
    let (project_id, path) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "select project_id, path from candidate where id = ?1",
            params![id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|_| "That candidate is gone".to_string())?
    };
    let (_slug, root) = label::project_folder(&app, project_id)?;
    Ok(cached_image(&root, id, &path, "preview", 640, 78))
}

#[tauri::command]
pub fn candidates_list(
    app: AppHandle,
    project_id: i64,
    state: Option<String>,
    before_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<CandidateView>, String> {
    let state = state.unwrap_or_default();
    let state = if state.is_empty() { None } else { Some(state) };
    let limit = limit.unwrap_or(400).clamp(1, 400) as i64;

    let app_state = app.state::<AppState>();
    let conn = app_state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select k.id, k.project_id, k.class_id, c.name, c.color, k.path, k.score,
                    k.state, k.created_at
             from candidate k join class c on c.id = k.class_id
             where k.project_id = ?1 and (?2 is null or k.state = ?2)
               and (?3 is null or k.id < ?3)
             order by k.id desc limit ?4",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id, state, before_id, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, f32>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let rows = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    drop(conn);

    let (_slug, root) = label::project_folder(&app, project_id)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, project_id, class_id, class_name, color, path, score, state, created_at)| {
                CandidateView {
                    id,
                    project_id,
                    class_id,
                    class_name,
                    color,
                    thumb: String::new(),
                    thumb_path: cached_thumb_path(&root, id, &path),
                    path,
                    score,
                    state,
                    created_at,
                }
            },
        )
        .collect())
}

/// The examples nearest a candidate, for the evidence beside it on the review
/// screen. Ordered by similarity, best first, and capped by `limit`.
#[tauri::command]
pub fn candidate_nearest(
    app: AppHandle,
    id: i64,
    limit: Option<u32>,
) -> Result<Vec<ExampleView>, String> {
    let (project_id, class_id, path) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "select project_id, class_id, path from candidate where id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| "That candidate is gone".to_string())?
    };

    let image = image::open(&path)
        .map_err(|error| format!("Could not open the candidate: {error}"))?
        .to_rgba8();
    let candidate = matcher::embed(&image);

    let pool = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        matcher::load_pool(&conn, project_id)?
    };
    let nearest = pool.nearest(&candidate, class_id, limit.unwrap_or(3).max(1) as usize);
    if nearest.is_empty() {
        return Ok(Vec::new());
    }

    // Only the nearest examples are needed here. Loading the whole class would
    // reopen and base64 every example crop for each selected candidate.
    let rows: Vec<(i64, i64, String, String, String, String)> = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        for (example_id, _) in &nearest {
            if let Ok(row) = conn.query_row(
                "select id, class_id, path, coalesce(bbox, ''), role, created_at
                 from example where id = ?1",
                params![example_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            ) {
                rows.push(row);
            }
        }
        rows
    };

    let mut by_id: std::collections::HashMap<i64, ExampleView> = rows
        .into_iter()
        .map(|(id, class_id, path, bbox, role, created_at)| {
            let (width, height) = image::image_dimensions(&path).unwrap_or((0, 0));
            (
                id,
                ExampleView {
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
                },
            )
        })
        .collect();
    Ok(nearest
        .into_iter()
        .filter_map(|(example_id, _)| by_id.remove(&example_id))
        .collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoAcceptReport {
    pub accepted: usize,
    pub failed: usize,
}

/// Accepts every pending candidate at or above the chosen floor. Each accept
/// goes through the same copy-and-embed path as the review buttons, so the new
/// positives are immediately usable by the matcher.
#[tauri::command]
pub fn candidate_auto_accept(
    app: AppHandle,
    project_id: i64,
    floor: f32,
) -> Result<AutoAcceptReport, String> {
    let floor = floor.clamp(0.0, 1.0);
    let ids: Vec<i64> = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare(
                "select id from candidate
                 where project_id = ?1 and state = 'pending' and score >= ?2
                 order by id desc",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id, floor], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| e.to_string())?);
        }
        ids
    };

    let mut accepted = 0usize;
    let mut failed = 0usize;
    for id in ids {
        match candidate_resolve(app.clone(), id, "accept".to_string(), None) {
            Ok(()) => accepted += 1,
            Err(_) => failed += 1,
        }
    }
    Ok(AutoAcceptReport { accepted, failed })
}

#[tauri::command]
pub fn candidate_resolve(
    app: AppHandle,
    id: i64,
    decision: String,
    class_id: Option<i64>,
) -> Result<(), String> {
    let decision = decision.trim().to_lowercase();
    if !matches!(decision.as_str(), "accept" | "reject" | "skip") {
        return Err("A candidate is accepted, rejected or skipped".into());
    }

    let (project_id, current_class, path) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "select project_id, class_id, path from candidate where id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| "That candidate is gone".to_string())?
    };
    let target_class = class_id.unwrap_or(current_class);

    if decision == "skip" {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "update candidate set state = 'skipped' where id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let role = if decision == "accept" {
        "positive"
    } else {
        "negative"
    };
    let (_slug, root) = label::project_folder(&app, project_id)?;
    let class_name: String = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "select name from class where id = ?1",
            params![target_class],
            |row| row.get(0),
        )
        .map_err(|_| "That class is gone".to_string())?
    };

    let folder = root
        .join("examples")
        .join(crate::db::slugify(&class_name));
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let destination = folder.join(format!("harvest-{}.png", label::now_millis()));
    std::fs::copy(&path, &destination)
        .map_err(|error| format!("Could not keep the candidate crop: {error}"))?;

    let image = image::open(&destination)
        .map_err(|error| format!("Could not read the candidate crop: {error}"))?
        .to_rgba8();
    let embedding = matcher::embed(&image);

    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "insert into example
           (class_id, frame_id, path, role, origin, bbox, embedding, embedding_model, created_at)
         values (?1, null, ?2, ?3, 'harvest', null, ?4, ?5, datetime('now'))",
        params![
            target_class,
            destination.to_string_lossy().to_string(),
            role,
            matcher::to_blob(&embedding),
            matcher::EMBEDDING_MODEL
        ],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "update candidate set state = ?1, class_id = ?2 where id = ?3",
        params![
            if decision == "accept" {
                "accepted"
            } else {
                "rejected"
            },
            target_class,
            id
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReport {
    pub deleted: usize,
}

/// Removes candidate rows and the crops that belong to them. Accepted
/// candidates have already been copied into the class folder, so deleting the
/// candidate leaves that example in place.
fn delete_candidates(app: &AppHandle, ids: Vec<i64>) -> Result<usize, String> {
    let mut removed = 0usize;
    for id in ids {
        let row = {
            let state = app.state::<AppState>();
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            conn.query_row(
                "select project_id, path from candidate where id = ?1",
                params![id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .ok()
        };
        let Some((project_id, path)) = row else {
            continue;
        };
        {
            let state = app.state::<AppState>();
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            conn.execute("delete from candidate where id = ?1", params![id])
                .map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_file(&path);
        if let Ok((_slug, root)) = label::project_folder(app, project_id) {
            let thumbs = root.join("thumbs");
            let _ = std::fs::remove_file(thumbs.join(format!("candidate-{id}-thumb.jpg")));
            let _ = std::fs::remove_file(thumbs.join(format!("candidate-{id}-preview.jpg")));
        }
        removed += 1;
    }
    Ok(removed)
}

#[tauri::command]
pub fn candidate_delete(app: AppHandle, id: i64) -> Result<(), String> {
    delete_candidates(&app, vec![id])?;
    Ok(())
}

/// Empties the review queue for a project. Only candidate rows and their crops
/// go; examples created by accepting a candidate are left alone.
#[tauri::command]
pub fn candidate_delete_all(app: AppHandle, project_id: i64) -> Result<DeleteReport, String> {
    let ids: Vec<i64> = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare("select id from candidate where project_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| e.to_string())?);
        }
        ids
    };
    let deleted = delete_candidates(&app, ids)?;
    Ok(DeleteReport { deleted })
}
