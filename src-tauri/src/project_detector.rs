//! Project-trained detector import and A/B evaluation. The imported graph must
//! satisfy the same input/output contract as the shipped RF-DETR models and is
//! warmed with a blank tensor before it is stored.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::capture::{crop_region, EXPANSION_SCORE_THRESHOLD, SCORE_THRESHOLD};
use crate::detector::{Detection, Detector};
use crate::matcher;
use crate::AppState;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetectorView {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub path: String,
    pub input_size: u32,
    pub active: bool,
    pub created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonSide {
    pub name: String,
    pub input_size: u32,
    pub frames: usize,
    pub proposals: usize,
    pub matches: usize,
    pub average_ms: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonFrame {
    pub frame_id: i64,
    pub index: i64,
    pub shipped_proposals: usize,
    pub shipped_matches: usize,
    pub shipped_ms: f32,
    pub project_proposals: usize,
    pub project_matches: usize,
    pub project_ms: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectorComparison {
    pub shipped: ComparisonSide,
    pub project: ComparisonSide,
    pub frames: Vec<ComparisonFrame>,
}

#[tauri::command]
pub fn detector_import(
    app: AppHandle,
    project_id: i64,
    path: String,
    name: Option<String>,
) -> Result<ProjectDetectorView, String> {
    let source = Path::new(path.trim());
    if !source.is_file() {
        return Err(format!("There is no ONNX file at {}", source.display()));
    }
    if source
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("onnx"))
        != Some(true)
    {
        return Err("Pick a .onnx model file".into());
    }

    let detector = Detector::load_imported(source)?;
    let input_size = detector.input_size();
    let detail = detector.describe();
    drop(detector);

    let mut name = name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        name = source
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "Project detector".to_string());
    }
    if name.chars().count() > 80 {
        return Err("Keep the detector name under 80 characters".into());
    }

    let (_slug, root) = crate::label::project_folder(&app, project_id)?;
    let models = root.join("models");
    std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    let destination = models.join(format!("detector-{}.onnx", crate::label::now_millis()));
    std::fs::copy(source, &destination).map_err(|e| format!("Could not copy the model: {e}"))?;

    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let inserted = conn.execute(
        "insert into project_detector (project_id, name, path, input_size, created_at)
         values (?1, ?2, ?3, ?4, datetime('now'))",
        params![
            project_id,
            name,
            destination.to_string_lossy().to_string(),
            input_size as i64
        ],
    );
    if inserted.is_err() {
        let _ = std::fs::remove_file(&destination);
        return Err("Could not save the imported detector".into());
    }
    let id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row(
            "select created_at from project_detector where id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    drop(conn);

    let _ = detail;
    Ok(ProjectDetectorView {
        id,
        project_id,
        name,
        path: destination.to_string_lossy().to_string(),
        input_size,
        active: false,
        created_at,
    })
}

#[tauri::command]
pub fn detector_list(app: AppHandle, project_id: i64) -> Result<Vec<ProjectDetectorView>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select id, project_id, name, path, input_size, active, created_at
             from project_detector where project_id = ?1 order by id desc",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok(ProjectDetectorView {
                id: row.get(0)?,
                project_id: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                input_size: row.get(4)?,
                active: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

pub(crate) fn active_model(
    app: &AppHandle,
    project_id: i64,
) -> Option<(String, u32, String)> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().ok()?;
    conn.query_row(
        "select path, input_size, name from project_detector
         where project_id = ?1 and active = 1 limit 1",
        params![project_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .ok()
}

#[tauri::command]
pub fn detector_activate(
    app: AppHandle,
    project_id: i64,
    id: i64,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.capture.lock().map_err(|e| e.to_string())?.is_some() {
        return Err("Stop capture before changing the live detector".into());
    }
    let (path, input_size, name) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "select path, input_size, name from project_detector
             where id = ?1 and project_id = ?2",
            params![id, project_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| "That detector is not in this project".to_string())?
    };
    let detector = Detector::load(&PathBuf::from(&path), input_size)?;
    {
        let mut guard = state.detector.lock().map_err(|e| e.to_string())?;
        *guard = Some(detector);
    }
    if let Ok(mut guard) = state.loaded_model.lock() {
        *guard = Some(format!("project:{name}"));
    }
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "update project_detector set active = 0 where project_id = ?1",
        params![project_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "update project_detector set active = 1 where id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn detector_deactivate(app: AppHandle, project_id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.capture.lock().map_err(|e| e.to_string())?.is_some() {
        return Err("Stop capture before changing the live detector".into());
    }
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "update project_detector set active = 0 where project_id = ?1",
            params![project_id],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(entry) = crate::model::preferred(&app) {
        let _ = crate::model::load_now(&app, entry);
    }
    Ok(())
}

#[tauri::command]
pub fn detector_delete(app: AppHandle, id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let path: Option<String> = conn
        .query_row(
            "select path from project_detector where id = ?1",
            params![id],
            |row| row.get(0),
        )
        .ok();
    conn.execute("delete from project_detector where id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    drop(conn);
    if let Some(path) = path {
        let _ = std::fs::remove_file(PathBuf::from(path));
    }
    Ok(())
}

fn matched_boxes(image: &RgbaImage, proposals: Vec<Detection>, pool: &matcher::Pool) -> Vec<Detection> {
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

    let mut scored = Vec::new();
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
                scored.push((detection, hit));
            }
        }
    }

    matcher::refine(&scored, SCORE_THRESHOLD, EXPANSION_SCORE_THRESHOLD)
        .into_iter()
        .map(|(mut detection, hit)| {
            detection.label = hit.name;
            detection.score = hit.score;
            detection
        })
        .collect()
}

