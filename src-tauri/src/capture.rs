//! The live loop, rebuilt around three clocks.
//!
//! A still screenshot costs a fixed setup per frame, which is why the old loop
//! sat near three frames a second. Capture now opens a duplication session once
//! and receives frames from the compositor, so the only per frame cost is
//! handing the buffer to the next stage.
//!
//! The stages no longer share a thread:
//!
//! - the capture thread acquires frames and overwrites one slot. It never
//!   encodes, resizes, or touches SQLite, so a slow detector cannot slow it.
//! - the inference thread takes the newest frame, runs proposals, matching and
//!   tracking, and publishes boxes stamped with the capture time.
//! - the overlay paints on its own clock and extrapolates between publishes.
//!
//! A slow stage therefore drops work instead of adding latency, which is the
//! only way a 75 ms model and a 60 fps capture can coexist.

use std::collections::VecDeque;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use xcap::{Monitor, Window};

use crate::detector::Detection;
use crate::matcher;
use crate::source::{self, Crop};
use crate::tracker::{TrackedBox, Tracker};
use crate::AppState;

const REPORT_INTERVAL: Duration = Duration::from_millis(500);
/// The display update interval. Sixty is the useful ceiling, and anything faster
/// is frame churn nothing downstream can consume. Fifteen rather than sixteen so
/// a 60 Hz source, which updates every 16.7 ms, never lands just under the cap
/// and gets pushed to the next vsync.
pub(crate) const DEFAULT_INTERVAL_MS: u64 = 15;
const PREVIEW_INTERVAL: Duration = Duration::from_millis(800);
const PREVIEW_WIDTH: u32 = 640;
/// How long the inference thread waits before looking at the slot again. Two
/// milliseconds is under a frame at 240 Hz and costs nothing while idle.
const IDLE_POLL: Duration = Duration::from_millis(2);
/// A live frame older than this is left in the slot so the producer can replace
/// it, rather than starting a box on a frame that is already most of a display
/// period old. The fallback keeps slow still and sequence sources moving.
const FRAME_FRESHNESS: Duration = Duration::from_millis(8);
const STALE_FALLBACK: Duration = Duration::from_millis(250);
const POOL_RELOAD: Duration = Duration::from_secs(2);
/// Replay runs on a fixed clock. Twenty frames a second is the Phase 2 exit
/// rate, so a replay is comparable to a live run rather than a stress test.
const SEQUENCE_HZ: f32 = 20.0;
/// The detector's primary floor. A proposal below this never seeds a match on
/// its own; it can only support a seed it encloses.
pub(crate) const SCORE_THRESHOLD: f32 = 0.2;
/// The wider pool read for expansion. Rock and wall proposals sit here, so they
/// must not become matches unless a primary seed already landed nearby.
pub(crate) const EXPANSION_SCORE_THRESHOLD: f32 = 0.05;
const HARVEST_BAND_FRACTION: f32 = 0.1;
const HARVEST_BAND_MIN: f32 = 0.04;
const HARVEST_BAND_MAX: f32 = 0.12;
/// Candidates per capture session, so a bad pool cannot fill the disk. The
/// `capture.harvest_cap` setting overrides this; 0 turns harvesting off for a
/// run without touching the floor.
const HARVEST_CAP_DEFAULT: u32 = 40;
const HARVEST_CAP_MAX: u32 = 400;
const HARVEST_INTERVAL: Duration = Duration::from_millis(400);
const RECORD_INTERVAL: Duration = Duration::from_millis(250);
const RECORD_CAP: i64 = 1200;

fn harvest_floor(threshold: f32) -> f32 {
    threshold - (threshold * HARVEST_BAND_FRACTION).clamp(HARVEST_BAND_MIN, HARVEST_BAND_MAX)
}

/// Where candidate harvesting starts. A match floor is the user's answer to how
/// far below the line a proposal is still worth reviewing, so it becomes the
/// cutoff whenever one is set. With no floor, the near-miss band keeps a fresh
/// project from filling its queue with everything the detector proposes.
fn harvest_cutoff(threshold: f32, floor: f32) -> f32 {
    if floor > 0.0 {
        floor
    } else {
        harvest_floor(threshold)
    }
}

/// Reads the per session candidate cap. An unreadable value falls back to the
/// default so a bad setting cannot stop a capture run from harvesting at all.
fn read_harvest_cap(conn: &rusqlite::Connection) -> u32 {
    conn.query_row(
        "select value from setting where key = 'capture.harvest_cap'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|value| value.trim().parse::<u32>().ok())
    .map(|value| value.min(HARVEST_CAP_MAX))
    .unwrap_or(HARVEST_CAP_DEFAULT)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStats {
    pub running: bool,
    pub monitor: String,
    /// Which capture path produced this run: duplication, still capture, a
    /// window, or a replayed sequence.
    pub backend: String,
    pub source: String,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub capture_ms: f32,
    pub capture_p50_ms: f32,
    pub capture_p95_ms: f32,
    pub capture_max_ms: f32,
    /// Interval between delivered frames. This is the number that shows a
    /// duplication session keeping up, because its own work is nearly free.
    pub frame_gap_ms: f32,
    pub inference_ms: f32,
    pub preprocess_ms: f32,
    pub match_ms: f32,
    pub latency_ms: f32,
    pub dropped: u64,
    pub detections: u32,
    pub best_score: Option<f32>,
    pub best_class: Option<String>,
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
pub struct CaptureBenchmark {
    pub monitor: String,
    pub width: u32,
    pub height: u32,
    pub full_ms: f32,
    pub half_region_ms: f32,
    pub center_region_ms: f32,
    pub samples: u32,
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
    pub preprocess_ms: f32,
    /// Capture timestamp to publish, in milliseconds. Paint adds at most one
    /// frame on top of this.
    pub latency_ms: f32,
    /// Epoch milliseconds of capture and of publish, so the overlay can tell how
    /// old its boxes are and extrapolate them without guessing.
    pub captured_at_ms: f64,
    pub published_at_ms: f64,
    pub crop: Option<Crop>,
    /// Class name to its label colour, so the overlay draws each box in the
    /// colour the project already uses on the Label page.
    pub colors: std::collections::HashMap<String, String>,
    pub boxes: Vec<TrackedBox>,
}

/// A frame buffer that goes back to the pool when its last holder drops it.
///
/// Without this, a 2560 by 1440 frame is 14.7 MB allocated and freed sixty times
/// a second. The first soak run grew the working set by 48 MB in twenty seconds
/// doing exactly that; reuse is the fix, not a smaller budget.
pub struct FrameBuffer {
    data: Vec<u8>,
    pool: Option<Weak<BufferPool>>,
}

impl FrameBuffer {
    #[cfg(test)]
    fn plain(data: Vec<u8>) -> Self {
        Self { data, pool: None }
    }
}

impl Deref for FrameBuffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.data
    }
}

impl Drop for FrameBuffer {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.as_ref().and_then(Weak::upgrade) {
            pool.release(std::mem::take(&mut self.data));
        }
    }
}

/// The handful of full display buffers the pipeline actually needs at once: one
/// being written, one in the slot, one being read. Anything more is churn.
struct BufferPool {
    spare: Mutex<Vec<Vec<u8>>>,
}

impl BufferPool {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            spare: Mutex::new(Vec::new()),
        })
    }

    fn checkout(&self, len: usize) -> Vec<u8> {
        let mut spare = self
            .spare
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while let Some(mut buffer) = spare.pop() {
            if buffer.capacity() >= len {
                buffer.resize(len, 0);
                return buffer;
            }
        }
        vec![0u8; len]
    }

    fn release(&self, mut buffer: Vec<u8>) {
        let mut spare = self
            .spare
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Two spares cover the slot and the frame being copied. A larger pool
        // only holds memory that the next second would allocate anyway.
        if spare.len() < 2 {
            buffer.clear();
            spare.push(buffer);
        }
    }
}

/// One captured frame plus everything a consumer needs to place its boxes.
pub struct Frame {
    pub raw: Arc<FrameBuffer>,
    pub width: u32,
    pub height: u32,
    pub seq: u64,
    pub captured_at: Instant,
    /// The inference region, normalized to the captured frame. Detections are
    /// expressed inside it and consumers map them back for the overlay.
    pub crop: Option<Crop>,
}

/// Exactly one frame, overwritten by the producer. A consumer either gets the
/// newest frame or nothing, so latency can never accumulate behind a slow stage.
#[derive(Default)]
pub struct FrameSlot {
    current: Mutex<Option<Frame>>,
    dropped: Mutex<u64>,
}

