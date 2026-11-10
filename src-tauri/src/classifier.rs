//! Project-trained crop classifier. A classifier is an ONNX graph that takes
//! one square crop and returns one score per class, so live scoring costs the
//! same whether a class has four hundred examples or forty thousand.
//!
//! The graph contract is deliberately smaller than the detector's: any input
//! name, a static square NCHW float input, and one output whose element count
//! equals the label count. Preprocessing matches the detector (stretch, RGB,
//! divided by 255, ImageNet mean and standard deviation), so a crop classifier
//! trained on the exported crops sees the same pixels here as in training.
//!
//! Labels come from a JSON array beside the model (`robot.labels.json` for
//! `robot.onnx`). Without that file the project's classes are used in id order.

use std::path::{Path, PathBuf};

use ort::session::Session;
use ort::value::{Tensor, ValueType};
use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::detector::{open_session, preprocess};
use crate::label::{now_millis, project_folder};
use crate::AppState;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectClassifierView {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub path: String,
    pub input_size: u32,
    pub labels: Vec<String>,
    pub active: bool,
    pub created_at: String,
}

pub struct Classifier {
    session: Session,
    input_name: String,
    input_size: u32,
    labels: Vec<String>,
    output_index: usize,
    provider_note: Option<String>,
}

impl Classifier {
    /// Opens and warms an imported graph. The blank run is how the output is
    /// found and how a model whose score count disagrees with its labels is
    /// refused before it is stored.
    pub fn load(path: &Path, labels: Vec<String>) -> Result<Self, String> {
        if labels.is_empty() {
            return Err("The classifier has no class labels".into());
        }
        let (mut session, _provider, provider_note) = open_session(path)?;
        let input = session
            .inputs()
            .first()
            .ok_or("The model has no inputs")?;
        let input_name = input.name().to_string();
        let input_size = match input.dtype() {
            ValueType::Tensor { shape, .. } => {
                let dims: Vec<i64> = shape.iter().copied().collect();
                if dims.len() != 4 || dims[1] != 3 || dims[2] <= 0 || dims[2] != dims[3] {
                    return Err(format!(
                        "Expected a static NCHW float crop input, found {dims:?}."
                    ));
                }
                dims[2] as u32
            }
            _ => return Err("The model input is not a float tensor".into()),
        };
        if input_size < 32 {
            return Err(format!("A {input_size} px crop is too small to classify"));
        }

        let zeros = vec![0f32; 3 * (input_size as usize) * (input_size as usize)];
        let tensor = Tensor::from_array((
            [1usize, 3, input_size as usize, input_size as usize],
            zeros.into_boxed_slice(),
        ))
        .map_err(|error| error.to_string())?;
        let output_count = session.outputs().len();
        let outputs = session
            .run(ort::inputs![input_name.as_str() => tensor])
            .map_err(|error| format!("The classifier did not run on a blank crop: {error}"))?;

        let mut output_index = None;
        let mut found = Vec::new();
        for index in 0..output_count {
            match outputs[index].try_extract_tensor::<f32>() {
                Ok((_, values)) => {
                    found.push(values.len());
                    if values.len() == labels.len() && output_index.is_none() {
                        output_index = Some(index);
                    }
                }
                Err(_) => found.push(0),
            }
        }
        let output_index = output_index.ok_or_else(|| {
            format!(
                "Expected one output with {} scores, found {found:?}.",
                labels.len()
            )
        })?;
        // The outputs borrow the session, so release them before storing it.
        drop(outputs);

        Ok(Classifier {
            session,
            input_name,
            input_size,
            labels,
            output_index,
            provider_note,
        })
    }

