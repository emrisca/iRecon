//! Fetch the pinned RF-DETR Nano ONNX export using the same code path the app uses.
//!
//! cargo run --example fetch_model

use std::env;
use std::path::PathBuf;

use irecon_lib::model;

fn main() -> Result<(), String> {
    let home = env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let destination = env::var("IRECON_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(home)
                .join("Library/Application Support/com.emrisca.irecon/models/rf-detr-nano.onnx")
        });

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    println!("source: {}", model::MODEL_URL);
    println!("target: {}", destination.display());
    println!("sha256: {}", model::MODEL_SHA256);

    let mut last = 0u64;
    model::fetch(model::MODEL_URL, &destination, model::MODEL_SHA256, |done, total| {
        if done.saturating_sub(last) >= 8 * 1024 * 1024 || done == total {
            last = done;
            println!("{} of {} MB", done / 1_000_000, total / 1_000_000);
        }
    })?;

    let (digest, bytes) = model::sha256_file(&destination)?;
    println!("verified {bytes} bytes, sha256 {digest}");
    Ok(())
}
