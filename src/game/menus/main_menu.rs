//! Main menu: a column of textured buttons over a background, plus a Settings screen.
//!
//! Layout is authored for a 1920x1080 reference and scaled uniformly with the surface
//! (`min(width / 1920, height / 1080)`), so the menu keeps its proportions at any
//! resolution and DPI. All runtime geometry is in physical pixels.
//!
//! Art comes from `assets/ui/` (see `examples/generate_placeholder_assets.rs` for the
//! expected files). Missing or broken files are logged and replaced by flat colors.

use std::path::Path;

use super::screen_flow::{FlowEvent, ScreenFlow, Transition};
use crate::engine::animation::{Easing, Tween};
use crate::engine::assets::{self, MaskImage, RgbaImage};
use crate::engine::render::GpuContext;
use crate::engine::render::ui_renderer::{
    LabelDraw, LabelId, Sprite, SpriteMaterial, SpriteShape, TextureSetId, UiFrame, UiRenderer,
};
use crate::engine::ui::geometry::{NineSlice, NineSliceLayout, Rect, cover_uv};
use crate::engine::ui::hit::{AlphaMask, DEFAULT_ALPHA_THRESHOLD, Hit, HitTarget, topmost_hit};
use crate::engine::ui::pointer::PointerRouter;

const REFERENCE_SIZE: [f32; 2] = [1920.0, 1080.0];
const MENU_LEFT: f32 = 110.0;
const MENU_TOP: f32 = 430.0;
const BUTTON_SIZE: [f32; 2] = [380.0, 54.0];
const BUTTON_PITCH: f32 = 68.0;
/// Extra space before buttons that start a new section (e.g. Exit).
const SECTION_GAP: f32 = 36.0;
const TITLE_HEIGHT: f32 = 76.0;
const PANEL_WIDTH: f32 = 780.0;
const LABEL_SIZE: f32 = 26.0;
const TITLE_SIZE: f32 = 40.0;

/// 9-slice borders of `button_base.webp` (512x128), in texels.
const BUTTON_BORDERS: NineSlice = NineSlice {
    left: 48.0,
    right: 48.0,
    top: 40.0,
    bottom: 40.0,
};
const BUTTON_FALLBACK_SIZE: [u32; 2] = [512, 128];

/// Linear RGB.
const GLOW_COLOR: [f32; 3] = [1.0, 0.62, 0.25];
const GLOW_INTENSITY: f32 = 0.55;
const LABEL_COLOR: [u8; 4] = [232, 214, 178, 255];
const LABEL_HOVER_COLOR: [u8; 4] = [255, 241, 210, 255];
const LABEL_DISABLED_COLOR: [u8; 4] = [128, 116, 100, 255];
const TITLE_COLOR: [u8; 4] = [226, 196, 140, 255];

const HOVER_IN: f32 = 0.16;
const HOVER_OUT: f32 = 0.35;
const PRESS_DURATION: f32 = 0.06;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main = 0,
    Settings = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    NewGame,
    LoadGame,
    OpenSettings,
    Back,
    Exit,
}

struct ButtonSpec {
    label: &'static str,
    command: Command,
    enabled: bool,
    starts_section: bool,
}

/// New game / Load game stay disabled until there is a game to start or load.
const MAIN_BUTTONS: [ButtonSpec; 4] = [
    ButtonSpec {
        label: "New game",
        command: Command::NewGame,
        enabled: false,
        starts_section: false,
    },
    ButtonSpec {
        label: "Load game",
        command: Command::LoadGame,
        enabled: false,
        starts_section: false,
    },
    ButtonSpec {
        label: "Settings",
        command: Command::OpenSettings,
        enabled: true,
        starts_section: false,
    },
    ButtonSpec {
        label: "Exit",
        command: Command::Exit,
        enabled: true,
        starts_section: true,
    },
];

const SETTINGS_BUTTONS: [ButtonSpec; 1] = [ButtonSpec {
    label: "Back",
    command: Command::Back,
    enabled: true,
    starts_section: false,
}];

/// What the application has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEvent {
    Exit,
}

struct Button {
    spec: &'static ButtonSpec,
    label: LabelId,
    layout: NineSliceLayout,
    hover: Tween,
    press: Tween,
}

struct ScreenUi {
    title: Option<(LabelId, Rect)>,
    buttons: Vec<Button>,
}

