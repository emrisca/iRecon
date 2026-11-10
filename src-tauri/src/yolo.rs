//! Stock YOLO detection export adapter.
//!
//! Ultralytics exports one tensor of candidate boxes and class scores over a
//! letterboxed input, with no NMS. This module letterboxes the frame, decodes
//! the candidates, keeps the best class per anchor, applies per-class NMS, and
//! maps the boxes back to the original frame so the rest of the app sees the
//! same normalised `Detection` values the RF-DETR path produces.

use std::path::Path;

use crate::detector::Detection;

/// Ultralytics pads to the longest side with 114 grey, then normalises with
/// ImageNet values, the same way the detector models are trained.
const PAD: [f32; 3] = [114.0, 114.0, 114.0];
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
const NMS_IOU: f32 = 0.45;
/// Sorting keeps this many candidates before NMS. A 640 px export offers 8400
/// anchors, and thousands can clear a low threshold on a noisy frame.
const CANDIDATE_LIMIT: usize = 300;
const DETECTION_LIMIT: usize = 100;

#[derive(Clone, Copy)]
pub struct Shape {
    pub channels: usize,
    pub anchors: usize,
    /// `[1, 4 + classes, anchors]` when true, `[1, anchors, 4 + classes]` otherwise.
    pub channel_major: bool,
}

impl Shape {
    /// Accepts both export layouts. The channel count carries the class count,
    /// so the smaller axis is channels and the larger one is anchors. A real
    /// export has thousands of anchors and tens of classes, which makes the
    /// guess unambiguous; a square shape is refused rather than guessed.
    pub fn from_dims(dims: &[i64]) -> Result<Self, String> {
        if dims.len() != 3 || dims[0] != 1 {
            return Err(format!(
                "Expected one [1, channels, anchors] YOLO output, found {dims:?}."
            ));
        }
        let (first, second) = (dims[1], dims[2]);
        let (channels, anchors, channel_major) =
            if (5..=2048).contains(&first) && second > first {
                (first as usize, second as usize, true)
            } else if (5..=2048).contains(&second) && first > second {
                (second as usize, first as usize, false)
            } else {
                return Err(format!(
                    "Could not read the YOLO layout from {dims:?}; expected one axis of 4 plus classes and a larger anchor axis."
                ));
            };
        Ok(Shape {
            channels,
            anchors,
            channel_major,
        })
    }

    pub fn classes(&self) -> usize {
        self.channels.saturating_sub(4)
    }
}

/// Class names come from `<model>.labels.json`. An 80 class export falls back
/// to the COCO names the shipped detector already carries; anything else gets
/// numbered names, which are enough because matching overwrites the label.
pub fn labels(path: &Path, classes: usize) -> Vec<String> {
    let sidecar = path.with_extension("labels.json");
    if let Ok(text) = std::fs::read_to_string(sidecar) {
        if let Ok(raw) = serde_json::from_str::<Vec<String>>(&text) {
            let names: Vec<String> = raw
                .into_iter()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect();
            if names.len() == classes {
                return names;
            }
        }
    }
    if classes == 80 {
        return crate::detector::CLASS_NAMES
            .iter()
            .skip(1)
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string())
            .collect();
    }
    (0..classes)
        .map(|index| format!("class {}", index + 1))
        .collect()
}

