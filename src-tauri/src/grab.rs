//! Grabbing one frame of the screen without the app ending up in it.
//!
//! Every platform follows the same shape: take a [`ScreenSource`], ask whether
//! it already leaves this app's windows out, and fall back to hiding, proving
//! and restoring when it does not. A backend that can exclude windows natively
//! (Windows Graphics Capture with `SetWindowDisplayAffinity`, macOS
//! ScreenCaptureKit with `exceptingWindows`) implements the trait and reports
//! `excludes_own_windows() == true`; `grab` and its callers do not change.

use std::time::{Duration, Instant};

use image::RgbaImage;
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::capture::pick_monitor;

/// Share of a hidden window's rectangle that must change before it is
/// considered gone.
///
/// The rectangle is checked on its own, never the whole display: motion in
/// another window must not stand in for iRecon disappearing. Hiding an opaque
/// window rewrites the area it covered, so a genuine grab clears this by a wide
/// margin, while a clock tick or a moving cursor stays far below it.
const WINDOW_GONE_FRACTION: f32 = 0.08;

/// The part of the captured image an app window covered before it was hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WindowRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// A display the app can read pixels from.
///
/// The trait is deliberately not `Send` or `Sync`: a grab happens on the thread
/// that asked for it, and `xcap::Monitor` is not `Send` on Windows because it
/// wraps a raw `HMONITOR`. A backend that needs to cross threads wraps itself.
pub(crate) trait ScreenSource {
    /// One frame of the display, in RGBA.
    fn capture(&self) -> Result<RgbaImage, String>;

    /// Whether [`ScreenSource::capture`] already leaves this app's windows out.
    ///
    /// A composited display does not, and then the only clean frame is the hide,
    /// prove and restore path.
    fn excludes_own_windows(&self) -> bool {
        false
    }
}

/// The current backend on every platform: xcap reads the composited display, so
/// iRecon's own windows are in the picture unless they are hidden first.
struct DisplaySource {
    monitor: xcap::Monitor,
}

impl DisplaySource {
    fn primary() -> Result<Self, String> {
        Ok(Self {
            monitor: pick_monitor()?,
        })
    }
}

impl ScreenSource for DisplaySource {
    fn capture(&self) -> Result<RgbaImage, String> {
        self.monitor
            .capture_image()
            .map_err(|error| format!("Could not capture the screen: {error}"))
    }
}

/// Picks the best source this platform offers.
///
/// The Windows Graphics Capture backend and the ScreenCaptureKit backend plug in
/// here, each returning a source whose `excludes_own_windows` is true.
fn primary_source() -> Result<Box<dyn ScreenSource>, String> {
    Ok(Box::new(DisplaySource::primary()?))
}

/// How a source can produce a frame that does not contain the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrabPlan {
    /// The source leaves the app out already; capture and return.
    Direct,
    /// The display includes the app; hide the windows, capture, prove, restore.
    HideFirst,
}

pub(crate) fn plan_for(source: &dyn ScreenSource) -> GrabPlan {
    if source.excludes_own_windows() {
        GrabPlan::Direct
    } else {
        GrabPlan::HideFirst
    }
}

/// Grabs one frame with iRecon's own windows left out, however this platform can
/// manage it. The caller gets a frame, or an error that says why there is none.
pub(crate) fn grab(app: &AppHandle) -> Result<RgbaImage, String> {
    let source = primary_source()?;
    grab_with(app, source.as_ref())
}

pub(crate) fn grab_with(app: &AppHandle, source: &dyn ScreenSource) -> Result<RgbaImage, String> {
    match plan_for(source) {
        GrabPlan::Direct => source.capture(),
        GrabPlan::HideFirst => grab_with_hidden_windows(app, source),
    }
}