struct ButtonArt {
    textures: TextureSetId,
    size: [f32; 2],
    /// Hit shape: the base texture's alpha (not the glow mask).
    mask: AlphaMask,
}

pub struct MainMenu {
    background: TextureSetId,
    background_size: [f32; 2],
    panel: TextureSetId,
    button_art: ButtonArt,
    screens: [ScreenUi; 2],
    flow: ScreenFlow<Screen>,
    router: PointerRouter,
    pointer: Option<[f32; 2]>,
    viewport: [f32; 2],
    scale: f32,
}

impl MainMenu {
    pub fn new(
        gpu: &GpuContext,
        renderer: &mut UiRenderer,
        assets_dir: &Path,
        viewport: [u32; 2],
    ) -> Self {
        let (device, queue) = (gpu.device(), gpu.queue());
        let ui_dir = assets_dir.join("ui");

        let background_image = load_color(
            &ui_dir.join("menu_background.webp"),
            [1, 1],
            [24, 26, 32, 255],
        );
        let background_size = [
            background_image.width as f32,
            background_image.height as f32,
        ];
        let background =
            renderer.create_texture_set(device, queue, "menu background", &background_image, None);

        let panel_image = load_color(
            &ui_dir.join("menu_panel_shade.webp"),
            [1, 1],
            [0, 0, 0, 190],
        );
        let panel =
            renderer.create_texture_set(device, queue, "menu panel shade", &panel_image, None);

        let button_image = load_color(
            &ui_dir.join("button_base.webp"),
            BUTTON_FALLBACK_SIZE,
            [88, 60, 40, 255],
        );
        let glow_image = load_mask(&ui_dir.join("button_glow_mask.webp"));
        let button_art = ButtonArt {
            textures: renderer.create_texture_set(
                device,
                queue,
                "menu button",
                &button_image,
                glow_image.as_ref(),
            ),
            size: [button_image.width as f32, button_image.height as f32],
            mask: AlphaMask::from_rgba(
                button_image.width,
                button_image.height,
                &button_image.pixels,
            ),
        };

        let mut make_screen = |title: Option<&str>, specs: &'static [ButtonSpec]| ScreenUi {
            title: title.map(|text| (renderer.create_label(text), Rect::new(0.0, 0.0, 0.0, 0.0))),
            buttons: specs
                .iter()
                .map(|spec| Button {
                    spec,
                    label: renderer.create_label(spec.label),
                    layout: NineSliceLayout::new(
                        Rect::new(0.0, 0.0, 0.0, 0.0),
                        button_art.size,
                        BUTTON_BORDERS,
                    ),
                    hover: Tween::new(0.0),
                    press: Tween::new(0.0),
                })
                .collect(),
        };
        let screens = [
            make_screen(None, &MAIN_BUTTONS),
            make_screen(Some("Settings"), &SETTINGS_BUTTONS),
        ];

