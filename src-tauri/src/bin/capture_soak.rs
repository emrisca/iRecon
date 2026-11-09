//! The Phase 0 gate harness. Run it from `apps/irecon/src-tauri`:
//!
//! ```text
//! cargo run --release --bin capture_soak -- --seconds 600
//! ```
//!
//! It drives the production capture clock at full primary display resolution
//! through the same Graphics Capture handler the live loop uses: no crop, no
//! preview encoding, no SQLite, no model. The default window is ten minutes, so
//! the sixty-second sustained requirement is contained inside it.
//!
//! Exit code 0 means G1, G2 and G5 passed. Exit code 1 means at least one missed
//! and the numbers above say which.

use std::process::ExitCode;

use irecon_lib::soak::{self, SoakConfig, MAX_COST_P50_MS, MAX_COST_P95_MS, MAX_DROP_PCT, MAX_RSS_GROWTH_MB, MIN_FPS};

fn main() -> ExitCode {
    let mut config = SoakConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                if let Some(value) = args.next() {
                    match value.parse::<f32>() {
                        Ok(seconds) if seconds > 0.0 => config.seconds = seconds,
                        _ => {
                            eprintln!("--seconds needs a positive number, got {value}");
                            return ExitCode::from(2);
                        }
                    }
                }
            }
            "--interval-ms" => {
                if let Some(value) = args.next() {
                    match value.parse::<u64>() {
                        Ok(interval) if interval > 0 => config.interval_ms = interval,
                        _ => {
                            eprintln!("--interval-ms needs a positive integer, got {value}");
                            return ExitCode::from(2);
                        }
                    }
                }
            }
            "--help" | "-h" => {
                println!("capture_soak [--seconds 600] [--interval-ms 16]");
                println!();
                println!("Runs the production capture clock on the primary display and");
                println!("prints G1 (rate), G2 (cost) and G5 (RSS, drops, errors).");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let requested = config.seconds;
    let interval = config.interval_ms;
    println!(
        "soak          starting a {:.0} s run at a {} ms update interval",
        requested, interval
    );

    let report = match soak::run(config) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("soak          failed: {error}");
            return ExitCode::from(2);
        }
    };

    let summary = &report.summary;
    println!(
        "capture       {} {}x{}  backend {}  cadence {} ms  wall {:.1} s",
        summary.label, summary.width, summary.height, summary.backend, report.cadence_ms, summary.elapsed_s
    );
    println!(
        "frames        {} delivered  {} overwritten ({:.2} %)  {} errors  {} pickups",
        summary.frames,
        summary.overwrites,
        report.drop_pct(),
        summary.errors,
        report.pickups
    );
    println!(
        "rate          {:.1} fps over the run  {:.1} fps over the last {:.0} s",
        summary.fps_total, summary.fps_window, summary.window_s
    );
    println!(
        "capture cost  p50 {:.2} ms  p95 {:.2} ms  max {:.2} ms  (frame interval p50 {:.1} ms  p95 {:.1} ms)",
        summary.cost_p50, summary.cost_p95, summary.cost_max, summary.gap_p50, summary.gap_p95
    );
    println!(
        "publish       p50 {:.2} ms  p95 {:.2} ms  max {:.2} ms  (capture callback to slot publish)",
        summary.publish_p50, summary.publish_p95, summary.publish_max
    );
    println!(
        "pickup        p50 {:.2} ms  p95 {:.2} ms  max {:.2} ms  (to consumer, excludes model and paint)",
        report.pickup_p50_ms, report.pickup_p95_ms, report.pickup_max_ms
    );
    println!(
        "rss           start {:.1} MB  end {:.1} MB  growth {:+.1} MB",
        report.rss_start_mb(),
        report.rss_end_mb(),
        report.rss_growth_mb()
    );
    let marks = report
        .rss
        .iter()
        .map(|sample| format!("{:.0}s {:.1} MB", sample.at_s, sample.mb))
        .collect::<Vec<_>>()
        .join("  ");
    println!("rss marks     {marks}");
    if let Some(note) = &summary.note {
        println!("note          {note}");
    }

    let gate = |passed: bool| if passed { "PASS" } else { "FAIL" };
    println!(
        "G1 rate       {}  {:.1} fps run / {:.1} fps last {:.0} s, floor {:.0}",
        gate(report.gate_g1()),
        summary.fps_total,
        summary.fps_window,
        summary.window_s,
        MIN_FPS
    );
    println!(
        "G2 cost       {}  p50 {:.2} ms (max {:.0})  p95 {:.2} ms (max {:.0})",
        gate(report.gate_g2()),
        summary.cost_p50,
        MAX_COST_P50_MS,
        summary.cost_p95,
        MAX_COST_P95_MS
    );
    println!(
        "G5 soak       {}  growth {:+.1} MB (max {:.0})  drops {:.2} % (max {:.1})  errors {}",
        gate(report.gate_g5()),
        report.rss_growth_mb(),
        MAX_RSS_GROWTH_MB,
        report.drop_pct(),
        MAX_DROP_PCT,
        summary.errors
    );

    if report.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
