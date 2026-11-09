//! Recorded capture sessions. A recording keeps a downsampled frame plus the
//! detector proposals at a fixed interval, so the current matcher pool can be
//! replayed over the same frames without launching the game again.

use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::capture::{crop_region, EXPANSION_SCORE_THRESHOLD, SCORE_THRESHOLD};
use crate::detector::Detection;
use crate::matcher;
use crate::AppState;

const SESSION_BUDGET_BYTES: u64 = 1_000_000_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: i64,
    pub project_id: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: i64,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub detections: u32,
    pub model: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFrameView {
    pub id: i64,
    pub session_id: i64,
    pub index: i64,
    pub at_ms: i64,
    pub width: u32,
    pub height: u32,
    pub boxes: Vec<Detection>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvalFrame {
    pub frame_id: i64,
    pub boxes: Vec<Detection>,
}

pub struct SessionStart {
    pub id: i64,
    pub root: PathBuf,
}

pub fn start(app: &AppHandle, project_id: i64) -> Result<SessionStart, String> {
    let (_slug, project_root) = crate::label::project_folder(app, project_id)?;
    let state = app.state::<AppState>();
    let model = state
        .loaded_model
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    let id = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "insert into capture_session (project_id, started_at, model)
             values (?1, datetime('now'), ?2)",
            params![project_id, model],
        )
        .map_err(|e| e.to_string())?;
        conn.last_insert_rowid()
    };
    let root = project_root.join("sessions").join(id.to_string());
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(SessionStart { id, root })
}

pub fn finish(app: &AppHandle, id: i64) -> Result<(), String> {
    let project_id = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "update capture_session set ended_at = datetime('now')
             where id = ?1 and ended_at is null",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        conn.query_row(
            "select project_id from capture_session where id = ?1",
            params![id],
            |row| row.get::<_, i64>(0),
        )
        .ok()
    };
    if let Some(project_id) = project_id {
        let _ = prune(app, project_id);
    }
    Ok(())
}

fn size_of(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                size_of(&path)
            } else {
                entry.metadata().map(|meta| meta.len()).unwrap_or(0)
            }
        })
        .sum()
}

/// Keeps the newest recording even if it alone exceeds the budget, then removes
/// oldest recordings until the project's session folder fits in 1 GB.
pub fn prune(app: &AppHandle, project_id: i64) -> Result<(), String> {
    let (root, mut sessions) = {
        let (_slug, root) = crate::label::project_folder(app, project_id)?;
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare("select id from capture_session where project_id = ?1 order by id")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![project_id], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        let ids = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        (root, ids)
    };
    let mut total: u64 = sessions
        .iter()
        .map(|id| size_of(&root.join("sessions").join(id.to_string())))
        .sum();
    while total > SESSION_BUDGET_BYTES && sessions.len() > 1 {
        let oldest = sessions.remove(0);
        let folder = root.join("sessions").join(oldest.to_string());
        let bytes = size_of(&folder);
        let _ = std::fs::remove_dir_all(&folder);
        let state = app.state::<AppState>();
        if let Ok(conn) = state.db.lock() {
            let _ = conn.execute("delete from capture_session where id = ?1", params![oldest]);
        }
        total = total.saturating_sub(bytes);
    }
    Ok(())
}

/// Writes one already-downsampled frame and its detection snapshots. The
/// proposal list is kept separately from the matched boxes so a later run can
/// score the same proposals against the current pool. The live loop holds a raw
/// RGBA buffer rather than an `RgbaImage`, so the recorder takes the buffer
/// directly and avoids a full frame copy per write.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_raw(
    app: &AppHandle,
    id: i64,
    root: &Path,
    index: i64,
    at_ms: i64,
    raw: &[u8],
    width: u32,
    height: u32,
    proposals: &[Detection],
    boxes: &[crate::tracker::TrackedBox],
) -> bool {
    let (bytes, width, height) = crate::capture::jpeg_preview(raw, width, height);
    if bytes.is_empty() {
        return false;
    }
    let path = root.join(format!("frame-{index:05}.jpg"));
    if std::fs::write(&path, &bytes).is_err() {
        return false;
    }
    let boxes_json = match serde_json::to_string(boxes) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let proposals_json = match serde_json::to_string(proposals) {
        Ok(value) => value,
        Err(_) => return false,
    };

    let state = app.state::<AppState>();
    let Ok(conn) = state.db.lock() else {
        return false;
    };
    let inserted = conn.execute(
        "insert or replace into session_frame
         (session_id, frame_index, path, width, height, at_ms, boxes, proposals)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            index,
            path.to_string_lossy().to_string(),
            width as i64,
            height as i64,
            at_ms,
            boxes_json,
            proposals_json
        ],
    );
    if inserted.is_err() {
        return false;
    }
    let _ = conn.execute(
        "update capture_session
         set frames = ?1, detections = coalesce(detections, 0) + ?2,
             width = ?3, height = ?4
         where id = ?5",
        params![index + 1, boxes.len() as i64, width as i64, height as i64, id],
    );
    true
}

