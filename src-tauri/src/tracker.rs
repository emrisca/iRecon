//! IoU tracker.
//!
//! Detections arrive one frame at a time with no identity. This matches them to
//! existing tracks by overlap and label, nudges each box toward its new position
//! instead of snapping, and lets a track coast through a few missing frames.
//! The result is a box that keeps its id and does not blink out because one
//! frame failed to clear the threshold.

use serde::Serialize;

use crate::detector::Detection;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedBox {
    pub track: u64,
    pub label: String,
    pub score: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Frames this track has been seen in.
    pub seen: u32,
}

struct Track {
    id: u64,
    label: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    score: f32,
    hits: u32,
    misses: u32,
}

pub struct Tracker {
    tracks: Vec<Track>,
    next_id: u64,
    iou_min: f32,
    max_misses: u32,
    keep: f32,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            next_id: 1,
            iou_min: 0.3,
            max_misses: 3,
            // Share of the previous box that survives each update. Halving the
            // jump removes frame-to-frame jitter without visible lag.
            keep: 0.5,
        }
    }
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes this frame's detections and returns the boxes to draw, with ids.
    pub fn update(&mut self, detections: Vec<Detection>) -> Vec<TrackedBox> {
        let mut pairs: Vec<(usize, usize, f32)> = Vec::new();
        for (track_index, track) in self.tracks.iter().enumerate() {
            for (detection_index, detection) in detections.iter().enumerate() {
                if detection.label != track.label {
                    continue;
                }
                let overlap = iou(
                    (track.x, track.y, track.w, track.h),
                    (detection.x, detection.y, detection.w, detection.h),
                );
                if overlap >= self.iou_min {
                    pairs.push((track_index, detection_index, overlap));
                }
            }
        }
        // Best overlap first, one detection and one track per match.
        pairs.sort_by(|left, right| right.2.total_cmp(&left.2));

        let mut taken = vec![false; detections.len()];
        let mut matches: Vec<Option<usize>> = vec![None; self.tracks.len()];
        for (track_index, detection_index, _) in pairs {
            if matches[track_index].is_some() || taken[detection_index] {
                continue;
            }
            matches[track_index] = Some(detection_index);
            taken[detection_index] = true;
        }

        for (track_index, matched) in matches.iter().enumerate() {
            let track = &mut self.tracks[track_index];
            match matched {
                Some(detection_index) => {
                    let detection = &detections[*detection_index];
                    track.x = track.x * self.keep + detection.x * (1.0 - self.keep);
                    track.y = track.y * self.keep + detection.y * (1.0 - self.keep);
                    track.w = track.w * self.keep + detection.w * (1.0 - self.keep);
                    track.h = track.h * self.keep + detection.h * (1.0 - self.keep);
                    track.score = detection.score;
                    track.hits += 1;
                    track.misses = 0;
                }
                None => track.misses += 1,
            }
        }

        for (index, detection) in detections.into_iter().enumerate() {
            if taken[index] {
                continue;
            }
            self.tracks.push(Track {
                id: self.next_id,
                label: detection.label,
                x: detection.x,
                y: detection.y,
                w: detection.w,
                h: detection.h,
                score: detection.score,
                hits: 1,
                misses: 0,
            });
            self.next_id += 1;
        }

        self.tracks.retain(|track| track.misses <= self.max_misses);

        let mut boxes: Vec<TrackedBox> = self
            .tracks
            .iter()
            // A brand new track that misses immediately is just noise; one that
            // has been seen a few times may coast for a couple of frames.
            .filter(|track| track.misses == 0 || (track.hits >= 3 && track.misses <= 2))
            .map(|track| TrackedBox {
                track: track.id,
                label: track.label.clone(),
                score: track.score,
                x: track.x,
                y: track.y,
                w: track.w,
                h: track.h,
                seen: track.hits,
            })
            .collect();
        boxes.sort_by(|left, right| right.score.total_cmp(&left.score));
        boxes
    }
}

fn iou(left: (f32, f32, f32, f32), right: (f32, f32, f32, f32)) -> f32 {
    let (lx, ly, lw, lh) = left;
    let (rx, ry, rw, rh) = right;
    let x0 = lx.max(rx);
    let y0 = ly.max(ry);
    let x1 = (lx + lw).min(rx + rw);
    let y1 = (ly + lh).min(ry + rh);
    let intersection = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let union = lw * lh + rw * rh - intersection;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detection(label: &str, x: f32, y: f32) -> Detection {
        Detection {
            label: label.to_string(),
            score: 0.9,
            x,
            y,
            w: 0.2,
            h: 0.2,
        }
    }

    #[test]
    fn a_box_keeps_its_id_across_frames() {
        let mut tracker = Tracker::new();
        let first = tracker.update(vec![detection("crate", 0.2, 0.2)]);
        let second = tracker.update(vec![detection("crate", 0.21, 0.2)]);
        assert_eq!(first[0].track, second[0].track);
        assert_eq!(second[0].seen, 2);
    }

    #[test]
    fn a_far_away_box_gets_a_new_id() {
        let mut tracker = Tracker::new();
        let first = tracker.update(vec![detection("crate", 0.05, 0.05)]);
        let second = tracker.update(vec![detection("crate", 0.7, 0.7)]);
        assert_ne!(first[0].track, second[0].track);
    }

    #[test]
    fn a_label_change_starts_a_new_track() {
        let mut tracker = Tracker::new();
        let first = tracker.update(vec![detection("crate", 0.2, 0.2)]);
        let second = tracker.update(vec![detection("sky", 0.2, 0.2)]);
        assert_ne!(first[0].track, second[0].track);
        assert_eq!(second[0].label, "sky");
    }

    #[test]
    fn smoothing_lands_between_the_two_boxes() {
        let mut tracker = Tracker::new();
        tracker.update(vec![detection("crate", 0.20, 0.2)]);
        let smoothed = tracker.update(vec![detection("crate", 0.24, 0.2)]);
        assert!(smoothed[0].x > 0.20 && smoothed[0].x < 0.24, "x was {}", smoothed[0].x);
    }

    #[test]
    fn a_tracked_box_coasts_through_a_missing_frame_then_goes() {
        let mut tracker = Tracker::new();
        for _ in 0..3 {
            tracker.update(vec![detection("crate", 0.2, 0.2)]);
        }
        let coasting = tracker.update(Vec::new());
        assert_eq!(coasting.len(), 1);
        assert_eq!(coasting[0].seen, 3);

        for _ in 0..4 {
            tracker.update(Vec::new());
        }
        assert!(tracker.update(Vec::new()).is_empty());
    }

    #[test]
    fn a_single_sighting_does_not_coast() {
        let mut tracker = Tracker::new();
        tracker.update(vec![detection("crate", 0.2, 0.2)]);
        assert!(tracker.update(Vec::new()).is_empty());
    }

    #[test]
    fn overlapping_boxes_alone_do_not_match_across_labels() {
        let mut tracker = Tracker::new();
        tracker.update(vec![detection("crate", 0.2, 0.2)]);
        let second = tracker.update(vec![detection("crate", 0.2, 0.2), detection("sky", 0.2, 0.2)]);
        assert_eq!(second.len(), 2);
        assert_ne!(second[0].track, second[1].track);
    }
}
