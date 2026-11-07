use std::sync::{Arc, Mutex};

use tauri::Manager;

mod capture;
mod db;
mod overlay;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub capture: Mutex<Option<capture::CaptureHandle>>,
    pub stats: Arc<Mutex<Option<capture::CaptureStats>>>,
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
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::projects_list,
            db::project_create,
            db::project_delete,
            capture::capture_start,
            capture::capture_stop,
            capture::capture_status,
            overlay::overlay_show,
            overlay::overlay_hide,
        ])
        .run(tauri::generate_context!())
        .expect("iRecon failed to start");
}
