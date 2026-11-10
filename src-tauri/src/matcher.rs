//! Project-local matching.
//!
//! An example is reduced to a small, deterministic fingerprint: colour and
//! structure summarised on a fixed grid, centred, plus a global colour
//! histogram, and normalised to unit length. Stored as little-endian `f32` it
//! compares with a dot product, so a pool of a few thousand examples scores in
//! well under a millisecond.
//!
//! The fingerprint is deliberately not an ONNX encoder yet. It is a baseline
//! that works with no second model download, and the storage format (a blob and
//! the model name that produced it) does not change when one replaces it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use image::RgbaImage;
use serde::Serialize;

use crate::detector::Detection;

/// Bumped whenever the fingerprint changes: old rows then re-embed lazily
/// instead of being compared against a different vector space.
pub const EMBEDDING_MODEL: &str = "irecon-fingerprint-v4";

/// Live matching compares every proposal against every loaded example, so the
/// number of examples per role is bounded. The `matcher.max_examples` setting
/// sets that bound and starts at the old hardcoded 400; 0 means every example,
/// which costs matcher time on a large class. A cap keeps a spread of the role,
/// not the newest rows, because a captured dataset's newest rows are often one
/// scene repeated.
pub const DEFAULT_MAX_EXAMPLES: i64 = 400;
const MAX_EXAMPLES_LIMIT: i64 = 100_000;

