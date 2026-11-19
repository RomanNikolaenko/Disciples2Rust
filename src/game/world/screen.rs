//! The world map screen: the terrain seen from above, a camera the player moves and
//! zooms, the tile under the pointer, and a way back to the main menu.
//!
//! Controls: WASD / arrow keys or the pointer at a screen edge pan; dragging with the
//! right or middle button pans; the wheel zooms about the pointer; Esc asks whether to
//! return to the main menu.
//!
//! Art: `assets/maps/village/terrain.webp` covers the whole map (a placeholder painted by
//! `examples/generate_placeholder_assets.rs` from `map.toml`). It is drawn as one sprite
//! on the background layer, calibrated like the rest of the game image; the hover
//! outline goes on the background layer too, the HUD text on the group layer and the
//! dialog and fades on the overlay.
//!
//! The map is a 3D scene seen by an isometric camera (see
//! `engine::scene::iso_camera`): the painted terrain as ground, the scenery standing on
//! it (see [`super::scenery`]) and the figures (see [`super::figure`]), all drawn into
//! the 3D layer, which the background layer shows calibrated like the rest of the game
//! image. The hero starts on the plains tile nearest the map's center, with the camera
//! on it; the dragon rests on the plains a few tiles away. These placements only show
//! the models; no game rules use them yet.

use std::path::Path;
use std::rc::Rc;

use glam::Vec2;

use super::figure::{DRAGON, Figure, FigureKind, FigureModel, HERO};
use super::map::{MapError, Terrain, TerrainMap};
use super::scenery::{Scenery, SceneryConfig};
use crate::engine::animation::{Easing, Tween};
use crate::engine::render::GpuContext;
use crate::engine::render::scene_renderer::{SceneFrame, SceneRenderer};
use crate::engine::render::ui_renderer::{
    LabelId, TextAlign, TextStyle, TextureSetId, UiFrame, UiRenderer,
};
use crate::engine::scene::iso_camera::IsoCamera;
use crate::engine::ui::geometry::Rect;
use crate::engine::ui::pointer::{PointerPosition, PointerRouter};
use crate::game::locale::Language;
use crate::game::menus::widgets::{
    self, DIALOG_CANCEL, DIALOG_CONFIRM, Dialog, FULL_UV, UiKit, game_image_material, palette,
    ui_scale,
};
use crate::game::settings::DisplaySettings;

/// The map started by "New game": an empty plain (48x48), its scenery described in the
/// folder's `scenery.toml`.
const START_MAP: &str = "maps/plain";

/// Screen pixels per world unit across the view at the 1920x1080 reference (scaled with
/// the UI; a tile's diamond is √2 times as wide): the zoom a new game starts at, and the
/// closest and farthest allowed. The farthest still shows a good part of the map
/// without drawing all of its scenery at once.
const START_ZOOM: f32 = 42.0;
const MAX_ZOOM: f32 = 96.0;
const MIN_ZOOM: f32 = 22.0;
/// Each wheel notch zooms by this factor; touchpads scroll in pixels, one notch per
/// `PIXELS_PER_NOTCH`.
const ZOOM_STEP: f32 = 1.18;
const PIXELS_PER_NOTCH: f32 = 100.0;
/// How quickly the zoom catches up with the wheel (1/s, exponential; frame-rate
/// independent).
const ZOOM_SHARPNESS: f32 = 14.0;
/// Keyboard and edge panning speed, reference pixels per second.
const PAN_SPEED: f32 = 1100.0;
/// The pointer within this many reference pixels of a screen edge pans the view.
const EDGE_ZONE: f32 = 8.0;

const FADE_IN: f32 = 0.5;
const FADE_OUT: f32 = 0.3;
/// Terrain name of the hovered tile, bottom left (reference px: x, bottom margin, h).
const TILE_INFO: [f32; 3] = [40.0, 32.0, 36.0];
const TILE_INFO_TEXT_SIZE: f32 = 22.0;
const EXIT_DIALOG_ID: usize = 0;
const EXIT_MESSAGE: &str = "This game will not be saved.";
/// The dragon rests on the plains tile nearest this offset from the hero's tile.
const DRAGON_FROM_HERO: (usize, usize) = (4, 1);

