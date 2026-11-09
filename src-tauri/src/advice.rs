//! Coverage hints and threshold advice, read from the fingerprints already in
//! the pool. Nothing here is stored: it describes the examples as they are now.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::matcher;
use crate::AppState;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassAdvice {
    pub class_id: i64,
    pub name: String,
    pub positives: usize,
    pub negatives: usize,
    /// Median similarity between a positive and its nearest other positive.
    pub similarity: f32,
    pub threshold: f32,
    pub suggested: Option<f32>,
    pub hints: Vec<String>,
}

fn round_to_five(value: f32) -> f32 {
    (value * 20.0).round() / 20.0
}

fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[values.len() / 2])
}

#[tauri::command]
pub fn class_advice(app: AppHandle, project_id: i64) -> Result<Vec<ClassAdvice>, String> {
    let pool = {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        matcher::load_pool(&conn, project_id)?
    };

    let mut advice = Vec::new();
    for class in &pool.classes {
        let positives = class.positives.len();
        let negatives = class.negatives.len();

        // Leave one out: how close each positive sits to its nearest sibling.
        // Identical crops score near 1.0, which is the signal for "all examples
        // look alike" rather than "the class is well covered".
        let mut nearest: Vec<f32> = Vec::new();
        for (index, (_, embedding)) in class.positives.iter().enumerate() {
            let best = class
                .positives
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, (_, other))| matcher::cosine(embedding, other))
                .fold(f32::MIN, f32::max);
            if best > f32::MIN {
                nearest.push(best);
            }
        }
        let similarity = median(&mut nearest).unwrap_or(0.0);

        let closest_negative = class
            .negatives
            .iter()
            .map(|negative| {
                class
                    .positives
                    .iter()
                    .map(|(_, positive)| matcher::cosine(negative, positive))
                    .fold(f32::MIN, f32::max)
            })
            .fold(f32::MIN, f32::max);

        let mut hints = Vec::new();
        if positives == 0 {
            hints.push("No positive example yet, so this class cannot match anything.".to_string());
        } else if positives < 3 {
            hints.push(format!(
                "Only {positives} positive{}. Three is the minimum for a stable pool.",
                if positives == 1 { "" } else { "s" }
            ));
        }
        if positives > 0 && negatives == 0 {
            hints.push("No negatives yet, so look-alike objects cannot be separated.".to_string());
        }
        if positives >= 2 && similarity >= 0.985 {
            hints.push(
                "Your positives are nearly identical. Add another angle, distance or lighting."
                    .to_string(),
            );
        }
        if positives >= 2 && similarity > 0.0 && similarity <= 0.78 {
            hints.push("Positives vary a lot. Consider splitting this class in two.".to_string());
        }
        if negatives > 0 && closest_negative >= 0.9 {
            hints.push("A negative is very close to your positives. Check that label.".to_string());
        }

        let suggested = if positives >= 2 && negatives >= 1 && closest_negative > f32::MIN {
            Some(round_to_five(((similarity + closest_negative) / 2.0).clamp(0.4, 0.95)))
        } else if positives >= 3 {
            Some(round_to_five((similarity - 0.1).clamp(0.4, 0.9)))
        } else {
            None
        };
        if let Some(suggested) = suggested {
            if (class.threshold - suggested).abs() >= 0.06 {
                hints.push(format!(
                    "Threshold is {:.2}; the examples suggest about {suggested:.2}.",
                    class.threshold
                ));
            }
        }

        advice.push(ClassAdvice {
            class_id: class.id,
            name: class.name.clone(),
            positives,
            negatives,
            similarity: similarity.max(0.0),
            threshold: class.threshold,
            suggested,
            hints,
        });
    }
    Ok(advice)
}