impl FrameSlot {
    pub fn publish(&self, frame: Frame) {
        if let Ok(mut guard) = self.current.lock() {
            if guard.is_some() {
                if let Ok(mut dropped) = self.dropped.lock() {
                    *dropped += 1;
                }
            }
            *guard = Some(frame);
        }
    }

    pub fn take(&self) -> Option<Frame> {
        self.current.lock().ok().and_then(|mut guard| guard.take())
    }

    /// Takes a frame only when it is recent enough to be worth processing.
    /// A stale frame stays in the slot so the producer's next publish counts
    /// as an overwrite and the freshness wait does not look like a frame that
    /// was processed.
    pub fn take_fresh(&self, max_age: Duration) -> Option<Frame> {
        let mut guard = self.current.lock().ok()?;
        let fresh = guard
            .as_ref()
            .map(|frame| frame.captured_at.elapsed() <= max_age)
            .unwrap_or(false);
        if fresh {
            guard.take()
        } else {
            None
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.lock().map(|guard| *guard).unwrap_or(0)
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.current.lock() {
            *guard = None;
        }
    }
}

/// Numbers the inference thread publishes for the capture thread's status line.
#[derive(Clone, Default)]
struct BoardValues {
    inference_ms: f32,
    preprocess_ms: f32,
    match_ms: f32,
    latency_ms: f32,
    detections: u32,
    best_score: Option<f32>,
    best_class: Option<String>,
    pool_classes: u32,
    pool_examples: u32,
    note: Option<String>,
}

#[derive(Default)]
pub struct StatsBoard {
    values: Mutex<BoardValues>,
}

impl StatsBoard {
    fn set(&self, patch: impl FnOnce(&mut BoardValues)) {
        if let Ok(mut guard) = self.values.lock() {
            patch(&mut guard);
        }
    }

    fn snapshot(&self) -> BoardValues {
        self.values
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }
}

pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
    inference_stop: Arc<AtomicBool>,
    slot: Arc<FrameSlot>,
}

impl CaptureHandle {
    fn halt(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.inference_stop.store(true, Ordering::Relaxed);
        self.slot.clear();
    }
}

/// What the run reads frames from. Resolved once, before the threads start.
enum Plan {
    Display {
        target: String,
        label: String,
        crop: Option<Crop>,
    },
    Window {
        target: String,
        label: String,
        crop: Option<Crop>,
    },
    Sequence {
        paths: Vec<PathBuf>,
        label: String,
        crop: Option<Crop>,
    },
}

impl Plan {
    fn label(&self) -> &str {
        match self {
            Plan::Display { label, .. } => label,
            Plan::Window { label, .. } => label,
            Plan::Sequence { label, .. } => label,
        }
    }

    fn crop(&self) -> Option<Crop> {
        match self {
            Plan::Display { crop, .. } => *crop,
            Plan::Window { crop, .. } => *crop,
            Plan::Sequence { crop, .. } => *crop,
        }
    }
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

fn monitor_by_target(target: &str) -> Result<Monitor, String> {
    if target.is_empty() {
        return pick_monitor();
    }
    let monitors = Monitor::all().map_err(|e| format!("Could not list displays: {e}"))?;
    monitors
        .iter()
        .find(|monitor| {
            monitor
                .id()
                .map(|id| id.to_string() == target)
                .unwrap_or(false)
        })
        .cloned()
        .ok_or_else(|| "That display is no longer connected".to_string())
}

fn plan_for(app: &AppHandle, source_id: Option<i64>) -> Result<Plan, String> {
    if let Some(id) = source_id {
        let source = source::source_row(app, id)?;
        let crop = source.crop;
        return match source.kind.as_str() {
            "display" => Ok(Plan::Display {
                target: source.target,
                label: source.label,
                crop,
            }),
            "window" => Ok(Plan::Window {
                target: source.target,
                label: source.label,
                crop,
            }),
            "file" | "folder" => Ok(Plan::Sequence {
                paths: source::sequence_paths(&source.kind, &source.target)?,
                label: source.label,
                crop,
            }),
            other => Err(format!("A {other} source cannot be captured")),
        };
    }
    let monitor = pick_monitor()?;
    Ok(Plan::Display {
        target: monitor.id().map(|id| id.to_string()).unwrap_or_default(),
        label: monitor
            .name()
            .unwrap_or_else(|_| "Primary display".to_string()),
        crop: None,
    })
}

#[tauri::command]
pub fn capture_status(state: tauri::State<'_, AppState>) -> Option<CaptureStats> {
    state.stats.lock().ok().and_then(|guard| guard.clone())
}

/// Still capture cost at three sizes. Kept because it is the number that proves
/// the still path cannot reach sixty, and because the duplication path needs a
/// comparison on the same machine on the same day.
#[tauri::command]
pub fn capture_benchmark(samples: Option<u32>) -> Result<CaptureBenchmark, String> {
    let monitor = pick_monitor()?;
    let width = monitor.width().map_err(|e| e.to_string())?;
    let height = monitor.height().map_err(|e| e.to_string())?;
    let count = samples.unwrap_or(3).clamp(1, 10);
    let mut full = 0f32;
    let mut half = 0f32;
    let mut center = 0f32;
    for _ in 0..count {
        let started = Instant::now();
        let _ = monitor.capture_image().map_err(|e| e.to_string())?;
        full += started.elapsed().as_secs_f32() * 1000.0;

        let started = Instant::now();
        let _ = monitor
            .capture_region(0, 0, (width / 2).max(1), (height / 2).max(1))
            .map_err(|e| e.to_string())?;
        half += started.elapsed().as_secs_f32() * 1000.0;

        let started = Instant::now();
        let _ = monitor
            .capture_region(
                (width.saturating_sub(1280)) / 2,
                (height.saturating_sub(720)) / 2,
                1280.min(width),
                720.min(height),
            )
            .map_err(|e| e.to_string())?;
        center += started.elapsed().as_secs_f32() * 1000.0;
    }
    let denominator = count as f32;
    Ok(CaptureBenchmark {
        monitor: monitor.name().unwrap_or_else(|_| "Primary display".to_string()),
        width,
        height,
        full_ms: full / denominator,
        half_region_ms: half / denominator,
        center_region_ms: center / denominator,
        samples: count,
    })
}

#[tauri::command]
pub fn capture_watch(state: tauri::State<'_, AppState>, watching: bool) {
    state.preview_watch.store(watching, Ordering::Relaxed);
}

#[tauri::command]
pub fn capture_start(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    project_id: Option<i64>,
    source_id: Option<i64>,
    record: Option<bool>,
) -> Result<(), String> {
    let mut slot = state.capture.lock().map_err(|e| e.to_string())?;
    if slot.is_some() {
        return Ok(());
    }

    let plan = plan_for(&app, source_id)?;
    let label = plan.label().to_string();
    let crop = plan.crop();

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
    if let Some(project_id) = project {
        if let Some((path, input_size, name)) = crate::project_detector::active_model(&app, project_id) {
            if let Ok(detector) = crate::detector::Detector::load(std::path::Path::new(&path), input_size)
            {
                if let Ok(mut guard) = state.detector.lock() {
                    *guard = Some(detector);
                }
                if let Ok(mut guard) = state.loaded_model.lock() {
                    *guard = Some(format!("project:{name}"));
                }
            }
        } else if state
            .loaded_model
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .map(|name| name.starts_with("project:"))
            .unwrap_or(false)
        {
            if let Some(entry) = crate::model::preferred(&app) {
                let _ = crate::model::load_now(&app, entry);
            }
        }
    }

    // The classifier belongs to the project too. Loading it here means the
    // inference thread never opens a graph, and a project without one keeps the
    // fingerprint matcher exactly as before.
    if let Some(project_id) = project {
        match crate::classifier::active_model(&app, project_id) {
            Some((path, labels, _name)) => {
                if let Ok(classifier) =
                    crate::classifier::Classifier::load(std::path::Path::new(&path), labels)
                {
                    if let Ok(mut guard) = state.classifier.lock() {
                        *guard = Some(classifier);
                    }
                }
            }
            None => {
                if let Ok(mut guard) = state.classifier.lock() {
                    *guard = None;
                }
            }
        }
    }

    let session = if record.unwrap_or(false) {
        project.and_then(|id| crate::session::start(&app, id).ok())
    } else {
        None
    };

    let stop = Arc::new(AtomicBool::new(false));
    let inference_stop = Arc::new(AtomicBool::new(false));
    let inference_halt = inference_stop.clone();
    let frame_slot = state.frame_slot.clone();
    let board = state.board.clone();
    let stats = state.stats.clone();
    let detector = state.detector.clone();
    let classifier = state.classifier.clone();
    let preview_watch = state.preview_watch.clone();
    let overlay_visible = state.overlay_visible.clone();

    frame_slot.clear();
    board.set(|values| *values = BoardValues::default());
    if let Ok(mut guard) = state.stats.lock() {
        *guard = None;
    }

    let capture_stop = stop.clone();
    let capture_app = app.clone();
    let capture_slot = frame_slot.clone();
    let capture_board = board.clone();
    let capture_stats = stats.clone();
    thread::Builder::new()
        .name("irecon-capture".into())
        .spawn(move || {
            let sink: Arc<dyn CaptureSink> = Arc::new(AppSink {
                app: capture_app.clone(),
                stats: capture_stats,
            });
            let run = Arc::new(Mutex::new(Run::new(
                sink,
                capture_slot,
                capture_board,
                capture_stop,
                label,
                crop,
            )));
            let outcome = match &plan {
                Plan::Display { target, .. } => {
                    display_run(&run, target, DEFAULT_INTERVAL_MS)
                }
                Plan::Window { target, .. } => {
                    window_run(&run, target, DEFAULT_INTERVAL_MS)
                }
                Plan::Sequence { paths, .. } => play_sequence(&run, paths),
            };
            let note = match outcome {
                Ok(()) => None,
                Err(error) => Some(error),
            };
            if let Ok(mut guard) = run.lock() {
                guard.finish(note);
            };
        })
        .map_err(|e| format!("Could not start the capture thread: {e}"))?;

    let inference_app = app.clone();
    let inference_slot = frame_slot.clone();
    let inference_board = board.clone();
    thread::Builder::new()
        .name("irecon-inference".into())
        .spawn(move || {
            inference_loop(
                inference_app,
                project,
                inference_slot,
                inference_stop,
                detector,
                classifier,
                inference_board,
                preview_watch,
                overlay_visible,
                session,
            );
        })
        .map_err(|e| format!("Could not start the inference thread: {e}"))?;

    *slot = Some(CaptureHandle {
        stop,
        inference_stop: inference_halt,
        slot: frame_slot,
    });
    Ok(())
}

#[tauri::command]
pub fn capture_stop(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut slot = state.capture.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = slot.take() {
        handle.halt();
    }
    if let Ok(mut guard) = state.stats.lock() {
        if let Some(stats) = guard.as_mut() {
            stats.running = false;
            let _ = app.emit("spike:stats", stats.clone());
        }
    }
    Ok(())
}

/// Where a run's snapshots go. The app emits them to the frontend and caches
/// them for `capture_status`; the soak harness throws them away, because its
/// numbers come from the counters at the end of the run.
pub(crate) trait CaptureSink: Send + Sync {
    fn publish(&self, stats: &CaptureStats);
    fn model(&self) -> Option<String>;
}

struct AppSink {
    app: AppHandle,
    stats: Arc<Mutex<Option<CaptureStats>>>,
}

impl CaptureSink for AppSink {
    fn publish(&self, stats: &CaptureStats) {
        if let Ok(mut guard) = self.stats.lock() {
            *guard = Some(stats.clone());
        }
        let _ = self.app.emit("spike:stats", stats.clone());
    }