        let mut menu = Self {
            background,
            background_size,
            panel,
            button_art,
            screens,
            flow: ScreenFlow::new(Screen::Main),
            router: PointerRouter::default(),
            pointer: None,
            viewport: [0.0; 2],
            scale: 1.0,
        };
        menu.resize(viewport);
        menu
    }

    /// Recomputes the layout for a new surface size (physical pixels).
    pub fn resize(&mut self, viewport: [u32; 2]) {
        if viewport[0] == 0 || viewport[1] == 0 {
            return;
        }
        self.viewport = [viewport[0] as f32, viewport[1] as f32];
        self.scale = ui_scale(self.viewport);
        for screen in &mut self.screens {
            let rects = layout_column(
                screen.title.is_some(),
                screen.buttons.iter().map(|b| b.spec.starts_section),
                self.scale,
            );
            if let Some((_, title_rect)) = &mut screen.title {
                *title_rect = rects.title;
            }
            for (button, rect) in screen.buttons.iter_mut().zip(rects.buttons) {
                button.layout = NineSliceLayout::new(rect, self.button_art.size, BUTTON_BORDERS);
            }
        }
        self.refresh_hover();
    }

    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        self.pointer = Some([x, y]);
        self.refresh_hover();
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.refresh_hover();
    }

    pub fn pointer_pressed(&mut self) {
        if self.flow.is_interactive() {
            self.router.press();
        }
    }

    pub fn pointer_released(&mut self) {
        let Some(index) = self.router.release() else {
            return;
        };
        if !self.flow.is_interactive() {
            return;
        }
        let command = self
            .current_screen()
            .buttons
            .get(index)
            .map(|b| b.spec.command);
        match command {
            Some(Command::OpenSettings) => self.flow.request(Transition::To(Screen::Settings)),
            Some(Command::Back) => self.flow.request(Transition::To(Screen::Main)),
            Some(Command::Exit) => self.flow.request(Transition::Exit),
            Some(Command::NewGame | Command::LoadGame) => {
                tracing::info!(?command, "not available yet");
            }
            None => {}
        }
        // A transition makes the menu non-interactive: let the hover glow fade out.
        self.refresh_hover();
    }

    /// Advances animations by `dt` seconds.
    pub fn update(&mut self, dt: f32) -> Option<MenuEvent> {
        match self.flow.update(dt) {
            FlowEvent::None => {}
            FlowEvent::ScreenChanged => {
                tracing::debug!(screen = ?self.flow.current(), "menu screen changed");
                for button in &mut self.screens[self.flow.current() as usize].buttons {
                    button.hover.set(0.0);
                    button.press.set(0.0);
                }
                self.router.reset();
                self.refresh_hover();
            }
            FlowEvent::ExitReady => return Some(MenuEvent::Exit),
        }

        let hovered = self.router.hovered();
        let pressed = self.router.pressed_visual();
        let screen = self.flow.current() as usize;
        for (index, button) in self.screens[screen].buttons.iter_mut().enumerate() {
            if hovered == Some(index) {
                button.hover.animate_to(1.0, HOVER_IN, Easing::EaseOutCubic);
            } else {
                button
                    .hover
                    .animate_to(0.0, HOVER_OUT, Easing::EaseOutCubic);
            }
            let press = if pressed == Some(index) { 1.0 } else { 0.0 };
            button
                .press
                .animate_to(press, PRESS_DURATION, Easing::Linear);
            button.hover.update(dt);
            button.press.update(dt);
        }
        None
    }

    /// Appends this frame's sprites and labels.
    pub fn build_frame(&self, frame: &mut UiFrame) {
        let [width, height] = self.viewport;
        frame.background.push(Sprite {
            textures: self.background,
            shape: SpriteShape::Stretched {
                rect: Rect::new(0.0, 0.0, width, height),
                uv: cover_uv(self.background_size, self.viewport),
            },
            material: SpriteMaterial::PLAIN,
        });
        frame.background.push(Sprite {
            textures: self.panel,
            shape: SpriteShape::Stretched {
                rect: Rect::new(0.0, 0.0, (PANEL_WIDTH * self.scale).round(), height),
                uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            },
            material: SpriteMaterial::PLAIN,
        });

        frame.group_opacity = self.flow.opacity();
        let screen = self.current_screen();
        if let Some((label, rect)) = screen.title {
            frame.group_labels.push(LabelDraw {
                label,
                rect,
                font_size: (TITLE_SIZE * self.scale).round(),
                color: TITLE_COLOR,
            });
        }
        for button in &screen.buttons {
            let hover = button.hover.value();
            frame.group_sprites.push(Sprite {
                textures: self.button_art.textures,
                shape: SpriteShape::NineSlice(button.layout),
                material: SpriteMaterial {
                    hover_amount: hover,
                    press_amount: button.press.value(),
                    disabled_amount: if button.spec.enabled { 0.0 } else { 1.0 },
                    opacity: 1.0,
                    glow_color: GLOW_COLOR,
                    glow_intensity: GLOW_INTENSITY,
                },
            });
            let color = if button.spec.enabled {
                lerp_color(LABEL_COLOR, LABEL_HOVER_COLOR, hover)
            } else {
                LABEL_DISABLED_COLOR
            };
            frame.group_labels.push(LabelDraw {
                label: button.label,
                rect: button.layout.rect(),
                font_size: (LABEL_SIZE * self.scale).round(),
                color,
            });
        }
    }

    fn current_screen(&self) -> &ScreenUi {
        &self.screens[self.flow.current() as usize]
    }

    /// Re-runs hit testing for the last pointer position.
    fn refresh_hover(&mut self) {
        let hovered = match self.pointer {
            Some([x, y]) if self.flow.is_interactive() => {
                let mask = &self.button_art.mask;
                let targets = self
                    .current_screen()
                    .buttons
                    .iter()
                    .map(|button| HitTarget {
                        layout: &button.layout,
                        mask,
                        interactive: button.spec.enabled,
                    });
                match topmost_hit(targets, x, y, DEFAULT_ALPHA_THRESHOLD) {
                    Hit::Interactive(index) => Some(index),
                    Hit::Blocked(_) | Hit::Nothing => None,
                }
            }
            _ => None,
        };
        self.router.set_hovered(hovered);
    }
}

