use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;
use serde::Serialize;

/// ONNX Runtime allows exactly one environment per process, so it is created once
/// and shared by every session iRecon opens.
fn environment() -> Result<&'static Environment, String> {
    static ENV: OnceLock<Environment> = OnceLock::new();
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    if let Some(env) = ENV.get() {
        return Ok(env);
    }
    let env = ort::init()
        .with_name("irecon")
        .build()
        .map_err(|e| format!("Could not start ONNX Runtime: {e}"))?;
    let _ = ENV.set(env);
    ENV.get()
        .ok_or_else(|| "The ONNX Runtime environment could not be stored".to_string())
}

/// The exported graph's input contract: name, static shape, RGB, plain stretch
/// resize to 384, divided by 255, then ImageNet mean and standard deviation.
pub const INPUT_NAME: &str = "input";
pub const INPUT_SIZE: u32 = 384;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Torchvision's 91 slot COCO layout, where slot 0 is background.
const CLASS_NAMES: [&str; 91] = [
    "background",
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "",
    "backpack",
    "umbrella",
    "",
    "",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "",
    "dining table",
    "",
    "",
    "toilet",
    "",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub label: String,
    pub score: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub struct Detector {
    session: Session,
    queries: usize,
    slots: usize,
}

impl Detector {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut session = Session::builder(environment()?)
            .map_err(|e| e.to_string())?
            .commit_from_file(path)
            .map_err(|e| format!("Could not open the ONNX model: {e}"))?;

        let input_name = session
            .inputs()
            .first()
            .map(|input| input.name().to_string())
            .ok_or("The model has no inputs")?;
        if input_name != INPUT_NAME {
            return Err(format!(
                "The model expects an input called {input_name}, but iRecon feeds {INPUT_NAME}."
            ));
        }

        let outputs: Vec<String> = session
            .outputs()
            .iter()
            .map(|output| output.name().to_string())
            .collect();
        for required in ["dets", "labels"] {
            if !outputs.iter().any(|name| name == required) {
                return Err(format!(
                    "The model is missing the {required} output. Found: {outputs:?}"
                ));
            }
        }

        // Sizes come from the graph, not from an assumption about the export.
        let (queries, slots) = probe(&mut session)?;
        Ok(Detector {
            session,
            queries,
            slots,
        })
    }

    pub fn describe(&self) -> String {
        format!(
            "RF-DETR Nano, {} px, {} queries, {} class slots",
            INPUT_SIZE, self.queries, self.slots
        )
    }

    pub fn queries(&self) -> usize {
        self.queries
    }

    pub fn slots(&self) -> usize {
        self.slots
    }

    /// Runs one frame and returns detections plus the time the graph took.
    pub fn analyse(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        threshold: f32,
    ) -> Result<(Vec<Detection>, f32), String> {
        let data = preprocess(rgba, width, height, INPUT_SIZE);
        let tensor = Tensor::from_array((
            [1usize, 3, INPUT_SIZE as usize, INPUT_SIZE as usize],
            data.into_boxed_slice(),
        ))
        .map_err(|e| e.to_string())?;

        // Names are read before the run: the outputs borrow the session mutably.
        let names = self
            .session
            .outputs()
            .iter()
            .map(|output| output.name().to_string())
            .collect::<Vec<_>>();

        let started = Instant::now();
        let outputs = self
            .session
            .run(ort::inputs![INPUT_NAME => tensor])
            .map_err(|e| format!("Inference failed: {e}"))?;
        let elapsed_ms = started.elapsed().as_secs_f32() * 1000.0;

        let mut boxes = Vec::new();
        let mut dets: Option<Vec<f32>> = None;
        let mut logits: Option<Vec<f32>> = None;
        for (index, name) in names.iter().enumerate() {
            let values = tensor_values(&outputs[index])?;
            match name.as_str() {
                "dets" => dets = Some(values),
                "labels" => logits = Some(values),
                _ => {}
            }
        }

        let dets = dets.ok_or("The model returned no boxes")?;
        let logits = logits.ok_or("The model returned no class scores")?;
        let queries = self.queries.max(1);
        let slots = self.slots.max(1);

        for query in 0..queries.min(dets.len() / 4).min(logits.len() / slots) {
            let base = query * slots;
            let mut best_slot = 0usize;
            let mut best_score = 0f32;
            for slot in 1..slots {
                let score = sigmoid(logits[base + slot]);
                if score > best_score {
                    best_score = score;
                    best_slot = slot;
                }
            }
            if best_score < threshold {
                continue;
            }

            let cx = dets[query * 4];
            let cy = dets[query * 4 + 1];
            let bw = dets[query * 4 + 2];
            let bh = dets[query * 4 + 3];
            let x = (cx - bw / 2.0).clamp(0.0, 1.0);
            let y = (cy - bh / 2.0).clamp(0.0, 1.0);
            let w = bw.clamp(0.0, 1.0 - x);
            let h = bh.clamp(0.0, 1.0 - y);
            if w <= 0.0 || h <= 0.0 {
                continue;
            }

            let label = CLASS_NAMES
                .get(best_slot)
                .filter(|name| !name.is_empty())
                .copied()
                .unwrap_or("object")
                .to_string();

            boxes.push(Detection {
                label,
                score: best_score,
                x,
                y,
                w,
                h,
            });
        }

        boxes.sort_by(|a, b| b.score.total_cmp(&a.score));
        boxes.truncate(100);
        Ok((boxes, elapsed_ms))
    }
}