    fn model(&self) -> Option<String> {
        self.app
            .state::<AppState>()
            .loaded_model
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }
}

/// One run's numbers, taken at the end. The harness reads these; the app never
/// does, because its status line is built from the same counters every 500 ms.
#[derive(Clone, Default)]
pub struct RunSummary {
    pub label: String,
    pub backend: String,
    pub width: u32,
    pub height: u32,
    pub elapsed_s: f32,
    pub frames: u64,
    pub errors: u64,
    pub overwrites: u64,
    pub fps_total: f32,
    pub fps_window: f32,
    pub window_s: f32,
    /// Capture callback start to slot publish. The copy and the handoff, without
    /// the model or the paint.
    pub publish_p50: f32,
    pub publish_p95: f32,
    pub publish_max: f32,
    pub cost_p50: f32,
    pub cost_p95: f32,
    pub cost_max: f32,
    pub gap_p50: f32,
    pub gap_p95: f32,
    pub note: Option<String>,
}

/// One capture run's counters. Frames go in, a status line comes out.
///
/// The run lives behind a mutex because the Graphics Capture callback owns the
/// frames: it arrives on a thread the capture loop only starts, not one it owns.
pub(crate) struct Run {
    sink: Arc<dyn CaptureSink>,
    slot: Arc<FrameSlot>,
    pool: Arc<BufferPool>,
    board: Arc<StatsBoard>,
    stop: Arc<AtomicBool>,
    started: Instant,
    label: String,
    crop: Option<Crop>,
    backend: String,
    note: Option<String>,
    width: u32,
    height: u32,
    frames: u64,
    errors: u64,
    frame_drops: u64,
    window_frames: u64,
    window_started: Instant,
    gaps: Histogram,
    costs: Histogram,
    publishes: Histogram,
    recent: VecDeque<Instant>,
    last_report: Instant,
    last_arrival: Option<Instant>,
}

impl Run {
    pub(crate) fn new(
        sink: Arc<dyn CaptureSink>,
        slot: Arc<FrameSlot>,
        board: Arc<StatsBoard>,
        stop: Arc<AtomicBool>,
        label: String,
        crop: Option<Crop>,
    ) -> Self {
        let now = Instant::now();
        Self {
            sink,
            slot,
            pool: BufferPool::new(),
            board,
            stop,
            started: now,
            label,
            crop,
            backend: "starting".into(),
            note: None,
            width: 0,
            height: 0,
            frames: 0,
            errors: 0,
            frame_drops: 0,
            window_frames: 0,
            window_started: now,
            gaps: Histogram::default(),
            costs: Histogram::default(),
            publishes: Histogram::default(),
            recent: VecDeque::new(),
            last_report: now,
            last_arrival: None,
        }
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// A buffer to copy a frame into. It returns to the pool when the published
    /// frame is dropped, so steady state allocates nothing.
    fn checkout(&self, len: usize) -> Vec<u8> {
        self.pool.checkout(len)
    }

    /// Milliseconds since the previous delivered frame, for the gap histogram.
    fn arrival(&mut self) -> Option<Duration> {
        let now = Instant::now();
        let gap = self
            .last_arrival
            .map(|previous| now.duration_since(previous));
        self.last_arrival = Some(now);
        gap
    }

    /// Applies the run's crop and publishes. Every backend goes through this, so
    /// a crop means the same thing whether it came from a live session or from a
    /// folder of images.
    fn accept(
        &mut self,
        raw: Vec<u8>,
        width: u32,
        height: u32,
        gap: Option<Duration>,
        cost: Duration,
        captured_at: Instant,
    ) {
        match self.crop {
            Some(crop) => match crop_rgba(&raw, width, height, &crop) {
                Some((raw, width, height)) => {
                    self.deliver(raw, width, height, gap, cost, captured_at)
                }
                None => self.fail("The crop sits outside the frame"),
            },
            None => self.deliver(raw, width, height, gap, cost, captured_at),
        }
    }

    fn deliver(
        &mut self,
        raw: Vec<u8>,
        width: u32,
        height: u32,
        gap: Option<Duration>,
        cost: Duration,
        captured_at: Instant,
    ) {
        let seq = self.frames + 1;
        let published_at = Instant::now();
        self.slot.publish(Frame {
            raw: Arc::new(FrameBuffer {
                data: raw,
                pool: Some(Arc::downgrade(&self.pool)),
            }),
            width,
            height,
            seq,
            captured_at,
            crop: self.crop,
        });
        self.width = width;
        self.height = height;
        self.frames += 1;
        self.window_frames += 1;
        self.recent.push_back(published_at);
        // Only the last seventy seconds matter, and trimming on every frame
        // keeps a ten minute run from holding every timestamp it ever saw.
        while let Some(oldest) = self.recent.front() {
            if published_at.duration_since(*oldest) > Duration::from_secs(70) {
                self.recent.pop_front();
            } else {
                break;
            }
        }
        if let Some(gap) = gap {
            self.gaps.record(gap.as_secs_f32() * 1000.0);
        }
        self.costs.record(cost.as_secs_f32() * 1000.0);
        self.publishes
            .record(published_at.duration_since(captured_at).as_secs_f32() * 1000.0);
    }

    fn fail(&mut self, error: impl std::fmt::Display) {
        self.errors += 1;
        if self.note.is_none() {
            self.note = Some(error.to_string());
        }
    }

    fn fps_window(&self, seconds: f32) -> f32 {
        if self.recent.is_empty() {
            return 0.0;
        }
        let now = Instant::now();
        let count = self
            .recent
            .iter()
            .filter(|at| now.duration_since(**at).as_secs_f32() <= seconds)
            .count();
        let span = self.started.elapsed().as_secs_f32().clamp(0.001, seconds);
        count as f32 / span
    }

    fn report_if_due(&mut self) {
        if self.last_report.elapsed() < REPORT_INTERVAL {
            return;
        }
        let window = self.window_started.elapsed().as_secs_f32().max(0.001);
        let (p50, p95, max) = self.costs.summary();
        let (gap, _, _) = self.gaps.summary();
        let board = self.board.snapshot();
        self.frame_drops = self.slot.dropped();
        let snapshot = CaptureStats {
            running: true,
            monitor: self.label.clone(),
            backend: self.backend.clone(),
            source: self.label.clone(),
            width: self.width,
            height: self.height,
            fps: self.window_frames as f32 / window,
            capture_ms: p50,
            capture_p50_ms: p50,
            capture_p95_ms: p95,
            capture_max_ms: max,
            frame_gap_ms: gap,
            inference_ms: board.inference_ms,
            preprocess_ms: board.preprocess_ms,
            match_ms: board.match_ms,
            latency_ms: board.latency_ms,
            dropped: self.frame_drops,
            detections: board.detections,
            best_score: board.best_score,
            best_class: board.best_class.clone(),
            pool_classes: board.pool_classes,
            pool_examples: board.pool_examples,
            frames: self.frames,
            errors: self.errors,
            elapsed_s: self.started.elapsed().as_secs_f32(),
            model: self.sink.model(),
            note: self.note.clone().or(board.note),
        };
        self.sink.publish(&snapshot);
        self.last_report = Instant::now();
        self.window_started = Instant::now();
        self.window_frames = 0;
    }

    /// Everything the soak harness reports, taken from the same counters the
    /// status line uses.
    pub(crate) fn summary(&self) -> RunSummary {
        let (cost_p50, cost_p95, cost_max) = self.costs.summary();
        let (gap_p50, gap_p95, _) = self.gaps.summary();
        let (publish_p50, publish_p95, publish_max) = self.publishes.summary();
        let elapsed = self.started.elapsed().as_secs_f32().max(0.001);
        RunSummary {
            label: self.label.clone(),
            backend: self.backend.clone(),
            width: self.width,
            height: self.height,
            elapsed_s: elapsed,
            frames: self.frames,
            errors: self.errors,
            overwrites: self.slot.dropped(),
            fps_total: self.frames as f32 / elapsed,
            fps_window: self.fps_window(60.0),
            window_s: 60.0,
            publish_p50,
            publish_p95,
            publish_max,
            cost_p50,
            cost_p95,
            cost_max,
            gap_p50,
            gap_p95,
            note: self.note.clone(),
        }
    }

    pub(crate) fn finish(&mut self, error: Option<String>) {
        if let Some(error) = error {
            self.note = Some(error);
        }
        self.slot.clear();
        let board = self.board.snapshot();
        let final_stats = CaptureStats {
            running: false,
            monitor: self.label.clone(),
            backend: self.backend.clone(),
            source: self.label.clone(),
            width: self.width,
            height: self.height,
            fps: 0.0,
            capture_ms: 0.0,
            capture_p50_ms: 0.0,
            capture_p95_ms: 0.0,
            capture_max_ms: 0.0,
            frame_gap_ms: 0.0,
            inference_ms: board.inference_ms,
            preprocess_ms: board.preprocess_ms,
            match_ms: board.match_ms,
            latency_ms: board.latency_ms,
            dropped: self.frame_drops,
            detections: 0,
            best_score: board.best_score,
            best_class: board.best_class,
            pool_classes: board.pool_classes,
            pool_examples: board.pool_examples,
            frames: self.frames,
            errors: self.errors,
            elapsed_s: self.started.elapsed().as_secs_f32(),
            model: self.sink.model(),
            note: self.note.clone().or(board.note),
        };
        self.sink.publish(&final_stats);
    }
}

/// Copies a decoded frame into a pooled buffer instead of a fresh allocation.
fn checkout_copy(run: &Arc<Mutex<Run>>, source: &[u8]) -> Vec<u8> {
    let mut buffer = match run.lock() {
        Ok(guard) => guard.checkout(source.len()),
        Err(_) => vec![0u8; source.len()],
    };
    buffer.copy_from_slice(source);
    buffer
}

/// The display chain: Graphics Capture, then duplication, then still capture.
/// Each step is a different API rather than a retry of the same failure, so a
/// machine that refuses one still has two paths left.
/// The primary display's refresh rate, straight from GDI. xcap does not
/// expose one, and the cadence decision depends on it.
#[cfg(windows)]
fn primary_refresh_hz() -> u32 {
    #[link(name = "user32")]
    extern "system" {
        fn GetDC(window: isize) -> isize;
        fn ReleaseDC(window: isize, dc: isize) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn GetDeviceCaps(dc: isize, index: i32) -> i32;
    }
    const VREFRESH: i32 = 116;
    unsafe {
        let dc = GetDC(0);
        if dc == 0 {
            return 60;
        }
        let hz = GetDeviceCaps(dc, VREFRESH);
        ReleaseDC(0, dc);
        if hz > 1 {
            hz as u32
        } else {
            60
        }
    }
}

#[cfg(not(windows))]
fn primary_refresh_hz() -> u32 {
    60
}
/// The minimum update interval, chosen from the display's frame period.
///
/// WGC compares the interval against compositor frames, so a value between one
/// and two periods delivers every second frame, one between two and three every
/// third, and so on. A fixed 15 ms is wrong on a 143 Hz display: it lands in the
/// three frame bucket and delivers 47.7 fps, which misses G1 while a 60 Hz
/// source on the same code passes.
pub fn display_cadence_ms(requested_ms: u64) -> u64 {
    cadence_for(primary_refresh_hz(), requested_ms)
}

fn cadence_for(refresh_hz: u32, requested_ms: u64) -> u64 {
    let refresh = refresh_hz.max(1) as f32;
    let period = 1000.0 / refresh;
    // Two display frames is the tightest cadence that stays under a 72 fps
    // ceiling; lower refresh rates keep every frame.
    let mut ticks = 1u32;
    while refresh / ticks as f32 > 72.0 {
        ticks += 1;
    }
    // One millisecond short of the tick boundary, so the comparator takes the
    // intended number of frames and never one extra.
    let cadence = ((period * ticks as f32).ceil() as u64).saturating_sub(1).max(1);
    cadence.min(requested_ms.max(1))
}

pub(crate) fn display_run(
    run: &Arc<Mutex<Run>>,
    target: &str,
    interval_ms: u64,
) -> Result<(), String> {
    // Graphics Capture is opened for the primary display, so a source that
    // points somewhere else goes straight to duplication instead of quietly
    // recording the wrong screen.
    let primary = match (pick_monitor(), monitor_by_target(target)) {
        (Ok(primary), Ok(chosen)) => primary.name().ok() == chosen.name().ok(),
        _ => false,
    };
    if primary {
        let cadence = cadence_for(primary_refresh_hz(), interval_ms);
        match wgc::display(run, cadence) {
            Ok(()) => return Ok(()),
            Err(_) => {
                if run.lock().map(|guard| guard.stopping()).unwrap_or(true) {
                    return Ok(());
                }
            }
        }
    }
    match stream_display(run, target) {
        Ok(()) => Ok(()),
        Err(error) => still_display(run, target, Some(error)),
    }
}

/// Duplication: one session, then a frame every time the compositor has one.
fn stream_display(run: &Arc<Mutex<Run>>, target: &str) -> Result<(), String> {
    let monitor = monitor_by_target(target)?;
    let (recorder, receiver) = monitor
        .video_recorder()
        .map_err(|e| format!("Desktop Duplication could not start: {e}"))?;
    recorder
        .start()
        .map_err(|e| format!("Desktop Duplication could not start: {e}"))?;
    if let Ok(mut guard) = run.lock() {
        guard.backend = "desktop duplication".into();
    }

    let result = loop {
        let stopping = run.lock().map(|guard| guard.stopping()).unwrap_or(true);
        if stopping {
            break Ok(());
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => {
                let gap = run.lock().ok().and_then(|mut guard| guard.arrival());
                let started = Instant::now();
                let width = frame.width;
                let height = frame.height;
                if let Ok(mut guard) = run.lock() {
                    guard.accept(frame.raw, width, height, gap, started.elapsed(), started);
                    guard.report_if_due();
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Ok(mut guard) = run.lock() {
                    guard.report_if_due();
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                break Err("The duplication session ended".to_string());
            }
        }
    };
    let _ = recorder.stop();
    result
}

/// Window capture prefers Graphics Capture, which is driven by the compositor
/// and delivers frames at the window's own rate. The still window path remains
/// for windows or drivers that refuse a capture item.
fn window_run(run: &Arc<Mutex<Run>>, target: &str, interval_ms: u64) -> Result<(), String> {
    let cadence = display_cadence_ms(interval_ms);
    match wgc::window(run, target, cadence) {
        Ok(()) => Ok(()),
        Err(error) => {
            if run.lock().map(|guard| guard.stopping()).unwrap_or(true) {
                return Ok(());
            }
            if let Ok(mut guard) = run.lock() {
                guard.note = Some(format!(
                    "{error}. Falling back to still window capture, which is much slower."
                ));
            }
            still_window(run, target)
        }
    }
}

/// Fallback when streaming is refused. Slow by construction, and the note says
/// so, because a silent three frames a second looks like a bug.
fn still_display(
    run: &Arc<Mutex<Run>>,
    target: &str,
    previous: Option<String>,
) -> Result<(), String> {
    let monitor = monitor_by_target(target)?;
    if let Ok(mut guard) = run.lock() {
        guard.backend = "still capture".into();
        guard.note = Some(match previous {
            Some(error) => format!("{error}. Falling back to still capture, which is much slower."),
            None => "Falling back to still capture, which is much slower.".to_string(),
        });
    }
    loop {
        let stopping = run.lock().map(|guard| guard.stopping()).unwrap_or(true);
        if stopping {
            break;
        }
        let started = Instant::now();
        let gap = run.lock().ok().and_then(|mut guard| guard.arrival());
        match monitor.capture_image() {
            Ok(image) => {
                let width = image.width();
                let height = image.height();
                let raw = checkout_copy(run, image.as_raw());
                if let Ok(mut guard) = run.lock() {
                    guard.accept(raw, width, height, gap, started.elapsed(), started);
                    guard.report_if_due();
                }
            }
            Err(error) => {
                if let Ok(mut guard) = run.lock() {
                    guard.fail(error);
                    guard.report_if_due();
                }
            }
        }
        let spent = started.elapsed();
        if spent < Duration::from_millis(16) {
            thread::sleep(Duration::from_millis(16) - spent);
        }
    }
    Ok(())
}

fn still_window(run: &Arc<Mutex<Run>>, target: &str) -> Result<(), String> {
    let id: u32 = target
        .parse()
        .map_err(|_| "That window is gone. Pick it again.".to_string())?;
    if let Ok(mut guard) = run.lock() {
        guard.backend = "still window capture".into();
    }
    loop {
        let stopping = run.lock().map(|guard| guard.stopping()).unwrap_or(true);
        if stopping {
            break;
        }
        let started = Instant::now();
        let gap = run.lock().ok().and_then(|mut guard| guard.arrival());
        let windows = Window::all().map_err(|e| format!("Could not list windows: {e}"))?;
        let Some(window) = windows
            .iter()
            .find(|window| window.id().map(|value| value == id).unwrap_or(false))
        else {
            if let Ok(mut guard) = run.lock() {
                guard.fail("That window is gone. Pick it again.");
            }
            break;
        };
        if window.is_minimized().unwrap_or(false) {
            if let Ok(mut guard) = run.lock() {
                guard.fail("That window is minimised, so there is nothing to read");
            }
        } else {
            match window.capture_image() {
                Ok(image) => {
                    let width = image.width();
                    let height = image.height();
                    let raw = checkout_copy(run, image.as_raw());
                    if let Ok(mut guard) = run.lock() {
                        guard.accept(raw, width, height, gap, started.elapsed(), started);
                        guard.report_if_due();
                    }
                }
                Err(error) => {
                    if let Ok(mut guard) = run.lock() {
                        guard.fail(error);
                        guard.report_if_due();
                    }
                }
            }
        }
        let spent = started.elapsed();
        if spent < Duration::from_millis(16) {
            thread::sleep(Duration::from_millis(16) - spent);
        }
    }
    Ok(())
}

/// A file or folder replayed through the same pipeline on a fixed clock, so the
/// inference path is identical to a live run.
fn play_sequence(run: &Arc<Mutex<Run>>, paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("That source has no frames".into());
    }
    if let Ok(mut guard) = run.lock() {
        guard.backend = format!("sequence {SEQUENCE_HZ:.0} Hz");
    }
    let interval = Duration::from_secs_f32(1.0 / SEQUENCE_HZ);
    let mut index = 0usize;
    let mut next = Instant::now();
    loop {
        let stopping = run.lock().map(|guard| guard.stopping()).unwrap_or(true);
        if stopping {
            break;
        }
        let path = &paths[index % paths.len()];
        let started = Instant::now();
        match image::open(path) {
            Ok(image) => {
                let image = image.to_rgba8();
                let width = image.width();
                let height = image.height();
                let raw = checkout_copy(run, image.as_raw());
                // Decode time belongs in the capture cost, not in the age of the
                // frame. Stamping before the decode made the freshness gate drop
                // replay frames whose JPEG took longer than 8 ms to open.
                let available_at = Instant::now();
                if let Ok(mut guard) = run.lock() {
                    guard.accept(
                        raw,
                        width,
                        height,
                        Some(interval),
                        started.elapsed(),
                        available_at,
                    );
                }
            }
            Err(error) => {
                if let Ok(mut guard) = run.lock() {
                    guard.fail(format!("Could not read {}: {error}", path.display()));
                }
            }
        }
        index += 1;
        if let Ok(mut guard) = run.lock() {
            guard.report_if_due();
        }

        next += interval;
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else if now.duration_since(next) > interval * 4 {
            // A slow decode must not make the player sprint to catch up.
            next = now;
        }
    }
    Ok(())
}

#[cfg(windows)]
mod wgc {
    use super::*;
    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame as CaptureFrame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::monitor::Monitor as CaptureMonitor;
    use windows_capture::window::Window as CaptureWindow;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    };

    /// One frame per compositor update, copied once into the shared slot. This
    /// handler is the capture clock; there is no polling loop behind it.
    struct Handler {
        run: Arc<Mutex<Run>>,
    }

    impl GraphicsCaptureApiHandler for Handler {
        type Flags = Arc<Mutex<Run>>;
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self { run: ctx.flags })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut CaptureFrame<'_>,
            capture_control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let Ok(mut run) = self.run.lock() else {
                capture_control.stop();
                return Ok(());
            };
            if run.stopping() {
                capture_control.stop();
                return Ok(());
            }
            let gap = run.arrival();
            let width = frame.width();
            let height = frame.height();
            let started = Instant::now();
            let copied = match frame.buffer() {
                Ok(mut buffer) => {
                    let padded = buffer.has_padding();
                    let pitch = buffer.row_pitch() as usize;
                    let row = width as usize * 4;
                    let rows = height as usize;
                    let source = buffer.as_raw_buffer();
                    if padded && pitch >= row {
                        // Row pitch is the compositor's alignment, not ours.
                        let mut out = run.checkout(row * rows);
                        for y in 0..rows {
                            let start = y * pitch;
                            if start + row > source.len() {
                                break;
                            }
                            out[y * row..(y + 1) * row]
                                .copy_from_slice(&source[start..start + row]);
                        }
                        Some(out)
                    } else {
                        let mut out = run.checkout(source.len());
                        out.copy_from_slice(source);
                        Some(out)
                    }
                }
                Err(_) => None,
            };
            match copied {
                Some(raw) => run.accept(raw, width, height, gap, started.elapsed(), started),
                None => run.fail("Graphics Capture returned a frame that could not be read"),
            }
            run.report_if_due();
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            if let Ok(mut run) = self.run.lock() {
                if !run.stopping() {
                    run.fail("The capture target closed");
                    run.stop.store(true, Ordering::Relaxed);
                }
            }
            Ok(())
        }
    }

