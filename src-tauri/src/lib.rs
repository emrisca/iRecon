use std::sync::{Arc, Mutex};

use tauri::Manager;

pub mod advice;
pub mod agent;
pub mod capture;
pub mod dataset;
pub mod db;
pub mod detector;
pub mod eval;
pub mod grab;
pub mod label;
pub mod library;
pub mod matcher;
pub mod model;
pub mod overlay;
pub mod review;
pub mod source;
pub mod tracker;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub capture: Mutex<Option<capture::CaptureHandle>>,
    pub stats: Arc<Mutex<Option<capture::CaptureStats>>>,
    pub detector: Arc<Mutex<Option<detector::Detector>>>,
    pub loaded_model: Arc<Mutex<Option<String>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let conn = db::open(&data_dir.join("irecon.sqlite"))?;
            app.manage(AppState {
                db: Mutex::new(conn),
                capture: Mutex::new(None),
                stats: Arc::new(Mutex::new(None)),
                detector: Arc::new(Mutex::new(None)),
                loaded_model: Arc::new(Mutex::new(None)),
            });

            // Load a verified model in the background so the window opens immediately.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Some(entry) = model::preferred(&handle) {
                    let _ = model::load_now(&handle, entry);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::projects_list,
            db::project_create,
            db::project_delete,
            advice::class_advice,
            label::frame_grab,
            label::classes_list,
            label::class_create,
            label::class_update,
            label::class_threshold,
            label::class_delete,
            label::example_create,
            label::examples_list,
            label::match_region,
            label::examples_embed,
            library::library_list,
            library::library_frame,
            library::library_import,
            library::library_import_folder,
            source::displays_list,
            source::windows_list,
            source::sources_list,
            source::source_add,
            source::source_crop,
            source::source_delete,
            source::source_preview,
            source::source_frame,
            review::candidates_list,
            review::candidate_nearest,
            review::candidate_resolve,
            eval::evaluate_frames,
            dataset::dataset_export,
            agent::agent_snapshot,
            agent::agent_act,
            model::model_catalog,
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
