use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use ort::environment::Environment;
use ort::session::Session;
use ort::value::{Tensor, ValueType};
use serde::{Deserialize, Serialize};

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

/// The exported graph's input contract: name, static square shape, RGB, plain
/// stretch resize, divided by 255, then ImageNet mean and standard deviation.
pub const INPUT_NAME: &str = "input";
/// Used only when a file is not one of the pinned catalog entries.
pub const FALLBACK_INPUT_SIZE: u32 = 384;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Torchvision's 91 slot COCO layout, where slot 0 is background.
pub(crate) const CLASS_NAMES: [&str; 91] = [
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

/// Where one frame's detector time went. `run_ms` is the number `analyse`
/// reported before the split existed.
#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisTiming {
    pub preprocess_ms: f32,
    pub run_ms: f32,
}

#[derive(Clone, Deserialize, Serialize)]
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
    input_size: u32,
    provider: &'static str,
    provider_note: Option<String>,
    input_name: String,
    /// One label per DETR class slot. Background slots are empty strings.
    slot_names: Vec<String>,
    /// Set when the graph is a stock YOLO export instead of a DETR export.
    yolo: Option<crate::yolo::Shape>,
    labels: Vec<String>,
}

impl Detector {
    /// `input_size` comes from the pinned catalog entry, because the graph
    /// refuses a tensor that is not its own static size.
    pub fn load(path: &Path, input_size: u32) -> Result<Self, String> {
        if input_size < 32 {
            return Err(format!("A {input_size} px input is too small to run"));
        }
        let (mut session, provider, provider_note) = open_session(path)?;

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
        let (queries, slots) = probe(&mut session, input_size)?;
        let slot_names = slot_names_for_model(path, slots)?;
        Ok(Detector {
            session,
            queries,
            slots,
            input_size,
            provider,
            provider_note,
            input_name: INPUT_NAME.to_string(),
            slot_names,
            yolo: None,
            labels: Vec::new(),
        })
    }