/// Area-average resize into a padded square, returning channel-major float
/// data plus the scale and padding needed to map predictions back.
pub fn letterbox(rgba: &[u8], width: u32, height: u32, size: u32) -> (Vec<f32>, f32, f32, f32) {
    let plane = (size * size) as usize;
    let mut data = vec![0f32; plane * 3];
    for channel in 0..3 {
        let value = (PAD[channel] / 255.0 - MEAN[channel]) / STD[channel];
        for pixel in 0..plane {
            data[channel * plane + pixel] = value;
        }
    }
    if width == 0 || height == 0 || rgba.len() < 4 {
        return (data, 1.0, 0.0, 0.0);
    }

    let scale = (size as f32 / width as f32).min(size as f32 / height as f32);
    let new_w = ((width as f32 * scale).round() as u32).clamp(1, size);
    let new_h = ((height as f32 * scale).round() as u32).clamp(1, size);
    let pad_x = (size - new_w) / 2;
    let pad_y = (size - new_h) / 2;

    for y in 0..new_h {
        let sy0 = (y as u64 * height as u64 / new_h as u64) as u32;
        let sy1 = (((y + 1) as u64 * height as u64 / new_h as u64) as u32).max(sy0 + 1);
        for x in 0..new_w {
            let sx0 = (x as u64 * width as u64 / new_w as u64) as u32;
            let sx1 = (((x + 1) as u64 * width as u64 / new_w as u64) as u32).max(sx0 + 1);
            let mut sums = [0f32; 3];
            let mut count = 0f32;
            for sy in sy0..sy1.min(height) {
                for sx in sx0..sx1.min(width) {
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
            let offset = ((y + pad_y) * size + (x + pad_x)) as usize;
            for channel in 0..3 {
                let value = sums[channel] / count / 255.0;
                data[channel * plane + offset] = (value - MEAN[channel]) / STD[channel];
            }
        }
    }
    (data, scale, pad_x as f32, pad_y as f32)
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

fn overlap(left: &Detection, right: &Detection) -> f32 {
    let x0 = left.x.max(right.x);
    let y0 = left.y.max(right.y);
    let x1 = (left.x + left.w).min(right.x + right.w);
    let y1 = (left.y + left.h).min(right.y + right.h);
    let intersection = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let union = left.w * left.h + right.w * right.h - intersection;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

/// Turns the export's raw output into normalised detections. Boxes are centre
/// and size in letterboxed input pixels, so they are unpadded, divided by the
/// letterbox scale, then divided by the original frame size.
#[allow(clippy::too_many_arguments)]
pub fn decode(
    values: &[f32],
    shape: Shape,
    labels: &[String],
    threshold: f32,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
    frame_width: u32,
    frame_height: u32,
) -> Vec<Detection> {
    if scale <= 0.0 || frame_width == 0 || frame_height == 0 {
        return Vec::new();
    }
    if values.len() < shape.channels * shape.anchors {
        return Vec::new();
    }
    let address = |channel: usize, anchor: usize| {
        if shape.channel_major {
            channel * shape.anchors + anchor
        } else {
            anchor * shape.channels + channel
        }
    };

    // Ultralytics YOLOv8 and YOLO11 exports already apply sigmoid; a model
    // exported from raw logits does not. Values outside 0 to 1 identify the
    // latter without a per-model flag.
    let mut needs_sigmoid = false;
    'scan: for channel in 4..shape.channels {
        for anchor in 0..shape.anchors {
            let value = values[address(channel, anchor)];
            if value < 0.0 || value > 1.0 {
                needs_sigmoid = true;
                break 'scan;
            }
        }
    }

    let mut candidates = Vec::new();
    for anchor in 0..shape.anchors {
        let class_start = 4;
        let mut best = class_start;
        let mut best_score = f32::NEG_INFINITY;
        for channel in class_start..shape.channels {
            let raw = values[address(channel, anchor)];
            let score = if needs_sigmoid { sigmoid(raw) } else { raw };
            if score > best_score {
                best_score = score;
                best = channel;
            }
        }
        if !best_score.is_finite() || best_score < threshold {
            continue;
        }

        let cx = values[address(0, anchor)];
        let cy = values[address(1, anchor)];
        let width = values[address(2, anchor)];
        let height = values[address(3, anchor)];
        if width <= 0.0 || height <= 0.0 {
            continue;
        }

        let x = ((cx - width / 2.0 - pad_x) / scale / frame_width as f32).clamp(0.0, 1.0);
        let y = ((cy - height / 2.0 - pad_y) / scale / frame_height as f32).clamp(0.0, 1.0);
        let right =
            ((cx + width / 2.0 - pad_x) / scale / frame_width as f32).clamp(0.0, 1.0);
        let bottom =
            ((cy + height / 2.0 - pad_y) / scale / frame_height as f32).clamp(0.0, 1.0);
        let w = right - x;
        let h = bottom - y;
        if w <= 0.0 || h <= 0.0 {
            continue;
        }

        let label = labels
            .get(best - class_start)
            .cloned()
            .unwrap_or_else(|| format!("class {}", best - class_start + 1));
        candidates.push(Detection {
            label,
            score: best_score,
            x,
            y,
            w,
            h,
        });
    }

    candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
    candidates.truncate(CANDIDATE_LIMIT);

    let mut kept: Vec<Detection> = Vec::new();
    for candidate in candidates {
        if kept.len() >= DETECTION_LIMIT {
            break;
        }
        let clear = kept
            .iter()
            .all(|other| other.label != candidate.label || overlap(other, &candidate) <= NMS_IOU);
        if clear {
            kept.push(candidate);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(x: f32, y: f32, w: f32, h: f32, score: f32, label: &str) -> Detection {
        Detection {
            label: label.to_string(),
            score,
            x,
            y,
            w,
            h,
        }
    }

    #[test]
    fn both_export_layouts_are_recognised() {
        let first = Shape::from_dims(&[1, 84, 8400]).expect("channel major");
        assert_eq!(first.channels, 84);
        assert_eq!(first.anchors, 8400);
        assert_eq!(first.classes(), 80);
        assert!(first.channel_major);

        let second = Shape::from_dims(&[1, 8400, 84]).expect("anchor major");
        assert_eq!(second.channels, 84);
        assert!(!second.channel_major);

        assert!(Shape::from_dims(&[1, 84, 84]).is_err());
    }

    #[test]
    fn a_yolo_box_becomes_a_normalised_detection() {
        let mut values = vec![0f32; 6 * 4];
        for anchor in 0..4 {
            values[anchor] = 32.0;
            values[4 + anchor] = 32.0;
            values[8 + anchor] = 16.0;
            values[12 + anchor] = 16.0;
        }
        values[16] = 0.9;
        values[20] = 0.1;
        let shape = Shape {
            channels: 6,
            anchors: 4,
            channel_major: true,
        };
        let labels = vec!["Enemy".to_string(), "Ally".to_string()];
        let boxes = decode(&values, shape, &labels, 0.2, 1.0, 0.0, 0.0, 64, 64);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].label, "Enemy");
        assert!((boxes[0].x - 0.375).abs() < 1e-5, "x was {}", boxes[0].x);
        assert!((boxes[0].y - 0.375).abs() < 1e-5);
        assert!((boxes[0].w - 0.25).abs() < 1e-5);
        assert!((boxes[0].h - 0.25).abs() < 1e-5);
    }

    #[test]
    fn overlapping_boxes_of_one_class_are_suppressed() {
        let shape = Shape {
            channels: 5,
            anchors: 2,
            channel_major: true,
        };
        // Anchor 0 is the stronger of two nearly identical boxes.
        let values = vec![
            32.0, 32.0, // cx
            32.0, 32.0, // cy
            16.0, 16.0, // w
            16.0, 16.0, // h
            0.9, 0.8, // class
        ];
        let boxes = decode(&values, shape, &["Enemy".to_string()], 0.2, 1.0, 0.0, 0.0, 64, 64);
        assert_eq!(boxes.len(), 1);
        assert!((boxes[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn a_letterboxed_frame_keeps_its_aspect_and_padding() {
        let frame = vec![0u8; 1280 * 720 * 4];
        let (data, scale, pad_x, pad_y) = letterbox(&frame, 1280, 720, 640);
        assert_eq!(data.len(), 3 * 640 * 640);
        assert!((scale - 0.5).abs() < 1e-6, "scale was {scale}");
        assert!((pad_x - 0.0).abs() < 1e-6);
        assert!((pad_y - 140.0).abs() < 1e-6);
    }

    #[test]
    fn a_letterboxed_box_maps_back_to_the_frame() {
        // A 1280 by 720 frame letterboxed into 640 has scale 0.5 and 140 px top
        // padding, so a 64 px box at the centre of the square is 128 px there.
        let shape = Shape {
            channels: 5,
            anchors: 1,
            channel_major: true,
        };
        let values = vec![320.0, 320.0, 64.0, 64.0, 0.9];
        let boxes = decode(
            &values,
            shape,
            &["Enemy".to_string()],
            0.2,
            0.5,
            0.0,
            140.0,
            1280,
            720,
        );
        assert_eq!(boxes.len(), 1);
        assert!((boxes[0].x - 0.45).abs() < 1e-5, "x was {}", boxes[0].x);
        assert!((boxes[0].y - 0.4111).abs() < 1e-3, "y was {}", boxes[0].y);
        assert!((boxes[0].w - 0.1).abs() < 1e-5);
        assert!((boxes[0].h - 0.1778).abs() < 1e-3);
    }

    #[test]
    fn nms_ignores_boxes_of_other_classes() {
        let left = detection(0.1, 0.1, 0.2, 0.2, 0.9, "Enemy");
        let right = detection(0.11, 0.11, 0.2, 0.2, 0.8, "Ally");
        assert!(overlap(&left, &right) > 0.5);
    }
}