/// What the application has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldEvent {
    /// The player left for the main menu (the fade-out has finished).
    ExitToMenu,
}

/// A pan direction from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanKey {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Default, Clone, Copy)]
struct HeldKeys {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl HeldKeys {
    fn direction(self) -> Vec2 {
        let axis = |negative: bool, positive: bool| f32::from(positive) - f32::from(negative);
        Vec2::new(axis(self.left, self.right), axis(self.up, self.down))
    }
}

pub struct WorldScreen {
    kit: Rc<UiKit>,
    map: TerrainMap,
    /// The ground, trees, rocks and buildings.
    scenery: Scenery,
    /// `None` when the model could not be loaded (the map still works).
    hero: Option<Figure>,
    /// The other figures: creatures.
    figures: Vec<Figure>,
    /// Every loaded figure model, to free on leaving.
    models: Vec<Rc<FigureModel>>,
    /// The 3D layer's image, as the UI sees it.
    scene_texture: TextureSetId,
    camera: IsoCamera,
    /// Where the smooth zoom is heading, and the screen point it zooms about.
    zoom_target: f32,
    zoom_anchor: Vec2,
    viewport: [f32; 2],
    scale: f32,
    display: DisplaySettings,
    language: Language,
    pointer: PointerPosition,
    /// Dialog buttons: hover, press and click.
    router: PointerRouter,
    /// Last pointer position while a right/middle drag pans the view.
    drag: Option<Vec2>,
    keys: HeldKeys,
    focused: bool,
    exit_dialog: Dialog,
    exit_dialog_open: bool,
    tile_label: LabelId,
    shown_tile: Option<(usize, usize)>,
    /// Black over everything: fades out on entry, back in when leaving.
    fade: Tween,
    leaving: bool,
}

impl WorldScreen {
    /// `scene_texture` shows the 3D layer that `scene` draws into.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gpu: &GpuContext,
        renderer: &mut UiRenderer,
        scene: &mut SceneRenderer,
        scene_texture: TextureSetId,
        kit: Rc<UiKit>,
        assets_dir: &Path,
        viewport: [u32; 2],
        display: DisplaySettings,
    ) -> Result<Self, MapError> {
        let map_dir = assets_dir.join(START_MAP);
        let map = TerrainMap::load(&map_dir.join("map.toml"))?;
        let mut image =
            widgets::load_color(&map_dir.join("terrain.webp"), [1, 1], [70, 90, 60, 255]);
        let max_side = gpu.device().limits().max_texture_dimension_2d;
        if image.width > max_side || image.height > max_side {
            tracing::error!(
                width = image.width,
                height = image.height,
                max_side,
                "terrain image exceeds the GPU's texture size; using a flat color"
            );
            image = crate::engine::assets::RgbaImage::solid(1, 1, [70, 90, 60, 255]);
        }
        // The art should cover the grid at a whole number of texels per tile.
        let expected = image.width as f32 / map.width() as f32 * map.height() as f32;
        if (expected - image.height as f32).abs() > 1.0 && image.width > 1 {
            tracing::warn!(
                width = image.width,
                height = image.height,
                tiles = ?[map.width(), map.height()],
                "terrain image does not match the map's shape; it is stretched"
            );
        }

        let center = (map.width() / 2, map.height() / 2);
        let start = map.nearest(center, Terrain::Plains).unwrap_or(center);
        let mut models = Vec::new();
        // A model that fails to load is reported; the map works without it.
        let mut load = |kind: &'static FigureKind| match FigureModel::load(
            kind,
            assets_dir,
            gpu.device(),
            gpu.queue(),
            scene,
        ) {
            Ok(model) => {
                models.push(Rc::clone(&model));
                Some(model)
            }
            Err(error) => {
                tracing::error!("a figure cannot be shown: {error}");
                None
            }
        };
        let hero = load(&HERO).map(|model| Figure::new(model, start));
        let mut figures = Vec::new();
        // The dragon's model is not published with the game (its license is unknown):
        // without it the map simply has no dragon.
        let dragon = if assets_dir.join(DRAGON.model).is_file() {
            load(&DRAGON)
        } else {
            tracing::info!("no dragon in this build");
            None
        };
        if let Some(dragon) = dragon {
            let near = (start.0 + DRAGON_FROM_HERO.0, start.1 + DRAGON_FROM_HERO.1);
            let lair = map.by_distance(near, Terrain::Plains);
            if let Some(&tile) = lair.iter().find(|&&tile| tile != start) {
                figures.push(Figure::new(dragon, tile));
            }
        }
        let reserved: Vec<(usize, usize)> = hero
            .iter()
            .chain(&figures)
            .map(|figure| {
                let feet = figure.feet();
                (feet.x as usize, feet.y as usize)
            })
            .collect();
        let scenery = Scenery::new(
            &map,
            &SceneryConfig::load(&map_dir.join("scenery.toml")),
            &reserved,
            image,
            assets_dir,
            gpu.device(),
            gpu.queue(),
            scene,
        );

