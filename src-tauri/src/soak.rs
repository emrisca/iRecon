//! The Phase 0 soak harness.
//!
//! It drives the production capture clock: the same `Run`, the same frame slot,
//! the same Graphics Capture handler and settings the live loop uses. What it
//! leaves out is everything after the slot (the model, preview encoding, SQLite,
//! the overlay), because those are measured in the app or need a human.
//!
//! One command, from `apps/irecon/src-tauri`:
//!
//! ```text
//! cargo run --release --bin capture_soak -- --seconds 600
//! ```
//!
//! It exits non-zero when G1, G2 or G5 miss, so it can be run unattended.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::capture::{self, CaptureSink, CaptureStats, FrameSlot, Run, RunSummary, StatsBoard};

/// G1: sustained capture rate, over the whole run and over its last 60 s.
pub const MIN_FPS: f32 = 55.0;
/// G2: per-frame capture cost, nearest rank.
pub const MAX_COST_P50_MS: f32 = 12.0;
pub const MAX_COST_P95_MS: f32 = 20.0;
/// G5: working set growth across the run.
pub const MAX_RSS_GROWTH_MB: f32 = 50.0;
/// G5: overwritten frames as a share of everything the capture clock produced.
pub const MAX_DROP_PCT: f32 = 1.0;

/// A sink with nowhere to go: the harness reads the counters once, at the end.
struct NullSink;

impl CaptureSink for NullSink {
    fn publish(&self, _stats: &CaptureStats) {}

    fn model(&self) -> Option<String> {
        None
    }
}

pub struct SoakConfig {
    pub seconds: f32,
    pub interval_ms: u64,
}

impl Default for SoakConfig {
    fn default() -> Self {
        Self {
            seconds: 600.0,
            interval_ms: 15,
        }
    }
}

#[derive(Clone)]
pub struct RssSample {
    pub at_s: f32,
    pub mb: f32,
}

pub struct SoakReport {
    pub summary: RunSummary,
    pub interval_ms: u64,
    /// The interval the capture clock actually used on this display.
    pub cadence_ms: u64,
    pub requested_seconds: f32,
    /// Frames a consumer took off the slot, which is what the inference clock
    /// sees. Zero pickups means nothing ever reached a consumer.
    pub pickups: u64,
    /// Capture callback to consumer pickup. This excludes the model and the
    /// paint, so it is a floor, not G3.
    pub pickup_p50_ms: f32,
    pub pickup_p95_ms: f32,
    pub pickup_max_ms: f32,
    pub rss: Vec<RssSample>,
}

impl SoakReport {
    pub fn rss_start_mb(&self) -> f32 {
        self.rss.first().map(|sample| sample.mb).unwrap_or(0.0)
    }

    pub fn rss_end_mb(&self) -> f32 {
        self.rss.last().map(|sample| sample.mb).unwrap_or(0.0)
    }

    pub fn rss_growth_mb(&self) -> f32 {
        self.rss_end_mb() - self.rss_start_mb()
    }

    pub fn drop_pct(&self) -> f32 {
        let total = self.summary.frames + self.summary.overwrites;
        if total == 0 {
            return 100.0;
        }
        self.summary.overwrites as f32 / total as f32 * 100.0
    }

    pub fn gate_g1(&self) -> bool {
        self.summary.frames > 0
            && self.summary.fps_total >= MIN_FPS
            && self.summary.fps_window >= MIN_FPS
    }

    pub fn gate_g2(&self) -> bool {
        self.summary.cost_p50 > 0.0
            && self.summary.cost_p50 <= MAX_COST_P50_MS
            && self.summary.cost_p95 <= MAX_COST_P95_MS
    }

    pub fn gate_g5(&self) -> bool {
        self.rss_growth_mb() <= MAX_RSS_GROWTH_MB
            && self.drop_pct() <= MAX_DROP_PCT
            && self.summary.errors == 0
    }

    pub fn passed(&self) -> bool {
        self.gate_g1() && self.gate_g2() && self.gate_g5()
    }
}