#[tauri::command]
pub fn session_list(app: AppHandle, project_id: i64) -> Result<Vec<SessionView>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select id, project_id, started_at, ended_at,
                    cast((julianday(coalesce(ended_at, datetime('now'))) - julianday(started_at)) * 86400000 as integer),
                    width, height, frames, detections, model
             from capture_session where project_id = ?1 order by id desc",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok(SessionView {
                id: row.get(0)?,
                project_id: row.get(1)?,
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                duration_ms: row.get(4)?,
                width: row.get(5)?,
                height: row.get(6)?,
                frames: row.get(7)?,
                detections: row.get(8)?,
                model: row.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn session_frames(app: AppHandle, session_id: i64) -> Result<Vec<SessionFrameView>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select id, session_id, frame_index, at_ms, width, height, boxes
             from session_frame where session_id = ?1 order by frame_index",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![session_id], |row| {
            let boxes: String = row.get(6)?;
            Ok(SessionFrameView {
                id: row.get(0)?,
                session_id: row.get(1)?,
                index: row.get(2)?,
                at_ms: row.get(3)?,
                width: row.get(4)?,
                height: row.get(5)?,
                boxes: serde_json::from_str(&boxes).unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn session_frame_image(app: AppHandle, frame_id: i64) -> Result<String, String> {
    let path: String = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "select path from session_frame where id = ?1",
            params![frame_id],
            |row| row.get(0),
        )
        .map_err(|_| "That recorded frame is gone".to_string())?
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("Could not read the frame: {e}"))?;
    Ok(crate::label::jpeg_data_url(&bytes))
}

