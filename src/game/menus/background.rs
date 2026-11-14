//! The menu background: the painting `assets/ui/menu_background.png`, brought to life
//! with a few light touches drawn over it:
//! - a slow "breathing" camera: the view zooms in towards the sun by a few percent and
//!   back, so the picture never sits completely still;
//! - the light around the sun swells and fades;
//! - fireflies drift and blink over the meadow.
//!
//! Positions (the sun, the meadow) are fractions of the painting (`u` right, `v` down)
//! and follow the zoom, so the effects stay on their spots at any aspect ratio.
//! The soft dots of light are generated here, not loaded.

use std::f32::consts::TAU;
use std::path::Path;

use super::widgets;
use crate::engine::assets::RgbaImage;
use crate::engine::render::GpuContext;
use crate::engine::render::ui_renderer::{Sprite, SpriteMaterial, TextureSetId, UiRenderer};
use crate::engine::ui::geometry::{Rect, cover_uv};

/// The sun in the painting.
const SUN: [f32; 2] = [0.658, 0.265];
/// One zoom cycle (in and back out), seconds.
const ZOOM_PERIOD: f32 = 80.0;
/// The deepest zoom: the view narrows by this fraction.
const ZOOM_DEPTH: f32 = 0.04;

/// The sun's light: one swell every this many seconds, opacity between these values.
const GLOW_PERIOD: f32 = 7.0;
const GLOW_OPACITY: [f32; 2] = [0.08, 0.2];
/// Diameter of the sun's light, as a fraction of the painting's height.
const GLOW_SIZE: f32 = 0.62;
const GLOW_COLOR: [u8; 3] = [255, 238, 200];

const FIREFLY_COUNT: u32 = 16;
/// Fireflies wander around homes inside this part of the painting: u0, v0, u1, v1. It
/// is the lit meadow right of the menu buttons and left of the stream.
const MEADOW: [f32; 4] = [0.30, 0.66, 0.68, 0.90];
/// How far a firefly strays from its home (fraction of the painting).
const WANDER: [f32; 2] = [0.022, 0.016];
/// Sprite size range, as a fraction of the screen height (the core is much smaller).
const FIREFLY_SIZE: [f32; 2] = [0.013, 0.021];
const FIREFLY_COLOR: [u8; 3] = [255, 240, 170];

/// Generated light sprites: side length in texels.
const DOT_SIZE: u32 = 64;

pub struct MenuBackground {
    painting: TextureSetId,
    painting_size: [f32; 2],
    firefly: TextureSetId,
    glow: TextureSetId,
    /// Seconds since the menu opened. `f64` so phases stay precise in long sessions.
    time: f64,
}