/// Uniform UI scale relative to the 1920x1080 reference.
fn ui_scale(viewport: [f32; 2]) -> f32 {
    (viewport[0] / REFERENCE_SIZE[0]).min(viewport[1] / REFERENCE_SIZE[1])
}

struct ColumnRects {
    title: Rect,
    buttons: Vec<Rect>,
}

/// Places an optional title and a column of buttons, snapped to whole pixels.
fn layout_column(
    has_title: bool,
    section_starts: impl Iterator<Item = bool>,
    scale: f32,
) -> ColumnRects {
    let snap = |x: f32, y: f32, w: f32, h: f32| {
        Rect::new(
            (x * scale).round(),
            (y * scale).round(),
            (w * scale).round(),
            (h * scale).round(),
        )
    };
    let title = snap(MENU_LEFT, MENU_TOP, BUTTON_SIZE[0], TITLE_HEIGHT);
    let mut y = if has_title {
        MENU_TOP + TITLE_HEIGHT
    } else {
        MENU_TOP
    };
    let buttons = section_starts
        .enumerate()
        .map(|(index, starts_section)| {
            if starts_section && index > 0 {
                y += SECTION_GAP;
            }
            let rect = snap(MENU_LEFT, y, BUTTON_SIZE[0], BUTTON_SIZE[1]);
            y += BUTTON_PITCH;
            rect
        })
        .collect();
    ColumnRects { title, buttons }
}

fn lerp_color(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| {
        (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8
    })
}

fn load_color(path: &Path, fallback_size: [u32; 2], fallback: [u8; 4]) -> RgbaImage {
    assets::load_rgba(path).unwrap_or_else(|error| {
        tracing::error!("{error}; using a flat placeholder");
        RgbaImage::solid(fallback_size[0], fallback_size[1], fallback)
    })
}

fn load_mask(path: &Path) -> Option<MaskImage> {
    assets::load_mask(path)
        .inspect_err(|error| tracing::error!("{error}; buttons will not glow"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_follows_the_limiting_axis() {
        assert_eq!(ui_scale([1920.0, 1080.0]), 1.0);
        assert_eq!(ui_scale([3840.0, 2160.0]), 2.0);
        assert!((ui_scale([1600.0, 900.0]) - 0.8333).abs() < 1e-3);
        // Ultrawide: height limits.
        assert_eq!(ui_scale([2560.0, 1080.0]), 1.0);
    }

    #[test]
    fn column_applies_pitch_and_section_gap() {
        let rects = layout_column(false, [false, false, false, true].into_iter(), 1.0);
        let ys: Vec<f32> = rects.buttons.iter().map(|r| r.y).collect();
        assert_eq!(ys, [430.0, 498.0, 566.0, 670.0]);
        assert!(
            rects
                .buttons
                .iter()
                .all(|r| r.x == 110.0 && r.w == 380.0 && r.h == 54.0)
        );
    }

    #[test]
    fn title_pushes_buttons_down_and_layout_scales() {
        let rects = layout_column(true, [false].into_iter(), 0.5);
        assert_eq!(rects.title, Rect::new(55.0, 215.0, 190.0, 38.0));
        assert_eq!(rects.buttons[0], Rect::new(55.0, 253.0, 190.0, 27.0));
    }

    #[test]
    fn label_color_interpolates() {
        assert_eq!(
            lerp_color([0, 0, 0, 255], [200, 100, 50, 255], 0.5),
            [100, 50, 25, 255]
        );
        assert_eq!(
            lerp_color([0, 0, 0, 0], [10, 10, 10, 10], 2.0),
            [10, 10, 10, 10]
        );
    }

    #[test]
    fn button_specs_match_the_menu_design() {
        let enabled: Vec<&str> = MAIN_BUTTONS
            .iter()
            .filter(|b| b.enabled)
            .map(|b| b.label)
            .collect();
        assert_eq!(enabled, ["Settings", "Exit"]);
        assert_eq!(SETTINGS_BUTTONS[0].command, Command::Back);
    }
}
