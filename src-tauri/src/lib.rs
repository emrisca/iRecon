use std::sync::{Arc, Mutex};

use tauri::Manager;

pub mod capture;
pub mod db;
pub mod detector;
pub mod label;
pub mod model;
pub mod overlay;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub capture: Mutex<Option<capture::CaptureHandle>>,
    pub stats: Arc<Mutex<Option<capture::CaptureStats>>>,
    pub detector: Arc<Mutex<Option<detector::Detector>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let conn = db::open(&data_dir.join("irecon.sqlite"))?;
            app.manage(AppState {
                db: Mutex::new(conn),
                capture: Mutex::new(None),
                stats: Arc::new(Mutex::new(None)),
                detector: Arc::new(Mutex::new(None)),
            });

            // Load a verified model in the background so the window opens immediately.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let present = model::model_path(&handle)
                    .map(|path| path.exists())
                    .unwrap_or(false);
                if present {
                    let _ = model::load_now(&handle);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::projects_list,
            db::project_create,
            db::project_delete,
            label::frame_grab,
            label::classes_list,
            label::class_create,
            label::class_delete,
            label::example_create,
            label::examples_list,
            model::model_status,
            model::model_download,
            model::model_load,
            capture::capture_start,
            capture::capture_stop,
            capture::capture_status,
            overlay::overlay_show,
            overlay::overlay_hide,
        ])
        .run(tauri::generate_context!())
        .expect("iRecon failed to start");
}
