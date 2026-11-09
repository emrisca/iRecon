//! Dataset export: labeled frames and boxes in a training-ready layout, with a
//! manifest that says exactly what was written and what was skipped.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::json;
use tauri::Manager;

use crate::db::slugify;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetExport {
    pub path: String,
    pub frames: usize,
    pub boxes: usize,
    pub classes: usize,
    pub crops: usize,
    pub skipped: usize,
}

#[derive(Clone, Copy)]
struct NormalizedBox {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct ExampleRow {
    id: i64,
    class_id: i64,
    bbox: Option<String>,
    frame_id: Option<i64>,
    crop: String,
    frame: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    role: String,
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0)
}

fn parse_box(raw: &str) -> Option<NormalizedBox> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let read = |key: &str| value.get(key)?.as_f64().map(|number| number as f32);
    let parsed = NormalizedBox {
        x: read("x")?,
        y: read("y")?,
        w: read("w")?,
        h: read("h")?,
    };
    let inside = |number: f32| (0.0..=1.0).contains(&number);
    if !inside(parsed.x) || !inside(parsed.y) || parsed.w <= 0.0 || parsed.h <= 0.0 {
        return None;
    }
    if !inside(parsed.x + parsed.w) || !inside(parsed.y + parsed.h) {
        return None;
    }
    Some(parsed)
}