    pub fn display(run: &Arc<Mutex<Run>>, interval_ms: u64) -> Result<(), String> {
        let monitor = CaptureMonitor::primary()
            .map_err(|e| format!("Graphics Capture could not find the primary display: {e}"))?;
        if let Ok(mut guard) = run.lock() {
            guard.backend = "graphics capture".into();
        }
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Include,
            // Sixty is the useful ceiling. A 143 Hz display would otherwise hand
            // over about 1.7 GB of RGBA a second that nothing can consume.
            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(interval_ms.max(1))),
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            run.clone(),
        );
        let control = Handler::start_free_threaded(settings)
            .map_err(|error| format!("Graphics Capture could not start: {error:?}"))?;
        // A completely still desktop produces no frames, so stopping from inside
        // the callback would leave the session open until something moved. The
        // control handle ends it directly, which is what makes both the app's
        // stop button and the soak harness return promptly.
        loop {
            if run.lock().map(|guard| guard.stopping()).unwrap_or(true) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        control
            .stop()
            .map_err(|error| format!("Graphics Capture did not stop cleanly: {error:?}"))
    }

    pub fn window(
        run: &Arc<Mutex<Run>>,
        target: &str,
        interval_ms: u64,
    ) -> Result<(), String> {
        let id: u32 = target
            .parse()
            .map_err(|_| "That window is gone. Pick it again.".to_string())?;
        let window = CaptureWindow::from_raw_hwnd(id as usize as *mut std::ffi::c_void);
        if !window.is_valid() {
            return Err("Graphics Capture cannot read that window".into());
        }
        if let Ok(mut guard) = run.lock() {
            guard.backend = "window graphics capture".into();
        }
        let settings = Settings::new(
            window,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Include,
            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(interval_ms.max(1))),
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            run.clone(),
        );
        let control = Handler::start_free_threaded(settings).map_err(|error| {
            format!("Graphics Capture could not start for this window: {error:?}")
        })?;
        loop {
            if run.lock().map(|guard| guard.stopping()).unwrap_or(true) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        control
            .stop()
            .map_err(|error| format!("Graphics Capture did not stop cleanly: {error:?}"))
    }
}

