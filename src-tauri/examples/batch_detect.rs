//! Run the shipped detector over a folder of frames and write CSV, for the
//! Phase 0 question B measurement.
//!
//! cargo run --example batch_detect -- path/to/frames
//! cargo run --example batch_detect -- path/to/frames --out /tmp/batch --threshold 0.3
//!
//! Writes three files into the output folder:
//!   frames.csv   one row per frame: size, boxes, car-like boxes, inference ms
//!   boxes.csv    one row per box: file, label, score, normalized corners
//!   classes.csv  one row per class: total boxes across the folder
//!
//! Recall is deliberately absent: it needs a human count of the objects that are
//! actually visible in each frame. The car-like column is the numerator.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use irecon_lib::detector::Detector;
use irecon_lib::model;

const CAR_LABELS: [&str; 4] = ["car", "truck", "bus", "motorcycle"];
const FRAME_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

fn usage() {
    println!("usage: batch_detect <frames-dir> [--out dir] [--model file] [--threshold 0.35] [--limit n]");
    println!();
    println!("  frames-dir   folder to scan for png and jpeg frames, subfolders included");
    println!("  --out        where the CSV files go, default ./irecon-batch");
    println!("  --model      ONNX file, default IRECON_MODEL or the downloaded app model");
    println!("  --threshold  score cutoff, default 0.35");
    println!("  --limit      stop after this many frames");
}