        let viewport = viewport.map(|side| side as f32);
        let scale = ui_scale(viewport);
        let world = Vec2::new(map.width() as f32, map.height() as f32);
        let mut camera = IsoCamera::new(
            world,
            Vec2::from(viewport),
            START_ZOOM * scale,
            MIN_ZOOM * scale,
            MAX_ZOOM * scale,
        );
        camera.look_at(Vec2::new(start.0 as f32 + 0.5, start.1 as f32 + 0.5));
        let language = display.language;
        let mut exit_dialog = Dialog::new(
            renderer,
            "Return to the main menu?",
            ["Main menu", "Cancel"],
        );
        exit_dialog.relabel(renderer, language);
        // Nothing is saved yet: say so before the player leaves.
        exit_dialog.set_message(renderer, language.tr(EXIT_MESSAGE));
        let tile_label = renderer.create_label(
            "",
            TextStyle {
                bold: false,
                align: TextAlign::Left,
            },
        );
        let mut fade = Tween::new(1.0);
        fade.animate_to(0.0, FADE_IN, Easing::EaseOutCubic);
        let mut screen = Self {
            kit,
            map,
            scenery,
            hero,
            figures,
            models,
            scene_texture,
            zoom_target: camera.zoom(),
            zoom_anchor: Vec2::from(viewport) / 2.0,
            camera,
            viewport,
            scale,
            display,
            language,
            pointer: PointerPosition::default(),
            router: PointerRouter::default(),
            drag: None,
            keys: HeldKeys::default(),
            focused: true,
            exit_dialog,
            exit_dialog_open: false,
            tile_label,
            shown_tile: None,
            fade,
            leaving: false,
        };
        screen.layout_dialog();
        tracing::info!(
            tiles = ?[screen.map.width(), screen.map.height()],
            zoom = screen.camera.zoom(),
            hero = ?start,
            figures = screen.figures.len(),
            "world map opened"
        );
        Ok(screen)
    }

    /// Frees the GPU resources this screen created; call when leaving it for good.
    pub fn release(self, scene: &mut SceneRenderer) {
        self.scenery.release(scene);
        for model in &self.models {
            model.release(scene);
        }
    }

    pub fn resize(&mut self, viewport: [u32; 2]) {
        self.viewport = viewport.map(|side| side as f32);
        let previous_scale = self.scale;
        self.scale = ui_scale(self.viewport);
        self.camera.set_viewport(Vec2::from(self.viewport));
        // Keep the same view of the world when the UI scale changes with the window.
        if previous_scale > 0.0 && self.scale > 0.0 {
            let ratio = self.scale / previous_scale;
            let center = Vec2::from(self.viewport) / 2.0;
            self.camera.zoom_at(center, self.camera.zoom() * ratio);
        }
        self.zoom_target = self.camera.zoom();
        self.layout_dialog();
        self.refresh_hover();
    }

    /// Display calibration changed (brightness, gamma).
    pub fn set_display(&mut self, display: DisplaySettings) {
        self.display = display;
    }

    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        self.pointer.moved([x, y]);
        let position = Vec2::new(x, y);
        if let Some(last) = self.drag {
            self.camera.pan_screen(position - last);
            self.drag = Some(position);
        }
        self.refresh_hover();
    }

    pub fn pointer_left(&mut self) {
        self.pointer.left();
        self.refresh_hover();
    }

    pub fn pointer_entered(&mut self) {
        self.pointer.entered();
        self.refresh_hover();
    }

    /// Focus went elsewhere: forget held keys, drags and presses.
    pub fn focus_lost(&mut self) {
        self.focused = false;
        self.keys = HeldKeys::default();
        self.drag = None;
        self.router.reset();
        self.refresh_hover();
    }

    pub fn focus_gained(&mut self) {
        self.focused = true;
    }

    /// Primary button down: presses a dialog button.
    pub fn pointer_pressed(&mut self) {
        if self.pointer.pressed() {
            self.refresh_hover();
        }
        if self.exit_dialog_open && !self.leaving {
            self.router.press();
        }
    }

    pub fn pointer_released(&mut self) {
        let Some(id) = self.router.release() else {
            return;
        };
        if !self.exit_dialog_open || self.leaving {
            return;
        }
        if id == EXIT_DIALOG_ID + DIALOG_CONFIRM {
            self.leave();
        } else if id == EXIT_DIALOG_ID + DIALOG_CANCEL {
            self.exit_dialog_open = false;
        }
        self.refresh_hover();
    }

    /// Right or middle button down: start dragging the map.
    pub fn drag_pressed(&mut self) {
        if self.pointer.pressed() {
            self.refresh_hover();
        }
        if self.exit_dialog_open || self.leaving {
            return;
        }
        self.drag = self.pointer.get().map(Vec2::from);
    }

    pub fn drag_released(&mut self) {
        self.drag = None;
    }

    pub fn scroll_lines(&mut self, lines: f32) {
        self.zoom_by(lines);
    }

    pub fn scroll_pixels(&mut self, pixels: f32) {
        self.zoom_by(pixels / PIXELS_PER_NOTCH);
    }

    pub fn pan_key(&mut self, key: PanKey, pressed: bool) {
        let held = pressed && self.focused;
        match key {
            PanKey::Left => self.keys.left = held,
            PanKey::Right => self.keys.right = held,
            PanKey::Up => self.keys.up = held,
            PanKey::Down => self.keys.down = held,
        }
    }

    /// Esc: open the return-to-menu dialog, or close it.
    pub fn back(&mut self) {
        if self.leaving {
            return;
        }
        self.exit_dialog_open = !self.exit_dialog_open;
        if self.exit_dialog_open {
            self.exit_dialog.show();
            self.keys = HeldKeys::default();
            self.drag = None;
        }
        self.router.reset();
        self.refresh_hover();
    }

    /// Enter / Space: confirm the open dialog.
    pub fn activate(&mut self) {
        if self.exit_dialog_open && !self.leaving {
            self.leave();
        }
    }

    pub fn update(&mut self, dt: f32) -> Option<WorldEvent> {
        self.fade.update(dt);
        for figure in self.hero.iter_mut().chain(&mut self.figures) {
            figure.update(dt);
        }
        if self.leaving {
            return (self.fade.is_settled() && self.fade.value() >= 1.0)
                .then_some(WorldEvent::ExitToMenu);
        }

        // Smooth zoom: approach the target in log space, about the wheel's anchor.
        let zoom = self.camera.zoom();
        if (zoom - self.zoom_target).abs() > 1e-3 {
            let t = 1.0 - (-ZOOM_SHARPNESS * dt).exp();
            let next = (zoom.ln() + (self.zoom_target.ln() - zoom.ln()) * t).exp();
            let next = if (next - self.zoom_target).abs() < 1e-3 {
                self.zoom_target
            } else {
                next
            };
            self.camera.zoom_at(self.zoom_anchor, next);
        }

        if !self.exit_dialog_open {
            let direction = self.keys.direction() + self.edge_direction();
            if direction != Vec2::ZERO {
                let speed = PAN_SPEED * self.scale * dt;
                // `pan_screen` follows a drag; moving the view right drags the world left.
                self.camera
                    .pan_screen(-direction.normalize_or_zero() * speed);
            }
        }
        self.refresh_hover();
        let hovered = self.router.hovered();
        let pressed = self.router.pressed_visual();
        self.exit_dialog
            .animate(hovered, pressed, EXIT_DIALOG_ID, dt);
        None
    }

    /// The UI frame, and the 3D layer's models in `scene`.
    pub fn build_frame(
        &mut self,
        renderer: &mut UiRenderer,
        frame: &mut UiFrame,
        scene: &mut SceneFrame,
    ) {
        let kit = Rc::clone(&self.kit);
        frame.group_opacity = 1.0;
        let hovered_tile = self.hovered_tile();
        self.scenery.draw(&self.camera, hovered_tile, scene);
        for figure in self.hero.iter_mut().chain(&mut self.figures) {
            figure.draw(&self.camera, self.scenery.relief(), scene);
        }
        // The whole map is the 3D layer's image.
        let viewport = Vec2::from(self.viewport);
        widgets::push_image(
            &mut frame.background,
            self.scene_texture,
            Rect::new(0.0, 0.0, viewport.x, viewport.y),
            FULL_UV,
            game_image_material(&self.display),
        );
        if hovered_tile != self.shown_tile {
            self.shown_tile = hovered_tile;
            let text = hovered_tile
                .and_then(|(x, y)| self.map.get(x, y).map(|terrain| (x, y, terrain)))
                .map(|(x, y, terrain)| {
                    format!(
                        "{} · {}, {}",
                        self.language.tr(terrain.name()),
                        x + 1,
                        y + 1
                    )
                })
                .unwrap_or_default();
            renderer.set_label_text(self.tile_label, &text);
        }
        if self.shown_tile.is_some() {
            let [x, bottom, h] = TILE_INFO;
            let rect = Rect::new(
                (x * self.scale).round(),
                (self.viewport[1] - (bottom + h) * self.scale).round(),
                (600.0 * self.scale).round(),
                (h * self.scale).round(),
            );
            widgets::push_label(
                frame,
                self.tile_label,
                rect,
                (TILE_INFO_TEXT_SIZE * self.scale).round(),
                palette::TEXT,
                true,
            );
        }

        if self.exit_dialog_open {
            self.exit_dialog.draw(frame, &kit);
        }
        let fade = self.fade.value();
        if fade > 0.0 {
            let alpha = (fade * 255.0).round() as u8;
            widgets::push_flat(
                &mut frame.overlay_sprites,
                &kit,
                Rect::new(0.0, 0.0, self.viewport[0], self.viewport[1]),
                [0, 0, 0, alpha],
            );
        }
    }

    fn leave(&mut self) {
        self.leaving = true;
        self.exit_dialog_open = false;
        self.keys = HeldKeys::default();
        self.drag = None;
        self.fade.animate_to(1.0, FADE_OUT, Easing::EaseInOutCubic);
        tracing::info!("leaving the world map");
    }

    fn zoom_by(&mut self, notches: f32) {
        if self.exit_dialog_open || self.leaving || !notches.is_finite() {
            return;
        }
        let target = self.zoom_target * ZOOM_STEP.powf(notches);
        self.zoom_target = self.camera.clamp_zoom_value(target);
        self.zoom_anchor = self
            .pointer
            .get()
            .map_or(Vec2::from(self.viewport) / 2.0, Vec2::from);
    }

    /// Pointer near a screen edge (only while the window has focus and nothing is
    /// dragged): which way to pan.
    fn edge_direction(&self) -> Vec2 {
        let Some([x, y]) = self.pointer.get() else {
            return Vec2::ZERO;
        };
        if !self.focused || self.drag.is_some() {
            return Vec2::ZERO;
        }
        edge_pan(
            Vec2::new(x, y),
            Vec2::from(self.viewport),
            EDGE_ZONE * self.scale,
        )
    }

    fn hovered_tile(&self) -> Option<(usize, usize)> {
        if self.exit_dialog_open || self.leaving {
            return None;
        }
        let [x, y] = self.pointer.get()?;
        // The first ground along the line of sight: a hill's near slope hides what is
        // behind it.
        let (origin, forward) = self.camera.screen_ray(Vec2::new(x, y));
        let ground = self.scenery.relief().hit(origin, forward)?;
        self.map.tile_at(ground.x, ground.z)
    }

    fn layout_dialog(&mut self) {
        let kit = Rc::clone(&self.kit);
        self.exit_dialog.layout(self.scale, self.viewport, &kit);
    }

    fn refresh_hover(&mut self) {
        let hovered = match self.pointer.get() {
            Some([x, y]) if self.exit_dialog_open && !self.leaving => self
                .exit_dialog
                .hit_test(x, y, &self.kit, EXIT_DIALOG_ID)
                .interactive(),
            _ => None,
        };
        self.router.set_hovered(hovered);
    }
}

