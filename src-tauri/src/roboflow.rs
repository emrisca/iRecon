//! Roboflow dataset import: an extracted YOLO export becomes frames, classes
//! and examples in one step, so a downloaded dataset is usable without a script.
//!
//! A Roboflow export is a folder with `data.yaml` and split folders such as
//! `train/images` next to `train/labels`. Each label line is
//! `class centre_x centre_y width height`, normalised to the image. The images
//! are copied into the project, the boxes become examples with their crops, and
//! class names come from `data.yaml`, so names match the export rather than a
//! guessed index.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};
use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::db::slugify;
use crate::label;
use crate::matcher;
use crate::source::is_image;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoboflowReport {
    pub frames: usize,
    pub examples: usize,
    pub classes: usize,
    /// Images already in the project, images without a readable picture, and
    /// boxes that could not be used.
    pub skipped: usize,
    pub cancelled: bool,
    pub errors: Vec<String>,
}

/// Sent while an import runs, so the Library page shows how far it has got.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoboflowProgress {
    pub processed: usize,
    pub total: usize,
    pub frames: usize,
    pub examples: usize,
    pub skipped: usize,
}

static IMPORT_RUNNING: AtomicBool = AtomicBool::new(false);
static IMPORT_CANCEL: AtomicBool = AtomicBool::new(false);
/// Rows are committed in batches, so one long transaction never holds the
/// database and work done before a cancel is already visible.
const BATCH: usize = 25;

#[derive(Clone, Copy, Debug)]
pub(crate) struct YoloBox {
    pub class_index: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

fn clean_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_comment = match trimmed.find(" #") {
        Some(index) => &trimmed[..index],
        None => trimmed,
    };
    without_comment
        .trim()
        .trim_matches(|ch| ch == '\'' || ch == '"')
        .trim()
        .to_string()
}

fn parse_inline_names(value: &str) -> Vec<String> {
    let body = value.trim();
    let body = body
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .or_else(|| {
            body.strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
        });
    let Some(body) = body else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for part in body.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let raw = match part.split_once(':') {
            Some((key, value)) if key.trim().parse::<usize>().is_ok() => value,
            _ => part,
        };
        let name = clean_name(raw);
        if !name.is_empty() {
            names.push(name);
        }
    }
    names
}

/// Reads the `names:` block of a YOLO `data.yaml`. Both the list form and the
/// index map form are common, and Roboflow writes the list inline.
pub(crate) fn parse_names(text: &str) -> Vec<String> {
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        let Some(inline) = trimmed.strip_prefix("names:") else {
            continue;
        };
        if !inline.trim().is_empty() {
            return parse_inline_names(inline);
        }

        let mut list: Vec<String> = Vec::new();
        let mut map: BTreeMap<usize, String> = BTreeMap::new();
        while let Some(next) = lines.peek() {
            let indented = next.len() != next.trim_start().len();
            let row = next.trim();
            if row.is_empty() {
                lines.next();
                continue;
            }
            if !indented {
                break;
            }
            lines.next();
            let row = row.trim_start_matches('-').trim();
            if let Some((key, value)) = row.split_once(':') {
                if let Ok(index) = key.trim().parse::<usize>() {
                    map.insert(index, clean_name(value));
                    continue;
                }
            }
            list.push(clean_name(row));
        }
        if !map.is_empty() {
            return map.into_values().filter(|name| !name.is_empty()).collect();
        }
        return list.into_iter().filter(|name| !name.is_empty()).collect();
    }
    Vec::new()
}

/// One YOLO detection line. Six fields appear on exports that carry a
/// confidence, and both are accepted.
pub(crate) fn parse_label_line(line: &str) -> Option<YoloBox> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 5 && parts.len() != 6 {
        return None;
    }
    let class_index = parts[0].parse::<usize>().ok()?;
    let centre_x = parts[1].parse::<f32>().ok()?;
    let centre_y = parts[2].parse::<f32>().ok()?;
    let raw_w = parts[3].parse::<f32>().ok()?;
    let raw_h = parts[4].parse::<f32>().ok()?;
    if !(0.0..=1.0).contains(&centre_x) || !(0.0..=1.0).contains(&centre_y) {
        return None;
    }
    if raw_w <= 0.0 || raw_h <= 0.0 {
        return None;
    }
    let x = (centre_x - raw_w / 2.0).clamp(0.0, 1.0);
    let y = (centre_y - raw_h / 2.0).clamp(0.0, 1.0);
    let w = raw_w.min(1.0 - x);
    let h = raw_h.min(1.0 - y);
    if w < 0.001 || h < 0.001 {
        return None;
    }
    Some(YoloBox {
        class_index,
        x,
        y,
        w,
        h,
    })
}

