//! Generates the placeholder art for the main menu into `assets/ui/`.
//!
//! Everything here is procedural and original; the files are meant to be replaced by real
//! art with the same names, sizes and channel conventions:
//!
//! - `button_base.webp`       512x128 RGBA, straight alpha. The alpha channel is also the
//!   button's hit shape. 9-slice borders: 48 px left/right, 40 px top/bottom.
//! - `button_glow_mask.webp`  512x128 grayscale, linear. White = glows on hover.
//! - `menu_background.webp`   1920x1080 RGB, opaque.
//! - `menu_panel_shade.webp`  512x8 RGBA, black with a horizontal alpha falloff.
//!
//! Run with: `cargo run --example generate_placeholder_assets`

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use image::ExtendedColorType;
use image::codecs::webp::WebPEncoder;

const OUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ui");

const BUTTON_W: u32 = 512;
const BUTTON_H: u32 = 128;
/// Inset of the button outline from the texture edge (room for antialiasing).
const BUTTON_INSET: f32 = 6.0;
const BUTTON_RADIUS: f32 = 30.0;
const BORDER_WIDTH: f32 = 9.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = Path::new(OUT_DIR);
    std::fs::create_dir_all(out)?;

    let (base, glow) = button_textures();
    write_webp(
        &out.join("button_base.webp"),
        &base,
        BUTTON_W,
        BUTTON_H,
        ExtendedColorType::Rgba8,
    )?;
    write_webp(
        &out.join("button_glow_mask.webp"),
        &glow,
        BUTTON_W,
        BUTTON_H,
        ExtendedColorType::L8,
    )?;

    let (w, h) = (1920, 1080);
    write_webp(
        &out.join("menu_background.webp"),
        &background(w, h),
        w,
        h,
        ExtendedColorType::Rgb8,
    )?;

    let (w, h) = (512, 8);
    write_webp(
        &out.join("menu_panel_shade.webp"),
        &panel_shade(w, h),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    Ok(())
}

