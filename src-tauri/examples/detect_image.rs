//! Run the detector on one image and print what it found.
//!
//! cargo run --example detect_image -- path/to/image.jpg

use std::env;
use std::path::PathBuf;

use irecon_lib::detector::Detector;

fn main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let image_path = args.next().ok_or("Pass an image path")?;
    let model_path = match args.next() {
        Some(path) => PathBuf::from(path),
        None => {
            let home = env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
            PathBuf::from(home)
                .join("Library/Application Support/com.emrisca.irecon/models/rf-detr-nano.onnx")
        }
    };

    let image = image::open(&image_path)
        .map_err(|e| format!("Could not open {image_path}: {e}"))?
        .to_rgba8();
    let (width, height) = (image.width(), image.height());

    let mut detector = Detector::load(&model_path)?;
    println!("{}", detector.describe());

    let (boxes, elapsed) = detector.analyse(image.as_raw(), width, height, 0.35)?;
    println!("{} boxes in {elapsed:.1} ms on {width} by {height}", boxes.len());
    for detection in boxes.iter().take(12) {
        println!(
            "{:<14} {:.2}  x {:.3} y {:.3} w {:.3} h {:.3}",
            detection.label, detection.score, detection.x, detection.y, detection.w, detection.h
        );
    }
    Ok(())
}