    pub fn input_size(&self) -> u32 {
        self.input_size
    }

    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} px crop, {} classes",
            self.input_size,
            self.labels.len()
        );
        if let Some(note) = &self.provider_note {
            text.push_str(&format!("; {note}"));
        }
        text
    }

    /// Softmax over the graph's scores, returning the winning label index and
    /// every probability. A model exported with raw logits and one exported
    /// with probabilities both decode correctly, because softmax of a
    /// probability vector preserves its ranking.
    pub fn classify(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(usize, Vec<f32>), String> {
        let data = preprocess(rgba, width, height, self.input_size);
        let tensor = Tensor::from_array((
            [1usize, 3, self.input_size as usize, self.input_size as usize],
            data.into_boxed_slice(),
        ))
        .map_err(|error| error.to_string())?;
        let input_name = self.input_name.clone();
        let outputs = self
            .session
            .run(ort::inputs![input_name.as_str() => tensor])
            .map_err(|error| format!("Classification failed: {error}"))?;
        let (_, values) = outputs[self.output_index]
            .try_extract_tensor::<f32>()
            .map_err(|error| error.to_string())?;
        if values.len() != self.labels.len() {
            return Err(format!(
                "The classifier returned {} scores for {} labels",
                values.len(),
                self.labels.len()
            ));
        }

        let highest = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut probabilities = Vec::with_capacity(values.len());
        let mut total = 0.0f32;
        for value in values {
            let weight = (value - highest).exp();
            total += weight;
            probabilities.push(weight);
        }
        if total > 0.0 {
            for probability in probabilities.iter_mut() {
                *probability /= total;
            }
        }

        let mut best = 0;
        let mut best_score = -1.0f32;
        for (index, probability) in probabilities.iter().enumerate() {
            if *probability > best_score {
                best_score = *probability;
                best = index;
            }
        }
        Ok((best, probabilities))
    }
}

fn parse_labels(text: &str) -> Result<Vec<String>, String> {
    let raw: Vec<String> = serde_json::from_str(text)
        .map_err(|error| format!("Labels must be a JSON array of class names: {error}"))?;
    let labels: Vec<String> = raw
        .into_iter()
        .map(|label| label.trim().to_string())
        .filter(|label| !label.is_empty())
        .collect();
    if labels.is_empty() {
        return Err("The labels file has no class names".into());
    }
    Ok(labels)
}

/// `robot.onnx` reads `robot.labels.json`. The sidecar keeps the output order
/// explicit, which an index guessed from the project cannot guarantee.
fn labels_beside(path: &Path) -> Result<Option<Vec<String>>, String> {
    let sidecar = path.with_extension("labels.json");
    if !sidecar.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&sidecar)
        .map_err(|error| format!("Could not read {}: {error}", sidecar.display()))?;
    let labels = parse_labels(&text)
        .map_err(|error| format!("{}: {error}", sidecar.display()))?;
    Ok(Some(labels))
}

fn project_labels(
    conn: &rusqlite::Connection,
    project_id: i64,
) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("select name from class where project_id = ?1 order by id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

fn view(
    id: i64,
    project_id: i64,
    name: String,
    path: String,
    input_size: u32,
    labels: Vec<String>,
    active: bool,
    created_at: String,
) -> ProjectClassifierView {
    ProjectClassifierView {
        id,
        project_id,
        name,
        path,
        input_size,
        labels,
        active,
        created_at,
    }
}

#[tauri::command]
pub fn classifier_import(
    app: AppHandle,
    project_id: i64,
    path: String,
    name: Option<String>,
) -> Result<ProjectClassifierView, String> {
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
        return Err("Pick a .onnx classifier file".into());
    }

    let labels = match labels_beside(source)? {
        Some(labels) => labels,
        None => {
            let state = app.state::<AppState>();
            let conn = state.db.lock().map_err(|error| error.to_string())?;
            let labels = project_labels(&conn, project_id)?;
            if labels.is_empty() {
                return Err(
                    "Add project classes first, or put a labels JSON array beside the model"
                        .into(),
                );
            }
            labels
        }
    };

    let classifier = Classifier::load(source, labels.clone())?;
    let input_size = classifier.input_size();
    drop(classifier);

    let mut name = name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        name = source
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "Project classifier".to_string());
    }
    if name.chars().count() > 80 {
        return Err("Keep the classifier name under 80 characters".into());
    }

    let (_slug, root) = project_folder(&app, project_id)?;
    let models = root.join("models");
    std::fs::create_dir_all(&models).map_err(|error| error.to_string())?;
    let destination = models.join(format!("classifier-{}.onnx", now_millis()));
    std::fs::copy(source, &destination)
        .map_err(|error| format!("Could not copy the model: {error}"))?;

    let encoded = serde_json::to_string(&labels).map_err(|error| error.to_string())?;
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|error| error.to_string())?;
    let inserted = conn.execute(
        "insert into project_classifier (project_id, name, path, input_size, labels, created_at)
         values (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        params![
            project_id,
            name,
            destination.to_string_lossy().to_string(),
            input_size as i64,
            encoded
        ],
    );
    if inserted.is_err() {
        let _ = std::fs::remove_file(&destination);
        return Err("Could not save the imported classifier".into());
    }
    let id = conn.last_insert_rowid();
    let created_at: String = conn
        .query_row(
            "select created_at from project_classifier where id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    drop(conn);

    Ok(view(
        id,
        project_id,
        name,
        destination.to_string_lossy().to_string(),
        input_size,
        labels,
        false,
        created_at,
    ))
}

#[tauri::command]
pub fn classifier_list(
    app: AppHandle,
    project_id: i64,
) -> Result<Vec<ProjectClassifierView>, String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "select id, project_id, name, path, input_size, labels, active, created_at
             from project_classifier where project_id = ?1 order by id desc",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut classifiers = Vec::new();
    for row in rows {
        let (id, project_id, name, path, input_size, encoded, active, created_at) =
            row.map_err(|error| error.to_string())?;
        let labels = serde_json::from_str::<Vec<String>>(&encoded).unwrap_or_default();
        classifiers.push(view(
            id,
            project_id,
            name,
            path,
            input_size,
            labels,
            active,
            created_at,
        ));
    }
    Ok(classifiers)
}