    /// Opens a project-trained ONNX file and derives its static square input
    /// from the graph. Two contracts are accepted: the RF-DETR `dets` and
    /// `labels` pair, or a stock YOLO detection export with one output of
    /// boxes and class scores. Both are warmed with a blank tensor before the
    /// file is stored.
    pub fn load_imported(path: &Path) -> Result<Self, String> {
        let (mut session, provider, provider_note) = open_session(path)?;
        let (input_name, input_size) = {
            let input = session
                .inputs()
                .first()
                .ok_or("The model has no inputs")?;
            let name = input.name().to_string();
            let size = match input.dtype() {
                ValueType::Tensor { shape, .. } => {
                    let dims: Vec<i64> = shape.iter().copied().collect();
                    if dims.len() != 4 || dims[1] != 3 || dims[2] <= 0 || dims[2] != dims[3] {
                        return Err(format!(
                            "Expected a static NCHW float input with three channels and a square image, found {dims:?}."
                        ));
                    }
                    dims[2] as u32
                }
                _ => return Err("The model input is not a float tensor".into()),
            };
            (name, size)
        };
        let outputs: Vec<String> = session
            .outputs()
            .iter()
            .map(|output| output.name().to_string())
            .collect();
        let has_detr_outputs = outputs.iter().any(|name| name == "dets")
            && outputs.iter().any(|name| name == "labels");

        if has_detr_outputs {
            if input_name != INPUT_NAME {
                return Err(format!(
                    "The model expects an input called {input_name}, but iRecon feeds {INPUT_NAME}."
                ));
            }
            let (queries, slots) = probe(&mut session, input_size)?;
            let slot_names = slot_names_for_model(path, slots)?;
            return Ok(Detector {
                session,
                queries,
                slots,
                input_size,
                provider,
                provider_note,
                input_name,
                slot_names,
                yolo: None,
                labels: Vec::new(),
            });
        }

        if outputs.len() != 1 {
            return Err(format!(
                "The model is missing the dets and labels outputs. Found: {outputs:?}"
            ));
        }
        let dims: Vec<i64> = match session.outputs()[0].dtype() {
            ValueType::Tensor { shape, .. } => shape.iter().copied().collect(),
            _ => return Err("The YOLO output is not a float tensor".into()),
        };
        let yolo = crate::yolo::Shape::from_dims(&dims)?;
        let labels = crate::yolo::labels(path, yolo.classes());

        // Warm the graph the same way the pinned models are warmed.
        let zeros = vec![0f32; 3 * (input_size as usize) * (input_size as usize)];
        let tensor = Tensor::from_array((
            [1usize, 3, input_size as usize, input_size as usize],
            zeros.into_boxed_slice(),
        ))
        .map_err(|error| error.to_string())?;
        session
            .run(ort::inputs![input_name.as_str() => tensor])
            .map_err(|error| format!("The model did not run on a blank input: {error}"))?;

        Ok(Detector {
            session,
            queries: yolo.anchors,
            slots: yolo.channels,
            input_size,
            provider,
            provider_note,
            input_name,
            slot_names: Vec::new(),
            yolo: Some(yolo),
            labels,
        })
    }

    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} px input, {} queries, {} class slots, {}",
            self.input_size, self.queries, self.slots, self.provider
        );
        if let Some(note) = &self.provider_note {
            text.push_str(&format!("; {note}"));
        }
        text
    }

    pub fn input_size(&self) -> u32 {
        self.input_size
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
        let (boxes, timing) = self.analyse_timed(rgba, width, height, threshold)?;
        Ok((boxes, timing.run_ms))
    }

    /// Runs one frame and splits the cost into preprocessing and the graph.
    /// Preprocessing reads every source pixel, so folding it into the inference
    /// number makes an expensive resize look like a slow model.
    pub fn analyse_timed(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        threshold: f32,
    ) -> Result<(Vec<Detection>, AnalysisTiming), String> {
        if let Some(shape) = self.yolo {
            return self.analyse_yolo(rgba, width, height, threshold, shape);
        }
        let size = self.input_size;
        let preprocess_started = Instant::now();
        let data = preprocess(rgba, width, height, size);
        let preprocess_ms = preprocess_started.elapsed().as_secs_f32() * 1000.0;
        let tensor = Tensor::from_array((
            [1usize, 3, size as usize, size as usize],
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
        let boxes = decode_detr(
            &dets,
            &logits,
            self.queries,
            self.slots,
            &self.slot_names,
            threshold,
        );
        Ok((
            boxes,
            AnalysisTiming {
                preprocess_ms,
                run_ms: elapsed_ms,
            },
        ))
    }

    fn analyse_yolo(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        threshold: f32,
        shape: crate::yolo::Shape,
    ) -> Result<(Vec<Detection>, AnalysisTiming), String> {
        let size = self.input_size;
        let preprocess_started = Instant::now();
        let (data, scale, pad_x, pad_y) = crate::yolo::letterbox(rgba, width, height, size);
        let preprocess_ms = preprocess_started.elapsed().as_secs_f32() * 1000.0;
        let tensor = Tensor::from_array((
            [1usize, 3, size as usize, size as usize],
            data.into_boxed_slice(),
        ))
        .map_err(|error| error.to_string())?;
        let input_name = self.input_name.clone();

        let started = Instant::now();
        let outputs = self
            .session
            .run(ort::inputs![input_name.as_str() => tensor])
            .map_err(|error| format!("Inference failed: {error}"))?;
        let elapsed_ms = started.elapsed().as_secs_f32() * 1000.0;
        let values = tensor_values(&outputs[0])?;
        let boxes = crate::yolo::decode(
            &values,
            shape,
            &self.labels,
            threshold,
            scale,
            pad_x,
            pad_y,
            width,
            height,
        );
        Ok((
            boxes,
            AnalysisTiming {
                preprocess_ms,
                run_ms: elapsed_ms,
            },
        ))
    }
}

/// Opens the graph on DirectML when Windows offers it, then retries on CPU if
/// registration or graph loading fails. The retry is intentional: a machine with
/// an old driver should still run the detector, just more slowly.
pub(crate) fn open_session(path: &Path) -> Result<(Session, &'static str, Option<String>), String> {
    #[cfg(windows)]
    {
        let directml = ort::ep::DirectML::default()
            .with_performance_preference(
                ort::ep::directml::PerformancePreference::HighPerformance,
            )
            .build()
            .error_on_failure();
        let attempt = Session::builder(environment()?)
            .map_err(|error| error.to_string())
            .and_then(|builder| {
                builder
                    .with_execution_providers([directml])
                    .map_err(|error| error.to_string())
            })
            .and_then(|mut builder| {
                builder
                    .commit_from_file(path)
                    .map_err(|error| error.to_string())
            });
        match attempt {
            Ok(session) => return Ok((session, "DirectML", None)),
            Err(error) => {
                let note = format!("DirectML unavailable, using CPU: {error}");
                let session = Session::builder(environment()?)
                    .map_err(|error| format!("Could not open the ONNX model: {error}"))
                    .and_then(|mut builder| {
                        builder
                            .commit_from_file(path)
                            .map_err(|error| format!("Could not open the ONNX model: {error}"))
                    })?;
                return Ok((session, "CPU", Some(note)));
            }
        }
    }
    #[cfg(not(windows))]
    {
        let session = Session::builder(environment()?)
            .map_err(|error| format!("Could not open the ONNX model: {error}"))
            .and_then(|mut builder| {
                builder
                    .commit_from_file(path)
                    .map_err(|error| format!("Could not open the ONNX model: {error}"))
            })?;
        Ok((session, "CPU", None))
    }
}

fn probe(session: &mut Session, input_size: u32) -> Result<(usize, usize), String> {
    let zeros = vec![0f32; 3 * (input_size * input_size) as usize];
    let tensor = Tensor::from_array((
        [1usize, 3, input_size as usize, input_size as usize],
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

    let mut dets_len = 0usize;
    let mut labels_len = 0usize;
    for (index, name) in names.iter().enumerate() {
        let values = tensor_values(&outputs[index])?;
        match name.as_str() {
            "dets" => dets_len = values.len(),
            "labels" => labels_len = values.len(),
            _ => {}
        }
    }
    if dets_len == 0 || labels_len == 0 {
        return Err("Could not read the model's output shapes".into());
    }
    if dets_len % 4 != 0 {
        return Err(format!(
            "The dets output has {dets_len} values, which is not a whole number of boxes."
        ));
    }
    let queries = dets_len / 4;
    if labels_len % queries != 0 {
        return Err(format!(
            "The labels output has {labels_len} values, which does not divide into {queries} queries."
        ));
    }
    let slots = labels_len / queries;
    if slots == 0 {
        return Err("The model has no class slots".into());
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

/// Scores every DETR slot that has a name, so a custom model can use its own
/// class count and background position instead of COCO's 91 slot layout.
fn decode_detr(
    dets: &[f32],
    logits: &[f32],
    queries: usize,
    slots: usize,
    slot_names: &[String],
    threshold: f32,
) -> Vec<Detection> {
    let slots = slots.max(1);
    let count = queries
        .min(dets.len() / 4)
        .min(logits.len() / slots);
    let mut boxes = Vec::new();

    for query in 0..count {
        let base = query * slots;
        let mut best_slot = None;
        let mut best_score = 0f32;
        for slot in 0..slots {
            if slot_names.get(slot).map_or(true, |name| name.is_empty()) {
                continue;
            }
            let score = sigmoid(logits[base + slot]);
            if score > best_score {
                best_score = score;
                best_slot = Some(slot);
            }
        }
        let Some(slot) = best_slot else {
            continue;
        };
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

        boxes.push(Detection {
            label: slot_names[slot].clone(),
            score: best_score,
            x,
            y,
            w,
            h,
        });
    }

    boxes.sort_by(|a, b| b.score.total_cmp(&a.score));
    boxes.truncate(100);
    boxes
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LabelSidecar {
    Names(Vec<String>),
    Table {
        names: Vec<String>,
        #[serde(default)]
        background: Option<i64>,
    },
}

/// Reads `<model>.labels.json` when it exists, otherwise falls back to the
/// stock COCO names or a numbered layout for a custom export.
fn slot_names_for_model(path: &Path, slots: usize) -> Result<Vec<String>, String> {
    let sidecar = path.with_extension("labels.json");
    if !sidecar.exists() {
        return Ok(default_slot_names(slots));
    }

    let text = std::fs::read_to_string(&sidecar)
        .map_err(|error| format!("Could not read {}: {error}", sidecar.display()))?;
    let parsed: LabelSidecar = serde_json::from_str(&text)
        .map_err(|error| format!("Could not read {}: {error}", sidecar.display()))?;
    let (names, background) = match parsed {
        LabelSidecar::Names(names) => (names, None),
        LabelSidecar::Table { names, background } => (names, background),
    };
    build_slot_names(slots, names, background)
        .map_err(|error| format!("{}: {error}", sidecar.display()))
}

fn default_slot_names(slots: usize) -> Vec<String> {
    if slots == 91 {
        return CLASS_NAMES
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if index == 0 {
                    String::new()
                } else {
                    (*name).to_string()
                }
            })
            .collect();
    }
    if slots == 1 {
        return vec!["object".to_string()];
    }
    (0..slots)
        .map(|index| {
            if index + 1 == slots {
                String::new()
            } else {
                format!("class {}", index + 1)
            }
        })
        .collect()
}

fn build_slot_names(
    slots: usize,
    names: Vec<String>,
    background: Option<i64>,
) -> Result<Vec<String>, String> {
    if slots == 0 {
        return Err("The model has no class slots".into());
    }
    if names.is_empty() {
        return Err("The labels sidecar has no class names".into());
    }
    let names: Vec<String> = names
        .into_iter()
        .map(|name| name.trim().to_string())
        .collect();
    if names.iter().any(|name| name.is_empty()) {
        return Err("Every class name in the labels sidecar must be non-empty".into());
    }

    if names.len() == slots {
        if background.is_some() {
            return Err(format!(
                "A background slot needs {} names, not {}.",
                slots - 1,
                slots
            ));
        }
        return Ok(names);
    }

    if names.len() + 1 == slots {
        let index = match background {
            Some(-1) => slots - 1,
            Some(index) if index >= 0 && (index as usize) < slots => index as usize,
            Some(_) => {
                return Err(format!(
                    "Background must be -1 or a slot from 0 to {}.",
                    slots - 1
                ))
            }
            None => {
                if slots == 91 {
                    0
                } else {
                    slots - 1
                }
            }
        };
        let mut output = names;
        output.insert(index, String::new());
        return Ok(output);
    }

    Err(format!(
        "The labels sidecar has {} names for {} slots. Use {} names with a background slot or {} without one.",
        names.len(),
        slots,
        slots - 1,
        slots
    ))
}

/// Area average downscale into CHW, RGB, scaled and normalised the way the export
/// expects. Stretched to a square, not letterboxed, matching the graph.
pub(crate) fn preprocess(rgba: &[u8], width: u32, height: u32, size: u32) -> Vec<f32> {
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
    fn a_custom_slot_layout_scores_classes_and_skips_background() {
        let dets = [
            0.5, 0.5, 0.2, 0.2, 0.5, 0.5, 0.2, 0.2, 0.5, 0.5, 0.2, 0.2, 0.5, 0.5, 0.2,
            0.2,
        ];
        let logits = [
            -7.0, 4.0, -6.0, -7.0, -6.0, 4.0, 4.0, -7.0, -6.0, -7.0, -7.0, -7.0,
        ];
        let names = vec!["Enemy".to_string(), "Ally".to_string(), String::new()];
        let boxes = decode_detr(&dets, &logits, 4, 3, &names, 0.5);
        assert_eq!(boxes.len(), 2);
        assert!(boxes.iter().any(|detection| detection.label == "Enemy"));
        assert!(boxes.iter().any(|detection| detection.label == "Ally"));
        assert!(boxes
            .iter()
            .all(|detection| (detection.score - 0.982).abs() < 0.01));
        assert!(boxes
            .iter()
            .all(|detection| (detection.x - 0.4).abs() < 1e-6));
    }

    #[test]
    fn a_single_slot_without_background_is_scored() {
        let names = vec!["object".to_string()];
        let boxes = decode_detr(&[0.5, 0.5, 0.2, 0.2], &[4.0], 1, 1, &names, 0.5);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].label, "object");
    }

    #[test]
    fn the_default_layout_is_coco_for_91_slots_and_numbered_otherwise() {
        let coco = default_slot_names(91);
        assert_eq!(coco.len(), 91);
        assert!(coco[0].is_empty());
        assert_eq!(coco[1], "person");
        assert_eq!(coco[90], "toothbrush");
        assert_eq!(default_slot_names(1), vec!["object"]);
        assert_eq!(
            default_slot_names(3),
            vec![
                "class 1".to_string(),
                "class 2".to_string(),
                String::new()
            ]
        );
    }

    #[test]
    fn a_labels_sidecar_can_name_custom_slots() {
        let dir = Path::new("target").join(format!(
            "detector-sidecar-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("custom.onnx");
        std::fs::write(&model, b"placeholder").unwrap();

        std::fs::write(
            dir.join("custom.labels.json"),
            r#"{"names":["Enemy","Ally"],"background":-1}"#,
        )
        .unwrap();
        assert_eq!(
            slot_names_for_model(&model, 3).unwrap(),
            vec!["Enemy", "Ally", ""]
        );

        std::fs::write(dir.join("custom.labels.json"), r#"["Enemy","Ally"]"#).unwrap();
        assert_eq!(
            slot_names_for_model(&model, 3).unwrap(),
            vec!["Enemy", "Ally", ""]
        );
        assert_eq!(
            slot_names_for_model(&model, 2).unwrap(),
            vec!["Enemy", "Ally"]
        );

        std::fs::write(
            dir.join("custom.labels.json"),
            r#"{"names":["Enemy"],"background":0}"#,
        )
        .unwrap();
        assert_eq!(
            slot_names_for_model(&model, 2).unwrap(),
            vec!["", "Enemy"]
        );

        let _ = std::fs::remove_dir_all(&dir);
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

        let mut detector = Detector::load(&path, FALLBACK_INPUT_SIZE).expect("the model should load");
        assert_eq!(detector.queries(), 300, "RF-DETR exposes 300 queries");
        assert_eq!(detector.slots(), 91, "COCO-91 logits, background at slot 0");

        let size = detector.input_size();
        let blank = vec![128u8; size as usize * size as usize * 4];
        let (boxes, elapsed) = detector
            .analyse(&blank, size, size, 0.3)
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
