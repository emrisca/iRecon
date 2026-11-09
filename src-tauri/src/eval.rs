//! Offline evaluation: runs the loaded detector and the current pool over frames
//! that were not used as examples, so a pool change can be measured rather than
//! guessed. Ground truth is stored per frame and class, which is what turns
//! "some boxes were drawn" into recall and false positives.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::capture::{crop_region, EXPANSION_SCORE_THRESHOLD, SCORE_THRESHOLD};
use crate::detector::Detection;
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
    /// The boxes that cleared their class threshold, ready to draw or report.
    pub matched_boxes: Vec<Detection>,
    /// Class ids that cleared their threshold somewhere in this frame.
    pub matched_class_ids: Vec<i64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassRecall {
    pub class_id: i64,
    pub name: String,
    pub color: String,
    /// Evaluated frames marked as containing this class.
    pub marked: usize,
    /// Marked frames where at least one box cleared the threshold.
    pub hits: usize,
    /// Unmarked frames where a box still cleared the threshold.
    pub false_positives: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalReport {
    pub frames: Vec<EvalFrame>,
    pub class_recall: Vec<ClassRecall>,
    /// Frames where at least one proposal cleared its class threshold. With a
    /// held-out set that is known to contain the class, this is the recall.
    pub frames_with_match: usize,
    pub proposals: usize,
    pub matches: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameTruth {
    pub frame_id: i64,
    pub class_ids: Vec<i64>,
}

#[tauri::command]
pub fn frame_truth_list(app: AppHandle, project_id: i64) -> Result<Vec<FrameTruth>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select f.id, ft.class_id
             from frame f
             left join frame_truth ft on ft.frame_id = f.id
             where f.project_id = ?1
             order by f.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut truths: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for row in rows {
        let (frame_id, class_id) = row.map_err(|e| e.to_string())?;
        let entry = truths.entry(frame_id).or_default();
        if let Some(class_id) = class_id {
            entry.push(class_id);
        }
    }
    Ok(truths
        .into_iter()
        .map(|(frame_id, class_ids)| FrameTruth {
            frame_id,
            class_ids,
        })
        .collect())
}

#[tauri::command]
pub fn frame_truth_set(
    app: AppHandle,
    frame_id: i64,
    class_id: i64,
    present: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let valid: i64 = conn
        .query_row(
            "select count(*) from frame f
             join class c on c.project_id = f.project_id
             where f.id = ?1 and c.id = ?2",
            params![frame_id, class_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if valid == 0 {
        return Err("That frame and class are not in the same project".into());
    }
    if present {
        conn.execute(
            "insert or ignore into frame_truth (frame_id, class_id) values (?1, ?2)",
            params![frame_id, class_id],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "delete from frame_truth where frame_id = ?1 and class_id = ?2",
            params![frame_id, class_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
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
        let (boxes, _) = detector.analyse(
            image.as_raw(),
            image.width(),
            image.height(),
            EXPANSION_SCORE_THRESHOLD,
        )?;

        let mut matches = 0usize;
        let mut matched_boxes: Vec<Detection> = Vec::new();
        let mut matched_class_ids: HashSet<i64> = HashSet::new();
        let mut best_score: Option<f32> = None;
        let mut best_class: Option<String> = None;
        let mut scored: Vec<(Detection, matcher::LiveMatch)> = Vec::new();
        let mut primary_proposals = 0usize;
        for detection in &boxes {
            if detection.w * detection.h >= matcher::MAX_PROPOSAL_AREA {
                continue;
            }
            if detection.score >= SCORE_THRESHOLD {
                primary_proposals += 1;
            }
            let Some(crop) =
                crop_region(image.as_raw(), image.width(), image.height(), detection)
            else {
                continue;
            };
            let candidate = matcher::embed(&crop);
            if let Some(hit) = pool.best(&candidate) {
                if detection.score >= SCORE_THRESHOLD
                    && best_score.map(|score| hit.score > score).unwrap_or(true)
                {
                    best_score = Some(hit.score);
                    best_class = Some(hit.name.clone());
                }
                scored.push((detection.clone(), hit));
            }
        }

        for (mut detection, hit) in matcher::refine(
            &scored,
            SCORE_THRESHOLD,
            EXPANSION_SCORE_THRESHOLD,
        ) {
            matches += 1;
            matched_class_ids.insert(hit.class_id);
            detection.label = hit.name;
            detection.score = hit.score;
            matched_boxes.push(detection);
        }

        if matches > 0 {
            frames_with_match += 1;
        }
        total_proposals += primary_proposals;
        total_matches += matches;
        frames.push(EvalFrame {
            frame_id,
            width: image.width(),
            height: image.height(),
            proposals: primary_proposals,
            matches,
            best_score,
            best_class,
            matched_boxes,
            matched_class_ids: matched_class_ids.into_iter().collect(),
        });
    }
    drop(detector_guard);

    // Recall needs the marks for exactly the frames that were evaluated.
    let evaluated: HashSet<i64> = frames.iter().map(|frame| frame.frame_id).collect();
    let (classes, truths): (Vec<(i64, String, String)>, BTreeMap<i64, BTreeSet<i64>>) = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut statement = conn
            .prepare("select id, name, color from class where project_id = ?1 order by id")
            .map_err(|e| e.to_string())?;
        let classes = statement
            .query_map(params![project_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        drop(statement);

        let mut statement = conn
            .prepare(
                "select ft.frame_id, ft.class_id
                 from frame_truth ft
                 join frame f on f.id = ft.frame_id
                 where f.project_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut truths: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
        for row in statement
            .query_map(params![project_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| e.to_string())?
        {
            let (frame_id, class_id) = row.map_err(|e| e.to_string())?;
            truths.entry(frame_id).or_default().insert(class_id);
        }
        (classes, truths)
    };

    let class_recall = classes
        .into_iter()
        .map(|(class_id, name, color)| {
            let mut marked = 0usize;
            let mut hits = 0usize;
            let mut false_positives = 0usize;
            for frame in &frames {
                let is_marked = truths
                    .get(&frame.frame_id)
                    .map(|ids| ids.contains(&class_id))
                    .unwrap_or(false);
                let matched = frame.matched_class_ids.contains(&class_id);
                if is_marked {
                    marked += 1;
                    if matched {
                        hits += 1;
                    }
                } else if matched {
                    false_positives += 1;
                }
            }
            ClassRecall {
                class_id,
                name,
                color,
                marked,
                hits,
                false_positives,
            }
        })
        .collect();

    // Keep the evaluated set honest: only frames actually run are counted.
    debug_assert!(evaluated.len() == frames.len());

    Ok(EvalReport {
        frames,
        class_recall,
        frames_with_match,
        proposals: total_proposals,
        matches: total_matches,
    })
}
