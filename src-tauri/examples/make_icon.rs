//! Draw the app icon: a silver rounded square with a black crosshair.
//!
//! cargo run --example make_icon
//! npx tauri icon src-tauri/icons/source.png
//!
//! Everything is drawn at 4x and downsampled, so the ring and the ticks have
//! clean edges without a graphics dependency.

use image::{Rgba, RgbaImage};

const SIZE: u32 = 1024;
const SUPERSAMPLE: u32 = 4;

const BACKGROUND: Rgba<u8> = Rgba([0xeb, 0xec, 0xee, 0xff]);
const INK: Rgba<u8> = Rgba([0x11, 0x12, 0x14, 0xff]);

fn main() -> Result<(), String> {
    let scale = SUPERSAMPLE as f32;
    let canvas_size = SIZE * SUPERSAMPLE;
    let edge = canvas_size as f32;
    let corner = 232.0 * scale;

    let center = edge / 2.0;
    let ring = 296.0 * scale;
    let stroke = 35.0 * scale;
    let tick_in = 118.0 * scale;
    let tick_out = 28.0 * scale;
    let tick_half = stroke / 2.0;

    let mut canvas = RgbaImage::new(canvas_size, canvas_size);

    for y in 0..canvas_size {
        for x in 0..canvas_size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !inside_rounded_rect(px, py, edge, corner) {
                continue;
            }

            let dx = px - center;
            let dy = py - center;
            let distance = (dx * dx + dy * dy).sqrt();

            let on_ring = (distance - ring).abs() <= stroke / 2.0;
            let on_tick = tick(dx, dy, ring, tick_in, tick_out, tick_half);

            canvas.put_pixel(x, y, if on_ring || on_tick { INK } else { BACKGROUND });
        }
    }

    let icon = image::imageops::resize(
        &canvas,
        SIZE,
        SIZE,
        image::imageops::FilterType::Lanczos3,
    );
    let path = std::path::Path::new("icons/source.png");
    icon.save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    println!("wrote {} at {SIZE} px", path.display());
    Ok(())
}

fn inside_rounded_rect(px: f32, py: f32, size: f32, corner: f32) -> bool {
    let half = size / 2.0 - corner;
    let qx = ((px - size / 2.0).abs() - half).max(0.0);
    let qy = ((py - size / 2.0).abs() - half).max(0.0);
    (qx * qx + qy * qy).sqrt() <= corner
}

/// Four ticks at the cardinal points, crossing the ring and reaching inward.
fn tick(dx: f32, dy: f32, ring: f32, inner: f32, outer: f32, half: f32) -> bool {
    let near = ring - inner;
    let far = ring + outer;

    let top = dx.abs() <= half && dy <= -near && dy >= -far;
    let bottom = dx.abs() <= half && dy >= near && dy <= far;
    let left = dy.abs() <= half && dx <= -near && dx >= -far;
    let right = dy.abs() <= half && dx >= near && dx <= far;

    top || bottom || left || right
}
