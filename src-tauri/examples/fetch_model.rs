//! Fetch one pinned detector from the catalog using the same code path the app uses.
//!
//! cargo run --example fetch_model
//! cargo run --example fetch_model -- rfdetr-small

use std::env;
use std::path::PathBuf;

use irecon_lib::model;

fn main() -> Result<(), String> {
    let home = env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let id = env::args().nth(1).unwrap_or_else(|| "rfdetr-nano".to_string());
    let entry = model::spec(&id).ok_or_else(|| {
        let ids: Vec<&str> = model::CATALOG.iter().map(|entry| entry.id).collect();
        format!("Unknown model {id}. Choose one of: {}.", ids.join(", "))
    })?;

    let directory = PathBuf::from(home)
        .join("Library/Application Support/com.emrisca.irecon/models");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let destination = env::var("IRECON_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| directory.join(entry.file));

    println!("model: {}", entry.name);
    println!("source: {}", entry.url);
    println!("target: {}", destination.display());
    println!("sha256: {}", entry.sha256);

    let mut last = 0u64;
    model::fetch(
        entry.url,
        &destination,
        entry.sha256,
        entry.bytes,
        |done, total| {
            if done.saturating_sub(last) >= 8 * 1024 * 1024 || done == total {
                last = done;
                println!("{} of {} MB", done / 1_000_000, total / 1_000_000);
            }
        },
    )?;

    let (digest, bytes) = model::sha256_file(&destination)?;
    println!("verified {bytes} bytes, sha256 {digest}");
    Ok(())
}