#[cfg(not(windows))]
mod wgc {
    use super::*;

    pub fn display(_run: &Arc<Mutex<Run>>, _interval_ms: u64) -> Result<(), String> {
        Err("Graphics Capture is only available on Windows".into())
    }

    pub fn window(
        _run: &Arc<Mutex<Run>>,
        _target: &str,
        _interval_ms: u64,
    ) -> Result<(), String> {
        Err("Graphics Capture is only available on Windows".into())
    }
}

#[derive(Default)]
struct Histogram {
    samples: Vec<f32>,
}

impl Histogram {
    fn record(&mut self, ms: f32) {
        // Ten minutes at 143 Hz is about 86k samples. The cap is headroom rather
        // than a window: the gate is about the whole run, so nothing is dropped.
        if self.samples.len() < 200_000 {
            self.samples.push(ms);
        }
    }

    /// Nearest rank over every sample recorded so far. It does not consume the
    /// samples, so a status line every 500 ms and one summary at the end agree.
    fn summary(&self) -> (f32, f32, f32) {
        if self.samples.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|left, right| left.total_cmp(right));
        let count = sorted.len();
        let p50 = sorted[(count - 1) / 2];
        let p95_index = ((count as f32 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(count - 1);
        let p95 = sorted[p95_index];
        let max = sorted[count - 1];
        (p50, p95, max)
    }
}

/// Scores one proposal. The crop is built inside the worker so the full frame
/// is shared read-only and no crop buffers cross a thread boundary.
fn match_proposal(
    raw: &[u8],
    width: u32,
    height: u32,
    detection: &Detection,
    pool: &matcher::Pool,
) -> Option<matcher::LiveMatch> {
    let candidate = matcher::embed_detection(raw, width, height, detection)?;
    let guess = pool.best(&candidate)?;
    if guess.score < pool.floor {
        return None;
    }
    Some(guess)
}

/// Scores primary proposals in parallel, preserving their original order so the
/// refinement and dedupe rules see exactly the sequence they saw before. Four
/// workers are enough for the handful of proposals a frame produces and avoid
/// oversubscribing the CPU that capture and the detector also need.
fn score_primary(
    raw: &[u8],
    width: u32,
    height: u32,
    primary: Vec<Detection>,
    pool: &matcher::Pool,
) -> Vec<(Detection, matcher::LiveMatch)> {
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(4)
        .min(primary.len());
    if workers < 2 {
        return primary
            .into_iter()
            .filter_map(|detection| {
                match_proposal(raw, width, height, &detection, pool)
                    .map(|guess| (detection, guess))
            })
            .collect();
    }

    let chunk = (primary.len() + workers - 1) / workers;
    let mut slots: Vec<Option<matcher::LiveMatch>> = (0..primary.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        for (detections, results) in primary.chunks(chunk).zip(slots.chunks_mut(chunk)) {
            scope.spawn(move || {
                for (detection, slot) in detections.iter().zip(results.iter_mut()) {
                    *slot = match_proposal(raw, width, height, detection, pool);
                }
            });
        }
    });

