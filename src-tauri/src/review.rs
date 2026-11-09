//! Candidate review: the uncertain proposals the capture loop saved.
//!
//! A candidate is a crop the matcher was unsure about, plus the class it nearly
//! matched and the score it got. Accepting writes a positive example, rejecting
//! writes a negative, and both take effect on the next pool reload.

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
    pub preview: String,
    pub score: f32,
    pub state: String,
    pub created_at: String,
}

fn preview_of(path: &str) -> String {
    image::open(path)
        .map(|image| label::preview(&image.to_rgba8()))
        .unwrap_or_default()
}

#[tauri::command]
pub fn candidates_list(
    app: AppHandle,
    project_id: i64,
    state: Option<String>,
) -> Result<Vec<CandidateView>, String> {
    let state = state.unwrap_or_default();
    let state = if state.is_empty() { None } else { Some(state) };

    let app_state = app.state::<AppState>();
    let conn = app_state.db.lock().map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(
            "select k.id, k.project_id, k.class_id, c.name, c.color, k.path, k.score,
                    k.state, k.created_at
             from candidate k join class c on c.id = k.class_id
             where k.project_id = ?1 and (?2 is null or k.state = ?2)
             order by k.id desc limit 200",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id, state], |row| {
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

    let mut candidates = Vec::new();
    for row in rows {
        let (id, project_id, class_id, class_name, color, path, score, state, created_at) =
            row.map_err(|e| e.to_string())?;
        candidates.push(CandidateView {
            id,
            project_id,
            class_id,
            class_name,
            color,
            preview: preview_of(&path),
            path,
            score,
            state,
            created_at,
        });
    }
    Ok(candidates)
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

    let rows = label::examples_list(app.clone(), class_id)?;
    let mut by_id: std::collections::HashMap<i64, ExampleView> = rows
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    Ok(nearest
        .into_iter()
        .filter_map(|(example_id, _)| by_id.remove(&example_id))
        .collect())
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