/// Runs the capture clock for the requested wall-clock window, sampling RSS at
/// the start, every minute, and at the end.
pub fn run(config: SoakConfig) -> Result<SoakReport, String> {
    let seconds = config.seconds.max(1.0);
    let slot = Arc::new(FrameSlot::default());
    let board = Arc::new(StatsBoard::default());
    let stop = Arc::new(AtomicBool::new(false));
    let sink: Arc<dyn CaptureSink> = Arc::new(NullSink);
    let run = Arc::new(Mutex::new(Run::new(
        sink,
        slot.clone(),
        board.clone(),
        stop.clone(),
        "Primary display".to_string(),
        None,
    )));

    let rss_start = rss_mb();

    let (done_tx, done_rx) = mpsc::channel::<Result<(), String>>();
    let capture_run = run.clone();
    let capture_thread = thread::Builder::new()
        .name("soak-capture".into())
        .spawn(move || {
            let outcome = capture::display_run(&capture_run, "", config.interval_ms);
            if let Ok(mut guard) = capture_run.lock() {
                guard.finish(outcome.as_ref().err().cloned());
            }
            let _ = done_tx.send(outcome);
        })
        .map_err(|error| format!("Could not start the capture thread: {error}"))?;

    let consumer_slot = slot.clone();
    let consumer_stop = stop.clone();
    let consumer = thread::Builder::new()
        .name("soak-consumer".into())
        .spawn(move || {
            let mut ages: Vec<f32> = Vec::new();
            let mut pickups = 0u64;
            while !consumer_stop.load(Ordering::Relaxed) {
                match consumer_slot.take() {
                    Some(frame) => {
                        ages.push(frame.captured_at.elapsed().as_secs_f32() * 1000.0);
                        pickups += 1;
                    }
                    None => thread::sleep(Duration::from_millis(2)),
                }
            }
            (pickups, ages)
        })
        .map_err(|error| format!("Could not start the consumer thread: {error}"))?;

    let started = Instant::now();
    // The buffer pool settles within a couple of frames. Sampling before that
    // would count its unavoidable three buffers as growth, which is not what the
    // gate is for; the gate is about the run leaking, not warming up.
    while started.elapsed().as_secs_f32() < 2.0 {
        thread::sleep(Duration::from_millis(50));
    }
    let mut rss = vec![RssSample {
        at_s: started.elapsed().as_secs_f32(),
        mb: rss_mb().or(rss_start).unwrap_or(0.0),
    }];
    let mut next_mark = 60.0f32;
    while started.elapsed().as_secs_f32() < seconds {
        thread::sleep(Duration::from_millis(200));
        let elapsed = started.elapsed().as_secs_f32();
        if elapsed >= next_mark {
            rss.push(RssSample {
                at_s: next_mark,
                mb: rss_mb().unwrap_or(0.0),
            });
            next_mark += 60.0;
        }
    }
    let elapsed = started.elapsed().as_secs_f32();
    rss.push(RssSample {
        at_s: elapsed,
        mb: rss_mb().unwrap_or(0.0),
    });

    stop.store(true, Ordering::Relaxed);
    // Graphics Capture stops through its control handle, so this returns even
    // when the desktop is completely still; the timeout is only a guard.
    let stopped = done_rx.recv_timeout(Duration::from_secs(5)).is_ok();

    let (pickups, ages) = consumer.join().unwrap_or((0, Vec::new()));
    let summary = run
        .lock()
        .map(|guard| guard.summary())
        .map_err(|_| "The capture state was poisoned".to_string())?;
    let (pickup_p50_ms, pickup_p95_ms, pickup_max_ms) = percentiles(ages);

    let mut summary = summary;
    if !stopped && summary.note.is_none() {
        summary.note = Some("The capture thread did not acknowledge the stop".into());
    }
    let _ = capture_thread;

    Ok(SoakReport {
        summary,
        interval_ms: config.interval_ms,
        cadence_ms: capture::display_cadence_ms(config.interval_ms),
        requested_seconds: seconds,
        pickups,
        pickup_p50_ms,
        pickup_p95_ms,
        pickup_max_ms,
        rss,
    })
}

/// Nearest rank, the same rule the capture histograms use.
fn percentiles(mut samples: Vec<f32>) -> (f32, f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    samples.sort_by(|left, right| left.total_cmp(right));
    let count = samples.len();
    let p50 = samples[(count - 1) / 2];
    let p95 = samples[((count as f32 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(count - 1)];
    (p50, p95, samples[count - 1])
}

#[cfg(windows)]
fn rss_mb() -> Option<f32> {
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }

    let mut counters = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<ProcessMemoryCounters>() as u32,
        )
    };
    if ok == 0 {
        None
    } else {
        Some(counters.working_set_size as f32 / (1024.0 * 1024.0))
    }
}

#[cfg(not(windows))]
fn rss_mb() -> Option<f32> {
    None
}
