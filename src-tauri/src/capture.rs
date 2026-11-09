use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use xcap::Monitor;

use crate::detector::Detection;
use crate::matcher;
use crate::tracker::{TrackedBox, Tracker};
use crate::AppState;

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const REPORT_INTERVAL: Duration = Duration::from_millis(500);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(800);
const PREVIEW_WIDTH: u32 = 640;
const SCORE_THRESHOLD: f32 = 0.35;
/// How far under a class's threshold a proposal still counts as a candidate.
/// Wider and the queue fills with junk; narrower and it only sees near misses
/// that were about to match anyway.
const HARVEST_BAND: f32 = 0.12;
/// Candidates per capture session, so a bad pool cannot fill the disk.
const HARVEST_CAP: u32 = 40;
const HARVEST_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStats {
    pub running: bool,
    pub monitor: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub capture_ms: f32,
    pub inference_ms: f32,
    pub detections: u32,
    pub pool_classes: u32,
    pub pool_examples: u32,
    pub frames: u64,
    pub errors: u64,
    pub elapsed_s: f32,
    pub model: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FramePayload {
    pub data: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionPayload {
    pub width: u32,
    pub height: u32,
    pub inference_ms: f32,
    pub boxes: Vec<TrackedBox>,
}

pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
}

pub(crate) fn pick_monitor() -> Result<Monitor, String> {
    let monitors = Monitor::all().map_err(|e| format!("Could not list displays: {e}"))?;
    if monitors.is_empty() {
        return Err("No displays found".into());
    }
    let index = monitors
        .iter()
        .position(|monitor| monitor.is_primary().unwrap_or(false))
        .unwrap_or(0);
    Ok(monitors[index].clone())
}

#[tauri::command]
pub fn capture_status(state: tauri::State<'_, AppState>) -> Option<CaptureStats> {
    state.stats.lock().ok().and_then(|guard| guard.clone())
}

#[tauri::command]
pub fn capture_start(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<(), String> {
    let mut slot = state.capture.lock().map_err(|e| e.to_string())?;
    if slot.is_some() {
        return Ok(());
    }

    // The handle is not `Send` on every platform (on Windows `xcap` holds a raw
    // `HMONITOR`), so it is picked again inside the capture thread instead of
    // being moved into it. The label decides which display the thread reads.
    let label = pick_monitor()?
        .name()
        .unwrap_or_else(|_| "Primary display".to_string());

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();

    // The pool belongs to one project. With no explicit choice, the most
    // recently created project is the one being labelled.
    let project = project_id.or_else(|| {
        state.db.lock().ok().and_then(|conn| {
            conn.query_row(
                "select id from project order by created_at desc, id desc limit 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .ok()
        })
    });
    let stats = state.stats.clone();
    let detector = state.detector.clone();
    let app_handle = app.clone();

    thread::Builder::new()
        .name("irecon-capture".into())
        .spawn(move || {
            let monitor = match pick_monitor() {
                Ok(monitor) => monitor,
                Err(error) => {
                    if let Ok(mut guard) = stats.lock() {
                        *guard = Some(CaptureStats {
                            running: false,
                            monitor: label.clone(),
                            width: 0,
                            height: 0,
                            fps: 0.0,
                            capture_ms: 0.0,
                            inference_ms: 0.0,
                            detections: 0,
                            pool_classes: 0,
                            pool_examples: 0,
                            frames: 0,
                            errors: 0,
                            elapsed_s: 0.0,
                            model: None,
                            note: Some(error),
                        });
                    }
                    return;
                }
            };
            let mut frames: u64 = 0;
            let mut errors: u64 = 0;
            let mut note: Option<String> = None;
            let mut size = (0u32, 0u32);
            let mut inference_ms = 0f32;
            let mut detections = 0u32;
            let mut model_name: Option<String> = None;
            let started = Instant::now();
            let mut last_report = Instant::now();
            let mut last_preview = Instant::now() - PREVIEW_INTERVAL;
            let mut window_frames: u64 = 0;
            let mut window_cost = Duration::ZERO;
            let mut pool = matcher::Pool::default();
            let mut pool_at = Instant::now() - Duration::from_secs(5);
            let mut tracker = Tracker::new();
            let mut harvested: u32 = 0;
            let mut last_harvest = Instant::now() - HARVEST_INTERVAL;
            let mut harvest_root: Option<PathBuf> = None;

            while !stop_thread.load(Ordering::Relaxed) {
                let tick = Instant::now();

                // Reload on a slow tick, so a new example starts scoring without
                // restarting the capture thread.
                if pool_at.elapsed() >= Duration::from_secs(2) {
                    pool_at = Instant::now();
                    if let Some(id) = project {
                        let owned = app_handle.state::<AppState>();
                        let guard = owned.db.lock();
                        let loaded = match guard.as_ref() {
                            Ok(conn) => matcher::load_pool(conn, id),
                            Err(error) => Err(error.to_string()),
                        };
                        drop(guard);
                        drop(owned);
                        match loaded {
                            Ok(next) => pool = next,
                            Err(error) => note = Some(error),
                        }
                    }
                }

                match monitor.capture_image() {
                    Ok(image) => {
                        frames += 1;
                        window_frames += 1;
                        size = (image.width(), image.height());
                        window_cost += tick.elapsed();

                        if last_preview.elapsed() >= PREVIEW_INTERVAL {
                            let (bytes, width, height) =
                                bmp_preview(image.as_raw(), image.width(), image.height());
                            let payload = FramePayload {
                                data: format!(
                                    "data:image/bmp;base64,{}",
                                    base64::engine::general_purpose::STANDARD.encode(bytes)
                                ),
                                width,
                                height,
                            };
                            let _ = app_handle.emit("spike:frame", payload);
                            last_preview = Instant::now();
                        }

                        if let Ok(mut guard) = detector.lock() {
                            if let Some(model) = guard.as_mut() {
                                model_name = Some(model.describe());
                                match model.analyse(
                                    image.as_raw(),
                                    image.width(),
                                    image.height(),
                                    SCORE_THRESHOLD,
                                ) {
                                    Ok((boxes, elapsed)) => {
                                        inference_ms = elapsed;
                                        // Every proposal is cropped from the full frame and
                                        // scored against the project pool. A class with no
                                        // examples cannot match, so an empty project draws
                                        // nothing rather than unlabelled guesses.
                                        let matched: Vec<Detection> = boxes
                                            .into_iter()
                                            .filter_map(|detection| {
                                                let crop = crop_region(
                                                    image.as_raw(),
                                                    image.width(),
                                                    image.height(),
                                                    &detection,
                                                )?;
                                                let candidate = matcher::embed(&crop);
                                                if let Some(hit) = pool.classify(&candidate) {
                                                    return Some(Detection {
                                                        label: hit.name,
                                                        score: hit.score,
                                                        ..detection
                                                    });
                                                }

                                                // A proposal just under its class's
                                                // threshold is the one worth asking
                                                // about, so it is saved for review.
                                                if harvested < HARVEST_CAP
                                                    && last_harvest.elapsed() >= HARVEST_INTERVAL
                                                {
                                                    if let Some(guess) = pool.best(&candidate) {
                                                        if guess.score
                                                            >= guess.threshold - HARVEST_BAND
                                                            && guess.score < guess.threshold
                                                        {
                                                            if let Some(project) = project {
                                                                if save_candidate(
                                                                    &app_handle,
                                                                    project,
                                                                    guess.class_id,
                                                                    guess.score,
                                                                    &crop,
                                                                    &mut harvest_root,
                                                                ) {
                                                                    harvested += 1;
                                                                    last_harvest = Instant::now();
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                None
                                            })
                                            .collect();
                                        let tracked = tracker.update(matched);
                                        detections = tracked.len() as u32;
                                        if pool.examples == 0 && note.is_none() {
                                            note = Some(
                                                "No examples in this project yet, so no proposal can match a class."
                                                    .to_string(),
                                            );
                                        }
                                        let _ = app_handle.emit(
                                            "spike:detections",
                                            DetectionPayload {
                                                width: image.width(),
                                                height: image.height(),
                                                inference_ms: elapsed,
                                                boxes: tracked,
                                            },
                                        );
                                    }
                                    Err(error) => {
                                        note = Some(error);
                                    }
                                }
                            } else {
                                model_name = None;
                                detections = 0;
                            }
                        }
                    }
                    Err(error) => {
                        errors += 1;
                        if note.is_none() {
                            note = Some(error.to_string());
                        }
                    }
                }

                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }

                if last_report.elapsed() >= REPORT_INTERVAL {
                    let seconds = last_report.elapsed().as_secs_f32().max(0.001);
                    let snapshot = CaptureStats {
                        running: true,
                        monitor: label.clone(),
                        width: size.0,
                        height: size.1,
                        fps: window_frames as f32 / seconds,
                        capture_ms: if window_frames > 0 {
                            (window_cost.as_secs_f32() * 1000.0) / window_frames as f32
                        } else {
                            0.0
                        },
                        inference_ms,
                        detections,
                        pool_classes: pool.classes.len() as u32,
                        pool_examples: pool.examples as u32,
                        frames,
                        errors,
                        elapsed_s: started.elapsed().as_secs_f32(),
                        model: model_name.clone(),
                        note: note.clone(),
                    };
                    if let Ok(mut guard) = stats.lock() {
                        *guard = Some(snapshot.clone());
                    }
                    let _ = app_handle.emit("spike:stats", snapshot);
                    window_frames = 0;
                    window_cost = Duration::ZERO;
                    last_report = Instant::now();
                }

                let spent = tick.elapsed();
                if spent < FRAME_INTERVAL {
                    thread::sleep(FRAME_INTERVAL - spent);
                }
            }

            let final_stats = CaptureStats {
                running: false,
                monitor: label,
                width: size.0,
                height: size.1,
                fps: 0.0,
                capture_ms: 0.0,
                inference_ms,
                detections: 0,
                pool_classes: pool.classes.len() as u32,
                pool_examples: pool.examples as u32,
                frames,
                errors,
                elapsed_s: started.elapsed().as_secs_f32(),
                model: model_name,
                note,
            };
            if let Ok(mut guard) = stats.lock() {
                *guard = Some(final_stats.clone());
            }
            let _ = app_handle.emit("spike:stats", final_stats);
        })
        .map_err(|e| format!("Could not start the capture thread: {e}"))?;

    *slot = Some(CaptureHandle { stop });
    Ok(())
}

#[tauri::command]
pub fn capture_stop(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut slot = state.capture.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = slot.take() {
        handle.stop.store(true, Ordering::Relaxed);
    }
    if let Ok(mut guard) = state.stats.lock() {
        if let Some(stats) = guard.as_mut() {
            stats.running = false;
            let _ = app.emit("spike:stats", stats.clone());
        }
    }
    Ok(())
}

/// Writes one uncertain crop into the project's candidates folder and records
/// it, so the review screen can ask about it later. Returns whether it landed.
fn save_candidate(
    app: &AppHandle,
    project_id: i64,
    class_id: i64,
    score: f32,
    crop: &image::RgbaImage,
    root: &mut Option<PathBuf>,
) -> bool {
    if root.is_none() {
        *root = crate::label::project_folder(app, project_id)
            .ok()
            .map(|(_, folder)| folder);
    }
    let Some(folder) = root.as_ref() else {
        return false;
    };
    let candidates = folder.join("candidates");
    if std::fs::create_dir_all(&candidates).is_err() {
        return false;
    }
    let path = candidates.join(format!("{}-{class_id}.png", crate::label::now_millis()));
    if crop
        .save_with_format(&path, image::ImageFormat::Png)
        .is_err()
    {
        return false;
    }

    let state = app.state::<AppState>();
    let Ok(conn) = state.db.lock() else {
        return false;
    };
    conn.execute(
        "insert into candidate (project_id, class_id, path, score, state, created_at)
         values (?1, ?2, ?3, ?4, 'pending', datetime('now'))",
        rusqlite::params![
            project_id,
            class_id,
            path.to_string_lossy().to_string(),
            score
        ],
    )
    .is_ok()
}

/// Crops a normalized detection out of a raw RGBA frame.
fn crop_region(
    raw: &[u8],
    width: u32,
    height: u32,
    detection: &Detection,
) -> Option<image::RgbaImage> {
    if width == 0 || height == 0 {
        return None;
    }
    let x0 = ((detection.x.clamp(0.0, 1.0) * width as f32).round() as u32).min(width - 1);
    let y0 = ((detection.y.clamp(0.0, 1.0) * height as f32).round() as u32).min(height - 1);
    let x1 = (((detection.x + detection.w).clamp(0.0, 1.0) * width as f32).round() as u32)
        .min(width);
    let y1 = (((detection.y + detection.h).clamp(0.0, 1.0) * height as f32).round() as u32)
        .min(height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut crop = image::RgbaImage::new(x1 - x0, y1 - y0);
    for y in 0..(y1 - y0) {
        for x in 0..(x1 - x0) {
            let source = (((y0 + y) * width + (x0 + x)) * 4) as usize;
            if source + 3 >= raw.len() {
                return None;
            }
            crop.put_pixel(
                x,
                y,
                image::Rgba([raw[source], raw[source + 1], raw[source + 2], 255]),
            );
        }
    }
    Some(crop)
}

/// A spike-only preview encoder. It downsamples the frame by sampling and writes a
/// 32-bit BMP, so the preview needs no image crate and no PNG encoder.
fn bmp_preview(raw: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    if width == 0 || height == 0 || raw.len() < 4 {
        return (Vec::new(), 0, 0);
    }

    let out_width = PREVIEW_WIDTH.min(width);
    let out_height = ((out_width as f32) * (height as f32) / (width as f32))
        .round()
        .max(1.0) as u32;
    let stride = (out_width * 4) as usize;
    let mut pixels = vec![0u8; stride * out_height as usize];

    for y in 0..out_height {
        let source_y = (y as u64 * height as u64 / out_height as u64) as u32;
        for x in 0..out_width {
            let source_x = (x as u64 * width as u64 / out_width as u64) as u32;
            let source = ((source_y * width + source_x) * 4) as usize;
            let target = (y * out_width * 4 + x * 4) as usize;
            if source + 3 < raw.len() && target + 3 < pixels.len() {
                pixels[target] = raw[source + 2];
                pixels[target + 1] = raw[source + 1];
                pixels[target + 2] = raw[source];
                pixels[target + 3] = 255;
            }
        }
    }

    let file_size = 54 + pixels.len();
    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(out_width as i32).to_le_bytes());
    out.extend_from_slice(&(out_height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    for row in (0..out_height).rev() {
        let start = (row * out_width * 4) as usize;
        out.extend_from_slice(&pixels[start..start + out_width as usize * 4]);
    }

    (out, out_width, out_height)
}
