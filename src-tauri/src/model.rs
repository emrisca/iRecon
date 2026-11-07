use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

/// One downloadable detector. Size and SHA-256 are pinned, so a file that does
/// not match the entry is refused rather than run.
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub note: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub input_size: u32,
}

/// Apache-2.0 RF-DETR COCO exports. Both share the input name `input`, the
/// `dets` and `labels` outputs and the 91 slot class layout; only the square
/// input and the query count differ between them.
pub const CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "rfdetr-nano",
        name: "RF-DETR Nano",
        note: "384 px input and the default: the cheapest proposal pass.",
        file: "rf-detr-nano.onnx",
        url: "https://huggingface.co/open-noodle/rfdetr-nano/resolve/main/detection/model.onnx",
        sha256: "21045851f3906c7d424874363fe933477fda7210bad0d7a69659fb98f8845890",
        bytes: 107_835_234,
        input_size: 384,
    },
    ModelSpec {
        id: "rfdetr-small",
        name: "RF-DETR Small",
        note: "512 px input. Costs more per frame and reaches smaller objects.",
        file: "rf-detr-small.onnx",
        url: "https://huggingface.co/open-noodle/rfdetr-small/resolve/main/detection/model.onnx",
        sha256: "31fe724f8d5dbff5d09eb4c2c44d2d4083c086c167aec54e11e694e652307f9b",
        bytes: 114_448_791,
        input_size: 512,
    },
];

pub fn spec(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|entry| entry.id == id)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: String,
    pub name: String,
    pub note: String,
    pub path: String,
    pub installed: bool,
    pub verified: bool,
    pub bytes: u64,
    pub expected_bytes: u64,
    pub expected_sha: String,
    pub input_size: u32,
    pub url: String,
    pub active: bool,
    pub detail: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub active: Option<String>,
    pub models: Vec<ModelStatus>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProgress {
    pub id: String,
    pub downloaded: u64,
    pub total: u64,
    pub phase: String,
    pub message: String,
}

pub fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn model_file(app: &AppHandle, spec: &ModelSpec) -> Result<PathBuf, String> {
    Ok(models_dir(app)?.join(spec.file))
}

fn marker_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(models_dir(app)?.join("active.txt"))
}

pub fn active_id(app: &AppHandle) -> Option<String> {
    let raw = fs::read_to_string(marker_path(app).ok()?).ok()?;
    let id = raw.trim().to_string();
    spec(&id).map(|_| id)
}

fn set_active(app: &AppHandle, id: &str) -> Result<(), String> {
    fs::write(marker_path(app)?, format!("{id}\n")).map_err(|e| e.to_string())
}

/// The model to load at startup: the last one chosen while it is still on disk,
/// otherwise the first catalog entry that is installed.
pub fn preferred(app: &AppHandle) -> Option<&'static ModelSpec> {
    let on_disk = |entry: &&'static ModelSpec| {
        model_file(app, entry)
            .map(|path| path.exists())
            .unwrap_or(false)
    };
    if let Some(id) = active_id(app) {
        if let Some(entry) = spec(&id).filter(on_disk) {
            return Some(entry);
        }
    }
    CATALOG.iter().find(|entry| on_disk(entry))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn sha256_file(path: &Path) -> Result<(String, u64), String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((to_hex(&hasher.finalize()[..]), total))
}

/// The catalog entry whose pinned hash matches this file, if any.
pub fn spec_for_file(path: &Path) -> Option<&'static ModelSpec> {
    let (digest, _) = sha256_file(path).ok()?;
    CATALOG.iter().find(|entry| entry.sha256 == digest)
}

