use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn overlay_show(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or("The overlay window is missing")?;
    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("No display found")?;

    let position = monitor.position();
    let size = monitor.size();

    window
        .set_position(tauri::PhysicalPosition::new(position.x, position.y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(tauri::PhysicalSize::new(size.width, size.height))
        .map_err(|e| e.to_string())?;
    window.set_always_on_top(true).map_err(|e| e.to_string())?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn overlay_hide(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or("The overlay window is missing")?;
    window.hide().map_err(|e| e.to_string())
}
