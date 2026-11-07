use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use xcap::Monitor;

use crate::AppState;

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const REPORT_INTERVAL: Duration = Duration::from_millis(500);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(800);
const PREVIEW_WIDTH: u32 = 640;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStats {
    pub running: bool,
    pub monitor: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub capture_ms: f32,
    pub frames: u64,
    pub errors: u64,
    pub elapsed_s: f32,
    pub note: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FramePayload {
    pub data: String,
    pub width: u32,
    pub height: u32,
}

pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
}

fn pick_monitor() -> Result<Monitor, String> {
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
pub fn capture_start(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut slot = state.capture.lock().map_err(|e| e.to_string())?;
    if slot.is_some() {
        return Ok(());
    }

    let monitor = pick_monitor()?;
    let label = monitor
        .name()
        .unwrap_or_else(|_| "Primary display".to_string());

    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let stats = state.stats.clone();
    let app_handle = app.clone();

    thread::Builder::new()
        .name("irecon-capture".into())
        .spawn(move || {
            let mut frames: u64 = 0;
            let mut errors: u64 = 0;
            let mut note: Option<String> = None;
            let mut size = (0u32, 0u32);
            let started = Instant::now();
            let mut last_report = Instant::now();
            let mut last_preview = Instant::now() - PREVIEW_INTERVAL;
            let mut window_frames: u64 = 0;
            let mut window_cost = Duration::ZERO;

            while !stop_thread.load(Ordering::Relaxed) {
                let tick = Instant::now();

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
                        frames,
                        errors,
                        elapsed_s: started.elapsed().as_secs_f32(),
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
                frames,
                errors,
                elapsed_s: started.elapsed().as_secs_f32(),
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