    primary
        .into_iter()
        .zip(slots)
        .filter_map(|(detection, guess)| guess.map(|guess| (detection, guess)))
        .collect()
}

/// A classifier score for one proposal. The crop is only built on this path,
/// because the fingerprint scorer can sample the raw frame without a copy.
fn classifier_match(
    model: &mut crate::classifier::Classifier,
    pool: &matcher::Pool,
    raw: &[u8],
    width: u32,
    height: u32,
    detection: &Detection,
) -> Option<matcher::LiveMatch> {
    let crop = crop_region(raw, width, height, detection)?;
    let (index, probabilities) = model
        .classify(crop.as_raw(), crop.width(), crop.height())
        .ok()?;
    let score = *probabilities.get(index)?;
    if score < pool.floor {
        return None;
    }
    let label = model.labels().get(index)?;
    let class = pool
        .classes
        .iter()
        .find(|class| class.name.eq_ignore_ascii_case(label))?;
    Some(matcher::LiveMatch {
        class_id: class.id,
        name: class.name.clone(),
        score,
        threshold: class.threshold,
    })
}

/// Classifier scoring is serial because one ONNX session is not re-entered
/// from several threads. A handful of proposals still costs a few
/// milliseconds, and the fingerprint path keeps its parallel workers.
fn score_primary_classified(
    raw: &[u8],
    width: u32,
    height: u32,
    primary: Vec<Detection>,
    pool: &matcher::Pool,
    model: &mut crate::classifier::Classifier,
) -> Vec<(Detection, matcher::LiveMatch)> {
    primary
        .into_iter()
        .filter_map(|detection| {
            classifier_match(model, pool, raw, width, height, &detection)
                .map(|guess| (detection, guess))
        })
        .collect()
}

