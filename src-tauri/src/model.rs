use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

pub const MODEL_FILE: &str = "rf-detr-nano.onnx";

/// Community ONNX export of Roboflow's Apache-2.0 RF-DETR Nano COCO weights.
/// The hash below is pinned: a different file is refused rather than run.
pub const MODEL_URL: &str =
    "https://huggingface.co/open-noodle/rfdetr-nano/resolve/main/detection/model.onnx";
pub const MODEL_SHA256: &str =
    "21045851f3906c7d424874363fe933477fda7210bad0d7a69659fb98f8845890";
pub const MODEL_BYTES: u64 = 107_835_234;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub path: String,
    pub installed: bool,
    pub verified: bool,
    pub bytes: u64,
    pub expected_bytes: u64,
    pub expected_sha: String,
    pub url: String,
    pub loaded: bool,
    pub detail: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProgress {
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

pub fn model_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(models_dir(app)?.join(MODEL_FILE))
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

/// Stream the model to `dest`, hashing as it arrives. The partial file is deleted
/// on any mismatch, so a corrupt download can never be mistaken for a usable model.
pub fn fetch(
    url: &str,
    dest: &Path,
    expected_sha: &str,
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

    let total = response.content_length().unwrap_or(MODEL_BYTES);
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
pub fn load_now(app: &AppHandle) -> Result<String, String> {
    let path = model_path(app)?;
    if !path.exists() {
        return Err("No model file yet. Download it first.".into());
    }
    let (digest, _) = sha256_file(&path)?;
    if digest != MODEL_SHA256 {
        return Err("The model file does not match the pinned hash, so it was not loaded.".into());
    }

    let detector = crate::detector::Detector::load(&path)?;
    let detail = detector.describe();
    let state = app.state::<AppState>();
    let mut guard = state.detector.lock().map_err(|e| e.to_string())?;
    *guard = Some(detector);
    Ok(detail)
}

#[tauri::command]
pub fn model_status(app: AppHandle) -> Result<ModelStatus, String> {
    let path = model_path(&app)?;
    let installed = path.exists();
    let mut bytes = 0u64;
    let mut verified = false;
    let mut detail = None;

    if installed {
        match sha256_file(&path) {
            Ok((digest, size)) => {
                bytes = size;
                verified = digest == MODEL_SHA256;
                if !verified {
                    detail = Some("This file is not the model iRecon expects.".into());
                }
            }
            Err(error) => detail = Some(error),
        }
    }

    let loaded = app
        .state::<AppState>()
        .detector
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false);

    Ok(ModelStatus {
        path: path.to_string_lossy().to_string(),
        installed,
        verified,
        bytes,
        expected_bytes: MODEL_BYTES,
        expected_sha: MODEL_SHA256.to_string(),
        url: MODEL_URL.to_string(),
        loaded,
        detail,
    })
}

#[tauri::command]
pub fn model_download(app: AppHandle) -> Result<(), String> {
    let path = model_path(&app)?;
    let handle = app.clone();

    std::thread::Builder::new()
        .name("irecon-model-download".into())
        .spawn(move || {
            let _ = handle.emit(
                "model:progress",
                ModelProgress {
                    downloaded: 0,
                    total: MODEL_BYTES,
                    phase: "downloading".into(),
                    message: "Starting the download".into(),
                },
            );

            let mut last = 0u64;
            let result = fetch(MODEL_URL, &path, MODEL_SHA256, |downloaded, total| {
                if downloaded.saturating_sub(last) >= 2 * 1024 * 1024 || downloaded == total {
                    last = downloaded;
                    let _ = handle.emit(
                        "model:progress",
                        ModelProgress {
                            downloaded,
                            total,
                            phase: "downloading".into(),
                            message: String::new(),
                        },
                    );
                }
            });

            let payload = match result {
                Ok(()) => {
                    let message = match load_now(&handle) {
                        Ok(detail) => format!("Downloaded, verified and loaded. {detail}"),
                        Err(error) => format!("Downloaded and verified, but not loaded: {error}"),
                    };
                    ModelProgress {
                        downloaded: MODEL_BYTES,
                        total: MODEL_BYTES,
                        phase: "ready".into(),
                        message,
                    }
                }
                Err(error) => ModelProgress {
                    downloaded: 0,
                    total: MODEL_BYTES,
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
pub fn model_load(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    std::thread::Builder::new()
        .name("irecon-model-load".into())
        .spawn(move || {
            let payload = match load_now(&handle) {
                Ok(message) => ModelProgress {
                    downloaded: MODEL_BYTES,
                    total: MODEL_BYTES,
                    phase: "loaded".into(),
                    message,
                },
                Err(error) => ModelProgress {
                    downloaded: 0,
                    total: MODEL_BYTES,
                    phase: "failed".into(),
                    message: error,
                },
            };
            let _ = handle.emit("model:progress", payload);
        })
        .map_err(|e| format!("Could not start the model thread: {e}"))?;
    Ok(())
}
