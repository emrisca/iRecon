use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

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
pub mod project_detector;
pub mod review;
pub mod roboflow;
pub mod session;
pub mod soak;
pub mod source;
pub mod tracker;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    /// The app-wide grab shortcut that is currently registered, and its label.
    pub grab_shortcut: Mutex<Option<(String, Shortcut)>>,
    pub capture: Mutex<Option<capture::CaptureHandle>>,
    pub stats: Arc<Mutex<Option<capture::CaptureStats>>>,
    pub detector: Arc<Mutex<Option<detector::Detector>>>,
    pub loaded_model: Arc<Mutex<Option<String>>>,
    /// True while a screen in the app is showing frames, so the capture thread
    /// does not encode a preview nobody looks at.
    pub preview_watch: Arc<AtomicBool>,
    /// True while the transparent overlay is on screen. Detection events are
    /// only emitted for a visible consumer.
    pub overlay_visible: Arc<AtomicBool>,
    /// True while the overlay draws the sine-wave calibration pattern instead of
    /// detections, which is how the paint clock is measured on its own.
    pub calibration: Arc<AtomicBool>,
    /// The one frame the capture thread and the inference thread agree on.
    pub frame_slot: Arc<capture::FrameSlot>,
    /// Numbers the inference thread reports to the capture thread's status line.
    pub board: Arc<capture::StatsBoard>,
    /// The newest paint measurement the overlay reported.
    pub paint: Arc<Mutex<Option<overlay::PaintStats>>>,
    /// When set, every paint report is appended here as CSV so a measurement run
    /// can be read back without anybody watching the screen.
    pub paint_log: Option<std::path::PathBuf>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let conn = db::open(&data_dir.join("irecon.sqlite"))?;
            app.manage(AppState {
                db: Mutex::new(conn),
                grab_shortcut: Mutex::new(None),
                capture: Mutex::new(None),
                stats: Arc::new(Mutex::new(None)),
                detector: Arc::new(Mutex::new(None)),
                loaded_model: Arc::new(Mutex::new(None)),
                preview_watch: Arc::new(AtomicBool::new(false)),
                overlay_visible: Arc::new(AtomicBool::new(false)),
                calibration: Arc::new(AtomicBool::new(false)),
                frame_slot: Arc::new(capture::FrameSlot::default()),
                board: Arc::new(capture::StatsBoard::default()),
                paint: Arc::new(Mutex::new(None)),
                paint_log: std::env::var("IRECON_PAINT_LOG")
                    .ok()
                    .map(std::path::PathBuf::from),
            });

            // Ctrl+Alt+O toggles the overlay from anywhere, including over a
            // focused game. A machine that refuses the binding still gets the
            // button in the capture lab, so this failure is logged, not fatal.
            let hotkey = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyO);
            if let Err(error) = app.global_shortcut().on_shortcut(
                hotkey,
                move |app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        let _ = overlay::toggle(app);
                    }
                },
            ) {
                eprintln!("iRecon could not register {}: {error}", overlay::HOTKEY);
            }

            // The Label shortcut is registered with the desktop as well, so it
            // still grabs a frame while iRecon sits behind another window.
            label::register_saved_grab_shortcut(app.handle());
            library::migrate_thumbnails(app.handle());

            // G4 measurement mode: show the calibration overlay by itself, with
            // no capture running, and optionally exit after a fixed window. Both
            // switches are environment variables so the shipped app behaves
            // normally unless a harness asks otherwise.
            if std::env::var("IRECON_PAINT_CALIBRATION").as_deref() == Ok("1") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    if let Err(error) = overlay::show(&handle) {
                        eprintln!("iRecon could not show the overlay for measurement: {error}");
                    }
                    // Wait for the overlay to report a painted frame before switching
                    // modes. The calibration event is useless if it lands before the
                    // page has registered its listeners, and a fixed delay only makes
                    // that race less likely: one run drew twenty boxes, the next drew
                    // none on the same code.
                    let state = handle.state::<AppState>();
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    while std::time::Instant::now() < deadline {
                        let painted = state
                            .paint
                            .lock()
                            .map(|paint| paint.is_some())
                            .unwrap_or(false);
                        if painted {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    state.calibration.store(true, Ordering::Relaxed);
                    let _ = handle.emit("spike:calibration", true);
                });
                if let Some(seconds) = std::env::var("IRECON_PAINT_SECONDS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                {
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(seconds));
                        handle.exit(0);
                    });
                }
            }

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
            db::project_rename,
            db::project_delete,
            db::project_match_floor,
            db::project_set_match_floor,
            db::setting_get,
            db::setting_set,
            advice::class_advice,
            label::frame_grab,
            label::grab_settings,
            label::grab_target_set,
            label::grab_shortcut_set,
            label::example_delete,
            label::classes_list,
            label::class_create,
            label::class_update,
            label::class_threshold,
            label::class_delete,
            label::classes_cleanup,
            label::classes_merge,
            label::example_create,
            label::examples_list,
            label::match_region,
            label::examples_embed,
            library::library_list,
            library::library_list_since,
            library::library_frame,
            library::library_frame_delete,
            library::library_delete_labeled,
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
            review::candidate_preview,
            review::candidate_nearest,
            review::candidate_auto_accept,
            review::candidate_resolve,
            review::candidate_delete,
            review::candidate_delete_all,
            session::session_list,
            session::session_frames,
            session::session_frame_image,
            session::session_evaluate,
            session::session_audit,
            session::session_delete,
            project_detector::detector_import,
            project_detector::detector_list,
            project_detector::detector_activate,
            project_detector::detector_deactivate,
            project_detector::detector_delete,
            project_detector::detector_compare,
            eval::evaluate_frames,
            eval::frame_truth_list,
            eval::frame_truth_set,
            dataset::dataset_export,
            roboflow::dataset_import_roboflow,
            roboflow::dataset_import_cancel,
            agent::agent_snapshot,
            agent::agent_act,
            model::model_catalog,
            model::model_download,
            model::model_load,
            capture::capture_start,
            capture::capture_stop,
            capture::capture_status,
            capture::capture_benchmark,
            capture::capture_watch,
            overlay::overlay_show,
            overlay::overlay_hide,
            overlay::overlay_toggle,
            overlay::overlay_state,
            overlay::overlay_calibration,
            overlay::overlay_paint,
            overlay::overlay_smoothing,
        ])
        .run(tauri::generate_context!())
        .expect("iRecon failed to start");
}
