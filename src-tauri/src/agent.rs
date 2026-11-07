//! A development-only surface an agent can drive without a mouse.
//!
//! Each action is the same function the UI calls, a snapshot returns state as
//! data rather than pixels, and every action appends one JSON line to
//! `agent-trace.jsonl` inside the project so a run can be audited afterwards.
//! Release builds refuse the whole surface: it can grab the screen and write
//! labels without a person confirming anything.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rusqlite::params;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::AppState;

const ACTIONS: [&str; 5] = ["grab", "label", "match", "export", "state"];

fn enabled() -> Result<(), String> {
    if cfg!(debug_assertions) {
        Ok(())
    } else {
        Err("The agent surface is only available in development builds.".to_string())
    }
}

fn validate_action(action: &str) -> Result<(), String> {
    if ACTIONS.contains(&action) {
        Ok(())
    } else {
        Err(format!(
            "Unknown action {action}. Use one of: {}.",
            ACTIONS.join(", ")
        ))
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

fn trace_line(action: &str, ok: bool, ms: u64, detail: &str) -> Value {
    json!({
        "ts": now_millis(),
        "action": action,
        "ok": ok,
        "ms": ms,
        "detail": detail,
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentClass {
    pub id: i64,
    pub name: String,
    pub examples: i64,
    pub matchable: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshot {
    pub project_id: i64,
    pub project: String,
    pub frames: i64,
    pub examples: i64,
    pub matchable: i64,
    pub last_frame: Option<i64>,
    pub embedding_model: String,
    pub model_loaded: bool,
    pub classes: Vec<AgentClass>,
}

fn project_dir(app: &AppHandle, project_id: i64) -> Result<PathBuf, String> {
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
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("projects")
        .join(slug);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn snapshot(app: &AppHandle, project_id: i64) -> Result<AgentSnapshot, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let project: String = conn
        .query_row(
            "select name from project where id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|_| "That project no longer exists".to_string())?;
    let frames: i64 = conn
        .query_row(
            "select count(*) from frame where project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let examples: i64 = conn
        .query_row(
            "select count(*) from example e join class c on c.id = e.class_id
             where c.project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let matchable: i64 = conn
        .query_row(
            "select count(*) from example e join class c on c.id = e.class_id
             where c.project_id = ?1 and e.embedding is not null and e.embedding_model = ?2",
            params![project_id, crate::matcher::EMBEDDING_MODEL],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let last_frame: Option<i64> = conn
        .query_row(
            "select max(id) from frame where project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .unwrap_or(None);

    let mut statement = conn
        .prepare(
            "select c.id, c.name,
                    (select count(*) from example e where e.class_id = c.id),
                    (select count(*) from example e where e.class_id = c.id
                       and e.embedding is not null and e.embedding_model = ?2)
             from class c where c.project_id = ?1 order by c.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id, crate::matcher::EMBEDDING_MODEL], |row| {
            Ok(AgentClass {
                id: row.get(0)?,
                name: row.get(1)?,
                examples: row.get(2)?,
                matchable: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut classes = Vec::new();
    for row in rows {
        classes.push(row.map_err(|e| e.to_string())?);
    }
    drop(statement);
    drop(conn);

    let model_loaded = app
        .state::<AppState>()
        .detector
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false);

    Ok(AgentSnapshot {
        project_id,
        project,
        frames,
        examples,
        matchable,
        last_frame,
        embedding_model: crate::matcher::EMBEDDING_MODEL.to_string(),
        model_loaded,
        classes,
    })
}

fn number(payload: &Value, key: &str) -> Result<f32, String> {
    payload
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .ok_or_else(|| format!("This action needs a numeric {key}"))
}

fn class_for(app: &AppHandle, project_id: i64, name: &str) -> Result<i64, String> {
    let state = app.state::<AppState>();
    let existing: Option<i64> = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "select id from class where project_id = ?1 and lower(name) = lower(?2)",
            params![project_id, name],
            |row| row.get(0),
        )
        .ok()
    };
    match existing {
        Some(id) => Ok(id),
        None => Ok(crate::label::class_create(app.clone(), project_id, name.to_string())?.id),
    }
}

fn run(app: &AppHandle, project_id: i64, action: &str, payload: &Value) -> Result<Value, String> {
    validate_action(action)?;
    match action {
        "state" => serde_json::to_value(snapshot(app, project_id)?).map_err(|e| e.to_string()),
        "grab" => serde_json::to_value(crate::label::frame_grab(app.clone(), project_id)?)
            .map_err(|e| e.to_string()),
        "label" => {
            let frame_id = payload
                .get("frameId")
                .and_then(Value::as_i64)
                .ok_or("label needs a numeric frameId")?;
            let name = payload
                .get("class")
                .and_then(Value::as_str)
                .ok_or("label needs a class name")?;
            let class_id = class_for(app, project_id, name)?;
            let example = crate::label::example_create(
                app.clone(),
                class_id,
                frame_id,
                number(payload, "x")?,
                number(payload, "y")?,
                number(payload, "w")?,
                number(payload, "h")?,
            )?;
            serde_json::to_value(example).map_err(|e| e.to_string())
        }
        "match" => {
            let frame_id = payload
                .get("frameId")
                .and_then(Value::as_i64)
                .ok_or("match needs a numeric frameId")?;
            serde_json::to_value(crate::label::match_region(
                app.clone(),
                project_id,
                frame_id,
                number(payload, "x")?,
                number(payload, "y")?,
                number(payload, "w")?,
                number(payload, "h")?,
            )?)
            .map_err(|e| e.to_string())
        }
        "export" => serde_json::to_value(crate::dataset::dataset_export(
            app.clone(),
            project_id,
        )?)
        .map_err(|e| e.to_string()),
        _ => unreachable!("validate_action covers every action"),
    }
}

fn append_trace(app: &AppHandle, project_id: i64, line: &Value) -> Result<(), String> {
    let dir = project_dir(app, project_id)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("agent-trace.jsonl"))
        .map_err(|e| e.to_string())?;
    writeln!(file, "{line}").map_err(|e| e.to_string())
}

#[tauri::command]
pub fn agent_snapshot(app: AppHandle, project_id: i64) -> Result<AgentSnapshot, String> {
    enabled()?;
    snapshot(&app, project_id)
}

#[tauri::command]
pub fn agent_act(
    app: AppHandle,
    project_id: i64,
    action: String,
    payload: Value,
) -> Result<Value, String> {
    enabled()?;
    let started = Instant::now();
    let result = run(&app, project_id, &action, &payload);
    let detail = result.as_ref().err().cloned().unwrap_or_default();
    let line = trace_line(
        &action,
        result.is_ok(),
        started.elapsed().as_millis() as u64,
        &detail,
    );
    let _ = append_trace(&app, project_id, &line);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advertised_action_validates() {
        for action in ACTIONS {
            assert!(validate_action(action).is_ok(), "{action} was rejected");
        }
    }

    #[test]
    fn an_unknown_action_lists_the_real_ones() {
        let error = validate_action("teleport").unwrap_err();
        assert!(error.contains("grab"));
        assert!(error.contains("export"));
    }

    #[test]
    fn a_trace_line_is_one_json_object() {
        let line = trace_line("grab", false, 12, "no display");
        let text = line.to_string();
        assert!(!text.contains('\n'));
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["action"], "grab");
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["ms"], 12);
    }
}
