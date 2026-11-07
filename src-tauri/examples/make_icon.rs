//! Draw the app icon: a silver alloy rounded square with a black crosshair.
//!
//! cargo run --example make_icon
//! npx tauri icon src-tauri/icons/source.png
//!
//! The metal is a short set of tonal stops with a specular band and a darker
//! lower edge, so it reads as a sheet of alloy rather than a flat fill. All of
//! it is drawn at 4x and downsampled, which keeps the ring and ticks clean.

use image::{Rgba, RgbaImage};

const SIZE: u32 = 1024;
const SUPERSAMPLE: u32 = 4;

const INK: Rgba<u8> = Rgba([0x12, 0x13, 0x15, 0xff]);

/// Position and colour of the alloy. Bright at the top, a cool sheen through the
/// middle, then a darker floor where the light falls off.
const STOPS: [(f32, [f32; 3]); 5] = [
    (0.00, [247.0, 249.0, 251.0]),
    (0.22, [232.0, 235.0, 239.0]),
    (0.46, [241.0, 243.0, 245.0]),
    (0.72, [198.0, 203.0, 209.0]),
    (1.00, [158.0, 164.0, 172.0]),
];

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

            canvas.put_pixel(
                x,
                y,
                if on_ring || on_tick {
                    INK
                } else {
                    metal(px, py, edge)
                },
            );
        }
    }

    let icon = image::imageops::resize(&canvas, SIZE, SIZE, image::imageops::FilterType::Lanczos3);
    let path = std::path::Path::new("icons/source.png");
    icon.save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    println!("wrote {} at {SIZE} px", path.display());
    Ok(())
}

/// Vertical tonal ramp, angled a few degrees so it reads as a sheet of metal,
/// with a specular band high up and a darker edge along the bottom.
fn metal(px: f32, py: f32, size: f32) -> Rgba<u8> {
    let tx = px / size;
    let ty = py / size;

    let t = (0.82 * ty + 0.18 * tx).clamp(0.0, 1.0);
    let color = sample(t);

    let sheen = (1.0 - ((ty - 0.20).abs() / 0.13)).max(0.0);
    let shade = (1.0 - ((ty - 0.95).abs() / 0.16)).max(0.0);
    let lift = sheen * 26.0 - shade * 24.0;

    let mut out = [0u8; 4];
    for channel in 0..3 {
        out[channel] = (color[channel] + lift).clamp(0.0, 255.0).round() as u8;
    }
    out[3] = 255;
    Rgba(out)
}

fn sample(t: f32) -> [f32; 3] {
    let mut lower = STOPS[0];
    let mut upper = STOPS[STOPS.len() - 1];
    for pair in STOPS.windows(2) {
        if t >= pair[0].0 && t <= pair[1].0 {
            lower = pair[0];
            upper = pair[1];
            break;
        }
    }

    let span = (upper.0 - lower.0).max(1e-4);
    let k = ((t - lower.0) / span).clamp(0.0, 1.0);
    [
        lower.1[0] + (upper.1[0] - lower.1[0]) * k,
        lower.1[1] + (upper.1[1] - lower.1[1]) * k,
        lower.1[2] + (upper.1[2] - lower.1[2]) * k,
    ]
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