fn main() -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut model_arg: Option<PathBuf> = None;
    let mut threshold = 0.35f32;
    let mut limit: Option<usize> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                out_dir = Some(PathBuf::from(
                    args.next().ok_or("--out needs a folder")?,
                ))
            }
            "--model" => {
                model_arg = Some(PathBuf::from(
                    args.next().ok_or("--model needs a file")?,
                ))
            }
            "--threshold" => {
                let raw = args.next().ok_or("--threshold needs a number")?;
                threshold = raw
                    .parse()
                    .map_err(|_| format!("{raw} is not a number"))?;
                if !(0.0..=1.0).contains(&threshold) {
                    return Err("--threshold must be between 0 and 1".into());
                }
            }
            "--limit" => {
                let raw = args.next().ok_or("--limit needs a count")?;
                limit = Some(
                    raw.parse()
                        .map_err(|_| format!("{raw} is not a count"))?,
                );
            }
            "-h" | "--help" => {
                usage();
                return Ok(());
            }
            other if other.starts_with("--") => {
                return Err(format!("Unknown option {other}. Try --help."))
            }
            other => input = Some(PathBuf::from(other)),
        }
    }

    let input = input.ok_or("Pass a folder of frames. Try --help.")?;
    if !input.is_dir() {
        return Err(format!("{} is not a folder", input.display()));
    }

    let frames = collect_frames(&input, limit)?;
    if frames.is_empty() {
        return Err(format!(
            "No png or jpeg frames in {}",
            input.display()
        ));
    }

    let model_path = match model_arg {
        Some(path) => path,
        None => match env::var("IRECON_MODEL") {
            Ok(path) => PathBuf::from(path),
            Err(_) => default_model_path()?,
        },
    };
    if !model_path.exists() {
        return Err(format!(
            "No model at {}. Run `cargo run --example fetch_model` first.",
            model_path.display()
        ));
    }

    let (digest, model_bytes) = model::sha256_file(&model_path)?;
    if digest != model::MODEL_SHA256 {
        return Err(format!(
            "{} is not the pinned model ({} bytes, sha256 {digest}).",
            model_path.display(),
            model_bytes
        ));
    }

    let mut detector = Detector::load(&model_path)?;
    println!("{}", detector.describe());
    println!(
        "model {} MB, hash verified, threshold {threshold:.2}",
        model_bytes / 1_000_000
    );
    println!("{} frames in {}", frames.len(), input.display());

    let out_dir = out_dir.unwrap_or_else(|| PathBuf::from("irecon-batch"));
    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Could not create {}: {e}", out_dir.display()))?;

    let mut frames_csv = String::from(
        "file,width,height,boxes,car_boxes,inference_ms,top_class,top_score\n",
    );
    let mut boxes_csv = String::from("file,label,score,x,y,w,h\n");
    let mut class_totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut timings: Vec<f32> = Vec::new();
    let mut total_boxes = 0usize;
    let mut car_boxes = 0usize;
    let mut empty_frames = 0usize;
    let mut skipped = 0usize;

    let started = Instant::now();
    for (index, path) in frames.iter().enumerate() {
        let image = match image::open(path) {
            Ok(image) => image.to_rgba8(),
            Err(error) => {
                skipped += 1;
                eprintln!("skip {}: {error}", path.display());
                continue;
            }
        };
        let (width, height) = (image.width(), image.height());
        let (boxes, elapsed) = detector.analyse(image.as_raw(), width, height, threshold)?;

        timings.push(elapsed);
        total_boxes += boxes.len();
        if boxes.is_empty() {
            empty_frames += 1;
        }

        let cars = boxes
            .iter()
            .filter(|detection| CAR_LABELS.contains(&detection.label.as_str()))
            .count();
        car_boxes += cars;

        let name = path
            .strip_prefix(&input)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let (top_class, top_score) = boxes
            .first()
            .map(|detection| (detection.label.clone(), detection.score))
            .unwrap_or_else(|| (String::new(), 0.0));

        for detection in &boxes {
            *class_totals.entry(detection.label.clone()).or_insert(0) += 1;
            boxes_csv.push_str(&format!(
                "{},{},{:.3},{:.4},{:.4},{:.4},{:.4}\n",
                csv_field(&name),
                csv_field(&detection.label),
                detection.score,
                detection.x,
                detection.y,
                detection.w,
                detection.h
            ));
        }

        frames_csv.push_str(&format!(
            "{},{width},{height},{},{cars},{elapsed:.1},{},{top_score:.3}\n",
            csv_field(&name),
            boxes.len(),
            csv_field(&top_class)
        ));

        if (index + 1) % 10 == 0 || index + 1 == frames.len() {
            println!(
                "{} of {} frames, {:0.1} s elapsed",
                index + 1,
                frames.len(),
                started.elapsed().as_secs_f32()
            );
        }
    }

    let wall = started.elapsed().as_secs_f32();
    timings.sort_by(|left, right| left.total_cmp(right));
    let mean = if timings.is_empty() {
        0.0
    } else {
        timings.iter().sum::<f32>() / timings.len() as f32
    };

    let mut classes_csv = String::from("class,boxes\n");
    for (label, count) in &class_totals {
        classes_csv.push_str(&format!("{},{count}\n", csv_field(label)));
    }

    fs::write(out_dir.join("frames.csv"), frames_csv)
        .map_err(|e| format!("Could not write frames.csv: {e}"))?;
    fs::write(out_dir.join("boxes.csv"), boxes_csv)
        .map_err(|e| format!("Could not write boxes.csv: {e}"))?;
    fs::write(out_dir.join("classes.csv"), classes_csv)
        .map_err(|e| format!("Could not write classes.csv: {e}"))?;

    let analysed = timings.len().max(1);
    println!();
    println!(
        "{} frames analysed, {skipped} skipped, {total_boxes} boxes, {empty_frames} frames with no boxes",
        timings.len()
    );
    println!(
        "inference ms: mean {mean:.1}, p50 {:.1}, p95 {:.1}, max {:.1}",
        percentile(&timings, 0.50),
        percentile(&timings, 0.95),
        timings.last().copied().unwrap_or(0.0)
    );
    println!(
        "wall {wall:.1} s, {:.1} frames per second end to end",
        analysed as f32 / wall.max(0.001)
    );
    println!("car-like boxes: {car_boxes}");
    let cars = class_totals.get("car").copied().unwrap_or(0);
    println!("boxes labelled car: {cars}");
    if !class_totals.is_empty() {
        let mut pairs: Vec<String> = class_totals
            .iter()
            .map(|(label, count)| format!("{label} {count}"))
            .collect();
        pairs.truncate(12);
        println!("top classes: {}", pairs.join(", "));
    }
    println!(
        "wrote {}, {}, {}",
        out_dir.join("frames.csv").display(),
        out_dir.join("boxes.csv").display(),
        out_dir.join("classes.csv").display()
    );
    println!("recall needs a human count of visible objects; car-like boxes above is the numerator");

    Ok(())
}

fn default_model_path() -> Result<PathBuf, String> {
    let home = env::var("HOME").map_err(|_| {
        "Set IRECON_MODEL or pass --model, there is no HOME to find the downloaded model".to_string()
    })?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support/com.emrisca.irecon/models/rf-detr-nano.onnx"))
}

fn collect_frames(dir: &Path, limit: Option<usize>) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current)
            .map_err(|e| format!("Could not read {}: {e}", current.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
                if name.starts_with('.') || name == "irecon-batch" {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let wanted = path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| {
                    FRAME_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
                })
                .unwrap_or(false);
            if wanted {
                files.push(path);
            }
        }
    }

    files.sort();
    if let Some(limit) = limit {
        files.truncate(limit);
    }
    Ok(files)
}

fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f32 - 1.0) * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