impl MenuBackground {
    pub fn load(gpu: &GpuContext, renderer: &mut UiRenderer, ui_dir: &Path) -> Self {
        let (device, queue) = (gpu.device(), gpu.queue());
        let image = widgets::load_color(
            &ui_dir.join("menu_background.png"),
            [1, 1],
            [24, 26, 32, 255],
        );
        let painting_size = [image.width as f32, image.height as f32];
        let painting = renderer.create_texture_set(device, queue, "menu background", &image, None);
        let firefly = renderer.create_texture_set(
            device,
            queue,
            "menu firefly",
            &light_dot(DOT_SIZE, firefly_alpha),
            None,
        );
        let glow = renderer.create_texture_set(
            device,
            queue,
            "menu sun glow",
            &light_dot(DOT_SIZE, glow_alpha),
            None,
        );
        Self {
            painting,
            painting_size,
            firefly,
            glow,
            time: 0.0,
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.time += f64::from(dt.max(0.0));
    }

    /// Draws the painting covering the viewport, then the light effects. `calibration`
    /// is the display's brightness/gamma material; it applies to everything here, since
    /// all of it is the game image rather than interface.
    pub fn draw(&self, sprites: &mut Vec<Sprite>, viewport: [f32; 2], calibration: SpriteMaterial) {
        let view = view_uv(self.painting_size, viewport, self.time);
        let full = Rect::new(0.0, 0.0, viewport[0], viewport[1]);
        widgets::push_image(sprites, self.painting, full, view, calibration);

        // Screen pixels per painting fraction, along each axis.
        let to_screen = |u: f32, v: f32| {
            [
                (u - view.x) / view.w * viewport[0],
                (v - view.y) / view.h * viewport[1],
            ]
        };
        let painting_height = viewport[1] / view.h;

        let glow_size = GLOW_SIZE * painting_height;
        let [x, y] = to_screen(SUN[0], SUN[1]);
        widgets::push_image(
            sprites,
            self.glow,
            Rect::new(
                x - glow_size / 2.0,
                y - glow_size / 2.0,
                glow_size,
                glow_size,
            ),
            widgets::FULL_UV,
            light_material(&calibration, GLOW_COLOR, glow_opacity(self.time)),
        );

        for index in 0..FIREFLY_COUNT {
            let fly = firefly(index, self.time);
            if fly.opacity <= 0.003 {
                continue;
            }
            let size = fly.size * viewport[1];
            let [x, y] = to_screen(fly.position[0], fly.position[1]);
            widgets::push_image(
                sprites,
                self.firefly,
                Rect::new(x - size / 2.0, y - size / 2.0, size, size),
                widgets::FULL_UV,
                light_material(&calibration, FIREFLY_COLOR, fly.opacity),
            );
        }
    }
}

/// The part of the painting on screen: the cover crop, narrowed towards the sun by the
/// breathing zoom. Always inside the cover crop (zooming about a point inside a
/// rectangle keeps the result inside it), so no edge of the painting ever shows.
fn view_uv(painting_size: [f32; 2], viewport: [f32; 2], time: f64) -> Rect {
    let cover = cover_uv(painting_size, viewport);
    let cycle = phase(time, ZOOM_PERIOD);
    // 1 at rest, eased in and out (cosine), deepest halfway through the cycle.
    let zoom = 1.0 + ZOOM_DEPTH * 0.5 * (1.0 - (cycle * TAU).cos());
    let focus = [
        SUN[0].clamp(cover.x, cover.x + cover.w),
        SUN[1].clamp(cover.y, cover.y + cover.h),
    ];
    let keep = 1.0 - 1.0 / zoom;
    Rect::new(
        cover.x + (focus[0] - cover.x) * keep,
        cover.y + (focus[1] - cover.y) * keep,
        cover.w / zoom,
        cover.h / zoom,
    )
}

/// Fraction of the current cycle, `0..1`. Computed in `f64` so long sessions do not
/// lose precision.
fn phase(time: f64, period: f32) -> f32 {
    (time / f64::from(period)).rem_euclid(1.0) as f32
}

fn glow_opacity(time: f64) -> f32 {
    let swell = 0.5 - 0.5 * (phase(time, GLOW_PERIOD) * TAU).cos();
    GLOW_OPACITY[0] + (GLOW_OPACITY[1] - GLOW_OPACITY[0]) * swell
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Firefly {
    /// Painting fractions.
    position: [f32; 2],
    /// Fraction of the screen height.
    size: f32,
    opacity: f32,
}

/// Where firefly `index` is at `time` and how brightly it glows. Each one has a fixed,
/// pseudo-random home, path and rhythm, so the swarm looks the same on every run.
fn firefly(index: u32, time: f64) -> Firefly {
    let random = |k: u32| unit_hash(index, k);
    let home = [
        MEADOW[0] + WANDER[0] + (MEADOW[2] - MEADOW[0] - 2.0 * WANDER[0]) * random(0),
        MEADOW[1] + WANDER[1] + (MEADOW[3] - MEADOW[1] - 2.0 * WANDER[1]) * random(1),
    ];
    // Two incommensurate sines per axis give a loose, never-repeating loop.
    let wander = |axis: usize, k: u32| {
        let slow = phase(time, 9.0 + 8.0 * random(k)) * TAU + TAU * random(k + 1);
        let fast = phase(time, 3.5 + 3.0 * random(k + 2)) * TAU + TAU * random(k + 3);
        WANDER[axis] * (0.7 * slow.sin() + 0.3 * fast.sin())
    };
    let position = [home[0] + wander(0, 2), home[1] + wander(1, 6)];
    // Mostly dim, with an occasional bright pulse.
    let blink = 0.5 + 0.5 * (phase(time, 4.0 + 6.0 * random(10)) * TAU + TAU * random(11)).sin();
    let opacity = 0.12 + 0.88 * blink.powi(4);
    let size = FIREFLY_SIZE[0] + (FIREFLY_SIZE[1] - FIREFLY_SIZE[0]) * random(12);
    Firefly {
        position,
        size,
        opacity,
    }
}

/// Deterministic pseudo-random value in `0..1` for (`index`, `key`).
fn unit_hash(index: u32, key: u32) -> f32 {
    let mut x = index
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(key.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(0x27D4_EB2F);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// A light sprite's material: the color, dimmed with the display calibration, at
/// `opacity`.
fn light_material(calibration: &SpriteMaterial, srgb: [u8; 3], opacity: f32) -> SpriteMaterial {
    let color = SpriteMaterial::tinted([srgb[0], srgb[1], srgb[2], 255]).tint;
    SpriteMaterial {
        tint: [
            color[0] * calibration.tint[0],
            color[1] * calibration.tint[1],
            color[2] * calibration.tint[2],
            opacity.clamp(0.0, 1.0),
        ],
        ..*calibration
    }
}

/// White square texture whose alpha is `alpha(r)`, `r` = distance from the center
/// (1 at the edge's midpoint). Straight alpha.
fn light_dot(size: u32, alpha: fn(f32) -> f32) -> RgbaImage {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    let half = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 + 0.5 - half) / half;
            let dy = (y as f32 + 0.5 - half) / half;
            let a = alpha((dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            pixels.extend_from_slice(&[255, 255, 255, (a * 255.0).round() as u8]);
        }
    }
    RgbaImage {
        width: size,
        height: size,
        pixels,
    }
}

/// Fades to nothing before the texture edge, so no square outline can show.
fn edge_fade(r: f32) -> f32 {
    1.0 - smoothstep(0.8, 1.0, r)
}

/// A bright core in a soft halo.
fn firefly_alpha(r: f32) -> f32 {
    ((-(r / 0.16).powi(2)).exp() + 0.4 * (-(r / 0.45).powi(2)).exp()) * edge_fade(r)
}

/// A broad, soft swell of light.
fn glow_alpha(r: f32) -> f32 {
    (-(r / 0.5).powi(2)).exp() * edge_fade(r)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAINTING: [f32; 2] = [1672.0, 941.0];

    fn inside(inner: Rect, outer: Rect) -> bool {
        const EPS: f32 = 1e-5;
        inner.x >= outer.x - EPS
            && inner.y >= outer.y - EPS
            && inner.x + inner.w <= outer.x + outer.w + EPS
            && inner.y + inner.h <= outer.y + outer.h + EPS
    }

    #[test]
    fn the_view_starts_at_the_cover_crop() {
        for viewport in [[1920.0, 1080.0], [2560.0, 1080.0], [1024.0, 768.0]] {
            assert_eq!(
                view_uv(PAINTING, viewport, 0.0),
                cover_uv(PAINTING, viewport)
            );
        }
    }

    #[test]
    fn the_breathing_zoom_never_shows_the_painting_edges() {
        for viewport in [
            [1920.0, 1080.0],
            [3440.0, 1440.0],
            [1280.0, 1024.0],
            [800.0, 1280.0],
        ] {
            let cover = cover_uv(PAINTING, viewport);
            for step in 0..=400 {
                let time = f64::from(step) * 0.37;
                let view = view_uv(PAINTING, viewport, time);
                assert!(inside(view, cover), "{viewport:?} at {time}: {view:?}");
                // No distortion: the view keeps the viewport's shape.
                let shape = view.w * PAINTING[0] / (view.h * PAINTING[1]);
                assert!((shape - viewport[0] / viewport[1]).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn the_zoom_is_deepest_halfway_and_returns() {
        let viewport = [1920.0, 1080.0];
        let cover = cover_uv(PAINTING, viewport);
        let half = view_uv(PAINTING, viewport, f64::from(ZOOM_PERIOD) / 2.0);
        assert!((cover.w / half.w - (1.0 + ZOOM_DEPTH)).abs() < 1e-4);
        let full = view_uv(PAINTING, viewport, f64::from(ZOOM_PERIOD));
        assert!((full.w - cover.w).abs() < 1e-5);
    }

    #[test]
    fn phases_stay_precise_after_a_long_session() {
        let day = 24.0 * 3600.0;
        let a = phase(day, GLOW_PERIOD);
        let b = phase(day + f64::from(GLOW_PERIOD), GLOW_PERIOD);
        assert!((a - b).abs() < 1e-4);
        assert!((0.0..1.0).contains(&phase(-3.0, 7.0)));
    }

    #[test]
    fn the_sun_glow_swells_between_its_limits() {
        for step in 0..200 {
            let opacity = glow_opacity(f64::from(step) * 0.11);
            assert!((GLOW_OPACITY[0] - 1e-6..=GLOW_OPACITY[1] + 1e-6).contains(&opacity));
        }
        assert!((glow_opacity(0.0) - GLOW_OPACITY[0]).abs() < 1e-6);
        let peak = glow_opacity(f64::from(GLOW_PERIOD) / 2.0);
        assert!((peak - GLOW_OPACITY[1]).abs() < 1e-5);
    }

    #[test]
    fn fireflies_stay_over_the_meadow_and_blink() {
        for index in 0..FIREFLY_COUNT {
            let (mut dimmest, mut brightest) = (1.0f32, 0.0f32);
            for step in 0..2000 {
                let fly = firefly(index, f64::from(step) * 0.05);
                let [u, v] = fly.position;
                assert!((MEADOW[0]..=MEADOW[2]).contains(&u), "fly {index}: u {u}");
                assert!((MEADOW[1]..=MEADOW[3]).contains(&v), "fly {index}: v {v}");
                assert!((FIREFLY_SIZE[0]..=FIREFLY_SIZE[1]).contains(&fly.size));
                assert!((0.0..=1.0).contains(&fly.opacity));
                dimmest = dimmest.min(fly.opacity);
                brightest = brightest.max(fly.opacity);
            }
            assert!(dimmest < 0.2 && brightest > 0.8, "fly {index} blinks");
        }
    }

    #[test]
    fn fireflies_differ_but_repeat_across_runs() {
        assert_eq!(firefly(3, 12.5), firefly(3, 12.5));
        assert_ne!(firefly(3, 12.5).position, firefly(4, 12.5).position);
        let homes: Vec<f32> = (0..FIREFLY_COUNT).map(|i| unit_hash(i, 0)).collect();
        assert!(homes.iter().all(|h| (0.0..1.0).contains(h)));
        let spread = homes.iter().cloned().fold(0.0f32, f32::max)
            - homes.iter().cloned().fold(1.0f32, f32::min);
        assert!(spread > 0.6, "homes spread across the meadow");
    }

    #[test]
    fn light_dots_are_soft_and_clear_at_the_edges() {
        for alpha in [firefly_alpha as fn(f32) -> f32, glow_alpha] {
            let dot = light_dot(DOT_SIZE, alpha);
            assert_eq!(dot.pixels.len(), (DOT_SIZE * DOT_SIZE * 4) as usize);
            let at = |x: u32, y: u32| dot.pixels[((y * DOT_SIZE + x) * 4 + 3) as usize];
            let centre = DOT_SIZE / 2;
            assert!(at(centre, centre) > 200);
            assert_eq!(at(0, 0), 0);
            assert_eq!(at(0, centre), 0);
            assert_eq!(at(centre, DOT_SIZE - 1), 0);
            assert!(dot.pixels.chunks(4).all(|p| p[..3] == [255, 255, 255]));
        }
    }

    #[test]
    fn light_follows_the_display_calibration() {
        let calibration = SpriteMaterial {
            tint: [0.5, 0.5, 0.5, 1.0],
            gamma: 1.4,
            ..SpriteMaterial::PLAIN
        };
        let material = light_material(&calibration, [255, 255, 255], 0.3);
        assert_eq!(material.tint, [0.5, 0.5, 0.5, 0.3]);
        assert_eq!(material.gamma, 1.4);
        assert_eq!(
            light_material(&calibration, [255, 255, 255], 2.0).tint[3],
            1.0
        );
    }
}