/// The classifier the next capture start should load. Kept beside
/// `project_detector::active_model` so both model slots follow one pattern.
pub(crate) fn active_model(
    app: &AppHandle,
    project_id: i64,
) -> Option<(String, Vec<String>, String)> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().ok()?;
    let (path, encoded, name): (String, String, String) = conn
        .query_row(
            "select path, labels, name from project_classifier
             where project_id = ?1 and active = 1 limit 1",
            params![project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok()?;
    let labels = serde_json::from_str::<Vec<String>>(&encoded).ok()?;
    Some((path, labels, name))
}

#[tauri::command]
pub fn classifier_activate(
    app: AppHandle,
    project_id: i64,
    id: i64,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state
        .capture
        .lock()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("Stop capture before changing the live classifier".into());
    }
    let (path, encoded) = {
        let conn = state.db.lock().map_err(|error| error.to_string())?;
        conn.query_row(
            "select path, labels from project_classifier
             where id = ?1 and project_id = ?2",
            params![id, project_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|_| "That classifier is not in this project".to_string())?
    };
    let labels = serde_json::from_str::<Vec<String>>(&encoded)
        .map_err(|_| "That classifier has unreadable labels".to_string())?;
    let classifier = Classifier::load(Path::new(&path), labels)?;
    {
        let mut guard = state
            .classifier
            .lock()
            .map_err(|error| error.to_string())?;
        *guard = Some(classifier);
    }
    let conn = state.db.lock().map_err(|error| error.to_string())?;
    conn.execute(
        "update project_classifier set active = 0 where project_id = ?1",
        params![project_id],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "update project_classifier set active = 1 where id = ?1",
        params![id],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn classifier_deactivate(app: AppHandle, project_id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state
        .capture
        .lock()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("Stop capture before changing the live classifier".into());
    }
    {
        let conn = state.db.lock().map_err(|error| error.to_string())?;
        conn.execute(
            "update project_classifier set active = 0 where project_id = ?1",
            params![project_id],
        )
        .map_err(|error| error.to_string())?;
    }
    let mut guard = state
        .classifier
        .lock()
        .map_err(|error| error.to_string())?;
    *guard = None;
    Ok(())
}

#[tauri::command]
pub fn classifier_delete(app: AppHandle, id: i64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let conn = state.db.lock().map_err(|error| error.to_string())?;
    let row: Option<(String, bool)> = conn
        .query_row(
            "select path, active from project_classifier where id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    conn.execute(
        "delete from project_classifier where id = ?1",
        params![id],
    )
    .map_err(|error| error.to_string())?;
    drop(conn);
    if let Some((path, active)) = row {
        let _ = std::fs::remove_file(PathBuf::from(path));
        if active {
            if let Ok(mut guard) = state.classifier.lock() {
                *guard = None;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_labels;

    #[test]
    fn labels_read_from_a_json_array() {
        let labels = parse_labels(r#"["Enemy", " background ", ""]"#).expect("labels");
        assert_eq!(labels, vec!["Enemy", "background"]);
    }

    #[test]
    fn an_empty_label_list_is_refused() {
        assert!(parse_labels("[]").is_err());
        assert!(parse_labels("not json").is_err());
    }
}
