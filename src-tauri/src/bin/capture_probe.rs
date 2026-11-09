//! Capture probe: the measurement half of the M1 capture sprint.
//!
//! It runs without the UI so a backend can be compared on the same machine and
//! the same scene without a dev build in the way. It measures the still path
//! (the old loop), the duplication path (the new live loop), and, when built
//! then the Graphics Capture path.
//!
//! ```text
//! cargo run --release --bin capture_probe -- 10
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use xcap::Monitor;

fn summary(mut samples: Vec<f32>) -> (f32, f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    samples.sort_by(|left, right| left.total_cmp(right));
    let count = samples.len();
    let p50 = samples[(count - 1) / 2];
    let p95 = samples[((count as f32 * 0.95) as usize).min(count - 1)];
    let max = samples[count - 1];
    (p50, p95, max)
}

fn still(monitor: &Monitor, seconds: f32) {
    let mut samples = Vec::new();
    let started = Instant::now();
    while started.elapsed().as_secs_f32() < seconds {
        let tick = Instant::now();
        match monitor.capture_image() {
            Ok(_) => samples.push(tick.elapsed().as_secs_f32() * 1000.0),
            Err(error) => {
                println!("still: {error}");
                return;
            }
        }
    }
    let (p50, p95, max) = summary(samples.clone());
    let fps = samples.len() as f32 / seconds;
    println!(
        "still        frames {:<5} fps {:>6.1}  p50 {:>7.1} ms  p95 {:>7.1} ms  max {:>7.1} ms",
        samples.len(),
        fps,
        p50,
        p95,
        max
    );
}

fn duplication(monitor: &Monitor, seconds: f32) {
    let (recorder, receiver) = match monitor.video_recorder() {
        Ok(pair) => pair,
        Err(error) => {
            println!("duplication  refused: {error}");
            return;
        }
    };
    if let Err(error) = recorder.start() {
        println!("duplication  could not start: {error}");
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let mut arrivals: Vec<f32> = Vec::new();
        let mut last: Option<Instant> = None;
        let mut bytes = 0usize;
        while !worker_stop.load(Ordering::Relaxed) && started.elapsed().as_secs_f32() < seconds {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(frame) => {
                    let now = Instant::now();
                    if let Some(previous) = last {
                        arrivals.push(now.duration_since(previous).as_secs_f32() * 1000.0);
                    }
                    last = Some(now);
                    bytes = frame.raw.len();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        (arrivals, bytes, started.elapsed().as_secs_f32())
    });
    let (arrivals, bytes, elapsed) = worker.join().unwrap_or((Vec::new(), 0, seconds));
    stop.store(true, Ordering::Relaxed);
    let _ = recorder.stop();
    let frames = arrivals.len().max(1);
    let (p50, p95, max) = summary(arrivals);
    println!(
        "duplication  frames {:<5} fps {:>6.1}  gap p50 {:>6.1} ms  p95 {:>6.1} ms  max {:>6.1} ms  frame {:>6.1} MB",
        frames,
        frames as f32 / elapsed.max(0.001),
        p50,
        p95,
        max,
        bytes as f32 / 1_000_000.0
    );
}

#[cfg(windows)]
fn graphics_capture(monitor: &Monitor, seconds: f32) {
    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::monitor::Monitor as WgcMonitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    };

    struct Probe {
        started: Instant,
        seconds: f32,
        last: Option<Instant>,
        gaps: Arc<std::sync::Mutex<Vec<f32>>>,
        bytes: Arc<std::sync::atomic::AtomicU64>,
    }

    impl GraphicsCaptureApiHandler for Probe {
        type Flags = (
            f32,
            Arc<std::sync::Mutex<Vec<f32>>>,
            Arc<std::sync::atomic::AtomicU64>,
        );
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self {
                started: Instant::now(),
                seconds: ctx.flags.0,
                last: None,
                gaps: ctx.flags.1,
                bytes: ctx.flags.2,
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            capture_control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let now = Instant::now();
            if let Some(previous) = self.last {
                if let Ok(mut gaps) = self.gaps.lock() {
                    gaps.push(now.duration_since(previous).as_secs_f32() * 1000.0);
                }
            }
            self.last = Some(now);
            // The same copy the live handler makes, so this number is the cost
            // the app actually pays rather than the cost of an idle callback.
            if let Ok(mut buffer) = frame.buffer() {
                let padded = buffer.has_padding();
                let pitch = buffer.row_pitch() as usize;
                let row = buffer.width() as usize * 4;
                let rows = buffer.height() as usize;
                let source = buffer.as_raw_buffer();
                let copied = if padded && pitch >= row {
                    let mut total = 0usize;
                    for y in 0..rows {
                        let start = y * pitch;
                        if start + row > source.len() {
                            break;
                        }
                        total += row;
                    }
                    total
                } else {
                    source.len()
                };
                self.bytes
                    .fetch_add(copied as u64, std::sync::atomic::Ordering::Relaxed);
            }
            if self.started.elapsed().as_secs_f32() >= self.seconds {
                capture_control.stop();
            }
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    let Ok(wgc_monitor) = WgcMonitor::primary() else {
        println!("graphics      could not read the primary monitor");
        return;
    };
    let _ = monitor;
    let gaps = Arc::new(std::sync::Mutex::new(Vec::new()));
    let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let settings = Settings::new(
        wgc_monitor,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Include,
        MinimumUpdateIntervalSettings::Custom(Duration::from_millis(1)),
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        (seconds, gaps.clone(), bytes.clone()),
    );
    let started = Instant::now();
    match Probe::start(settings) {
        Ok(()) => {
            let collected = gaps
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default();
            let frames = collected.len().max(1);
            let (p50, p95, max) = summary(collected);
            let elapsed = started.elapsed().as_secs_f32().max(0.001);
            println!(
                "graphics     frames {:<5} fps {:>6.1}  gap p50 {:>6.1} ms  p95 {:>6.1} ms  max {:>6.1} ms  copy {:>5.0} MB/s",
                frames,
                frames as f32 / elapsed,
                p50,
                p95,
                max,
                bytes.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1_000_000.0 / elapsed
            );
        }
        Err(error) => println!("graphics      failed: {error}"),
    }
}

fn main() {
    let seconds: f32 = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(6.0);
    let monitors = Monitor::all().expect("could not list displays");
    let monitor = monitors
        .iter()
        .find(|monitor| monitor.is_primary().unwrap_or(false))
        .cloned()
        .or_else(|| monitors.first().cloned())
        .expect("no display found");
    println!(
        "display      {} {}x{}  sampling {:.0} s",
        monitor.name().unwrap_or_else(|_| "primary".into()),
        monitor.width().unwrap_or(0),
        monitor.height().unwrap_or(0),
        seconds
    );
    still(&monitor, seconds);
    duplication(&monitor, seconds);
    #[cfg(windows)]
    graphics_capture(&monitor, seconds);
}