fn query_examples(conn: &Connection, project_id: i64) -> Result<Vec<ExampleRow>, String> {
    let mut statement = conn
        .prepare(
            "select e.id, e.class_id, e.bbox, e.frame_id, e.path, f.path, f.width, f.height,
                    e.role
             from example e
             join class c on c.id = e.class_id
             left join frame f on f.id = e.frame_id
             where c.project_id = ?1
             order by e.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok(ExampleRow {
                id: row.get(0)?,
                class_id: row.get(1)?,
                bbox: row.get(2)?,
                frame_id: row.get(3)?,
                crop: row.get(4)?,
                frame: row.get(5)?,
                width: row.get(6)?,
                height: row.get(7)?,
                role: row.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut examples = Vec::new();
    for row in rows {
        examples.push(row.map_err(|e| e.to_string())?);
    }
    Ok(examples)
}

fn query_classes(conn: &Connection, project_id: i64) -> Result<Vec<(i64, String)>, String> {
    let mut statement = conn
        .prepare("select id, name from class where project_id = ?1 order by id")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut classes = Vec::new();
    for row in rows {
        classes.push(row.map_err(|e| e.to_string())?);
    }
    Ok(classes)
}

/// Writes YOLO detection data plus the crops, and a manifest describing both.
///
/// Frames are the unit of the detection layout: one image and one label file per
/// source frame, boxes normalised to the frame, class indices from `classes.txt`.
/// Crops are kept too, so the same export also works as a classification or
/// embedding dataset without a second pass over the database.
pub(crate) fn write_dataset(
    conn: &Connection,
    project_id: i64,
    out: &Path,
) -> Result<DatasetExport, String> {
    let (name, slug): (String, String) = conn
        .query_row(
            "select name, slug from project where id = ?1",
            params![project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "That project no longer exists".to_string())?;

    let classes = query_classes(conn, project_id)?;
    let index_of: HashMap<i64, usize> = classes
        .iter()
        .enumerate()
        .map(|(index, (id, _))| (*id, index))
        .collect();
    let rows = query_examples(conn, project_id)?;

    for folder in ["images", "labels", "crops"] {
        fs::create_dir_all(out.join(folder)).map_err(|e| e.to_string())?;
    }

    let mut skipped = 0usize;
    let mut boxes = 0usize;
    let mut crops = 0usize;
    let mut class_counts = vec![0usize; classes.len()];
    let mut label_lines: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let mut frame_meta: BTreeMap<i64, (String, i64, i64)> = BTreeMap::new();

    for row in rows {
        if let Some(index) = index_of.get(&row.class_id) {
            let folder = out.join("crops").join(slugify(&classes[*index].1));
            if fs::create_dir_all(&folder).is_ok() {
                let target = folder.join(format!("{}.png", row.id));
                if fs::copy(&row.crop, target).is_ok() {
                    crops += 1;
                }
            }
        }

        // A negative keeps its crop, because it is useful as a classification
        // input, but it is not a box and must never be written as one.
        if row.role == "negative" {
            continue;
        }

        let Some(frame_id) = row.frame_id else {
            skipped += 1;
            continue;
        };
        let Some(frame_path) = row.frame.clone() else {
            skipped += 1;
            continue;
        };
        let Some(bbox) = row.bbox.as_deref().and_then(parse_box) else {
            skipped += 1;
            continue;
        };
        let Some(index) = index_of.get(&row.class_id).copied() else {
            skipped += 1;
            continue;
        };

        if !frame_meta.contains_key(&frame_id) {
            let target = out.join("images").join(format!("{frame_id}.png"));
            if let Err(error) = fs::copy(&frame_path, &target) {
                let _ = error;
                skipped += 1;
                continue;
            }
            frame_meta.insert(
                frame_id,
                (
                    target.to_string_lossy().to_string(),
                    row.width.unwrap_or(0),
                    row.height.unwrap_or(0),
                ),
            );
        }

        let centre_x = bbox.x + bbox.w / 2.0;
        let centre_y = bbox.y + bbox.h / 2.0;
        label_lines.entry(frame_id).or_default().push(format!(
            "{index} {centre_x:.6} {centre_y:.6} {:.6} {:.6}",
            bbox.w, bbox.h
        ));
        class_counts[index] += 1;
        boxes += 1;
    }

    for (frame_id, lines) in &label_lines {
        let mut text = lines.join("\n");
        text.push('\n');
        fs::write(out.join("labels").join(format!("{frame_id}.txt")), text)
            .map_err(|e| e.to_string())?;
    }

    let mut class_list = String::new();
    for (_, class_name) in &classes {
        class_list.push_str(class_name);
        class_list.push('\n');
    }
    fs::write(out.join("classes.txt"), class_list).map_err(|e| e.to_string())?;

    let mut yaml = String::from("path: .\ntrain: images\nval: images\nnames:\n");
    for (index, (_, class_name)) in classes.iter().enumerate() {
        yaml.push_str(&format!("  {index}: {class_name}\n"));
    }
    fs::write(out.join("data.yaml"), yaml).map_err(|e| e.to_string())?;

    let manifest = json!({
        "format": "yolo-detection",
        "created_at": now_millis(),
        "project": { "id": project_id, "name": name, "slug": slug },
        "embedding_model": crate::matcher::EMBEDDING_MODEL,
        "classes": classes
            .iter()
            .enumerate()
            .map(|(index, (id, class_name))| json!({
                "index": index,
                "id": id,
                "name": class_name,
                "boxes": class_counts[index],
            }))
            .collect::<Vec<_>>(),
        "counts": {
            "frames": label_lines.len(),
            "boxes": boxes,
            "crops": crops,
            "skipped": skipped,
        },
        "paths": {
            "images": "images",
            "labels": "labels",
            "crops": "crops",
            "classes": "classes.txt",
            "data": "data.yaml",
        },
        "images": frame_meta
            .iter()
            .map(|(frame_id, (path, width, height))| json!({
                "id": frame_id,
                "file": format!("images/{frame_id}.png"),
                "source": path,
                "width": width,
                "height": height,
                "boxes": label_lines.get(frame_id).map(|lines| lines.len()).unwrap_or(0),
            }))
            .collect::<Vec<_>>(),
    });
    fs::write(
        out.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    Ok(DatasetExport {
        path: out.to_string_lossy().to_string(),
        frames: label_lines.len(),
        boxes,
        classes: classes.len(),
        crops,
        skipped,
    })
}

#[tauri::command]
pub fn dataset_export(app: tauri::AppHandle, project_id: i64) -> Result<DatasetExport, String> {
    let state = app.state::<crate::AppState>();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let slug: String = conn
        .query_row(
            "select slug from project where id = ?1",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|_| "That project no longer exists".to_string())?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("projects")
        .join(&slug)
        .join("exports")
        .join(now_millis().to_string());
    let result = write_dataset(&conn, project_id, &root);
    drop(conn);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let stamp = now_millis();
        std::path::Path::new("target")
            .join(format!("dataset-test-{label}-{stamp}"))
    }

    #[test]
    fn a_box_is_only_valid_inside_the_frame() {
        assert!(parse_box("{\"x\":0.1,\"y\":0.2,\"w\":0.3,\"h\":0.4}").is_some());
        assert!(parse_box("{\"x\":0.9,\"y\":0.2,\"w\":0.3,\"h\":0.4}").is_none());
        assert!(parse_box("{\"x\":0.1,\"y\":0.2,\"w\":0,\"h\":0.4}").is_none());
        assert!(parse_box("not json").is_none());
    }

    #[test]
    fn an_export_writes_images_labels_and_a_manifest() {
        let dir = temp_dir("layout");
        fs::create_dir_all(&dir).unwrap();
        let database = dir.join("project.sqlite");
        let conn = crate::db::open(&database).unwrap();

        let frame_path = dir.join("frame.png");
        image::RgbaImage::from_pixel(64, 48, image::Rgba([30, 40, 50, 255]))
            .save(&frame_path)
            .unwrap();
        let crop_path = dir.join("crop.png");
        image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 80, 40, 255]))
            .save(&crop_path)
            .unwrap();

        conn.execute(
            "insert into project (id, name, slug, created_at) values (1, 'Arena', 'arena', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into frame (id, project_id, path, width, height, created_at)
             values (10, 1, ?1, 64, 48, 'now')",
            params![frame_path.to_string_lossy().to_string()],
        )
        .unwrap();
        conn.execute(
            "insert into class (id, project_id, name, color) values (20, 1, 'Crate', 'oklch(0.6 0.1 20)')",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into example (id, class_id, frame_id, path, bbox, created_at)
             values (30, 20, 10, ?1, ?2, 'now')",
            params![
                crop_path.to_string_lossy().to_string(),
                "{\"x\":0.25,\"y\":0.25,\"w\":0.5,\"h\":0.5}"
            ],
        )
        .unwrap();

        let out = dir.join("export");
        let result = write_dataset(&conn, 1, &out).unwrap();
        assert_eq!(result.frames, 1);
        assert_eq!(result.boxes, 1);
        assert_eq!(result.classes, 1);
        assert_eq!(result.crops, 1);
        assert_eq!(result.skipped, 0);

        let label = fs::read_to_string(out.join("labels/10.txt")).unwrap();
        assert_eq!(label.trim(), "0 0.500000 0.500000 0.500000 0.500000");
        assert!(out.join("images/10.png").exists());
        assert!(out.join("crops/crate/30.png").exists());
        assert!(out.join("data.yaml").exists());

        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(out.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["counts"]["boxes"], 1);
        assert_eq!(manifest["classes"][0]["name"], "Crate");
        assert_eq!(manifest["format"], "yolo-detection");
    }

    #[test]
    fn an_example_without_a_frame_is_counted_as_skipped() {
        let dir = temp_dir("skip");
        fs::create_dir_all(&dir).unwrap();
        let conn = crate::db::open(&dir.join("project.sqlite")).unwrap();
        let crop_path = dir.join("crop.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]))
            .save(&crop_path)
            .unwrap();

        conn.execute(
            "insert into project (id, name, slug, created_at) values (1, 'Arena', 'arena', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into class (id, project_id, name, color) values (20, 1, 'Crate', 'x')",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into example (id, class_id, path, bbox, created_at)
             values (30, 20, ?1, ?2, 'now')",
            params![
                crop_path.to_string_lossy().to_string(),
                "{\"x\":0.25,\"y\":0.25,\"w\":0.5,\"h\":0.5}"
            ],
        )
        .unwrap();

        let result = write_dataset(&conn, 1, &dir.join("export")).unwrap();
        assert_eq!(result.frames, 0);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.crops, 1);
    }
}
