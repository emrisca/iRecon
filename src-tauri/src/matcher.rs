//! Project-local matching.
//!
//! An example is reduced to a small, deterministic fingerprint: colour and
//! structure summarised on a fixed grid, centred and normalised to unit length.
//! Stored as little-endian `f32` it compares with a dot product, so a pool of a
//! few thousand examples scores in well under a millisecond.
//!
//! The fingerprint is deliberately not an ONNX encoder yet. It is a baseline
//! that works with no second model download, and the storage format (a blob and
//! the model name that produced it) does not change when one replaces it.

use std::collections::HashMap;

use image::RgbaImage;
use serde::Serialize;

/// Bumped whenever the fingerprint changes: old rows then re-embed lazily
/// instead of being compared against a different vector space.
pub const EMBEDDING_MODEL: &str = "irecon-fingerprint-v2";
pub const EMBEDDING_DIM: usize = 96;

const CELLS: usize = 4;
/// Per cell: red ratio, green ratio, luma, edge strength, hue cosine, hue sine.
const FEATURES: usize = 6;

/// One example reduced to a unit vector.
///
/// Each cell contributes colour ratios, brightness, edge strength and a
/// saturation-weighted hue direction. Subtracting the average cell afterwards
/// is the important part: a crop and its background share most of that average,
/// so removing it leaves where this crop differs from its own mean, which is
/// what actually separates one object from another. Without it every pair of
/// crops scored around 0.99 and no threshold could tell them apart.
pub fn embed(image: &RgbaImage) -> Vec<f32> {
    let width = image.width().max(1);
    let height = image.height().max(1);
    let cell_count = CELLS * CELLS;
    let mut sums = vec![[0f32; FEATURES]; cell_count];
    let mut counts = vec![0f32; cell_count];

    for (x, y, pixel) in image.enumerate_pixels() {
        let [r, g, b, _] = pixel.0;
        let (r, g, b) = (r as f32 + 1.0, g as f32 + 1.0, b as f32 + 1.0);
        let total = r + g + b;

        let cell_x = (x * CELLS as u32 / width).min(CELLS as u32 - 1) as usize;
        let cell_y = (y * CELLS as u32 / height).min(CELLS as u32 - 1) as usize;
        let cell = cell_y * CELLS + cell_x;

        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let right = image.get_pixel((x + 1).min(width - 1), y).0;
        let down = image.get_pixel(x, (y + 1).min(height - 1)).0;
        let right_luma = 0.2126 * right[0] as f32
            + 0.7152 * right[1] as f32
            + 0.0722 * right[2] as f32;
        let down_luma = 0.2126 * down[0] as f32
            + 0.7152 * down[1] as f32
            + 0.0722 * down[2] as f32;
        let edge = ((right_luma - luma).abs() + (down_luma - luma).abs()) / 2.0;

        let high = r.max(g).max(b);
        let low = r.min(g).min(b);
        let chroma = high - low;
        let saturation = chroma / high.max(1.0);
        let hue = if chroma <= 1.0 {
            0.0
        } else if high == r {
            ((g - b) / chroma).rem_euclid(6.0)
        } else if high == g {
            (b - r) / chroma + 2.0
        } else {
            (r - g) / chroma + 4.0
        };
        let angle = hue / 6.0 * std::f32::consts::TAU;

        sums[cell][0] += r / total;
        sums[cell][1] += g / total;
        sums[cell][2] += luma / 255.0;
        sums[cell][3] += edge / 255.0;
        sums[cell][4] += angle.cos() * saturation;
        sums[cell][5] += angle.sin() * saturation;
        counts[cell] += 1.0;
    }

    let mut values = vec![0f32; cell_count * FEATURES];
    for cell in 0..cell_count {
        let count = counts[cell].max(1.0);
        for feature in 0..FEATURES {
            values[cell * FEATURES + feature] = sums[cell][feature] / count;
        }
    }

    let mut centred = values.clone();
    for feature in 0..FEATURES {
        let mean = (0..cell_count)
            .map(|cell| values[cell * FEATURES + feature])
            .sum::<f32>()
            / cell_count as f32;
        for cell in 0..cell_count {
            centred[cell * FEATURES + feature] -= mean;
        }
    }
    normalize(&mut centred);

    // A flat crop has nothing to compare after centring, so fall back to its
    // absolute colour instead of returning a zero vector that matches nothing.
    if centred.iter().all(|value| value.abs() < 1e-6) {
        normalize(&mut values);
        return values;
    }
    centred
}

