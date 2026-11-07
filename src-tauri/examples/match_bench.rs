//! Measures how well the fingerprint separates labelled crops.
//!
//! cargo run --example match_bench -- path/to/crops
//! cargo run --example match_bench -- --demo
//!
//! The folder form expects one subfolder per class:
//!
//!   crops/crate/*.png
//!   crops/sky/*.png
//!   crops/__none__/*.png     (unknown objects, measured as false accepts)
//!
//! Every crop is scored leave-one-out against the rest of the pool, then a
//! threshold sweep reports coverage, precision and the false accept rate, so a
//! per class threshold can be chosen from data rather than guessed.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use irecon_lib::matcher::{self, cosine};

const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

struct Item {
    class: Option<String>,
    image: image::RgbaImage,
}

fn main() -> Result<(), String> {
    let mut root: Option<PathBuf> = None;
    let mut demo = false;
    let mut out: Option<PathBuf> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo" => demo = true,
            "--out" => out = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!("usage: match_bench <crops-dir> [--out file.csv] | --demo [--out file.csv]");
                return Ok(());
            }
            other => root = Some(PathBuf::from(other)),
        }
    }

    let items = if demo {
        demo_corpus()
    } else {
        let root = root.ok_or("usage: match_bench <crops-dir> [--out file.csv] | --demo")?;
        load_folder(&root)?
    };

    let labelled = items.iter().filter(|item| item.class.is_some()).count();
    let intruders = items.len() - labelled;
    if labelled == 0 {
        return Err("No labelled crops found. Use one subfolder per class.".into());
    }

    let embeddings: Vec<Vec<f32>> = items.iter().map(|item| matcher::embed(&item.image)).collect();

    // Leave-one-out: the crop is never in the pool it is compared against.
    let mut hits: Vec<(bool, f32)> = Vec::new();
    for index in 0..items.len() {
        let Some(class) = &items[index].class else {
            continue;
        };
        let mut banks: BTreeMap<&str, Vec<&Vec<f32>>> = BTreeMap::new();
        for other in 0..items.len() {
            if other == index {
                continue;
            }
            if let Some(other_class) = &items[other].class {
                banks.entry(other_class.as_str()).or_default().push(&embeddings[other]);
            }
        }
        let mut ranked: Vec<(&str, f32)> = banks
            .iter()
            .map(|(name, pool)| {
                let best = pool
                    .iter()
                    .map(|embedding| cosine(&embeddings[index], embedding))
                    .fold(-1.0f32, f32::max);
                (*name, best)
            })
            .collect();
        ranked.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some((top_name, score)) = ranked.first() {
            hits.push((*top_name == class.as_str(), *score));
        }
    }

    let mut intruder_scores: Vec<f32> = Vec::new();
    for index in 0..items.len() {
        if items[index].class.is_some() {
            continue;
        }
        let mut best = -1.0f32;
        for other in 0..items.len() {
            if other == index || items[other].class.is_none() {
                continue;
            }
            best = best.max(cosine(&embeddings[index], &embeddings[other]));
        }
        intruder_scores.push(best);
    }

    let correct = hits.iter().filter(|(hit, _)| *hit).count();
    let accuracy = correct as f32 / hits.len() as f32;
    let mean_correct = mean(hits.iter().filter(|(hit, _)| *hit).map(|(_, score)| *score));
    let mean_wrong = mean(hits.iter().filter(|(hit, _)| !*hit).map(|(_, score)| *score));

    println!(
        "{labelled} labelled crops across {} classes, {intruders} unknown crops, {accuracy:.3} top-1",
        items
            .iter()
            .filter_map(|item| item.class.as_deref())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
    println!(
        "mean score: {mean_correct:.3} when right, {mean_wrong:.3} when wrong"
    );
    println!();
    println!("threshold coverage precision false_accept");

    let mut rows = Vec::new();
    let mut best_threshold = (-0.20f32, f32::MIN);
    let mut calibrated: Option<f32> = None;
    let mut step = 0;
    while step <= 23 {
        let threshold = -0.20 + step as f32 * 0.05;
        let accepted_correct = hits.iter().filter(|(hit, score)| *hit && *score >= threshold).count();
        let accepted_wrong = hits.iter().filter(|(hit, score)| !*hit && *score >= threshold).count();
        let accepted_intruders = intruder_scores
            .iter()
            .filter(|score| **score >= threshold)
            .count();
        let coverage = accepted_correct as f32 / hits.len() as f32;
        let accepted = accepted_correct + accepted_wrong + accepted_intruders;
        let precision = if accepted == 0 {
            1.0
        } else {
            accepted_correct as f32 / accepted as f32
        };
        let denominator = (hits.len() + intruder_scores.len()).max(1);
        let false_accept = (accepted_wrong + accepted_intruders) as f32 / denominator as f32;
        println!("{threshold:.2} {coverage:.3} {precision:.3} {false_accept:.3}");
        rows.push(format!("{threshold:.2},{coverage:.3},{precision:.3},{false_accept:.3}"));

        if calibrated.is_none() && coverage >= 0.95 && false_accept <= 0.03 {
            calibrated = Some(threshold);
        }
        let objective = coverage - false_accept;
        if objective > best_threshold.1 {
            best_threshold = (threshold, objective);
        }
        step += 1;
    }

    println!();
    match calibrated {
        Some(threshold) => println!(
            "suggested threshold {threshold:.2}: the lowest that keeps 95 percent coverage and at most 3 percent false accepts"
        ),
        None => println!(
            "no threshold met 95 percent coverage with at most 3 percent false accepts; the best coverage minus false accepts was {:.2}",
            best_threshold.0
        ),
    }
    println!(
        "a perfect pool scores 1.0 on a class; anything near the wrong-crop mean is noise"
    );

    if let Some(path) = out {
        let mut csv = String::from("threshold,coverage,precision,false_accept\n");
        for row in rows {
            csv.push_str(&row);
            csv.push('\n');
        }
        fs::write(&path, csv).map_err(|e| e.to_string())?;
        println!("wrote {}", path.display());
    }

    Ok(())
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let mut total = 0.0;
    let mut count = 0usize;
    for value in values {
        total += value;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    total / count as f32
}

fn load_folder(root: &Path) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    let entries = fs::read_dir(root).map_err(|e| format!("Could not read {}: {e}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        let class = if name == "__none__" || name == "none" {
            None
        } else {
            Some(name)
        };
        let files = fs::read_dir(&path).map_err(|e| e.to_string())?;
        for file in files {
            let file = file.map_err(|e| e.to_string())?;
            let file_path = file.path();
            let extension = file_path
                .extension()
                .map(|value| value.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !EXTENSIONS.contains(&extension.as_str()) {
                continue;
            }
            if let Ok(image) = image::open(&file_path) {
                items.push(Item {
                    class: class.clone(),
                    image: image.to_rgba8(),
                });
            }
        }
    }
    Ok(items)
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(6364136223846793005).wrapping_add(1))
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// A stand-in corpus: six shapes, two of them red, on noisy backgrounds, plus
/// unknown blobs. It exists so the matcher can be measured before real frames
/// are available, not as a claim about gameplay crops.
fn demo_corpus() -> Vec<Item> {
    let classes = [
        "red-block",
        "red-ring",
        "green-disc",
        "blue-bar",
        "white-cross",
        "yellow-tri",
    ];
    let mut items = Vec::new();
    for (index, class) in classes.iter().enumerate() {
        for variant in 0..12usize {
            let seed = (index as u64 + 1) * 1_000 + variant as u64;
            items.push(Item {
                class: Some((*class).to_string()),
                image: scene(seed, class, variant),
            });
        }
    }
    for variant in 0..12usize {
        items.push(Item {
            class: None,
            image: intruder(7_000 + variant as u64),
        });
    }
    items
}

fn scene(seed: u64, class: &str, variant: usize) -> image::RgbaImage {
    let mut rng = Rng::new(seed);
    let mut image = image::RgbaImage::from_pixel(96, 96, image::Rgba([18, 20, 26, 255]));
    for pixel in image.pixels_mut() {
        let noise = (rng.next() % 12) as u8;
        *pixel = image::Rgba([18 + noise, 20 + noise, 26 + noise, 255]);
    }

    let scale = 0.78 + (variant % 5) as f32 * 0.06;
    let tint = 0.82 + (variant % 4) as f32 * 0.06;
    let cx = 48.0 + ((variant % 3) as f32 - 1.0) * 3.5;
    let cy = 48.0 + ((variant % 2) as f32 - 0.5) * 4.0;
    let base = match class {
        "red-block" => [225.0, 60.0, 45.0],
        "red-ring" => [212.0, 74.0, 58.0],
        "green-disc" => [60.0, 190.0, 95.0],
        "blue-bar" => [60.0, 105.0, 220.0],
        "white-cross" => [236.0, 236.0, 240.0],
        _ => [230.0, 195.0, 60.0],
    };

    for y in 0..96u32 {
        for x in 0..96u32 {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let inside = match class {
                "red-block" => dx.abs() < 22.0 * scale && dy.abs() < 14.0 * scale,
                "red-ring" => {
                    let radius = (dx * dx + dy * dy).sqrt();
                    radius > 12.0 * scale && radius < 24.0 * scale
                }
                "green-disc" => (dx * dx + dy * dy).sqrt() < 20.0 * scale,
                "blue-bar" => dx.abs() < 30.0 * scale && dy.abs() < 8.0 * scale,
                "white-cross" => {
                    (dx.abs() < 6.0 * scale && dy.abs() < 24.0 * scale)
                        || (dy.abs() < 6.0 * scale && dx.abs() < 24.0 * scale)
                }
                _ => dx.abs() + dy.abs() < 26.0 * scale,
            };
            if inside {
                let pixel = image.get_pixel_mut(x, y);
                for (channel, value) in base.iter().enumerate() {
                    let blended = value * tint * 0.9 + pixel.0[channel] as f32 * 0.1;
                    pixel.0[channel] = blended.clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    image
}

fn intruder(seed: u64) -> image::RgbaImage {
    let mut rng = Rng::new(seed);
    let mut image = image::RgbaImage::from_pixel(96, 96, image::Rgba([16, 18, 24, 255]));
    let cx = 28.0 + (rng.next() % 40) as f32;
    let cy = 28.0 + (rng.next() % 40) as f32;
    let rx = 9.0 + (rng.next() % 18) as f32;
    let ry = 9.0 + (rng.next() % 18) as f32;
    let color = [
        (rng.next() % 256) as u8,
        (rng.next() % 256) as u8,
        (rng.next() % 256) as u8,
    ];
    for y in 0..96u32 {
        for x in 0..96u32 {
            let nx = (x as f32 - cx) / rx;
            let ny = (y as f32 - cy) / ry;
            if nx * nx + ny * ny < 1.0 {
                image.put_pixel(x, y, image::Rgba([color[0], color[1], color[2], 255]));
            }
        }
    }
    image
}