/// The inference clock. It takes whatever the newest frame is, thinks about it
/// for as long as the model needs, and publishes boxes with the capture time
/// attached so the overlay can say how old they are.
#[allow(clippy::too_many_arguments)]
fn inference_loop(
    app: AppHandle,
    project: Option<i64>,
    slot: Arc<FrameSlot>,
    stop: Arc<AtomicBool>,
    detector: Arc<Mutex<Option<crate::detector::Detector>>>,
    classifier: Arc<Mutex<Option<crate::classifier::Classifier>>>,
    board: Arc<StatsBoard>,
    preview_watch: Arc<AtomicBool>,
    overlay_visible: Arc<AtomicBool>,
    session: Option<crate::session::SessionStart>,
) {
    let mut pool = matcher::Pool::default();
    let mut pool_at = Instant::now() - Duration::from_secs(5);
    let mut tracker = Tracker::new();
    let mut harvested: u32 = 0;
    let mut harvest_cap = HARVEST_CAP_DEFAULT;
    let mut last_harvest = Instant::now() - HARVEST_INTERVAL;
    let mut harvest_root: Option<PathBuf> = None;
    let mut last_preview = Instant::now() - PREVIEW_INTERVAL;
    let mut recorded_frames: i64 = 0;
    let mut last_record = Instant::now() - RECORD_INTERVAL;
    let started = Instant::now();
    let mut last_processed = Instant::now();
    let mut session = session;
    let mut model_name: Option<String> = None;

    while !stop.load(Ordering::Relaxed) {
        let frame = slot.take_fresh(FRAME_FRESHNESS).or_else(|| {
            if last_processed.elapsed() >= STALE_FALLBACK {
                slot.take()
            } else {
                None
            }
        });
        let Some(frame) = frame else {
            thread::sleep(IDLE_POLL);
            continue;
        };
        last_processed = Instant::now();

        // Reload on a slow tick, so a new example starts scoring without
        // restarting the capture thread.
        if pool_at.elapsed() >= POOL_RELOAD {
            pool_at = Instant::now();
            if let Some(id) = project {
                let owned = app.state::<AppState>();
                let guard = owned.db.lock();
                let loaded = match guard.as_ref() {
                    Ok(conn) => {
                        harvest_cap = read_harvest_cap(conn);
                        matcher::load_pool(conn, id)
                    }
                    Err(error) => Err(error.to_string()),
                };
                drop(guard);
                drop(owned);
                match loaded {
                    Ok(next) => {
                        pool = next;
                        board.set(|values| {
                            values.pool_classes = pool.classes.len() as u32;
                            values.pool_examples = pool.examples as u32;
                        });
                    }
                    Err(error) => board.set(|values| values.note = Some(error)),
                }
            }
        }

        if preview_watch.load(Ordering::Relaxed) && last_preview.elapsed() >= PREVIEW_INTERVAL {
            let (bytes, width, height) =
                jpeg_preview(&frame.raw, frame.width, frame.height);
            let payload = FramePayload {
                data: format!(
                    "data:image/jpeg;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                ),
                width,
                height,
            };
            let _ = app.emit("spike:frame", payload);
            last_preview = Instant::now();
        }

        let Some(project) = project else {
            board.set(|values| {
                values.note =
                    Some("No project selected, so there is no pool to match against.".into())
            });
            continue;
        };

        // A classifier scores proposals without a fingerprint pool, so an
        // empty project is only dead when neither scorer is available.
        let classifier_ready = classifier
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false);
        if pool.examples == 0 && !classifier_ready {
            board.set(|values| {
                values.detections = 0;
                values.best_score = None;
                values.best_class = None;
                values.note = Some(
                    "No examples in this project yet, so no proposal can match a class.".to_string(),
                );
            });
            continue;
        }

        let Ok(mut guard) = detector.lock() else {
            continue;
        };
        let Some(model) = guard.as_mut() else {
            board.set(|values| {
                values.detections = 0;
                values.best_score = None;
                values.best_class = None;
                values.note = Some("No detector is loaded yet.".to_string());
            });
            continue;
        };
        model_name = Some(model.describe());

        match model.analyse_timed(
            &frame.raw,
            frame.width,
            frame.height,
            EXPANSION_SCORE_THRESHOLD,
        ) {
            Ok((proposals, timing)) => {
                board.set(|values| {
                    values.inference_ms = timing.run_ms;
                    values.preprocess_ms = timing.preprocess_ms;
                    values.note = None;
                });
                let match_started = Instant::now();
                let mut frame_best: Option<(String, f32)> = None;
                let mut primary = Vec::new();
                let mut secondary = Vec::new();
                for detection in &proposals {
                    if detection.w * detection.h >= matcher::MAX_PROPOSAL_AREA {
                        continue;
                    }
                    if detection.score >= SCORE_THRESHOLD {
                        primary.push(detection.clone());
                    } else {
                        secondary.push(detection.clone());
                    }
                }

                let mut scored = {
                    let mut held = classifier.lock().ok();
                    match held.as_deref_mut() {
                        Some(Some(model)) => score_primary_classified(
                            &frame.raw,
                            frame.width,
                            frame.height,
                            primary,
                            &pool,
                            model,
                        ),
                        _ => score_primary(&frame.raw, frame.width, frame.height, primary, &pool),
                    }
                };
                for (_, guess) in &scored {
                    if frame_best
                        .as_ref()
                        .map(|(_, score)| guess.score > *score)
                        .unwrap_or(true)
                    {
                        frame_best = Some((guess.name.clone(), guess.score));
                    }
                }

                // A proposal under its class's threshold is the one worth
                // asking about, as long as it clears the match floor. Do the
                // write after publishing the boxes, so encoding and disk work
                // cannot delay this frame's overlay.
                let harvestable: Vec<usize> = scored
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, guess))| {
                        guess.score < guess.threshold
                            && guess.score >= harvest_cutoff(guess.threshold, pool.floor)
                    })
                    .map(|(index, _)| index)
                    .collect();

                // Low-confidence proposals are only read when they can contain
                // a seed that already matched.
                if scored
                    .iter()
                    .any(|(_, hit)| hit.score >= hit.threshold)
                {
                    for detection in secondary {
                        if !scored.iter().any(|(seed, hit)| {
                            hit.score >= hit.threshold && matcher::encloses(&detection, seed)
                        }) {
                            continue;
                        }
                        let guess = {
                            let mut held = classifier.lock().ok();
                            match held.as_deref_mut() {
                                Some(Some(model)) => classifier_match(
                                    model,
                                    &pool,
                                    &frame.raw,
                                    frame.width,
                                    frame.height,
                                    &detection,
                                ),
                                _ => {
                                    let Some(crop) = crop_region(
                                        &frame.raw,
                                        frame.width,
                                        frame.height,
                                        &detection,
                                    ) else {
                                        continue;
                                    };
                                    let candidate = matcher::embed(&crop);
                                    pool.best(&candidate)
                                        .filter(|guess| guess.score >= pool.floor)
                                }
                            }
                        };
                        let Some(guess) = guess else {
                            continue;
                        };
                        scored.push((detection, guess));
                    }
                }

                let matched: Vec<Detection> = matcher::refine(
                    &scored,
                    SCORE_THRESHOLD,
                    EXPANSION_SCORE_THRESHOLD,
                )
                .into_iter()
                .map(|(mut detection, guess)| {
                    detection.label = guess.name;
                    detection.score = guess.score;
                    detection
                })
                .collect();
                let match_ms = match_started.elapsed().as_secs_f32() * 1000.0;
                let tracked = tracker.update(matched);
                let latency_ms = frame.captured_at.elapsed().as_secs_f32() * 1000.0;
                let published_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|value| value.as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let captured_at = published_at - latency_ms as f64;

                board.set(|values| {
                    values.inference_ms = timing.run_ms;
                    values.preprocess_ms = timing.preprocess_ms;
                    values.match_ms = match_ms;
                    values.latency_ms = latency_ms;
                    values.detections = tracked.len() as u32;
                    values.best_score = frame_best.as_ref().map(|(_, score)| *score);
                    values.best_class = frame_best.as_ref().map(|(name, _)| name.clone());
                });

                if let Some(session) = session.as_ref() {
                    if recorded_frames < RECORD_CAP && last_record.elapsed() >= RECORD_INTERVAL {
                        let at_ms = started.elapsed().as_millis() as i64;
                        if crate::session::record_raw(
                            &app,
                            session.id,
                            &session.root,
                            recorded_frames,
                            at_ms,
                            &frame.raw,
                            frame.width,
                            frame.height,
                            &proposals,
                            &tracked,
                        ) {
                            recorded_frames += 1;
                            last_record = Instant::now();
                        }
                    }
                }

                if preview_watch.load(Ordering::Relaxed)
                    || overlay_visible.load(Ordering::Relaxed)
                {
                    let _ = app.emit(
                        "spike:detections",
                        DetectionPayload {
                            width: frame.width,
                            height: frame.height,
                            inference_ms: timing.run_ms,
                            preprocess_ms: timing.preprocess_ms,
                            latency_ms,
                            captured_at_ms: captured_at,
                            published_at_ms: published_at,
                            crop: frame.crop,
                            colors: pool
                                .classes
                                .iter()
                                .map(|class| (class.name.clone(), class.color.clone()))
                                .collect(),
                            boxes: tracked,
                        },
                    );
                }

                for index in harvestable {
                    if harvested >= harvest_cap || last_harvest.elapsed() < HARVEST_INTERVAL {
                        break;
                    }
                    let (detection, guess) = &scored[index];
                    let Some(crop) =
                        crop_region(&frame.raw, frame.width, frame.height, detection)
                    else {
                        continue;
                    };
                    if save_candidate(
                        &app,
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
            Err(error) => board.set(|values| values.note = Some(error)),
        }
    }

    if let Some(session) = session.take() {
        let _ = crate::session::finish(&app, session.id);
    }
    let _ = model_name;
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
    if conn
        .execute(
            "insert into candidate (project_id, class_id, path, score, state, created_at)
             values (?1, ?2, ?3, ?4, 'pending', datetime('now'))",
            rusqlite::params![
                project_id,
                class_id,
                path.to_string_lossy().to_string(),
                score
            ],
        )
        .is_err()
    {
        return false;
    }
    let id = conn.last_insert_rowid();
    drop(conn);

    let thumbs = folder.join("thumbs");
    if std::fs::create_dir_all(&thumbs).is_ok() {
        let bytes = crate::label::jpeg_bytes(crop, 160, 70);
        if !bytes.is_empty() {
            let _ = std::fs::write(thumbs.join(format!("candidate-{id}-thumb.jpg")), bytes);
        }
    }
    true
}