#[tauri::command]
pub fn detector_compare(
    app: AppHandle,
    project_id: i64,
    detector_id: i64,
    session_id: i64,
) -> Result<DetectorComparison, String> {
    let (project_name, project_path, input_size) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "select name, path, input_size from project_detector
             where id = ?1 and project_id = ?2",
            params![detector_id, project_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            },
        )
        .map_err(|_| "That project detector is not in this project".to_string())?
    };

    let frames: Vec<(i64, i64, String)> = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare(
                "select sf.id, sf.frame_index, sf.path
                 from session_frame sf
                 join capture_session cs on cs.id = sf.session_id
                 where cs.id = ?1 and cs.project_id = ?2
                 order by sf.frame_index",
            )
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![session_id, project_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let frames = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        frames
    };
    if frames.is_empty() {
        return Err("Record at least one frame before comparing detectors".into());
    }

    let pool = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        matcher::load_pool(&conn, project_id)?
    };
    let mut project_detector = Detector::load(&PathBuf::from(&project_path), input_size)?;
    let state = app.state::<AppState>();
    let mut shipped_guard = state.detector.lock().map_err(|e| e.to_string())?;
    let shipped = shipped_guard
        .as_mut()
        .ok_or("Load a shipped detector before comparing")?;

    let mut output = Vec::with_capacity(frames.len());
    let mut shipped_proposals = 0;
    let mut shipped_matches = 0;
    let mut shipped_ms = 0f32;
    let mut project_proposals = 0;
    let mut project_matches = 0;
    let mut project_ms = 0f32;

    for (frame_id, index, path) in frames {
        let image = image::open(&path)
            .map_err(|error| format!("Could not open frame {frame_id}: {error}"))?
            .to_rgba8();
        let (shipped_boxes, shipped_elapsed) = shipped.analyse(
            image.as_raw(),
            image.width(),
            image.height(),
            EXPANSION_SCORE_THRESHOLD,
        )?;
        let (project_boxes, project_elapsed) = project_detector.analyse(
            image.as_raw(),
            image.width(),
            image.height(),
            EXPANSION_SCORE_THRESHOLD,
        )?;
        let shipped_hits = matched_boxes(&image, shipped_boxes.clone(), &pool);
        let project_hits = matched_boxes(&image, project_boxes.clone(), &pool);
        shipped_proposals += shipped_boxes.len();
        project_proposals += project_boxes.len();
        shipped_matches += shipped_hits.len();
        project_matches += project_hits.len();
        shipped_ms += shipped_elapsed;
        project_ms += project_elapsed;
        output.push(ComparisonFrame {
            frame_id,
            index,
            shipped_proposals: shipped_boxes.len(),
            shipped_matches: shipped_hits.len(),
            shipped_ms: shipped_elapsed,
            project_proposals: project_boxes.len(),
            project_matches: project_hits.len(),
            project_ms: project_elapsed,
        });
    }

    let count = output.len().max(1) as f32;
    let shipped_name = state
        .loaded_model
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| "shipped model".to_string());
    Ok(DetectorComparison {
        shipped: ComparisonSide {
            name: shipped_name,
            input_size: shipped.input_size(),
            frames: output.len(),
            proposals: shipped_proposals,
            matches: shipped_matches,
            average_ms: shipped_ms / count,
        },
        project: ComparisonSide {
            name: project_name,
            input_size,
            frames: output.len(),
            proposals: project_proposals,
            matches: project_matches,
            average_ms: project_ms / count,
        },
        frames: output,
    })
}