pub fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for value in vector.iter_mut() {
            *value /= norm;
        }
    }
}

/// Both sides are stored normalized, so this is just the dot product.
pub fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let len = left.len().min(right.len());
    let mut dot = 0.0;
    for index in 0..len {
        dot += left[index] * right[index];
    }
    dot.clamp(-1.0, 1.0)
}

pub fn to_blob(vector: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

pub fn from_blob(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() != EMBEDDING_DIM * 4 {
        return None;
    }
    let mut vector = Vec::with_capacity(EMBEDDING_DIM);
    for chunk in blob.chunks_exact(4) {
        vector.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(vector)
}

#[derive(Default)]
pub struct Pool {
    pub classes: Vec<ClassPool>,
    pub examples: usize,
}

pub struct ClassPool {
    pub id: i64,
    pub name: String,
    pub threshold: f32,
    /// Example id beside its unit-length fingerprint, so the review screen can
    /// name the examples that made a candidate look interesting.
    pub positives: Vec<(i64, Vec<f32>)>,
    pub negatives: Vec<Vec<f32>>,
}

impl ClassPool {
    fn score(&self, candidate: &[f32]) -> f32 {
        let mut positive = -1.0f32;
        for (_, embedding) in &self.positives {
            positive = positive.max(cosine(candidate, embedding));
        }
        let mut negative = 0.0f32;
        for embedding in &self.negatives {
            negative = negative.max(cosine(candidate, embedding).max(0.0));
        }
        (positive - negative).clamp(0.0, 1.0)
    }
}

#[derive(Clone)]
pub struct LiveMatch {
    pub class_id: i64,
    pub name: String,
    pub score: f32,
    pub threshold: f32,
}

impl Pool {
    /// Best class whose score clears its own threshold. Classes without a
    /// positive example can never match, which keeps an empty project from
    /// drawing boxes.
    pub fn classify(&self, candidate: &[f32]) -> Option<LiveMatch> {
        let best = self.best(candidate)?;
        if best.score < best.threshold {
            return None;
        }
        Some(best)
    }

    /// The best class score whether or not it clears that class's threshold.
    /// Candidate harvesting uses it to find proposals that landed just under
    /// the line: close enough to be worth asking about.
    pub fn best(&self, candidate: &[f32]) -> Option<LiveMatch> {
        let mut best: Option<LiveMatch> = None;
        for class in &self.classes {
            if class.positives.is_empty() {
                continue;
            }
            let score = class.score(candidate);
            let better = best
                .as_ref()
                .map(|current| score > current.score)
                .unwrap_or(true);
            if better {
                best = Some(LiveMatch {
                    class_id: class.id,
                    name: class.name.clone(),
                    score,
                    threshold: class.threshold,
                });
            }
        }
        best
    }

    /// The examples nearest a candidate in one class, best first. The review
    /// screen shows these beside the candidate so an answer is an informed one.
    pub fn nearest(&self, candidate: &[f32], class_id: i64, limit: usize) -> Vec<(i64, f32)> {
        let Some(class) = self.classes.iter().find(|class| class.id == class_id) else {
            return Vec::new();
        };
        let mut scored: Vec<(i64, f32)> = class
            .positives
            .iter()
            .map(|(id, embedding)| (*id, cosine(candidate, embedding)))
            .collect();
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        scored
    }
}

/// Loads every class and matchable example for one project.
pub fn load_pool(conn: &rusqlite::Connection, project_id: i64) -> Result<Pool, String> {
    let mut statement = conn
        .prepare(
            "select c.id, c.name, c.threshold, e.id, e.role, e.embedding
             from class c
             left join example e
               on e.class_id = c.id and e.embedding is not null and e.embedding_model = ?2
             where c.project_id = ?1
             order by c.id, e.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(rusqlite::params![project_id, EMBEDDING_MODEL], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f32>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<Vec<u8>>>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut pool = Pool::default();
    let mut current: Option<i64> = None;
    for row in rows {
        let (id, name, threshold, example_id, role, blob) = row.map_err(|e| e.to_string())?;
        if current != Some(id) {
            pool.classes.push(ClassPool {
                id,
                name,
                threshold,
                positives: Vec::new(),
                negatives: Vec::new(),
            });
            current = Some(id);
        }
        let Some(blob) = blob else {
            continue;
        };
        let Some(embedding) = from_blob(&blob) else {
            continue;
        };
        if let Some(class) = pool.classes.last_mut() {
            match (role.as_deref(), example_id) {
                (Some("negative"), _) => class.negatives.push(embedding),
                (_, Some(example_id)) => class.positives.push((example_id, embedding)),
                _ => continue,
            }
            pool.examples += 1;
        }
    }
    Ok(pool)
}

pub struct PoolEntry {
    pub example_id: i64,
    pub class_id: i64,
    pub role: String,
    pub embedding: Vec<f32>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassMatch {
    pub class_id: i64,
    pub name: String,
    /// Best positive minus the best negative, clamped to zero. With no
    /// negatives this is simply the best similarity.
    pub score: f32,
    pub similarity: f32,
    pub margin: f32,
    pub best_example: i64,
    pub positives: usize,
    pub negatives: usize,
}

#[derive(Default)]
struct Tally {
    best_positive: f32,
    best_example: i64,
    positives: usize,
    best_negative: f32,
    negatives: usize,
}

/// Score every class that has at least one positive example, best first.
pub fn rank(
    candidate: &[f32],
    entries: &[PoolEntry],
    names: &HashMap<i64, String>,
) -> Vec<ClassMatch> {
    let mut tallies: HashMap<i64, Tally> = HashMap::new();
    for entry in entries {
        let similarity = cosine(candidate, &entry.embedding);
        let tally = tallies.entry(entry.class_id).or_insert_with(|| Tally {
            best_positive: -1.0,
            ..Tally::default()
        });
        if entry.role == "negative" {
            tally.negatives += 1;
            tally.best_negative = tally.best_negative.max(similarity);
        } else {
            tally.positives += 1;
            if similarity > tally.best_positive {
                tally.best_positive = similarity;
                tally.best_example = entry.example_id;
            }
        }
    }

    let mut matches: Vec<ClassMatch> = tallies
        .into_iter()
        .filter(|(_, tally)| tally.positives > 0)
        .map(|(class_id, tally)| {
            let negative = if tally.negatives > 0 {
                tally.best_negative.max(0.0)
            } else {
                0.0
            };
            let margin = tally.best_positive - negative;
            ClassMatch {
                class_id,
                name: names
                    .get(&class_id)
                    .cloned()
                    .unwrap_or_else(|| "Unknown class".to_string()),
                score: margin.clamp(0.0, 1.0),
                similarity: tally.best_positive,
                margin,
                best_example: tally.best_example,
                positives: tally.positives,
                negatives: tally.negatives,
            }
        })
        .collect();
    matches.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(32, 32, image::Rgba(color))
    }

    fn patterned() -> RgbaImage {
        let mut image = RgbaImage::from_pixel(64, 64, image::Rgba([20, 24, 32, 255]));
        for y in 8..24 {
            for x in 8..24 {
                image.put_pixel(x, y, image::Rgba([220, 120, 40, 255]));
            }
        }
        for y in 40..56 {
            for x in 40..56 {
                image.put_pixel(x, y, image::Rgba([40, 180, 120, 255]));
            }
        }
        image
    }

    #[test]
    fn an_embedding_has_unit_length() {
        let vector = embed(&patterned());
        assert_eq!(vector.len(), EMBEDDING_DIM);
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "norm was {norm}");
    }

    #[test]
    fn the_same_crop_scores_one() {
        let left = embed(&patterned());
        let right = embed(&patterned());
        assert!((cosine(&left, &right) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn different_colours_score_below_a_match() {
        let red = embed(&solid([230, 40, 30, 255]));
        let blue = embed(&solid([30, 60, 230, 255]));
        assert!(cosine(&red, &blue) < 0.9, "red vs blue was {}", cosine(&red, &blue));
    }

    #[test]
    fn a_blob_round_trips() {
        let vector = embed(&patterned());
        let blob = to_blob(&vector);
        assert_eq!(blob.len(), EMBEDDING_DIM * 4);
        assert_eq!(from_blob(&blob).unwrap(), vector);
        assert!(from_blob(&blob[..8]).is_none());
    }

    #[test]
    fn the_closest_class_is_ranked_first() {
        let candidate = embed(&patterned());
        let other = embed(&solid([10, 200, 220, 255]));
        let entries = vec![
            PoolEntry {
                example_id: 1,
                class_id: 7,
                role: "positive".into(),
                embedding: candidate.clone(),
            },
            PoolEntry {
                example_id: 2,
                class_id: 9,
                role: "positive".into(),
                embedding: other,
            },
        ];
        let mut names = HashMap::new();
        names.insert(7, "crate".to_string());
        names.insert(9, "sky".to_string());
        let ranked = rank(&candidate, &entries, &names);
        assert_eq!(ranked[0].name, "crate");
        assert!((ranked[0].score - 1.0).abs() < 1e-4);
        assert_eq!(ranked[0].best_example, 1);
    }

    #[test]
    fn a_negative_that_matches_reduces_the_margin() {
        let candidate = embed(&patterned());
        let entries = vec![
            PoolEntry {
                example_id: 1,
                class_id: 7,
                role: "positive".into(),
                embedding: candidate.clone(),
            },
            PoolEntry {
                example_id: 2,
                class_id: 7,
                role: "negative".into(),
                embedding: candidate.clone(),
            },
        ];
        let mut names = HashMap::new();
        names.insert(7, "crate".to_string());
        let ranked = rank(&candidate, &entries, &names);
        assert_eq!(ranked[0].negatives, 1);
        assert!(ranked[0].score < 0.01, "score was {}", ranked[0].score);
    }

    #[test]
    fn a_pool_keeps_only_classes_above_their_threshold() {
        let candidate = embed(&patterned());
        let pool = Pool {
            classes: vec![
                ClassPool {
                    id: 1,
                    name: "crate".into(),
                    threshold: 0.9,
                    positives: vec![(1, candidate.clone())],
                    negatives: Vec::new(),
                },
                ClassPool {
                    id: 2,
                    name: "sky".into(),
                    threshold: 0.9,
                    positives: vec![(1, candidate.clone())],
                    negatives: Vec::new(),
                },
            ],
            examples: 2,
        };
        let hit = pool.classify(&candidate).expect("a perfect match should clear 0.9");
        assert_eq!(hit.name, "crate");
        assert!((hit.score - 1.0).abs() < 1e-4);

        let unreachable = Pool {
            classes: vec![ClassPool {
                id: 3,
                name: "crate".into(),
                threshold: 1.1,
                positives: vec![(1, candidate.clone())],
                negatives: Vec::new(),
            }],
            examples: 1,
        };
        assert!(unreachable.classify(&candidate).is_none());
    }

    #[test]
    fn an_empty_pool_matches_nothing() {
        let candidate = embed(&patterned());
        assert!(Pool::default().classify(&candidate).is_none());
    }

    #[test]
    fn a_class_without_positives_is_not_a_candidate() {
        let candidate = embed(&patterned());
        let entries = vec![PoolEntry {
            example_id: 1,
            class_id: 7,
            role: "negative".into(),
            embedding: candidate,
        }];
        let mut names = HashMap::new();
        names.insert(7, "crate".to_string());
        assert!(rank(&entries[0].embedding, &entries, &names).is_empty());
    }
}