#[tauri::command]
pub fn session_evaluate(
    app: AppHandle,
    session_id: i64,
) -> Result<Vec<SessionEvalFrame>, String> {
    let (project_id, frames) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let project_id: i64 = conn
            .query_row(
                "select project_id from capture_session where id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(|_| "That session is gone".to_string())?;
        let mut statement = conn
            .prepare(
                "select id, path, proposals from session_frame
                 where session_id = ?1 order by frame_index",
            )
            .map_err(|e| e.to_string())?;
        let frames = statement
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        (project_id, frames)
    };

    let pool = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        matcher::load_pool(&conn, project_id)?
    };
    let mut output = Vec::with_capacity(frames.len());
    for (frame_id, path, raw_proposals) in frames {
        let proposals: Vec<Detection> = serde_json::from_str(&raw_proposals).unwrap_or_default();
        let image = match image::open(&path) {
            Ok(image) => image.to_rgba8(),
            Err(_) => continue,
        };
        let mut scored = Vec::new();
        let mut primary = Vec::new();
        let mut secondary = Vec::new();
        for detection in proposals {
            if detection.w * detection.h >= matcher::MAX_PROPOSAL_AREA {
                continue;
            }
            if detection.score >= SCORE_THRESHOLD {
                primary.push(detection);
            } else {
                secondary.push(detection);
            }
        }
        for detection in primary {
            let Some(crop) = crop_region(image.as_raw(), image.width(), image.height(), &detection)
            else {
                continue;
            };
            let candidate = matcher::embed(&crop);
            if let Some(hit) = pool.best(&candidate) {
                scored.push((detection, hit));
            }
        }
        if scored
            .iter()
            .any(|(_, hit)| hit.score >= hit.threshold)
        {
            for detection in secondary {
                if !scored.iter().any(|(seed, hit)| {
                    hit.score >= hit.threshold && matcher::encloses(&detection, seed)
                }) {
                    continue;
                }
                let Some(crop) = crop_region(image.as_raw(), image.width(), image.height(), &detection)
                else {
                    continue;
                };
                let candidate = matcher::embed(&crop);
                if let Some(hit) = pool.best(&candidate) {
                    if hit.score >= pool.floor {
                        scored.push((detection, hit));
                    }
                }
            }
        }
        let boxes = matcher::refine(&scored, SCORE_THRESHOLD, EXPANSION_SCORE_THRESHOLD)
            .into_iter()
            .map(|(mut detection, hit)| {
                detection.label = hit.name;
                detection.score = hit.score;
                detection
            })
            .collect();
        output.push(SessionEvalFrame { frame_id, boxes });
    }
    Ok(output)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFrame {
    pub frame_id: i64,
    pub index: i64,
    pub proposals: u32,
    pub primary: u32,
    pub vehicles: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAudit {
    pub session_id: i64,
    pub model: String,
    pub frames: u32,
    pub proposals: u32,
    pub primary: u32,
    pub vehicles: u32,
    pub average_ms: f32,
    pub rows: Vec<AuditFrame>,
}

/// Phase 0 question B in tool form: run the loaded detector over a recorded
/// session and count what it proposes, per frame. The frames are the recorded
/// previews rather than full-resolution captures, so treat the count as a
/// pessimistic reading. How many visible cars the boxes actually cover is still
/// a human judgement; this only removes the manual proposal counting.
#[tauri::command]
pub fn session_audit(app: AppHandle, session_id: i64) -> Result<SessionAudit, String> {
    let frames: Vec<(i64, i64, String)> = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare(
                "select id, frame_index, path from session_frame
                 where session_id = ?1 order by frame_index",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    if frames.is_empty() {
        return Err("Record at least one frame before auditing".into());
    }

    let state = app.state::<AppState>();
    let model = state
        .loaded_model
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| "loaded model".to_string());
    let mut guard = state.detector.lock().map_err(|e| e.to_string())?;
    let detector = guard
        .as_mut()
        .ok_or("Load a detector before auditing a session")?;

    let mut rows = Vec::with_capacity(frames.len());
    let mut proposals = 0u32;
    let mut primary = 0u32;
    let mut vehicles = 0u32;
    let mut total_ms = 0f32;
    for (frame_id, index, path) in frames {
        let image = match image::open(&path) {
            Ok(image) => image.to_rgba8(),
            Err(_) => continue,
        };
        let (boxes, run_ms) = detector.analyse(
            image.as_raw(),
            image.width(),
            image.height(),
            EXPANSION_SCORE_THRESHOLD,
        )?;
        let frame_primary = boxes
            .iter()
            .filter(|detection| detection.score >= SCORE_THRESHOLD)
            .count() as u32;
        let frame_vehicles = boxes
            .iter()
            .filter(|detection| {
                matches!(
                    detection.label.as_str(),
                    "car" | "truck" | "bus" | "motorcycle"
                )
            })
            .count() as u32;
        proposals += boxes.len() as u32;
        primary += frame_primary;
        vehicles += frame_vehicles;
        total_ms += run_ms;
        rows.push(AuditFrame {
            frame_id,
            index,
            proposals: boxes.len() as u32,
            primary: frame_primary,
            vehicles: frame_vehicles,
        });
    }

    let count = rows.len().max(1) as f32;
    Ok(SessionAudit {
        session_id,
        model,
        frames: rows.len() as u32,
        proposals,
        primary,
        vehicles,
        average_ms: total_ms / count,
        rows,
    })
}

#[tauri::command]
pub fn session_delete(app: AppHandle, id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    // The slug is read under the same lock as the delete, so this command
    // never calls project_folder, which locks the same mutex and deadlocked
    // the app when a session was deleted.
    let slug: Option<String> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let slug: Option<String> = conn
            .query_row(
                "select p.slug from capture_session s
                 join project p on p.id = s.project_id
                 where s.id = ?1",
                params![id],
                |row| row.get(0),
            )
            .ok();
        conn.execute("delete from capture_session where id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        slug
    };
    if let Some(slug) = slug {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("projects")
            .join(slug)
            .join("sessions")
            .join(id.to_string());
        let _ = std::fs::remove_dir_all(root);
    }
    Ok(())
}