/// Hides every iRecon window, grabs the screen underneath, and restores the
/// windows that were visible before.
///
/// The window's own visibility flag is not proof that the display no longer
/// shows it: the screen can still hand back a frame composited before the hide,
/// which is how the app ended up inside its own training frames. So the screen
/// is captured once before hiding, then compared after, and a frame is only
/// accepted when enough of the picture has changed to prove the window is gone.
/// A grab that never sees that change saves nothing and says why.
fn grab_with_hidden_windows(app: &AppHandle, source: &dyn ScreenSource) -> Result<RgbaImage, String> {
    let before = source.capture().ok();

    // Remember which windows were on screen, so a hidden overlay stays hidden
    // and the main window is the one that gets focus back.
    let windows: Vec<(WebviewWindow, bool)> = app
        .webview_windows()
        .into_values()
        .map(|window| {
            let was_visible = window.is_visible().unwrap_or(false);
            (window, was_visible)
        })
        .collect();

    let regions = before
        .as_ref()
        .map(|image| window_regions(&windows, image))
        .unwrap_or_default();

    let shown: Vec<&WebviewWindow> = windows
        .iter()
        .filter(|(_, was_visible)| *was_visible)
        .map(|(window, _)| window)
        .collect();

    for window in &shown {
        if let Err(error) = window.hide() {
            return Err(format!("Could not hide the window for the grab: {error}"));
        }
    }

    let hidden_deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < hidden_deadline {
        if shown
            .iter()
            .all(|window| !window.is_visible().unwrap_or(true))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }

    let mut captured: Option<RgbaImage> = None;
    let mut clear = false;
    let mut last_error: Option<String> = None;
    for attempt in 0..4 {
        std::thread::sleep(Duration::from_millis(if attempt == 0 { 80 } else { 110 }));
        match source.capture() {
            Ok(image) => {
                if frame_is_clear(before.as_ref(), &image, &regions) {
                    captured = Some(image);
                    clear = true;
                    break;
                }
                captured = Some(image);
            }
            Err(error) => last_error = Some(error),
        }
    }

    for window in &shown {
        let _ = window.show();
    }
    if let Some((window, _)) = windows.iter().find(|(window, _)| window.label() == "main") {
        let _ = window.set_focus();
    }

    if clear {
        return captured.ok_or_else(|| "Could not capture the screen".to_string());
    }
    Err(match (before.is_some(), last_error) {
        (false, _) => {
            "Could not read the screen before the grab, so this frame could not be checked. \
             Nothing was saved; press Grab frame again."
                .to_string()
        }
        (true, Some(error)) => error,
        (true, None) => {
            "iRecon hid itself, but the area it covered did not change, so the frame \
             would still contain the app. Nothing was saved; close any other iRecon \
             window or browser preview and press Grab frame again."
                .to_string()
        }
    })
}

/// The app windows that were on screen, in capture-image coordinates. A window
/// on another display is clipped to nothing and cannot affect this grab.
fn window_regions(windows: &[(WebviewWindow, bool)], image: &RgbaImage) -> Vec<WindowRegion> {
    let image_width = image.width() as i64;
    let image_height = image.height() as i64;
    windows
        .iter()
        .filter(|(_, was_visible)| *was_visible)
        .filter_map(|(window, _)| {
            let position = window.outer_position().ok()?;
            let size = window.outer_size().ok()?;
            let left = (position.x as i64).clamp(0, image_width);
            let top = (position.y as i64).clamp(0, image_height);
            let right = (position.x as i64 + size.width as i64).clamp(0, image_width);
            let bottom = (position.y as i64 + size.height as i64).clamp(0, image_height);
            if right <= left || bottom <= top {
                return None;
            }
            Some(WindowRegion {
                x: left as u32,
                y: top as u32,
                width: (right - left) as u32,
                height: (bottom - top) as u32,
            })
        })
        .collect()
}

/// A frame is clear only when a reference captured while the app was visible
/// exists and every visible app window's rectangle moved well past the
/// threshold. No reference means no proof, and no proof means no frame.
fn frame_is_clear(
    before: Option<&RgbaImage>,
    after: &RgbaImage,
    regions: &[WindowRegion],
) -> bool {
    let Some(previous) = before else {
        return false;
    };
    if previous.dimensions() != after.dimensions() {
        return true;
    }
    if regions.is_empty() {
        return changed_fraction(previous, after) > WINDOW_GONE_FRACTION;
    }
    regions
        .iter()
        .all(|region| changed_fraction_in_region(previous, after, *region) > WINDOW_GONE_FRACTION)
}

/// Share of pixels in one rectangle whose colour moved enough to count as a
/// change. This is the proof that the window covering that rectangle is gone.
fn changed_fraction_in_region(
    left: &RgbaImage,
    right: &RgbaImage,
    region: WindowRegion,
) -> f32 {
    if left.dimensions() != right.dimensions() {
        return 1.0;
    }
    let x0 = region.x.min(left.width());
    let y0 = region.y.min(left.height());
    let x1 = (region.x + region.width).min(left.width());
    let y1 = (region.y + region.height).min(left.height());
    if x1 <= x0 || y1 <= y0 {
        return 1.0;
    }
    let (left_raw, right_raw) = (left.as_raw(), right.as_raw());
    let stride = left.width() as usize * 4;
    let mut changed = 0usize;
    let mut total = 0usize;
    for y in y0..y1 {
        let row = y as usize * stride;
        for x in x0..x1 {
            let index = row + x as usize * 4;
            let delta = (left_raw[index] as i32 - right_raw[index] as i32).abs()
                + (left_raw[index + 1] as i32 - right_raw[index + 1] as i32).abs()
                + (left_raw[index + 2] as i32 - right_raw[index + 2] as i32).abs();
            if delta > 24 {
                changed += 1;
            }
            total += 1;
        }
    }
    changed as f32 / total.max(1) as f32
}