/// Stream the model to `dest`, hashing as it arrives. The partial file is deleted
/// on any mismatch, so a corrupt download can never be mistaken for a usable model.
pub fn fetch(
    url: &str,
    dest: &Path,
    expected_sha: &str,
    expected_bytes: u64,
    mut report: impl FnMut(u64, u64),
) -> Result<(), String> {
    let part = dest.with_extension("part");
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Could not start the download: {e}"))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("Could not reach the model host: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "The model host answered with status {}",
            response.status()
        ));
    }

    let total = response.content_length().unwrap_or(expected_bytes);
    let mut file = File::create(&part).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("The download stopped early: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
        hasher.update(&buffer[..read]);
        downloaded += read as u64;
        report(downloaded, total);
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    let digest = to_hex(&hasher.finalize()[..]);
    if digest != expected_sha {
        let _ = fs::remove_file(&part);
        return Err(format!(
            "Hash mismatch. Expected {expected_sha}, got {digest}. The file was discarded."
        ));
    }

    fs::rename(&part, dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// Verify the pinned hash and load the detector into app state.
pub fn load_now(app: &AppHandle, entry: &ModelSpec) -> Result<String, String> {
    let path = model_file(app, entry)?;
    if !path.exists() {
        return Err(format!("{} is not downloaded yet.", entry.name));
    }
    let (digest, _) = sha256_file(&path)?;
    if digest != entry.sha256 {
        return Err(format!(
            "{} does not match the pinned hash, so it was not loaded.",
            entry.name
        ));
    }

    let detector = crate::detector::Detector::load(&path, entry.input_size)?;
    let detail = detector.describe();
    let state = app.state::<AppState>();
    {
        let mut guard = state.detector.lock().map_err(|e| e.to_string())?;
        *guard = Some(detector);
    }
    if let Ok(mut guard) = state.loaded_model.lock() {
        *guard = Some(entry.id.to_string());
    }
    set_active(app, entry.id)?;
    Ok(detail)
}

fn status_for(
    app: &AppHandle,
    entry: &'static ModelSpec,
    loaded: Option<&str>,
) -> Result<ModelStatus, String> {
    let path = model_file(app, entry)?;
    let installed = path.exists();
    let mut bytes = 0u64;
    let mut verified = false;
    let mut detail = None;

    if installed {
        match sha256_file(&path) {
            Ok((digest, size)) => {
                bytes = size;
                verified = digest == entry.sha256;
                if !verified {
                    detail = Some("This file is not the model iRecon expects.".into());
                }
            }
            Err(error) => detail = Some(error),
        }
    }

    Ok(ModelStatus {
        id: entry.id.to_string(),
        name: entry.name.to_string(),
        note: entry.note.to_string(),
        path: path.to_string_lossy().to_string(),
        installed,
        verified,
        bytes,
        expected_bytes: entry.bytes,
        expected_sha: entry.sha256.to_string(),
        input_size: entry.input_size,
        url: entry.url.to_string(),
        active: loaded == Some(entry.id),
        detail,
    })
}

#[tauri::command]
pub fn model_catalog(app: AppHandle) -> Result<ModelCatalog, String> {
    let loaded = app
        .state::<AppState>()
        .loaded_model
        .lock()
        .ok()
        .and_then(|guard| guard.clone());

    let mut models = Vec::new();
    for entry in CATALOG {
        models.push(status_for(&app, entry, loaded.as_deref())?);
    }
    Ok(ModelCatalog {
        active: loaded,
        models,
    })
}

#[tauri::command]
pub fn model_download(app: AppHandle, id: String) -> Result<(), String> {
    let entry = spec(&id).ok_or_else(|| format!("Unknown model {id}."))?;
    let path = model_file(&app, entry)?;
    let handle = app.clone();

    std::thread::Builder::new()
        .name(format!("irecon-model-{}", entry.id))
        .spawn(move || {
            let _ = handle.emit(
                "model:progress",
                ModelProgress {
                    id: entry.id.to_string(),
                    downloaded: 0,
                    total: entry.bytes,
                    phase: "downloading".into(),
                    message: format!("Downloading {}", entry.name),
                },
            );

            let mut last = 0u64;
            let result = fetch(
                entry.url,
                &path,
                entry.sha256,
                entry.bytes,
                |downloaded, total| {
                    if downloaded.saturating_sub(last) >= 2 * 1024 * 1024 || downloaded == total {
                        last = downloaded;
                        let _ = handle.emit(
                            "model:progress",
                            ModelProgress {
                                id: entry.id.to_string(),
                                downloaded,
                                total,
                                phase: "downloading".into(),
                                message: String::new(),
                            },
                        );
                    }
                },
            );

            let payload = match result {
                Ok(()) => {
                    let message = match load_now(&handle, entry) {
                        Ok(detail) => format!("Downloaded, verified and loaded. {detail}"),
                        Err(error) => format!("Downloaded and verified, but not loaded: {error}"),
                    };
                    ModelProgress {
                        id: entry.id.to_string(),
                        downloaded: entry.bytes,
                        total: entry.bytes,
                        phase: "ready".into(),
                        message,
                    }
                }
                Err(error) => ModelProgress {
                    id: entry.id.to_string(),
                    downloaded: 0,
                    total: entry.bytes,
                    phase: "failed".into(),
                    message: error,
                },
            };
            let _ = handle.emit("model:progress", payload);
        })
        .map_err(|e| format!("Could not start the download thread: {e}"))?;

    Ok(())
}

#[tauri::command]
pub fn model_load(app: AppHandle, id: String) -> Result<(), String> {
    let entry = spec(&id).ok_or_else(|| format!("Unknown model {id}."))?;
    let handle = app.clone();
    std::thread::Builder::new()
        .name(format!("irecon-model-{}", entry.id))
        .spawn(move || {
            let payload = match load_now(&handle, entry) {
                Ok(message) => ModelProgress {
                    id: entry.id.to_string(),
                    downloaded: entry.bytes,
                    total: entry.bytes,
                    phase: "loaded".into(),
                    message,
                },
                Err(error) => ModelProgress {
                    id: entry.id.to_string(),
                    downloaded: 0,
                    total: entry.bytes,
                    phase: "failed".into(),
                    message: error,
                },
            };
            let _ = handle.emit("model:progress", payload);
        })
        .map_err(|e| format!("Could not start the model thread: {e}"))?;
    Ok(())
}