fn find_data_yaml(root: &Path, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let candidate = root.join("data.yaml");
    if candidate.is_file() {
        return Some(candidate);
    }
    let mut folders: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        if let Some(found) = find_data_yaml(&folder, depth - 1) {
            return Some(found);
        }
    }
    None
}

fn collect_images(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
            if name == "labels" || name.starts_with('.') {
                continue;
            }
            collect_images(&path, depth - 1, out);
        } else if is_image(&path) {
            out.push(path);
        }
    }
}

/// Roboflow puts `images` and `labels` side by side; a plain YOLO folder keeps
/// the label next to its image.
fn label_path(image: &Path) -> Option<PathBuf> {
    // Roboflow names carry dots (`name_jpg.rf.<hash>.jpg`), so appending to the
    // stem matters: `with_extension` would replace the hash, not add `.txt`.
    let name = format!("{}.txt", image.file_stem()?.to_str()?);
    let parent = image.parent()?;
    if parent.file_name().and_then(|value| value.to_str()) == Some("images") {
        let split = parent.parent()?;
        return Some(split.join("labels").join(name));
    }
    Some(parent.join(name))
}

/// A stable name per source file, so importing the same export twice does not
/// copy the same picture again under a new name.
fn short_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn ensure_class(conn: &Connection, project_id: i64, name: &str) -> Result<i64, String> {
    if let Ok(id) = conn.query_row(
        "select id from class where project_id = ?1 and lower(name) = lower(?2)",
        params![project_id, name],
        |row| row.get::<_, i64>(0),
    ) {
        return Ok(id);
    }
    let count: i64 = conn
        .query_row(
            "select count(*) from class where project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let color = label::PALETTE[(count as usize) % label::PALETTE.len()];
    conn.execute(
        "insert into class (project_id, name, color, threshold) values (?1, ?2, ?3, 0.45)",
        params![project_id, name, color],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

fn push_error(errors: &mut Vec<String>, message: String) {
    if errors.len() < 20 {
        errors.push(message);
    }
}

fn import_image(
    conn: &Connection,
    project_id: i64,
    dataset_root: &Path,
    image_path: &Path,
    names: &[String],
    frames_dir: &Path,
    examples_root: &Path,
    thumbs_dir: &Path,
    used_classes: &mut BTreeSet<i64>,
) -> Result<(bool, usize, usize), String> {
    let relative = image_path
        .strip_prefix(dataset_root)
        .unwrap_or(image_path)
        .to_string_lossy()
        .to_string();
    let stem = image_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let slug = slugify(stem);
    let extension = image_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let file_name = format!(
        "roboflow-{}-{}.{}",
        short_hash(&relative),
        if slug.is_empty() { "image" } else { &slug },
        extension
    );
    let destination = frames_dir.join(file_name);

    let already: i64 = conn
        .query_row(
            "select count(*) from frame where project_id = ?1 and path = ?2",
            params![project_id, destination.to_string_lossy().to_string()],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if already > 0 {
        return Ok((false, 0, 1));
    }

    let source = image::open(image_path).map_err(|e| format!("could not read the image: {e}"))?;
    let source = source.to_rgba8();
    let (width, height) = (source.width(), source.height());
    if width == 0 || height == 0 {
        return Err("the image is empty".into());
    }
    fs::copy(image_path, &destination)
        .map_err(|e| format!("could not copy it into the project: {e}"))?;

    conn.execute(
        "insert into frame (project_id, path, width, height, created_at)
         values (?1, ?2, ?3, ?4, datetime('now'))",
        params![
            project_id,
            destination.to_string_lossy().to_string(),
            width as i64,
            height as i64
        ],
    )
    .map_err(|e| e.to_string())?;
    let frame_id = conn.last_insert_rowid();

    // The decoded image is right here, so make the library thumbnail now instead
    // of decoding the same file again when the grid lists it.
    let thumb = label::jpeg_bytes(&source, label::THUMB_WIDTH, label::THUMB_QUALITY);
    if !thumb.is_empty() {
        let _ = fs::write(thumbs_dir.join(format!("{frame_id}.jpg")), thumb);
    }

    let mut boxes: Vec<YoloBox> = Vec::new();
    let mut skipped = 0usize;
    if let Some(labels) = label_path(image_path) {
        if labels.is_file() {
            let text = fs::read_to_string(&labels)
                .map_err(|e| format!("could not read {}: {e}", labels.display()))?;
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match parse_label_line(line) {
                    Some(parsed) if parsed.class_index < names.len() => boxes.push(parsed),
                    _ => skipped += 1,
                }
            }
        }
    }

    let mut written = 0usize;
    for (index, parsed) in boxes.iter().enumerate() {
        let name = &names[parsed.class_index];
        let class_id = ensure_class(conn, project_id, name)?;
        used_classes.insert(class_id);

        let x0 = (parsed.x * width as f32).round() as u32;
        let y0 = (parsed.y * height as f32).round() as u32;
        let x1 = ((parsed.x + parsed.w) * width as f32).round() as u32;
        let y1 = ((parsed.y + parsed.h) * height as f32).round() as u32;
        let x1 = x1.min(width);
        let y1 = y1.min(height);
        if x1 <= x0 || y1 <= y0 {
            skipped += 1;
            continue;
        }

        let crop = image::imageops::crop_imm(&source, x0, y0, x1 - x0, y1 - y0).to_image();
        let directory = examples_root.join(slugify(name));
        fs::create_dir_all(&directory)
            .map_err(|e| format!("could not make the example folder: {e}"))?;
        let path = directory.join(format!("{frame_id}-{index}.png"));
        // Fast PNG: the crop is training data, and the default filter chain
        // spends most import time chasing a few percent smaller files.
        let file =
            fs::File::create(&path).map_err(|e| format!("could not save the crop: {e}"))?;
        PngEncoder::new_with_quality(
            std::io::BufWriter::new(file),
            CompressionType::Fast,
            FilterType::Adaptive,
        )
        .write_image(
            crop.as_raw(),
            crop.width(),
            crop.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("could not save the crop: {e}"))?;

        let bbox = format!(
            "{{\"x\":{:.5},\"y\":{:.5},\"w\":{:.5},\"h\":{:.5}}}",
            parsed.x, parsed.y, parsed.w, parsed.h
        );
        let embedding = matcher::embed(&crop);
        conn.execute(
            "insert into example
               (class_id, frame_id, path, role, origin, bbox, embedding, embedding_model, created_at)
             values (?1, ?2, ?3, 'positive', 'roboflow', ?4, ?5, ?6, datetime('now'))",
            params![
                class_id,
                frame_id,
                path.to_string_lossy().to_string(),
                bbox,
                matcher::to_blob(&embedding),
                matcher::EMBEDDING_MODEL
            ],
        )
        .map_err(|e| e.to_string())?;
        written += 1;
    }

    Ok((true, written, skipped))
}

/// Pure import so a test can run it against a temporary project. The caller
/// resolves the project's `frames` and `examples` folders.
pub(crate) fn import_dataset(
    conn: &Connection,
    project_id: i64,
    dataset_root: &Path,
    frames_dir: &Path,
    examples_root: &Path,
    thumbs_dir: &Path,
    should_stop: &dyn Fn() -> bool,
    on_progress: &mut dyn FnMut(usize, usize, &RoboflowReport),
) -> Result<RoboflowReport, String> {
    if !dataset_root.is_dir() {
        return Err(format!("{} is not a folder", dataset_root.display()));
    }
    let yaml_path = find_data_yaml(dataset_root, 3).ok_or_else(|| {
        format!(
            "No data.yaml under {}. Point at the extracted export folder.",
            dataset_root.display()
        )
    })?;
    let yaml = fs::read_to_string(&yaml_path)
        .map_err(|e| format!("Could not read {}: {e}", yaml_path.display()))?;
    let names = parse_names(&yaml);
    if names.is_empty() {
        return Err(format!(
            "{} has no names list, so the classes are unknown",
            yaml_path.display()
        ));
    }

    let mut images = Vec::new();
    collect_images(dataset_root, 8, &mut images);
    if images.is_empty() {
        return Err(format!(
            "No PNG or JPEG images under {}",
            dataset_root.display()
        ));
    }

    fs::create_dir_all(frames_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(examples_root).map_err(|e| e.to_string())?;
    fs::create_dir_all(thumbs_dir).map_err(|e| e.to_string())?;

    let mut transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut report = RoboflowReport {
        frames: 0,
        examples: 0,
        classes: 0,
        skipped: 0,
        cancelled: false,
        errors: Vec::new(),
    };
    let mut used_classes = BTreeSet::new();
    let mut seen = BTreeSet::new();

    on_progress(0, images.len(), &report);
    for (index, image_path) in images.iter().enumerate() {
        if should_stop() {
            report.cancelled = true;
            break;
        }
        // A dataset can hold the same picture in two splits; the first one wins.
        if !seen.insert(short_hash(&image_path.to_string_lossy())) {
            report.skipped += 1;
            on_progress(index + 1, images.len(), &report);
            continue;
        }
        match import_image(
            &transaction,
            project_id,
            dataset_root,
            image_path,
            &names,
            frames_dir,
            examples_root,
            thumbs_dir,
            &mut used_classes,
        ) {
            Ok((added, examples, skipped)) => {
                if added {
                    report.frames += 1;
                }
                report.examples += examples;
                report.skipped += skipped;
            }
            Err(error) => {
                report.skipped += 1;
                push_error(&mut report.errors, format!("{}: {error}", image_path.display()));
            }
        }
        on_progress(index + 1, images.len(), &report);

        // Commit in batches: the library can list what has landed, and a long
        // import never holds one write transaction.
        if (index + 1) % BATCH == 0 {
            transaction.commit().map_err(|e| e.to_string())?;
            transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        }
    }

    transaction.commit().map_err(|e| e.to_string())?;
    report.classes = used_classes.len();
    Ok(report)
}

#[tauri::command(async)]
pub fn dataset_import_roboflow(
    app: AppHandle,
    project_id: i64,
    folder: String,
) -> Result<RoboflowReport, String> {
    if IMPORT_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("An import is already running".into());
    }
    IMPORT_CANCEL.store(false, Ordering::SeqCst);
    let outcome = (|| {
        let root = PathBuf::from(folder.trim());
        if root.as_os_str().is_empty() {
            return Err("Give the folder of the extracted Roboflow export".into());
        }
        let (_slug, project_root) = label::project_folder(&app, project_id)?;
        // Reading the project's own copies would import its frames back in.
        if root.starts_with(&project_root) {
            return Err("Point at the dataset folder, not the project folder".into());
        }
        let db_path = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("irecon.sqlite");
        // A second connection keeps the import's writes off the mutex every
        // screen command shares, which is what froze the window during a run.
        let conn = Connection::open(&db_path)
            .map_err(|e| format!("Could not open the project database: {e}"))?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");

        let mut last: Option<Instant> = None;
        let mut progress = |processed: usize, total: usize, report: &RoboflowReport| {
            // A 7k image import would otherwise flood the event queue.
            let edge = processed == 0 || processed == total;
            let ready = last
                .map(|at| at.elapsed() >= Duration::from_millis(120))
                .unwrap_or(true);
            if !edge && !ready {
                return;
            }
            last = Some(Instant::now());
            let _ = app.emit(
                "roboflow:import-progress",
                RoboflowProgress {
                    processed,
                    total,
                    frames: report.frames,
                    examples: report.examples,
                    skipped: report.skipped,
                },
            );
        };
        import_dataset(
            &conn,
            project_id,
            &root,
            &project_root.join("frames"),
            &project_root.join("examples"),
            &project_root.join("thumbs"),
            &|| IMPORT_CANCEL.load(Ordering::SeqCst),
            &mut progress,
        )
    })();
    IMPORT_RUNNING.store(false, Ordering::SeqCst);
    IMPORT_CANCEL.store(false, Ordering::SeqCst);
    outcome
}

/// Stops the running import at the next image. Work already committed is kept.
#[tauri::command]
pub fn dataset_import_cancel() -> bool {
    if !IMPORT_RUNNING.load(Ordering::SeqCst) {
        return false;
    }
    IMPORT_CANCEL.store(true, Ordering::SeqCst);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        std::path::Path::new("target")
            .join(format!("roboflow-test-{label}-{}", std::process::id()))
    }

    #[test]
    fn names_come_from_every_common_shape() {
        assert_eq!(
            parse_names("nc: 2\nnames: ['player', 'enemy']\n"),
            vec!["player".to_string(), "enemy".to_string()]
        );
        assert_eq!(
            parse_names("names:\n  - player\n  - enemy\nnc: 2\n"),
            vec!["player".to_string(), "enemy".to_string()]
        );
        assert_eq!(
            parse_names("names:\n  0: player\n  1: enemy\n"),
            vec!["player".to_string(), "enemy".to_string()]
        );
    }

    #[test]
    fn a_label_line_becomes_a_top_left_box() {
        let parsed = parse_label_line("0 0.5 0.5 0.2 0.4").unwrap();
        assert_eq!(parsed.class_index, 0);
        assert!((parsed.x - 0.4).abs() < 1e-6);
        assert!((parsed.y - 0.3).abs() < 1e-6);
        assert!((parsed.w - 0.2).abs() < 1e-6);
        assert!((parsed.h - 0.4).abs() < 1e-6);

        assert!(parse_label_line("0 0.5 0.5 0.2 0.4 0.91").is_some());
        assert!(parse_label_line("0 1.5 0.5 0.2 0.4").is_none());
        assert!(parse_label_line("0 0.5 0.5 0 0.4").is_none());
        assert!(parse_label_line("0 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1").is_none());
    }

    #[test]
    fn an_export_becomes_frames_classes_and_examples() {
        let dir = temp_dir("import");
        let frames = temp_dir("import-frames");
        let examples = temp_dir("import-examples");
        let thumbs = temp_dir("import-thumbs");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&frames);
        let _ = fs::remove_dir_all(&examples);
        let _ = fs::remove_dir_all(&thumbs);
        fs::create_dir_all(dir.join("train/images")).unwrap();
        fs::create_dir_all(dir.join("train/labels")).unwrap();
        image::RgbaImage::from_pixel(80, 40, image::Rgba([10, 20, 30, 255]))
            .save(dir.join("train/images/a_jpg.rf.deadbeef.png"))
            .unwrap();
        image::RgbaImage::from_pixel(80, 40, image::Rgba([40, 50, 60, 255]))
            .save(dir.join("train/images/b_jpg.rf.cafebabe.png"))
            .unwrap();
        fs::write(
            dir.join("data.yaml"),
            "train: train/images\nnc: 1\nnames: ['crate']\n",
        )
        .unwrap();
        fs::write(
            dir.join("train/labels/a_jpg.rf.deadbeef.txt"),
            "0 0.5 0.5 0.5 0.5\n0 0.25 0.25 0.2 0.2\n",
        )
        .unwrap();

        let conn = crate::db::open(&dir.join("project.sqlite")).unwrap();
        conn.execute(
            "insert into project (id, name, slug, created_at) values (1, 'Arena', 'arena', 'now')",
            [],
        )
        .unwrap();

        let report = import_dataset(
            &conn,
            1,
            &dir,
            &frames,
            &examples,
            &thumbs,
            &|| false,
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(report.frames, 2);
        assert_eq!(report.examples, 2);
        assert_eq!(report.classes, 1);
        assert_eq!(report.skipped, 0);

        let name: String = conn
            .query_row("select name from class", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "crate");
        let bbox: String = conn
            .query_row("select bbox from example order by id limit 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(bbox.contains("\"x\":0.25000"), "bbox was {bbox}");
        assert!(examples.join("crate").is_dir());

        let again =
            import_dataset(
                &conn,
                1,
                &dir,
                &frames,
                &examples,
                &thumbs,
                &|| false,
                &mut |_, _, _| {},
            )
            .unwrap();
        assert_eq!(again.frames, 0);
        assert_eq!(again.skipped, 2);

        let stopped =
            import_dataset(
                &conn,
                1,
                &dir,
                &frames,
                &examples,
                &thumbs,
                &|| true,
                &mut |_, _, _| {},
            )
            .unwrap();
        assert!(stopped.cancelled);
        assert_eq!(stopped.frames, 0);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&frames);
        let _ = fs::remove_dir_all(&examples);
        let _ = fs::remove_dir_all(&thumbs);
    }
}