fn probe(session: &mut Session) -> Result<(usize, usize), String> {
    let zeros = vec![0f32; 3 * (INPUT_SIZE * INPUT_SIZE) as usize];
    let tensor = Tensor::from_array((
        [1usize, 3, INPUT_SIZE as usize, INPUT_SIZE as usize],
        zeros.into_boxed_slice(),
    ))
    .map_err(|e| e.to_string())?;
    let names = session
        .outputs()
        .iter()
        .map(|output| output.name().to_string())
        .collect::<Vec<_>>();

    let outputs = session
        .run(ort::inputs![INPUT_NAME => tensor])
        .map_err(|e| format!("The model did not run on a blank input: {e}"))?;

    let mut queries = 0usize;
    let mut slots = 0usize;
    for (index, name) in names.iter().enumerate() {
        let values = tensor_values(&outputs[index])?;
        match name.as_str() {
            "dets" => queries = values.len() / 4,
            "labels" => {
                if queries > 0 {
                    slots = values.len() / queries;
                } else {
                    slots = 91;
                }
            }
            _ => {}
        }
    }
    if queries == 0 || slots == 0 {
        return Err("Could not read the model's output shapes".into());
    }
    Ok((queries, slots))
}

fn tensor_values(output: &ort::value::Value) -> Result<Vec<f32>, String> {
    let (_shape, data) = output
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;
    Ok(data.to_vec())
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

/// Area average downscale into CHW, RGB, scaled and normalised the way the export
/// expects. Stretched to a square, not letterboxed, matching the graph.
fn preprocess(rgba: &[u8], width: u32, height: u32, size: u32) -> Vec<f32> {
    let plane = (size * size) as usize;
    let mut data = vec![0f32; plane * 3];
    if width == 0 || height == 0 || rgba.len() < 4 {
        return data;
    }

    for y in 0..size {
        let y0 = (y as u64 * height as u64 / size as u64) as u32;
        let y1 = (((y + 1) as u64 * height as u64 / size as u64) as u32).max(y0 + 1);
        for x in 0..size {
            let x0 = (x as u64 * width as u64 / size as u64) as u32;
            let x1 = (((x + 1) as u64 * width as u64 / size as u64) as u32).max(x0 + 1);

            let mut sums = [0f32; 3];
            let mut count = 0f32;
            for sy in y0..y1.min(height) {
                for sx in x0..x1.min(width) {
                    let index = ((sy * width + sx) * 4) as usize;
                    if index + 3 >= rgba.len() {
                        continue;
                    }
                    sums[0] += rgba[index] as f32;
                    sums[1] += rgba[index + 1] as f32;
                    sums[2] += rgba[index + 2] as f32;
                    count += 1.0;
                }
            }
            if count == 0.0 {
                continue;
            }

            let offset = (y * size + x) as usize;
            for channel in 0..3 {
                let value = sums[channel] / count / 255.0;
                data[channel * plane + offset] = (value - MEAN[channel]) / STD[channel];
            }
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn model_candidates() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Ok(raw) = std::env::var("IRECON_MODEL") {
            paths.push(PathBuf::from(raw));
        }
        if let Ok(home) = std::env::var("HOME") {
            paths.push(PathBuf::from(home).join(
                "Library/Application Support/com.emrisca.irecon/models/rf-detr-nano.onnx",
            ));
        }
        paths
    }

    #[test]
    fn preprocess_matches_imagenet_normalisation() {
        // One red pixel stretched to 1 by 1.
        let data = preprocess(&[255, 0, 0, 255], 1, 1, 1);
        assert_eq!(data.len(), 3);
        assert!((data[0] - (1.0 - MEAN[0]) / STD[0]).abs() < 1e-5);
        assert!((data[1] - (0.0 - MEAN[1]) / STD[1]).abs() < 1e-5);
        assert!((data[2] - (0.0 - MEAN[2]) / STD[2]).abs() < 1e-5);
    }

    #[test]
    fn preprocess_writes_chw_order() {
        // Left half red, right half blue at 2 by 1, downscaled to 2 by 1.
        let rgba = [255, 0, 0, 255, 0, 0, 255, 255];
        let data = preprocess(&rgba, 2, 1, 2);
        assert_eq!(data.len(), 12);
        // Channel 0 is red: first pixel high, second low.
        assert!(data[0] > data[1]);
        // Channel 2 is blue: first pixel low, second high.
        assert!(data[8] < data[9]);
    }

    /// Pins the graph contract. Skips with a message when the model is not present,
    /// so a fresh clone without the 115 MB file still builds and tests clean.
    #[test]
    fn onnx_contract_is_pinned() {
        let Some(path) = model_candidates().into_iter().find(|path| path.exists()) else {
            eprintln!("skipping onnx_contract_is_pinned: no model file found");
            return;
        };

        let mut detector = Detector::load(&path).expect("the model should load");
        assert_eq!(detector.queries(), 300, "RF-DETR Nano exposes 300 queries");
        assert_eq!(detector.slots(), 91, "COCO-91 logits, background at slot 0");

        let blank = vec![128u8; INPUT_SIZE as usize * INPUT_SIZE as usize * 4];
        let (boxes, elapsed) = detector
            .analyse(&blank, INPUT_SIZE, INPUT_SIZE, 0.3)
            .expect("inference should run");
        assert!(elapsed >= 0.0);
        for detection in &boxes {
            assert!(detection.score.is_finite());
            assert!((0.0..=1.0).contains(&detection.score));
            assert!((0.0..=1.0).contains(&detection.x));
            assert!((0.0..=1.0).contains(&detection.y));
            assert!(detection.w > 0.0 && detection.w <= 1.0);
            assert!(detection.h > 0.0 && detection.h <= 1.0);
            assert!(!detection.label.is_empty());
        }
    }
}
