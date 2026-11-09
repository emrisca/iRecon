//! Offline evaluation: runs the loaded detector and the current pool over frames
//! that were not used as examples, so a pool change can be measured rather than
//! guessed. This is the first half of the replay harness Phase 4 will need.

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::capture::crop_region;
use crate::matcher;
use crate::AppState;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalFrame {
    pub frame_id: i64,
    pub width: u32,
    pub height: u32,
    pub proposals: usize,
    pub matches: usize,
    pub best_score: Option<f32>,
    pub best_class: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalReport {
    pub frames: Vec<EvalFrame>,
    /// Frames where at least one proposal cleared its class threshold. With a
    /// held-out set that is known to contain the class, this is the recall.
    pub frames_with_match: usize,
    pub proposals: usize,
    pub matches: usize,
}

#[tauri::command]
pub fn evaluate_frames(
    app: AppHandle,
    project_id: i64,
    frame_ids: Vec<i64>,
) -> Result<EvalReport, String> {
    if frame_ids.is_empty() {
        return Err("Give the evaluation at least one frame".into());
    }

    // Paths are read first: the detector lock is held for the whole run, and
    // taking the database lock inside it would invert the capture thread's order.
    let mut frames_to_read: Vec<(i64, String)> = Vec::new();
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare("select id, path from frame where id = ?1 and project_id = ?2")
            .map_err(|e| e.to_string())?;
        for frame_id in frame_ids {
            let row = statement
                .query_row(params![frame_id, project_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|_| format!("Frame {frame_id} is not in this project"))?;
            frames_to_read.push(row);
        }
    }

    let pool = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        matcher::load_pool(&conn, project_id)?
    };

    let state = app.state::<AppState>();
    let mut detector_guard = state.detector.lock().map_err(|e| e.to_string())?;
    let detector = detector_guard
        .as_mut()
        .ok_or("Load a model before evaluating frames")?;

    let mut frames = Vec::new();
    let mut frames_with_match = 0usize;
    let mut total_proposals = 0usize;
    let mut total_matches = 0usize;

    for (frame_id, path) in frames_to_read {
        let image = image::open(&path)
            .map_err(|error| format!("Could not open frame {frame_id}: {error}"))?
            .to_rgba8();
        let (boxes, _) =
            detector.analyse(image.as_raw(), image.width(), image.height(), 0.35)?;

        let mut matches = 0usize;
        let mut best_score: Option<f32> = None;
        let mut best_class: Option<String> = None;
        for detection in &boxes {
            let Some(crop) =
                crop_region(image.as_raw(), image.width(), image.height(), detection)
            else {
                continue;
            };
            let candidate = matcher::embed(&crop);
            if let Some(hit) = pool.best(&candidate) {
                if best_score.map(|score| hit.score > score).unwrap_or(true) {
                    best_score = Some(hit.score);
                    best_class = Some(hit.name.clone());
                }
                if hit.score >= hit.threshold {
                    matches += 1;
                }
            }
        }

        if matches > 0 {
            frames_with_match += 1;
        }
        total_proposals += boxes.len();
        total_matches += matches;
        frames.push(EvalFrame {
            frame_id,
            width: image.width(),
            height: image.height(),
            proposals: boxes.len(),
            matches,
            best_score,
            best_class,
        });
    }

    Ok(EvalReport {
        frames,
        frames_with_match,
        proposals: total_proposals,
        matches: total_matches,
    })
}
