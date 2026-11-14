//! Generates placeholder menu art into `assets/ui/`.
//!
//! Everything here is procedural and original; the files are meant to be replaced by real
//! art with the same names, sizes and channel conventions:
//!
//! - `menu_background.webp`   1920x1080 RGB, opaque.
//! - `menu_panel_shade.webp`  512x8 RGBA, black with a horizontal alpha falloff.
//! - `slider_knob.webp`       64x64 RGBA: metal diamond frame around a teal gem.
//! - `slider_knob_glow.webp`  64x64 grayscale, linear: the gem and a soft halo.
//! - `ornament_side.webp`     256x24 RGBA: gold line ending in a diamond on the left
//!   (drawn mirrored on the right side of titles).
//! - `ornament_divider.webp`  512x26 RGBA: gold line with a center diamond.
//! - `brush_underline.webp`   512x24 RGBA: orange brush stroke fading to the right.
//! - `selector_arrow.webp`    32x48 RGBA: bronze chevron pointing left (mirrored for the
//!   right arrow).
//!
//! The button art (`button_base.webp`, `button_glow_mask.webp`) is hand-made and is not
//! touched by this tool.
//!
//! Run with: `cargo run --example generate_placeholder_assets`

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use image::ExtendedColorType;
use image::codecs::webp::WebPEncoder;

const OUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ui");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = Path::new(OUT_DIR);
    std::fs::create_dir_all(out)?;

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

    let size = 64;
    let (knob, knob_glow) = slider_knob(size);
    write_webp(
        &out.join("slider_knob.webp"),
        &knob,
        size,
        size,
        ExtendedColorType::Rgba8,
    )?;
    write_webp(
        &out.join("slider_knob_glow.webp"),
        &knob_glow,
        size,
        size,
        ExtendedColorType::L8,
    )?;

    let (w, h) = (256, 24);
    write_webp(
        &out.join("ornament_side.webp"),
        &ornament_side(w, h),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    let (w, h) = (512, 26);
    write_webp(
        &out.join("ornament_divider.webp"),
        &ornament_divider(w, h),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    let (w, h) = (512, 24);
    write_webp(
        &out.join("brush_underline.webp"),
        &brush_underline(w, h),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    let (w, h) = (32, 48);
    write_webp(
        &out.join("selector_arrow.webp"),
        &selector_arrow(w, h),
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
// Small UI ornaments. Shapes use signed distances in pixels (negative inside) and an
// antialiased coverage of one pixel.

const GOLD: [f32; 3] = [215.0, 193.0, 163.0];

fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

/// Distance to a diamond outline of "radius" `r` (center to corner) and width `width`.
fn diamond_outline(dx: f32, dy: f32, r: f32, width: f32) -> f32 {
    ((dx.abs() + dy.abs() - r) / std::f32::consts::SQRT_2).abs() - width / 2.0
}

/// Distance to a filled diamond of "radius" `r`.
fn diamond_fill(dx: f32, dy: f32, r: f32) -> f32 {
    (dx.abs() + dy.abs() - r) / std::f32::consts::SQRT_2
}

fn push_rgba(out: &mut Vec<u8>, color: [f32; 3], alpha: f32) {
    out.extend_from_slice(&[
        to_u8(color[0]),
        to_u8(color[1]),
        to_u8(color[2]),
        to_u8(alpha * 255.0),
    ]);
}

fn slider_knob(size: u32) -> (Vec<u8>, Vec<u8>) {
    let center = size as f32 / 2.0;
    let outer = center - 2.0;
    let gem = outer * 0.52;
    let mut base = Vec::with_capacity((size * size * 4) as usize);
    let mut glow = Vec::with_capacity((size * size) as usize);
    for row in 0..size {
        for column in 0..size {
            let dx = column as f32 + 0.5 - center;
            let dy = row as f32 + 0.5 - center;
            let alpha = coverage(diamond_fill(dx, dy, outer));
            // Metal frame lit from the top-left, with a bright bevel along the edge.
            let light = (0.5 - (dx + dy) / (4.0 * outer)).clamp(0.0, 1.0);
            let mut metal = mix3([52.0, 48.0, 40.0], [150.0, 140.0, 118.0], light);
            let bevel = coverage(diamond_outline(dx, dy, outer - 2.5, 1.2));
            metal = mix3(metal, [190.0, 180.0, 160.0], 0.6 * bevel * light);
            let rim = coverage(diamond_outline(dx, dy, gem + 1.5, 1.6));
            metal = mix3(metal, [25.0, 22.0, 18.0], rim);
            // Faceted teal gem: the upper-left facets catch the light.
            let gem_cover = coverage(diamond_fill(dx, dy, gem));
            let facet = if dx + dy < 0.0 { 1.0 } else { 0.62 } * if dx < dy { 1.0 } else { 0.85 };
            let gem_color = mix3([20.0, 110.0, 112.0], [150.0, 255.0, 250.0], facet);
            push_rgba(&mut base, mix3(metal, gem_color, gem_cover), alpha);

            let halo = (-diamond_fill(dx, dy, gem).max(0.0) / 5.0).exp() * 0.55;
            glow.push(to_u8(gem_cover.max(halo) * 255.0));
        }
    }
    (base, glow)
}

fn ornament_side(width: u32, height: u32) -> Vec<u8> {
    let cy = height as f32 / 2.0;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for column in 0..width {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let diamond = diamond_outline(x - 14.0, y - cy, 8.0, 1.6);
            let dot = ((x - 30.0).powi(2) + (y - cy).powi(2)).sqrt() - 2.0;
            let end_dot = ((x - 250.0).powi(2) + (y - cy).powi(2)).sqrt() - 1.6;
            let line = if (38.0..246.0).contains(&x) {
                (y - cy).abs() - 0.8
            } else {
                f32::MAX
            };
            let d = diamond.min(dot).min(end_dot).min(line);
            push_rgba(&mut out, GOLD, coverage(d));
        }
    }
    out
}

fn ornament_divider(width: u32, height: u32) -> Vec<u8> {
    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for column in 0..width {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let (dx, dy) = (x - cx, y - cy);
            let outline = diamond_outline(dx, dy, 10.0, 1.6);
            let core = diamond_fill(dx, dy, 4.0);
            let rings = [-36.0, 36.0]
                .map(|offset| (((dx - offset).powi(2) + dy.powi(2)).sqrt() - 3.5).abs() - 0.7);
            let gap = dx.abs() > 16.0 && (dx.abs() - 36.0).abs() > 5.0;
            let line = if gap { dy.abs() - 0.75 } else { f32::MAX };
            let d = outline.min(core).min(rings[0]).min(rings[1]).min(line);
            // Fade the line towards both ends.
            let edge = 1.0 - (dx.abs() / cx);
            let fade = smoothstep(0.0, 0.25, edge);
            push_rgba(&mut out, GOLD, coverage(d) * fade);
        }
    }
    out
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1])).clamp(0.0, 1.0);
    let closest = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - closest[0]).powi(2) + (p[1] - closest[1]).powi(2)).sqrt()
}

