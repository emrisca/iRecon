//! The transparent overlay window.
//!
//! Everything here is deliberately small: the window is positioned once, made
//! click-through, and then only shown or hidden. Toggling is reachable from the
//! UI and from the global hotkey, so both paths go through the same functions.

use std::io::Write;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// The binding registered at startup. Shown in the capture lab so it is
/// discoverable without reading the source.
pub const HOTKEY: &str = "Ctrl+Alt+O";

/// What the overlay reports about its own paint clock. This is the only honest
/// way to answer "can the webview draw sixty boxes a second", because it is
/// measured where the drawing happens.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintStats {
    pub fps: f32,
    pub draw_ms: f32,
    pub boxes: u32,
    pub calibration: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayState {
    pub visible: bool,
    pub calibration: bool,
    pub hotkey: String,
}

pub fn show(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or("The overlay window is missing")?;
    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("No display found")?;

    let position = monitor.position();
    let size = monitor.size();

    window
        .set_position(tauri::PhysicalPosition::new(position.x, position.y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(tauri::PhysicalSize::new(size.width, size.height))
        .map_err(|e| e.to_string())?;
    window.set_always_on_top(true).map_err(|e| e.to_string())?;
    // Set before showing and again after: a window that is shown first can take
    // the first click of the frame it appears in.
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    app.state::<AppState>()
        .overlay_visible
        .store(true, Ordering::Relaxed);
    Ok(())
}

pub fn hide(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or("The overlay window is missing")?;
    window.hide().map_err(|e| e.to_string())?;
    app.state::<AppState>()
        .overlay_visible
        .store(false, Ordering::Relaxed);
    Ok(())
}

/// Returns the state after toggling, so a caller does not have to guess.
pub fn toggle(app: &AppHandle) -> Result<bool, String> {
    let visible = app
        .state::<AppState>()
        .overlay_visible
        .load(Ordering::Relaxed);
    if visible {
        hide(app)?;
        Ok(false)
    } else {
        show(app)?;
        Ok(true)
    }
}

pub fn state_of(app: &AppHandle) -> OverlayState {
    let state = app.state::<AppState>();
    OverlayState {
        visible: state.overlay_visible.load(Ordering::Relaxed),
        calibration: state.calibration.load(Ordering::Relaxed),
        hotkey: HOTKEY.to_string(),
    }
}

#[tauri::command]
pub fn overlay_show(app: AppHandle) -> Result<OverlayState, String> {
    show(&app)?;
    Ok(state_of(&app))
}

#[tauri::command]
pub fn overlay_hide(app: AppHandle) -> Result<OverlayState, String> {
    hide(&app)?;
    Ok(state_of(&app))
}

#[tauri::command]
pub fn overlay_toggle(app: AppHandle) -> Result<OverlayState, String> {
    toggle(&app)?;
    Ok(state_of(&app))
}

#[tauri::command]
pub fn overlay_state(app: AppHandle) -> OverlayState {
    state_of(&app)
}

/// Switches the overlay between the detection view and the sine-wave
/// calibration pattern. Calibration exists to measure the paint clock with no
/// capture running at all.
#[tauri::command]
pub fn overlay_calibration(app: AppHandle, enabled: bool) -> OverlayState {
    app.state::<AppState>()
        .calibration
        .store(enabled, Ordering::Relaxed);
    let _ = app.emit("spike:calibration", enabled);
    state_of(&app)
}

/// Default box easing in milliseconds. The overlay uses this until Capture lab
/// sends a different value, and the value is persisted under `overlay.smooth_ms`
/// so a tuning survives a restart.
const SMOOTH_DEFAULT_MS: f32 = 55.0;
const SMOOTH_MAX_MS: f32 = 300.0;

fn stored_smoothing(app: &AppHandle) -> f32 {
    let state = app.state::<AppState>();
    let Ok(conn) = state.db.lock() else {
        return SMOOTH_DEFAULT_MS;
    };
    conn.query_row(
        "select value from setting where key = 'overlay.smooth_ms'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|raw| raw.parse::<f32>().ok())
    .filter(|ms| ms.is_finite())
    .map(|ms| ms.clamp(0.0, SMOOTH_MAX_MS))
    .unwrap_or(SMOOTH_DEFAULT_MS)
}

/// Reads or changes how long a box takes to ease toward its newest position.
/// Zero draws boxes where the detection put them, which is what a jitter check
/// compares against. A change is persisted and pushed to the overlay, so it
/// applies to a running session without a restart.
#[tauri::command]
pub fn overlay_smoothing(app: AppHandle, value: Option<f32>) -> Result<f32, String> {
    let Some(requested) = value else {
        return Ok(stored_smoothing(&app));
    };
    if !requested.is_finite() {
        return Err("Smoothing needs a number of milliseconds".into());
    }
    let smooth = requested.clamp(0.0, SMOOTH_MAX_MS);
    {
        let state = app.state::<AppState>();
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "insert into setting (key, value) values ('overlay.smooth_ms', ?1)
             on conflict(key) do update set value = excluded.value",
            rusqlite::params![smooth.to_string()],
        )
        .map_err(|e| e.to_string())?;
    }
    let _ = app.emit("spike:smoothing", smooth);
    Ok(smooth)
}

/// Called by the overlay about once a second with what it measured.
#[tauri::command]
pub fn overlay_paint(app: AppHandle, fps: f32, draw_ms: f32, boxes: u32) {
    let calibration = app
        .state::<AppState>()
        .calibration
        .load(Ordering::Relaxed);
    let payload = PaintStats {
        fps: if fps.is_finite() { fps.clamp(0.0, 1000.0) } else { 0.0 },
        draw_ms: if draw_ms.is_finite() {
            draw_ms.clamp(0.0, 1000.0)
        } else {
            0.0
        },
        boxes: boxes.min(10_000),
        calibration,
    };
    let state = app.state::<AppState>();
    if let Ok(mut guard) = state.paint.lock() {
        *guard = Some(payload.clone());
    }
    // The paint clock is the one number that cannot be read from outside the
    // webview, so a measurement run appends it to a file instead of relying on
    // somebody watching the screen.
    if let Some(path) = state.paint_log.as_ref() {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or(0);
            let _ = writeln!(
                file,
                "{stamp},{:.3},{:.3},{},{}",
                payload.fps,
                payload.draw_ms,
                payload.boxes,
                if payload.calibration { 1 } else { 0 }
            );
        }
    }
    let _ = app.emit("spike:paint", payload);
}