/// Crops a normalized rectangle out of a raw RGBA buffer with one copy per row,
/// which is what keeps a region cheap enough to use per frame.
pub(crate) fn crop_rgba(
    raw: &[u8],
    width: u32,
    height: u32,
    crop: &Crop,
) -> Option<(Vec<u8>, u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    // A crop is stored normalized and validated when it is saved, so a rectangle
    // that is not inside the frame is a bug, not something to clamp silently.
    if crop.x < 0.0
        || crop.y < 0.0
        || crop.w <= 0.0
        || crop.h <= 0.0
        || crop.x + crop.w > 1.0001
        || crop.y + crop.h > 1.0001
    {
        return None;
    }
    let x0 = ((crop.x.clamp(0.0, 1.0) * width as f32).round() as u32).min(width - 1);
    let y0 = ((crop.y.clamp(0.0, 1.0) * height as f32).round() as u32).min(height - 1);
    let x1 = (((crop.x + crop.w).clamp(0.0, 1.0) * width as f32).round() as u32).min(width);
    let y1 = (((crop.y + crop.h).clamp(0.0, 1.0) * height as f32).round() as u32).min(height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let crop_width = x1 - x0;
    let crop_height = y1 - y0;
    let row_bytes = (crop_width * 4) as usize;
    let mut out = vec![0u8; row_bytes * crop_height as usize];
    for row in 0..crop_height {
        let source = (((y0 + row) * width + x0) * 4) as usize;
        let target = (row * row_bytes as u32) as usize;
        if source + row_bytes > raw.len() {
            return None;
        }
        out[target..target + row_bytes].copy_from_slice(&raw[source..source + row_bytes]);
    }
    Some((out, crop_width, crop_height))
}

/// Crops a normalized detection out of a raw RGBA frame.
pub(crate) fn crop_region(
    raw: &[u8],
    width: u32,
    height: u32,
    detection: &Detection,
) -> Option<image::RgbaImage> {
    let crop = Crop {
        x: detection.x,
        y: detection.y,
        w: detection.w,
        h: detection.h,
    };
    let (bytes, crop_width, crop_height) = crop_rgba(raw, width, height, &crop)?;
    image::RgbaImage::from_raw(crop_width, crop_height, bytes)
}

/// Downsamples the frame by sampling and encodes a small JPEG. The preview is
/// the one thing that crosses IPC every second, so a compressed image matters
/// more here than encode time.
pub(crate) fn jpeg_preview(raw: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    if width == 0 || height == 0 || raw.len() < 4 {
        return (Vec::new(), 0, 0);
    }

    let out_width = PREVIEW_WIDTH.min(width);
    let out_height = ((out_width as f32) * (height as f32) / (width as f32))
        .round()
        .max(1.0) as u32;
    let mut small = image::RgbImage::new(out_width, out_height);
    for y in 0..out_height {
        let source_y = (y as u64 * height as u64 / out_height as u64) as u32;
        for x in 0..out_width {
            let source_x = (x as u64 * width as u64 / out_width as u64) as u32;
            let source = ((source_y * width + source_x) * 4) as usize;
            if source + 3 < raw.len() {
                small.put_pixel(
                    x,
                    y,
                    image::Rgb([raw[source], raw[source + 1], raw[source + 2]]),
                );
            }
        }
    }

    let mut out = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 65);
    match encoder.encode_image(&small) {
        Ok(()) => (out, out_width, out_height),
        Err(_) => (Vec::new(), 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crop(x: f32, y: f32, w: f32, h: f32) -> Crop {
        Crop { x, y, w, h }
    }

    #[test]
    fn a_crop_reads_the_rectangle_it_names() {
        // Four pixels, each a different colour, cropped to the bottom right.
        let raw = vec![
            1, 0, 0, 255, 2, 0, 0, 255, //
            3, 0, 0, 255, 4, 0, 0, 255,
        ];
        let (bytes, width, height) = crop_rgba(&raw, 2, 2, &crop(0.5, 0.5, 0.5, 0.5)).unwrap();
        assert_eq!((width, height), (1, 1));
        assert_eq!(bytes, vec![4, 0, 0, 255]);
    }

    #[test]
    fn a_crop_outside_the_frame_is_refused() {
        let raw = vec![0u8; 16];
        assert!(crop_rgba(&raw, 2, 2, &crop(0.9, 0.9, 0.5, 0.5)).is_none());
        assert!(crop_rgba(&raw, 2, 2, &crop(0.0, 0.0, 0.0, 0.0)).is_none());
    }

    #[test]
    fn the_slot_keeps_only_the_newest_frame() {
        let slot = FrameSlot::default();
        let frame = |seq: u64| Frame {
            raw: Arc::new(FrameBuffer::plain(vec![0u8; 4])),
            width: 1,
            height: 1,
            seq,
            captured_at: Instant::now(),
            crop: None,
        };
        slot.publish(frame(1));
        slot.publish(frame(2));
        assert_eq!(slot.dropped(), 1);
        assert_eq!(slot.take().map(|frame| frame.seq), Some(2));
        assert!(slot.take().is_none());
    }

    #[test]
    fn a_stale_frame_is_left_for_the_next_publish() {
        let slot = FrameSlot::default();
        let frame = |seq: u64, age: Duration| Frame {
            raw: Arc::new(FrameBuffer::plain(vec![0u8; 4])),
            width: 1,
            height: 1,
            seq,
            captured_at: Instant::now() - age,
            crop: None,
        };
        slot.publish(frame(1, Duration::from_millis(30)));
        assert!(slot.take_fresh(Duration::from_millis(8)).is_none());
        assert_eq!(slot.take().map(|frame| frame.seq), Some(1));

        slot.publish(frame(2, Duration::ZERO));
        assert_eq!(
            slot.take_fresh(Duration::from_millis(8))
                .map(|frame| frame.seq),
            Some(2)
        );
    }

    #[test]
    fn percentiles_land_where_the_samples_say() {
        let mut histogram = Histogram::default();
        for value in 1..=100 {
            histogram.record(value as f32);
        }
        let (p50, p95, max) = histogram.summary();
        assert_eq!(p50, 50.0);
        assert_eq!(p95, 95.0);
        assert_eq!(max, 100.0);
        // The summary reads without consuming, so a status line and the final
        // gate summary describe the same samples rather than splitting them.
        let (again_p50, again_p95, again_max) = histogram.summary();
        assert_eq!((again_p50, again_p95, again_max), (50.0, 95.0, 100.0));
    }
}

#[cfg(test)]
mod harvest_tests {
    use super::{
        harvest_cutoff, harvest_floor, read_harvest_cap, HARVEST_CAP_DEFAULT, HARVEST_CAP_MAX,
    };

    #[test]
    fn the_floor_decides_where_harvesting_starts() {
        // A floor set by the user replaces the near-miss band, so anything
        // between it and the class threshold becomes reviewable.
        assert_eq!(harvest_cutoff(0.85, 0.45), 0.45);
        assert_eq!(harvest_cutoff(0.70, 0.63), 0.63);
    }

    #[test]
    fn the_cap_setting_is_clamped_and_defaults_when_unset() {
        let conn = rusqlite::Connection::open_in_memory().expect("memory db");
        conn.execute_batch("create table setting (key text primary key, value text not null);")
            .expect("setting table");
        assert_eq!(read_harvest_cap(&conn), HARVEST_CAP_DEFAULT);
        conn.execute(
            "insert into setting (key, value) values ('capture.harvest_cap', '120')",
            [],
        )
        .expect("cap row");
        assert_eq!(read_harvest_cap(&conn), 120);
        conn.execute(
            "update setting set value = '9000' where key = 'capture.harvest_cap'",
            [],
        )
        .expect("cap row");
        assert_eq!(read_harvest_cap(&conn), HARVEST_CAP_MAX);
        conn.execute(
            "update setting set value = 'many' where key = 'capture.harvest_cap'",
            [],
        )
        .expect("cap row");
        assert_eq!(read_harvest_cap(&conn), HARVEST_CAP_DEFAULT);
    }

    #[test]
    fn with_no_floor_the_band_still_guards_the_queue() {
        // Floor zero means off, so the band still keeps the queue from filling
        // with proposals that are nowhere near the class.
        assert_eq!(harvest_cutoff(0.70, 0.0), harvest_floor(0.70));
        assert!((harvest_cutoff(0.85, 0.0) - 0.765).abs() < 1e-6);
    }
}

#[cfg(test)]
mod cadence_tests {
    use super::cadence_for;

    #[test]
    fn cadence_follows_the_display_period() {
        // 143 Hz: 15 ms falls into the three frame bucket (47.7 fps), 12 ms
        // lands on two frames (71.5 fps).
        assert_eq!(cadence_for(143, 15), 13);
        // A 60 Hz display cannot beat the requested cap, so the cadence stays
        // at 15 ms; frames still arrive every 16.7 ms, so every one is taken.
        assert_eq!(cadence_for(60, 15), 15);
        // 165 Hz would need three frames for 18 ms, but the 15 ms request caps
        // it; 15 ms is 66.7 fps, still under the 72 fps ceiling.
        assert_eq!(cadence_for(165, 15), 15);
        // 240 Hz lands on four frames, but the 15 ms request still caps it:
        // the next frame after 15 ms is the fourth at 16.7 ms.
        assert_eq!(cadence_for(240, 15), 15);
    }
}