/// Bronze chevron pointing left, with a dark rim and a small barb at the tip.
fn selector_arrow(width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as f32, height as f32);
    let tip = [6.0, h / 2.0];
    let ends = [[w - 7.0, 6.0], [w - 7.0, h - 6.0]];
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for column in 0..width {
            let p = [column as f32 + 0.5, row as f32 + 0.5];
            let arms = segment_distance(p, tip, ends[0]).min(segment_distance(p, tip, ends[1]));
            let barb = segment_distance(p, [tip[0] - 2.0, tip[1]], [tip[0] + 6.0, tip[1]]);
            let d = arms.min(barb + 0.5) - 3.2;
            let rim = coverage(d);
            let core = coverage(d + 1.4);
            let light = 1.0 - p[1] / h;
            let bronze = mix3([92.0, 60.0, 30.0], [168.0, 118.0, 62.0], light);
            let color = mix3([30.0, 20.0, 10.0], bronze, core);
            push_rgba(&mut out, color, rim);
        }
    }
    out
}

fn brush_underline(width: u32, height: u32) -> Vec<u8> {
    let cy = height as f32 / 2.0;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for column in 0..width {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let u = x / width as f32;
            // Stroke tapers from left to right; ragged edges and bristle streaks.
            let half = 1.0 + 5.0 * (1.0 - u).powf(0.7);
            let wobble = (fbm(x / 40.0, 3.0, 3, 41) - 0.5) * 3.0;
            let edge_noise = (fbm(x / 6.0, y / 3.0, 2, 43) - 0.5) * 2.5;
            let d = (y - cy - wobble).abs() - half - edge_noise;
            let streaks = 0.7 + 0.3 * fbm(x / 9.0, y * 1.7, 2, 47);
            let fade = 1.0 - smoothstep(0.55, 1.0, u);
            let start = smoothstep(0.0, 0.02, u);
            let alpha = coverage(d) * streaks * fade * start;
            let tone = 0.85 + 0.3 * fbm(x / 20.0, y / 4.0, 2, 53);
            push_rgba(&mut out, scale3([196.0, 106.0, 44.0], tone), alpha);
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