/// Which way the pointer at `position` asks to pan: -1/0/1 per axis, for a band of
/// `zone` pixels along each edge of `viewport`.
fn edge_pan(position: Vec2, viewport: Vec2, zone: f32) -> Vec2 {
    let axis = |p: f32, size: f32| {
        if p < zone {
            -1.0
        } else if p >= size - zone {
            1.0
        } else {
            0.0
        }
    };
    if viewport.x <= 2.0 * zone || viewport.y <= 2.0 * zone {
        return Vec2::ZERO;
    }
    Vec2::new(axis(position.x, viewport.x), axis(position.y, viewport.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_keys_combine_into_a_direction() {
        let mut keys = HeldKeys::default();
        assert_eq!(keys.direction(), Vec2::ZERO);
        keys.left = true;
        keys.down = true;
        assert_eq!(keys.direction(), Vec2::new(-1.0, 1.0));
        keys.right = true;
        assert_eq!(
            keys.direction(),
            Vec2::new(0.0, 1.0),
            "opposite keys cancel"
        );
    }

    #[test]
    fn edges_pan_towards_them() {
        let view = Vec2::new(1920.0, 1080.0);
        assert_eq!(edge_pan(Vec2::new(960.0, 540.0), view, 8.0), Vec2::ZERO);
        assert_eq!(
            edge_pan(Vec2::new(2.0, 540.0), view, 8.0),
            Vec2::new(-1.0, 0.0)
        );
        assert_eq!(
            edge_pan(Vec2::new(1919.0, 1079.0), view, 8.0),
            Vec2::new(1.0, 1.0)
        );
        assert_eq!(
            edge_pan(Vec2::new(960.0, 0.0), view, 8.0),
            Vec2::new(0.0, -1.0)
        );
        // A viewport too small for two bands never pans.
        assert_eq!(
            edge_pan(Vec2::new(1.0, 1.0), Vec2::new(10.0, 10.0), 8.0),
            Vec2::ZERO
        );
    }

    #[test]
    fn dialog_texts_are_translated() {
        for text in [
            "Return to the main menu?",
            EXIT_MESSAGE,
            "Main menu",
            "Cancel",
        ] {
            assert!(crate::game::locale::is_translated(text), "{text}");
        }
    }
}