/// SQLite treats `limit -1` as no limit, so zero maps onto the existing query
/// without a second statement. An unreadable setting falls back to every
/// example rather than to a silent cap the user did not ask for.
fn example_limit(conn: &rusqlite::Connection) -> i64 {
    let limit = conn
        .query_row(
            "select value from setting where key = 'matcher.max_examples'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|value| *value >= 0)
        .unwrap_or(DEFAULT_MAX_EXAMPLES);
    if limit == 0 {
        -1
    } else {
        limit.min(MAX_EXAMPLES_LIMIT)
    }
}

const CELLS: usize = 8;
/// Per cell: red ratio, green ratio, luma, edge strength, hue cosine, hue sine.
const FEATURES: usize = 6;
const HIST_BINS: usize = 64;
const HIST_WEIGHT: f32 = 1.0;
/// The fingerprint is an 8 by 8 cell summary, so a crop does not need every
/// pixel. Reading a bounded sample keeps a full-frame proposal from costing
/// milliseconds while still giving each cell a dozen samples per axis.
const MAX_SAMPLES_PER_AXIS: u32 = 96;
/// Negatives are a veto, not an equal vote. Subtracting the whole best negative
/// similarity erased correct matches whenever any background crop scored
/// highly, so only a quarter of it counts against the best positive.
const NEGATIVE_WEIGHT: f32 = 0.25;

pub const EMBEDDING_DIM: usize = CELLS * CELLS * FEATURES + HIST_BINS;

/// A proposal covering more than a quarter of the frame is scene-wide noise
/// (the detector sometimes calls the whole picture a train), not an object.
pub const MAX_PROPOSAL_AREA: f32 = 0.25;
/// A larger box is only allowed to replace a small seed when its own matcher
/// score is this close to the seed's. It is not a second, lower threshold: the
/// seed still has to clear the class threshold on its own.
const EXPANSION_SCORE_MARGIN: f32 = 0.05;
const EXPANSION_MAX_AREA: f32 = 0.08;
const DEDUPE_IOU: f32 = 0.3;

/// One example reduced to a unit vector.
///
/// Each cell contributes colour ratios, brightness, edge strength and a
/// saturation-weighted hue direction. Subtracting the average cell afterwards
/// is the important part: a crop and its background share most of that average,
/// so removing it leaves where this crop differs from its own mean, which is
/// what actually separates one object from another. Without it every pair of
/// crops scored around 0.99 and no threshold could tell them apart. Pixels are
/// sampled at no more than 96 per axis; the cell means do not need a full scan.
pub fn embed(image: &RgbaImage) -> Vec<f32> {
    embed_with(image.width(), image.height(), |x, y| image.get_pixel(x, y).0)
}

/// Embeds a normalized detection directly from a raw RGBA frame. The sampling
/// positions and vector match `embed` on the cropped image, but no crop buffer
/// is allocated or copied for the live matcher.
pub fn embed_detection(
    raw: &[u8],
    frame_width: u32,
    frame_height: u32,
    detection: &Detection,
) -> Option<Vec<f32>> {
    let (x0, y0, width, height) = crop_bounds(frame_width, frame_height, detection)?;
    let needed = frame_width as usize * frame_height as usize * 4;
    if raw.len() < needed {
        return None;
    }
    Some(embed_with(width, height, |x, y| {
        let index = (((y0 + y) * frame_width + (x0 + x)) * 4) as usize;
        [raw[index], raw[index + 1], raw[index + 2], raw[index + 3]]
    }))
}

/// The same rounded, normalized rectangle `capture::crop_rgba` uses, kept here
/// so the direct frame sampler reads exactly the pixels the cropped image would
/// have contained.
fn crop_bounds(
    frame_width: u32,
    frame_height: u32,
    detection: &Detection,
) -> Option<(u32, u32, u32, u32)> {
    if frame_width == 0 || frame_height == 0 {
        return None;
    }
    if detection.x < 0.0
        || detection.y < 0.0
        || detection.w <= 0.0
        || detection.h <= 0.0
        || detection.x + detection.w > 1.0001
        || detection.y + detection.h > 1.0001
    {
        return None;
    }
    let x0 = ((detection.x.clamp(0.0, 1.0) * frame_width as f32).round() as u32)
        .min(frame_width - 1);
    let y0 = ((detection.y.clamp(0.0, 1.0) * frame_height as f32).round() as u32)
        .min(frame_height - 1);
    let x1 = (((detection.x + detection.w).clamp(0.0, 1.0) * frame_width as f32).round() as u32)
        .min(frame_width);
    let y1 =
        (((detection.y + detection.h).clamp(0.0, 1.0) * frame_height as f32).round() as u32)
            .min(frame_height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

fn embed_with<F>(width: u32, height: u32, mut pixel: F) -> Vec<f32>
where
    F: FnMut(u32, u32) -> [u8; 4],
{
    let width = width.max(1);
    let height = height.max(1);
    let step_x = ((width + MAX_SAMPLES_PER_AXIS - 1) / MAX_SAMPLES_PER_AXIS).max(1);
    let step_y = ((height + MAX_SAMPLES_PER_AXIS - 1) / MAX_SAMPLES_PER_AXIS).max(1);
    let cell_count = CELLS * CELLS;
    let mut sums = vec![[0f32; FEATURES]; cell_count];
    let mut counts = vec![0f32; cell_count];
    let mut histogram = vec![0f32; HIST_BINS];

    for y in (0..height).step_by(step_y as usize) {
        for x in (0..width).step_by(step_x as usize) {
            let raw = pixel(x, y);
            let [r, g, b, _] = raw;
            let (r, g, b) = (r as f32 + 1.0, g as f32 + 1.0, b as f32 + 1.0);
            let total = r + g + b;

            let cell_x = (x * CELLS as u32 / width).min(CELLS as u32 - 1) as usize;
            let cell_y = (y * CELLS as u32 / height).min(CELLS as u32 - 1) as usize;
            let cell = cell_y * CELLS + cell_x;

            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let right = pixel((x + 1).min(width - 1), y);
            let down = pixel(x, (y + 1).min(height - 1));
            let right_luma =
                0.2126 * right[0] as f32 + 0.7152 * right[1] as f32 + 0.0722 * right[2] as f32;
            let down_luma =
                0.2126 * down[0] as f32 + 0.7152 * down[1] as f32 + 0.0722 * down[2] as f32;
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

            let bin = (raw[0] as usize / 64).min(3) * 16
                + (raw[1] as usize / 64).min(3) * 4
                + (raw[2] as usize / 64).min(3);
            histogram[bin] += 1.0;
        }
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

    // Mean-centring removes each crop's average colour, which is what gives the
    // structural features their contrast, but it also throws away the strongest
    // signal for a white-and-red robot against a black-and-pink weapon. Put
    // colour back as a coarse global histogram and weight it against structure.
    normalize(&mut histogram);
    for value in histogram {
        centred.push(value * HIST_WEIGHT);
    }
    normalize(&mut centred);
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
    /// Minimum score for a proposal to be drawn or harvested. Zero means off.
    pub floor: f32,
}

pub struct ClassPool {
    pub id: i64,
    pub name: String,
    pub color: String,
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
        (positive - NEGATIVE_WEIGHT * negative).clamp(0.0, 1.0)
    }
}

#[derive(Clone)]
pub struct LiveMatch {
    pub class_id: i64,
    pub name: String,
    pub score: f32,
    pub threshold: f32,
}

fn area(detection: &Detection) -> f32 {
    detection.w * detection.h
}

pub fn encloses(outer: &Detection, inner: &Detection) -> bool {
    let x = inner.x + inner.w / 2.0;
    let y = inner.y + inner.h / 2.0;
    x >= outer.x && x <= outer.x + outer.w && y >= outer.y && y <= outer.y + outer.h
}

fn overlap(left: &Detection, right: &Detection) -> f32 {
    let x0 = left.x.max(right.x);
    let y0 = left.y.max(right.y);
    let x1 = (left.x + left.w).min(right.x + right.w);
    let y1 = (left.y + left.h).min(right.y + right.h);
    let intersection = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let union = area(left) + area(right) - intersection;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

/// Turns per-proposal scores into the boxes to draw.
///
/// A proposal only seeds a match when it clears both the detector's primary
/// floor and the class threshold. A tiny seed (a head or torso slice) is then
/// allowed to grow to a larger, close-scoring proposal that encloses it. This
/// keeps the rock and wall proposals that appear below the primary detector
/// floor from becoming matches on their own, while still giving a detected bot
/// its full body when one was proposed.
///
/// Seeds that land on the same object are deduplicated by overlap.
pub fn refine(
    scored: &[(Detection, LiveMatch)],
    primary_detector: f32,
    expansion_detector: f32,
) -> Vec<(Detection, LiveMatch)> {
    let mut refined: Vec<(Detection, LiveMatch)> = Vec::new();
    for (seed, hit) in scored {
        if seed.score < primary_detector || hit.score < hit.threshold {
            continue;
        }

        let seed_area = area(seed);
        let mut chosen = seed.clone();
        let mut chosen_area = seed_area;
        if seed_area < 0.02 {
            for (candidate, candidate_hit) in scored {
                if candidate_hit.class_id != hit.class_id
                    || candidate.score < expansion_detector
                    || candidate_hit.score + EXPANSION_SCORE_MARGIN < hit.score
                {
                    continue;
                }
                let candidate_area = area(candidate);
                if candidate_area < seed_area * 1.5
                    || candidate_area > seed_area * 40.0
                    || candidate_area > EXPANSION_MAX_AREA
                    || candidate.w < seed.w * 1.1
                    || candidate.h < seed.h * 1.1
                    || !encloses(candidate, seed)
                {
                    continue;
                }
                if candidate_area > chosen_area {
                    chosen = candidate.clone();
                    chosen_area = candidate_area;
                }
            }
        }

        refined.push((
            Detection {
                label: hit.name.clone(),
                score: hit.score,
                x: chosen.x,
                y: chosen.y,
                w: chosen.w,
                h: chosen.h,
            },
            hit.clone(),
        ));
    }

    let mut deduped: Vec<(Detection, LiveMatch)> = Vec::new();
    for (detection, hit) in refined {
        let found = deduped.iter_mut().find(|(existing, existing_hit)| {
            existing_hit.class_id == hit.class_id && overlap(existing, &detection) > DEDUPE_IOU
        });
        match found {
            Some((existing, existing_hit)) => {
                if area(&detection) > area(existing) {
                    *existing = detection;
                    *existing_hit = hit;
                }
            }
            None => deduped.push((detection, hit)),
        }
    }
    deduped
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

/// A capped role keeps a spread of its rows instead of the newest ones. The
/// selection is cached by the role's row count and newest id, so the two-second
/// pool reload recomputes it only after examples were added or removed.
struct CachedSelection {
    count: i64,
    newest: i64,
    ids: Vec<i64>,
}

fn selection_cache() -> &'static Mutex<HashMap<(i64, String, usize), CachedSelection>> {
    static CACHE: OnceLock<Mutex<HashMap<(i64, String, usize), CachedSelection>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Selection compares rows through a handful of fixed random directions rather
/// than all 448 values. That keeps a recompute in the low milliseconds, which
/// matters because it runs on the inference thread.
const PROJECTION_WIDTH: usize = 32;
/// Twice the cap is sampled evenly across the role, then narrowed by distance.
const CANDIDATE_FACTOR: usize = 2;
/// `id in (...)` is sent in chunks, so a large cap cannot hit a bind limit.
const ID_CHUNK: usize = 400;

fn projection_directions(dim: usize, width: usize) -> Vec<Vec<f32>> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut directions = Vec::with_capacity(width);
    for _ in 0..width {
        let mut direction = vec![0f32; dim];
        for value in direction.iter_mut() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *value = (state >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
        }
        normalize(&mut direction);
        directions.push(direction);
    }
    directions
}

/// Greedy farthest-point sampling: every pick is the row least similar to
/// everything chosen so far, so the cap covers the candidate set instead of
/// clustering on one scene, angle or colour.
fn farthest_point(rows: &[(i64, Vec<f32>)], cap: usize) -> Vec<i64> {
    if rows.len() <= cap {
        return rows.iter().map(|(id, _)| *id).collect();
    }
    let dim = rows[0].1.len();
    let directions = projection_directions(dim, PROJECTION_WIDTH.min(dim));
    let projected: Vec<Vec<f32>> = rows
        .iter()
        .map(|(_, embedding)| {
            let mut point: Vec<f32> = directions
                .iter()
                .map(|direction| cosine(embedding, direction))
                .collect();
            normalize(&mut point);
            point
        })
        .collect();

    // Start at the candidate closest to the centre of the set, so the first
    // pick is not an arbitrary edge.
    let mut mean = vec![0f32; projected[0].len()];
    for point in &projected {
        for (slot, value) in point.iter().enumerate() {
            mean[slot] += value;
        }
    }
    for value in mean.iter_mut() {
        *value /= projected.len() as f32;
    }
    normalize(&mut mean);
    let mut start = 0;
    let mut closest = f32::NEG_INFINITY;
    for (index, point) in projected.iter().enumerate() {
        let similarity = cosine(&mean, point);
        if similarity > closest {
            closest = similarity;
            start = index;
        }
    }

    let mut picked = vec![false; projected.len()];
    let mut nearest = vec![-1.0f32; projected.len()];
    picked[start] = true;
    let mut chosen = Vec::with_capacity(cap);
    chosen.push(rows[start].0);
    for (index, point) in projected.iter().enumerate() {
        nearest[index] = cosine(point, &projected[start]);
    }
    while chosen.len() < cap {
        let mut pick = 0;
        let mut lowest = f32::INFINITY;
        for (index, value) in nearest.iter().enumerate() {
            if !picked[index] && *value < lowest {
                lowest = *value;
                pick = index;
            }
        }
        picked[pick] = true;
        chosen.push(rows[pick].0);
        for (index, point) in projected.iter().enumerate() {
            if picked[index] {
                continue;
            }
            let similarity = cosine(point, &projected[pick]);
            if similarity > nearest[index] {
                nearest[index] = similarity;
            }
        }
    }
    chosen.sort_unstable();
    chosen
}

fn load_all(
    conn: &rusqlite::Connection,
    class_id: i64,
    role: &str,
) -> Result<Vec<(i64, Vec<f32>)>, String> {
    let mut statement = conn
        .prepare(
            "select id, embedding
             from example
             where class_id = ?1
               and role = ?2
               and embedding is not null
               and embedding_model = ?3
             order by id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(rusqlite::params![class_id, role, EMBEDDING_MODEL], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut loaded = Vec::new();
    for row in rows {
        let (example_id, blob) = row.map_err(|error| error.to_string())?;
        if let Some(embedding) = from_blob(&blob) {
            loaded.push((example_id, embedding));
        }
    }
    Ok(loaded)
}

fn load_selected(
    conn: &rusqlite::Connection,
    class_id: i64,
    role: &str,
    ids: &[i64],
) -> Result<Vec<(i64, Vec<f32>)>, String> {
    let mut loaded = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(ID_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut statement = conn
            .prepare(&format!(
                "select id, embedding
                 from example
                 where class_id = ? and role = ? and embedding_model = ?
                   and id in ({placeholders})
                 order by id"
            ))
            .map_err(|error| error.to_string())?;
        let mut values: Vec<rusqlite::types::Value> = Vec::with_capacity(chunk.len() + 3);
        values.push(class_id.into());
        values.push(rusqlite::types::Value::Text(role.to_string()));
        values.push(rusqlite::types::Value::Text(EMBEDDING_MODEL.to_string()));
        values.extend(chunk.iter().copied().map(rusqlite::types::Value::from));
        let rows = statement
            .query_map(rusqlite::params_from_iter(values.iter()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (example_id, blob) = row.map_err(|error| error.to_string())?;
            if let Some(embedding) = from_blob(&blob) {
                loaded.push((example_id, embedding));
            }
        }
    }
    Ok(loaded)
}

fn select_diverse(
    conn: &rusqlite::Connection,
    class_id: i64,
    role: &str,
    cap: usize,
) -> Result<Vec<i64>, String> {
    let mut ids = Vec::new();
    {
        let mut statement = conn
            .prepare(
                "select id from example
                 where class_id = ?1 and role = ?2
                   and embedding is not null and embedding_model = ?3
                 order by id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(rusqlite::params![class_id, role, EMBEDDING_MODEL], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            ids.push(row.map_err(|error| error.to_string())?);
        }
    }
    if ids.len() <= cap {
        return Ok(ids);
    }
    let target = cap
        .saturating_mul(CANDIDATE_FACTOR)
        .min(ids.len())
        .max(cap);
    let stride = ids.len() as f64 / target as f64;
    let mut candidate_ids = Vec::with_capacity(target);
    for slot in 0..target {
        let index = ((slot as f64) * stride) as usize;
        candidate_ids.push(ids[index.min(ids.len() - 1)]);
    }
    let rows = load_selected(conn, class_id, role, &candidate_ids)?;
    Ok(farthest_point(&rows, cap))
}

fn load_role(
    conn: &rusqlite::Connection,
    class_id: i64,
    role: &str,
    limit: i64,
) -> Result<Vec<(i64, Vec<f32>)>, String> {
    if limit < 0 {
        return load_all(conn, class_id, role);
    }
    let cap = limit as usize;
    let (count, newest): (i64, Option<i64>) = conn
        .query_row(
            "select count(*), max(id) from example where class_id = ?1 and role = ?2",
            rusqlite::params![class_id, role],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let newest = newest.unwrap_or(0);
    if count as usize <= cap {
        return load_all(conn, class_id, role);
    }
    let key = (class_id, role.to_string(), cap);
    let cached = selection_cache()
        .lock()
        .map_err(|_| "The selection cache is unavailable".to_string())?
        .get(&key)
        .filter(|entry| entry.count == count && entry.newest == newest)
        .map(|entry| entry.ids.clone());
    let ids = match cached {
        Some(ids) => ids,
        None => {
            let ids = select_diverse(conn, class_id, role, cap)?;
            selection_cache()
                .lock()
                .map_err(|_| "The selection cache is unavailable".to_string())?
                .insert(
                    key,
                    CachedSelection {
                        count,
                        newest,
                        ids: ids.clone(),
                    },
                );
            ids
        }
    };
    load_selected(conn, class_id, role, &ids)
}

/// Loads each class and a spread of its matchable examples for one project, up
/// to `matcher.max_examples` per role. Zero loads everything.
///
/// The cap used to keep the newest rows, which on a captured dataset often
/// means one scene. It now keeps a diverse selection: an evenly spaced
/// candidate set twice the cap, narrowed by farthest-point sampling in a
/// projected space. Selection is cached, so the two-second reload does not pay
/// for it unless examples changed.
pub fn load_pool(conn: &rusqlite::Connection, project_id: i64) -> Result<Pool, String> {
    let mut classes = conn
        .prepare(
            "select id, name, threshold, color
             from class
             where project_id = ?1
             order by id",
        )
        .map_err(|e| e.to_string())?;
    let classes = classes
        .query_map(rusqlite::params![project_id], |row| {
            Ok(ClassPool {
                id: row.get(0)?,
                name: row.get(1)?,
                threshold: row.get(2)?,
                color: row.get(3)?,
                positives: Vec::new(),
                negatives: Vec::new(),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut pool = Pool {
        classes,
        ..Pool::default()
    };

    let limit = example_limit(conn);
    for index in 0..pool.classes.len() {
        let class_id = pool.classes[index].id;
        let positives = load_role(conn, class_id, "positive", limit)?;
        let negatives = load_role(conn, class_id, "negative", limit)?;
        pool.examples += positives.len() + negatives.len();
        pool.classes[index].positives = positives;
        pool.classes[index].negatives = negatives
            .into_iter()
            .map(|(_, embedding)| embedding)
            .collect();
    }

    pool.floor = conn
        .query_row(
            "select match_floor from project where id = ?1",
            rusqlite::params![project_id],
            |row| row.get::<_, f32>(0),
        )
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
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
    /// Best positive minus a quarter of the best negative, clamped to zero.
    /// With no negatives this is simply the best similarity.
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
            let margin = tally.best_positive - NEGATIVE_WEIGHT * negative;
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
    fn a_frame_region_matches_a_cropped_image() {
        let mut frame = RgbaImage::from_pixel(240, 160, image::Rgba([20, 24, 32, 255]));
        for y in 40..120 {
            for x in 50..150 {
                frame.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        ((x * 5 + y * 3) % 256) as u8,
                        ((x * 7 + y * 11) % 256) as u8,
                        ((x * 13 + y * 2) % 256) as u8,
                        255,
                    ]),
                );
            }
        }
        let detection = Detection {
            label: "object".into(),
            score: 0.9,
            x: 0.08,
            y: 0.17,
            w: 0.47,
            h: 0.63,
        };
        let crop = crate::capture::crop_region(frame.as_raw(), 240, 160, &detection)
            .expect("the detection should crop");
        let from_image = embed(&crop);
        let from_frame = embed_detection(frame.as_raw(), 240, 160, &detection)
            .expect("the detection should embed from the frame");
        assert_eq!(from_image, from_frame);
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
        // A perfect negative subtracts its 0.25 weight, not the whole score.
        assert!(
            ranked[0].score > 0.5 && ranked[0].score < 0.8,
            "score was {}",
            ranked[0].score
        );
    }

    #[test]
    fn a_pool_keeps_only_classes_above_their_threshold() {
        let candidate = embed(&patterned());
        let pool = Pool {
            classes: vec![
                ClassPool {
                    id: 1,
                    name: "crate".into(),
                    color: "#fff".into(),
                    threshold: 0.9,
                    positives: vec![(1, candidate.clone())],
                    negatives: Vec::new(),
                },
                ClassPool {
                    id: 2,
                    name: "sky".into(),
                    color: "#fff".into(),
                    threshold: 0.9,
                    positives: vec![(1, candidate.clone())],
                    negatives: Vec::new(),
                },
            ],
            examples: 2,
            floor: 0.0,
        };
        let hit = pool.classify(&candidate).expect("a perfect match should clear 0.9");
        assert_eq!(hit.name, "crate");
        assert!((hit.score - 1.0).abs() < 1e-4);

        let unreachable = Pool {
            classes: vec![ClassPool {
                id: 3,
                name: "crate".into(),
                color: "#fff".into(),
                threshold: 1.1,
                positives: vec![(1, candidate.clone())],
                negatives: Vec::new(),
            }],
            examples: 1,
            floor: 0.0,
        };
        assert!(unreachable.classify(&candidate).is_none());
    }

    #[test]
    fn an_empty_pool_matches_nothing() {
        let candidate = embed(&patterned());
        assert!(Pool::default().classify(&candidate).is_none());
    }

    #[test]
    fn a_small_match_grows_to_an_enclosing_body_proposal() {
        let hit = |score: f32| LiveMatch {
            class_id: 1,
            name: "robot".into(),
            score,
            threshold: 0.45,
        };
        let seed = Detection {
            label: "robot".into(),
            score: 0.5,
            x: 0.50,
            y: 0.40,
            w: 0.02,
            h: 0.03,
        };
        let body = Detection {
            label: "robot".into(),
            score: 0.3,
            x: 0.47,
            y: 0.35,
            w: 0.09,
            h: 0.15,
        };
        let refined = refine(&[(seed, hit(0.50)), (body, hit(0.47))], 0.2, 0.1);
        assert_eq!(refined.len(), 1);
        assert!((refined[0].0.w - 0.09).abs() < 1e-6);
        assert!((refined[0].0.h - 0.15).abs() < 1e-6);
    }

    #[test]
    fn a_low_confidence_proposal_cannot_seed_a_match() {
        let rock = Detection {
            label: "robot".into(),
            score: 0.1,
            x: 0.2,
            y: 0.2,
            w: 0.2,
            h: 0.2,
        };
        let hit = LiveMatch {
            class_id: 1,
            name: "robot".into(),
            score: 0.9,
            threshold: 0.45,
        };
        assert!(refine(&[(rock, hit)], 0.2, 0.1).is_empty());
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
#[cfg(test)]
mod floor_tests {
    use super::{cosine, load_pool, to_blob, EMBEDDING_DIM, EMBEDDING_MODEL};

    fn schema(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "create table project (id integer primary key, match_floor real not null default 0.0);
             create table class (id integer primary key, project_id integer not null, name text not null, color text not null default '#fff', threshold real not null);
             create table example (id integer primary key, class_id integer not null, role text not null default 'positive', embedding blob, embedding_model text);
             create table setting (key text primary key, value text not null);",
        )
        .expect("schema");
    }

    #[test]
    fn a_pool_reads_and_clamps_the_project_match_floor() {
        let conn = rusqlite::Connection::open_in_memory().expect("memory db");
        schema(&conn);
        conn.execute("insert into project (id, match_floor) values (1, 0.7)", [])
            .expect("project");
        let pool = load_pool(&conn, 1).expect("pool");
        assert!((pool.floor - 0.7).abs() < 1e-6, "floor was {}", pool.floor);

        conn.execute("update project set match_floor = 4.0 where id = 1", [])
            .expect("floor update");
        assert_eq!(load_pool(&conn, 1).expect("pool").floor, 1.0);
    }

    #[test]
    fn a_pool_reads_the_example_cap_setting() {
        let conn = rusqlite::Connection::open_in_memory().expect("memory db");
        schema(&conn);
        conn.execute(
            "insert into class (id, project_id, name, threshold) values (1, 1, 'Enemy', 0.4)",
            [],
        )
        .expect("class");
        let blob = to_blob(&vec![0.5_f32; EMBEDDING_DIM]);
        for _ in 0..12 {
            conn.execute(
                "insert into example (class_id, role, embedding, embedding_model)
                 values (1, 'positive', ?1, ?2)",
                rusqlite::params![&blob, EMBEDDING_MODEL],
            )
            .expect("positive");
        }
        for _ in 0..3 {
            conn.execute(
                "insert into example (class_id, role, embedding, embedding_model)
                 values (1, 'negative', ?1, ?2)",
                rusqlite::params![&blob, EMBEDDING_MODEL],
            )
            .expect("negative");
        }

        // The default of 400 is above this small class, so every example loads.
        let all = load_pool(&conn, 1).expect("pool");
        assert_eq!(all.classes[0].positives.len(), 12);
        assert_eq!(all.classes[0].negatives.len(), 3);
        assert_eq!(all.examples, 15);

        conn.execute(
            "insert into setting (key, value) values ('matcher.max_examples', '5')",
            [],
        )
        .expect("setting");
        let capped = load_pool(&conn, 1).expect("pool");
        assert_eq!(capped.classes[0].positives.len(), 5);
        assert_eq!(capped.classes[0].negatives.len(), 3);
        assert_eq!(capped.examples, 8);

        // Zero turns the cap off again.
        conn.execute(
            "update setting set value = '0' where key = 'matcher.max_examples'",
            [],
        )
        .expect("setting");
        assert_eq!(load_pool(&conn, 1).expect("pool").examples, 15);
    }

    fn axis(index: usize) -> Vec<f32> {
        let mut vector = vec![0.0_f32; EMBEDDING_DIM];
        vector[index] = 1.0;
        vector
    }

    #[test]
    fn a_capped_pool_keeps_a_spread_instead_of_the_newest_rows() {
        let conn = rusqlite::Connection::open_in_memory().expect("memory db");
        schema(&conn);
        conn.execute(
            "insert into class (id, project_id, name, threshold) values (901, 1, 'Enemy', 0.4)",
            [],
        )
        .expect("class");
        let oldest = to_blob(&axis(0));
        let newest = to_blob(&axis(1));
        for _ in 0..30 {
            conn.execute(
                "insert into example (class_id, role, embedding, embedding_model)
                 values (901, 'positive', ?1, ?2)",
                rusqlite::params![&oldest, EMBEDDING_MODEL],
            )
            .expect("old positive");
        }
        for _ in 0..10 {
            conn.execute(
                "insert into example (class_id, role, embedding, embedding_model)
                 values (901, 'positive', ?1, ?2)",
                rusqlite::params![&newest, EMBEDDING_MODEL],
            )
            .expect("new positive");
        }
        conn.execute(
            "insert into setting (key, value) values ('matcher.max_examples', '10')",
            [],
        )
        .expect("setting");

        let pool = load_pool(&conn, 1).expect("pool");
        let positives = &pool.classes[0].positives;
        assert_eq!(positives.len(), 10);
        // The newest ten rows are all one vector. A spread keeps both clusters.
        let kept_oldest = positives
            .iter()
            .any(|(_, embedding)| cosine(embedding, &axis(0)) > 0.99);
        let kept_newest = positives
            .iter()
            .any(|(_, embedding)| cosine(embedding, &axis(1)) > 0.99);
        assert!(
            kept_oldest && kept_newest,
            "kept oldest: {kept_oldest}, kept newest: {kept_newest}"
        );
    }
}