fn write_webp(
    path: &Path,
    data: &[u8],
    width: u32,
    height: u32,
    color: ExtendedColorType,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = BufWriter::new(File::create(path)?);
    WebPEncoder::new_lossless(file).encode(data, width, height, color)?;
    println!(
        "wrote {} ({} bytes)",
        path.display(),
        std::fs::metadata(path)?.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Button

/// Signed distance to the button outline (negative inside), in texels.
fn button_distance(x: f32, y: f32) -> f32 {
    let half_w = BUTTON_W as f32 / 2.0 - BUTTON_INSET;
    let half_h = BUTTON_H as f32 / 2.0 - BUTTON_INSET;
    let px = (x - BUTTON_W as f32 / 2.0).abs() - (half_w - BUTTON_RADIUS);
    let py = (y - BUTTON_H as f32 / 2.0).abs() - (half_h - BUTTON_RADIUS);
    let outside = (px.max(0.0).powi(2) + py.max(0.0).powi(2)).sqrt();
    outside + px.max(py).min(0.0) - BUTTON_RADIUS
}

fn button_textures() -> (Vec<u8>, Vec<u8>) {
    let mut base = Vec::with_capacity((BUTTON_W * BUTTON_H * 4) as usize);
    let mut glow = Vec::with_capacity((BUTTON_W * BUTTON_H) as usize);
    for row in 0..BUTTON_H {
        for column in 0..BUTTON_W {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let d = button_distance(x, y);
            let alpha = (0.5 - d).clamp(0.0, 1.0);
            let vertical = y / BUTTON_H as f32;
            let depth = -d; // Distance inside the outline.

            let color = if depth < BORDER_WIDTH {
                // Bronze rim: lit from above, with a thin bright bevel line.
                let top = [196.0, 146.0, 92.0];
                let bottom = [92.0, 58.0, 34.0];
                let mut c = mix3(top, bottom, smoothstep(0.1, 0.9, vertical));
                let bevel = (-((depth - 2.5) / 1.2).powi(2)).exp() * (1.0 - vertical);
                c = add3(c, [70.0 * bevel, 55.0 * bevel, 35.0 * bevel]);
                let groove = (-((depth - BORDER_WIDTH + 1.5) / 1.0).powi(2)).exp();
                scale3(c, 1.0 - 0.45 * groove)
            } else {
                // Dark, grainy leather/wood plate with an inner shadow along the rim.
                let top = [86.0, 58.0, 38.0];
                let bottom = [42.0, 28.0, 19.0];
                let mut c = mix3(top, bottom, smoothstep(0.0, 1.0, vertical));
                let grain = fbm(x / 46.0, y / 9.0, 4, 11) - 0.5;
                c = scale3(c, 1.0 + 0.35 * grain);
                let shadow = (-(depth - BORDER_WIDTH) / 7.0).exp();
                scale3(c, 1.0 - 0.55 * shadow)
            };
            base.extend_from_slice(&[
                to_u8(color[0]),
                to_u8(color[1]),
                to_u8(color[2]),
                to_u8(alpha * 255.0),
            ]);

            // Glow: strongest just inside the rim, plus a soft wash over the plate.
            let rim = (-((depth - BORDER_WIDTH - 1.0) / 5.0).powi(2)).exp();
            let center_x = 1.0 - ((x / BUTTON_W as f32) - 0.5).abs() * 2.0;
            let wash = 0.4 * smoothstep(0.0, 0.6, center_x) * smoothstep(BORDER_WIDTH, 30.0, depth);
            let mask = rim.max(wash) * alpha;
            glow.push(to_u8(mask * 255.0));
        }
    }
    (base, glow)
}

// ---------------------------------------------------------------------------------------
// Background: a stormy dusk landscape built from noise layers.

fn background(width: u32, height: u32) -> Vec<u8> {
    let aspect = width as f32 / height as f32;
    let sun = [0.64, 0.33];
    let mut out = Vec::with_capacity((width * height * 3) as usize);
    for row in 0..height {
        for column in 0..width {
            let u = (column as f32 + 0.5) / width as f32;
            let v = (row as f32 + 0.5) / height as f32;

            // Sky gradient, warm haze near the horizon.
            let mut c = mix3(
                [16.0, 20.0, 28.0],
                [58.0, 66.0, 74.0],
                smoothstep(0.0, 0.45, v),
            );
            c = mix3(c, [168.0, 156.0, 128.0], smoothstep(0.38, 0.62, v));

            // Sun glow behind the clouds.
            let dx = (u - sun[0]) * aspect;
            let dy = v - sun[1];
            let sun_dist2 = dx * dx + dy * dy;
            let sun_glow = (-sun_dist2 / 0.03).exp();
            c = add3(c, scale3([255.0, 226.0, 170.0], 0.75 * sun_glow));

            // Clouds: warped fbm, denser towards the top, rims lit near the sun.
            let warp = fbm(u * 2.0 * aspect, v * 2.0, 3, 7);
            let n = fbm(u * 3.0 * aspect + 1.7 * warp, v * 5.5 + warp, 5, 3);
            let cover = 1.0 - smoothstep(0.25, 0.62, v);
            let density = smoothstep(0.42, 0.72, n) * cover;
            let lit = (-sun_dist2 / 0.09).exp() * (1.0 - smoothstep(0.55, 0.9, n));
            let cloud = add3([30.0, 33.0, 40.0], scale3([200.0, 182.0, 140.0], lit));
            c = mix3(c, cloud, 0.88 * density);

            // Hills: three ridge layers with atmospheric perspective.
            let ridges = [
                (0.60, 0.045, 3.0, 21, [96.0, 104.0, 108.0]),
                (0.67, 0.065, 5.0, 22, [52.0, 62.0, 56.0]),
                (0.79, 0.070, 8.0, 23, [22.0, 29.0, 22.0]),
            ];
            for (base_height, amplitude, frequency, seed, color) in ridges {
                let ridge = base_height - amplitude * fbm(u * frequency * aspect, 0.5, 4, seed);
                let coverage = smoothstep(ridge - 0.0015, ridge + 0.0015, v);
                let shade = 1.0 - 0.35 * smoothstep(ridge, ridge + 0.25, v);
                c = mix3(c, scale3(color, shade), coverage);
            }

            // Ground texture and a mist band at the horizon.
            let ground = smoothstep(0.78, 0.82, v);
            let grass = fbm(u * 60.0 * aspect, v * 140.0, 3, 31) - 0.5;
            c = scale3(c, 1.0 + 0.4 * grass * ground);
            let mist = (-((v - 0.645) / 0.03).powi(2)).exp() * 0.45;
            c = mix3(c, [150.0, 152.0, 146.0], mist);

            // Vignette.
            let vx = u - 0.5;
            let vy = v - 0.5;
            let vignette = 1.0 - 0.55 * smoothstep(0.25, 0.75, (vx * vx + vy * vy).sqrt());
            c = scale3(c, vignette);

            out.extend_from_slice(&[to_u8(c[0]), to_u8(c[1]), to_u8(c[2])]);
        }
    }
    out
}

/// Black with alpha fading from dense on the left to clear on the right.
fn panel_shade(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for _row in 0..height {
        for column in 0..width {
            let u = (column as f32 + 0.5) / width as f32;
            let alpha = 0.93 * (1.0 - smoothstep(0.45, 1.0, u));
            out.extend_from_slice(&[0, 0, 0, to_u8(alpha * 255.0)]);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Helpers

fn to_u8(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale3(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d)
        ^ (y as u32).wrapping_mul(0x1656_67b1)
        ^ seed.wrapping_mul(0x9e37_79b9);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let a = hash(ix, iy, seed);
    let b = hash(ix + 1, iy, seed);
    let c = hash(ix, iy + 1, seed);
    let d = hash(ix + 1, iy + 1, seed);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

/// Fractal value noise in roughly `[0, 1]`.
fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut frequency, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for octave in 0..octaves {
        sum += amplitude * value_noise(x * frequency, y * frequency, seed + octave);
        norm += amplitude;
        amplitude *= 0.5;
        frequency *= 2.03;
    }
    sum / norm
}