/// Share of pixels whose colour moved enough to count as a change.
fn changed_fraction(left: &RgbaImage, right: &RgbaImage) -> f32 {
    if left.dimensions() != right.dimensions() {
        return 1.0;
    }
    let (left_raw, right_raw) = (left.as_raw(), right.as_raw());
    let mut changed = 0usize;
    for (a, b) in left_raw.chunks_exact(4).zip(right_raw.chunks_exact(4)) {
        let delta = (a[0] as i32 - b[0] as i32).abs()
            + (a[1] as i32 - b[1] as i32).abs()
            + (a[2] as i32 - b[2] as i32).abs();
        if delta > 24 {
            changed += 1;
        }
    }
    changed as f32 / (left.width().max(1) * left.height().max(1)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Native;

    impl ScreenSource for Native {
        fn capture(&self) -> Result<RgbaImage, String> {
            Err("not used in this test".into())
        }

        fn excludes_own_windows(&self) -> bool {
            true
        }
    }

    struct Composited;

    impl ScreenSource for Composited {
        fn capture(&self) -> Result<RgbaImage, String> {
            Err("not used in this test".into())
        }
    }

    fn canvas(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba([20, 20, 24, 255]))
    }

    fn paint(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32) {
        for py in y..y + height {
            for px in x..x + width {
                image.put_pixel(px, py, image::Rgba([240, 240, 245, 255]));
            }
        }
    }

    #[test]
    fn a_native_source_needs_no_hiding() {
        assert_eq!(plan_for(&Native), GrabPlan::Direct);
    }

    #[test]
    fn a_composited_source_hides_first() {
        assert_eq!(plan_for(&Composited), GrabPlan::HideFirst);
    }

    fn region(x: u32, y: u32, width: u32, height: u32) -> WindowRegion {
        WindowRegion {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn an_unchanged_screen_is_not_clear() {
        let before = canvas(800, 600);
        let after = before.clone();
        assert!(!frame_is_clear(Some(&before), &after, &[]));
    }

    #[test]
    fn a_cursor_sized_change_is_not_clear() {
        let before = canvas(800, 600);
        let mut after = before.clone();
        paint(&mut after, 300, 300, 12, 18);
        assert!(!frame_is_clear(Some(&before), &after, &[]));
    }

    #[test]
    fn a_window_sized_change_is_clear() {
        let before = canvas(800, 600);
        let mut after = before.clone();
        paint(&mut after, 100, 100, 480, 360);
        assert!(frame_is_clear(Some(&before), &after, &[]));
    }

    #[test]
    fn motion_outside_the_app_window_is_not_clear() {
        let before = canvas(800, 600);
        let mut after = before.clone();
        paint(&mut after, 600, 100, 100, 100);
        assert!(!frame_is_clear(
            Some(&before),
            &after,
            &[region(100, 100, 480, 360)]
        ));
    }

    #[test]
    fn the_app_window_area_changing_is_clear() {
        let before = canvas(800, 600);
        let mut after = before.clone();
        paint(&mut after, 100, 100, 480, 360);
        assert!(frame_is_clear(
            Some(&before),
            &after,
            &[region(100, 100, 480, 360)]
        ));
    }

    #[test]
    fn one_unchanged_app_window_is_not_clear() {
        let before = canvas(800, 600);
        let mut after = before.clone();
        paint(&mut after, 100, 100, 480, 360);
        assert!(!frame_is_clear(
            Some(&before),
            &after,
            &[region(100, 100, 480, 360), region(620, 100, 100, 100)]
        ));
    }

    #[test]
    fn a_grab_without_a_reference_is_refused() {
        let after = canvas(800, 600);
        assert!(!frame_is_clear(None, &after, &[]));
    }

    #[test]
    fn a_resized_capture_counts_as_fully_changed() {
        let before = canvas(800, 600);
        let after = canvas(640, 480);
        assert_eq!(changed_fraction(&before, &after), 1.0);
    }
}
