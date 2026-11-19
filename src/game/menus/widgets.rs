//! Widgets shared by the menu screens and the world map's interface, and the art they
//! are drawn with.
//!
//! All geometry is in physical pixels; screens compute rects from their 1920x1080
//! reference layout times the UI scale.

use std::path::Path;

use crate::engine::animation::{Easing, Tween};
use crate::engine::assets::{self, MaskImage, RgbaImage};
use crate::engine::render::GpuContext;
use crate::engine::render::ui_renderer::{
    LabelDraw, LabelId, Sprite, SpriteMaterial, SpriteShape, TextAlign, TextStyle, TextureSetId,
    UiFrame, UiRenderer,
};
use crate::engine::ui::geometry::{NineSlice, NineSliceLayout, Rect};
use crate::engine::ui::hit::{
    AlphaMask, DEFAULT_ALPHA_THRESHOLD, Hit, HitShape, HitTarget, topmost_hit,
};
use crate::game::locale::Language;
use crate::game::settings::{DisplaySettings, MAX_VOLUME};

/// Uniform UI scale relative to the 1920x1080 reference.
pub fn ui_scale(viewport: [f32; 2]) -> f32 {
    (viewport[0] / REFERENCE_SIZE[0]).min(viewport[1] / REFERENCE_SIZE[1])
}

/// Material for the game image (menu background, world map): brightness and gamma
/// calibrate it in linear light, while the interface keeps its colors.
pub fn game_image_material(display: &DisplaySettings) -> SpriteMaterial {
    let factor = display.brightness_factor();
    SpriteMaterial {
        tint: [factor, factor, factor, 1.0],
        gamma: display.gamma_value(),
        ..SpriteMaterial::PLAIN
    }
}

/// sRGB colors shared by the menus (sampled from the mockups).
pub mod palette {
    pub const TEXT: [u8; 4] = [242, 240, 235, 255];
    pub const TEXT_SHADOW: [u8; 4] = [0, 0, 0, 170];
    pub const TEXT_DISABLED: [u8; 4] = [150, 146, 140, 255];
    pub const TEXT_SELECTED: [u8; 4] = [74, 70, 66, 255];
    pub const HEADING: [u8; 4] = [234, 176, 112, 255];
    /// Values changed but not applied yet.
    pub const TEXT_PENDING: [u8; 4] = [240, 196, 120, 255];
    pub const SLIDER_FILL: [u8; 4] = [240, 179, 116, 255];
    pub const SLIDER_REST: [u8; 4] = [116, 85, 52, 255];
    pub const ROW_LINE: [u8; 4] = [126, 83, 38, 255];
    /// Darkens the background behind the Settings and Credits screens.
    pub const OVERLAY: [u8; 4] = [0, 0, 0, 215];
    pub const SEGMENT_ON: [u8; 4] = [119, 89, 54, 255];
    pub const SEGMENT_OFF: [u8; 4] = [60, 45, 27, 255];
    /// Scrollbar thumb at rest (brightens to `SLIDER_FILL` on hover).
    pub const SCROLL_THUMB: [u8; 4] = [168, 124, 76, 255];
}

/// 9-slice borders of `button_base.webp` (texels): the end ornaments and frame corners
/// keep their proportions, only the stone middle stretches.
const BUTTON_BORDERS: NineSlice = NineSlice {
    left: 240.0,
    right: 240.0,
    top: 40.0,
    bottom: 40.0,
};
const BUTTON_FALLBACK_SIZE: [u32; 2] = [1410, 342];
/// Linear RGB: the turquoise the crystals light up with on hover.
const BUTTON_GLOW_COLOR: [f32; 3] = [0.06, 0.62, 0.78];
const BUTTON_GLOW_INTENSITY: f32 = 0.9;
/// Linear RGB: the slider knob's teal gem.
const KNOB_GLOW_COLOR: [f32; 3] = [0.25, 0.95, 0.95];
const KNOB_GLOW_INTENSITY: f32 = 0.8;
/// Selected state (current tab): near-black stone with unlit crystals.
const SELECTED_TINT: [f32; 4] = [0.09, 0.09, 0.09, 1.0];
/// Secondary buttons (Back to Menu): darker stone, normal text and behavior.
const DARK_TINT: [f32; 4] = [0.16, 0.16, 0.16, 1.0];

const HOVER_IN: f32 = 0.16;
const HOVER_OUT: f32 = 0.35;
const PRESS_DURATION: f32 = 0.06;

pub struct ButtonArt {
    pub textures: TextureSetId,
    pub size: [f32; 2],
    /// Hit shape: the base texture's alpha (not the glow mask).
    pub mask: AlphaMask,
}

/// Textures used by all menu screens, loaded once.
pub struct UiKit {
    pub button: ButtonArt,
    /// 1x1 white: flat-colored rectangles via material tint.
    pub white: TextureSetId,
    /// White fading to transparent left → right (underlines).
    pub fade_right: TextureSetId,
    pub knob: TextureSetId,
    pub brush: TextureSetId,
    pub ornament_divider: TextureSetId,
    /// Option selector arrow, pointing left (mirrored for the right arrow).
    pub arrow: TextureSetId,
}

impl UiKit {
    pub fn load(gpu: &GpuContext, renderer: &mut UiRenderer, ui_dir: &Path) -> Self {
        let (device, queue) = (gpu.device(), gpu.queue());
        let mut upload = |name: &str, image: &RgbaImage, glow: Option<&MaskImage>| {
            renderer.create_texture_set(device, queue, name, image, glow)
        };

        let button_image = load_color(
            &ui_dir.join("button_base.webp"),
            BUTTON_FALLBACK_SIZE,
            [88, 84, 80, 255],
        );
        let button_glow = load_mask(&ui_dir.join("button_glow_mask.webp"));
        let button = ButtonArt {
            textures: upload("menu button", &button_image, button_glow.as_ref()),
            size: [button_image.width as f32, button_image.height as f32],
            mask: AlphaMask::from_rgba(
                button_image.width,
                button_image.height,
                &button_image.pixels,
            ),
        };

        let knob_image = load_color(
            &ui_dir.join("slider_knob.webp"),
            [64, 64],
            [60, 200, 200, 255],
        );
        let knob_glow = load_mask(&ui_dir.join("slider_knob_glow.webp"));
        let knob = upload("slider knob", &knob_image, knob_glow.as_ref());

        let fade_pixels = (0..256u32)
            .flat_map(|x| [255, 255, 255, (255 - x) as u8])
            .collect();
        let fade = RgbaImage {
            width: 256,
            height: 1,
            pixels: fade_pixels,
        };

        Self {
            button,
            white: upload("white", &RgbaImage::solid(1, 1, [255; 4]), None),
            fade_right: upload("fade right", &fade, None),
            knob,
            brush: upload(
                "brush underline",
                &load_color(
                    &ui_dir.join("brush_underline.webp"),
                    [1, 1],
                    [196, 104, 40, 255],
                ),
                None,
            ),
            ornament_divider: upload(
                "ornament divider",
                &load_color(
                    &ui_dir.join("ornament_divider.webp"),
                    [1, 1],
                    [215, 193, 163, 255],
                ),
                None,
            ),
            arrow: upload(
                "selector arrow",
                &load_color(
                    &ui_dir.join("selector_arrow.webp"),
                    [1, 1],
                    [120, 84, 46, 255],
                ),
                None,
            ),
        }
    }
}

pub fn load_color(path: &Path, fallback_size: [u32; 2], fallback: [u8; 4]) -> RgbaImage {
    assets::load_rgba(path).unwrap_or_else(|error| {
        tracing::error!("{error}; using a flat placeholder");
        RgbaImage::solid(fallback_size[0], fallback_size[1], fallback)
    })
}

fn load_mask(path: &Path) -> Option<MaskImage> {
    assets::load_mask(path)
        .inspect_err(|error| tracing::error!("{error}; no glow for this element"))
        .ok()
}

/// Pushes a label onto the group layer, optionally with a soft drop shadow for
/// contrast on stone.
pub fn push_label(
    frame: &mut UiFrame,
    label: LabelId,
    rect: Rect,
    font_size: f32,
    color: [u8; 4],
    shadow: bool,
) {
    push_label_to(
        &mut frame.group_labels,
        label,
        rect,
        font_size,
        color,
        shadow,
    );
}

/// Like [`push_label`], into any label list (e.g. the overlay). The shadow fades with
/// the label's alpha.
pub fn push_label_to(
    labels: &mut Vec<LabelDraw>,
    label: LabelId,
    rect: Rect,
    font_size: f32,
    color: [u8; 4],
    shadow: bool,
) {
    if shadow {
        let offset = (font_size / 16.0).max(1.0).round();
        let mut shadow_color = palette::TEXT_SHADOW;
        shadow_color[3] = (u16::from(shadow_color[3]) * u16::from(color[3]) / 255) as u8;
        labels.push(LabelDraw {
            label,
            rect: Rect::new(rect.x + offset, rect.y + offset, rect.w, rect.h),
            font_size,
            color: shadow_color,
        });
    }
    labels.push(LabelDraw {
        label,
        rect,
        font_size,
        color,
    });
}

/// A stretched image (normalized `uv`, which may be mirrored with a negative width).
pub fn push_image(
    sprites: &mut Vec<Sprite>,
    textures: TextureSetId,
    rect: Rect,
    uv: Rect,
    material: SpriteMaterial,
) {
    sprites.push(Sprite {
        textures,
        shape: SpriteShape::Stretched { rect, uv },
        material,
    });
}

pub const FULL_UV: Rect = Rect::new(0.0, 0.0, 1.0, 1.0);

/// A flat-colored rectangle (sRGB RGBA).
pub fn push_flat(sprites: &mut Vec<Sprite>, kit: &UiKit, rect: Rect, color: [u8; 4]) {
    push_image(
        sprites,
        kit.white,
        rect,
        FULL_UV,
        SpriteMaterial::tinted(color),
    );
}

/// Converts a 1920x1080-reference rectangle to physical pixels, snapped to whole pixels.
pub fn scaled(scale: f32, x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(
        (x * scale).round(),
        (y * scale).round(),
        (w * scale).round(),
        (h * scale).round(),
    )
}

/// The 1920x1080 reference layout every menu screen is authored for.
pub const REFERENCE_SIZE: [f32; 2] = [1920.0, 1080.0];

/// Top-left of the scaled reference canvas inside the viewport. The canvas is centered,
/// so the authored layout stays centered in windows that are not exactly 16:9 (with the
/// reference coordinates alone the content would stick to the top-left corner).
pub fn canvas_origin(scale: f32, viewport: [f32; 2]) -> [f32; 2] {
    [
        ((viewport[0] - REFERENCE_SIZE[0] * scale) / 2.0)
            .max(0.0)
            .round(),
        ((viewport[1] - REFERENCE_SIZE[1] * scale) / 2.0)
            .max(0.0)
            .round(),
    ]
}

/// [`scaled`] shifted by `origin`: reference rect `x, y, w, h` on the centered canvas.
pub fn canvas_scaled(scale: f32, origin: [f32; 2], x: f32, y: f32, w: f32, h: f32) -> Rect {
    let mut rect = scaled(scale, x, y, w, h);
    rect.x += origin[0];
    rect.y += origin[1];
    rect
}

/// A plain rectangular button in the stone buttons' colors: their stone (cropped from
/// `button_base.webp`, without the ornaments) and label colors, inside a thin bronze
/// border that warms up on hover. A disabled button darkens the same way a disabled
/// stone button does.
pub struct FlatButton {
    label: LabelId,
    rect: Rect,
    pub enabled: bool,
    /// Size and label size in reference pixels (1920x1080), scaled with the UI; start
    /// as [`FLAT_BUTTON_SIZE`] / [`FLAT_BUTTON_TEXT_SIZE`]. Screens lay the button out
    /// with `size`; change them for a button that needs its own.
    pub size: [f32; 2],
    pub text_size: f32,
    hover: Tween,
    press: Tween,
}

/// Default size of a [`FlatButton`] (reference pixels) and of its label, as on the main
/// menu's Update button and the Credits screen's Back button. A button can be given its
/// own through its `size` / `text_size` fields.
pub const FLAT_BUTTON_SIZE: [f32; 2] = [220.0, 38.0];
pub const FLAT_BUTTON_TEXT_SIZE: f32 = 17.0;
/// sRGB border of a disabled [`FlatButton`] (enabled: [`palette::ROW_LINE`], warming
/// to [`palette::HEADING`] on hover).
const FLAT_BORDER_DISABLED: [u8; 4] = [60, 45, 27, 255];
/// Raises a [`FlatButton`] label by this fraction of its font size. Labels are centered
/// by their line box, which keeps room above the letters (for capitals and accents), so
/// the letters themselves sit low; measured on screen, this centers them.
const FLAT_LABEL_RAISE: f32 = 0.13;
/// The frame's pointed ends reach a little past the 9-slice borders into the stone;
/// the flat button's stone is taken this many texels further in.
const STONE_ORNAMENT_CLEARANCE: f32 = 60.0;

impl FlatButton {
    /// The label text is set at runtime with [`Self::set_text`].
    pub fn new(renderer: &mut UiRenderer) -> Self {
        Self {
            label: renderer.create_label("", TextStyle::CENTERED),
            rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            enabled: false,
            size: FLAT_BUTTON_SIZE,
            text_size: FLAT_BUTTON_TEXT_SIZE,
            hover: Tween::new(0.0),
            press: Tween::new(0.0),
        }
    }

    /// `rect` in physical pixels, normally `size` scaled.
    pub fn place(&mut self, rect: Rect) {
        self.rect = rect;
    }

    pub fn set_text(&self, renderer: &mut UiRenderer, text: &str) {
        renderer.set_label_text(self.label, text);
    }

    pub fn is_interactive(&self) -> bool {
        self.enabled
    }

    pub fn hit_target(&self, id: usize) -> HitTarget<'static> {
        HitTarget {
            id,
            shape: HitShape::Rectangle(self.rect),
            interactive: self.enabled,
        }
    }

    pub fn animate(&mut self, hovered: bool, pressed: bool, dt: f32) {
        let active = self.enabled && hovered;
        let (target, duration) = if active {
            (1.0, HOVER_IN)
        } else {
            (0.0, HOVER_OUT)
        };
        self.hover
            .animate_to(target, duration, Easing::EaseOutCubic);
        self.hover.update(dt);
        let press = if self.enabled && pressed { 1.0 } else { 0.0 };
        self.press
            .animate_to(press, PRESS_DURATION, Easing::EaseOutCubic);
        self.press.update(dt);
    }

    pub fn reset_animation(&mut self) {
        self.hover.set(0.0);
        self.press.set(0.0);
    }

    /// `scale`: the UI scale (physical pixels per reference pixel), for the label size.
    pub fn draw(&self, frame: &mut UiFrame, kit: &UiKit, scale: f32) {
        let rect = self.rect;
        let font_size = (self.text_size * scale).round();
        // Border in whole pixels, scaling with the button.
        let border = (rect.h / 26.0).round().max(1.0);
        let hover = self.hover.value() * (1.0 - 0.5 * self.press.value());
        let border_color = if self.enabled {
            mix_srgb(palette::ROW_LINE, palette::HEADING, hover)
        } else {
            FLAT_BORDER_DISABLED
        };
        let sprites = &mut frame.group_sprites;
        push_flat(sprites, kit, rect, border_color);
        let stone = inset(rect, border);
        let art = &kit.button;
        sprites.push(Sprite {
            textures: art.textures,
            shape: SpriteShape::Stretched {
                rect: stone,
                uv: stone_uv(art.size, [stone.w, stone.h]),
            },
            material: SpriteMaterial {
                press_amount: self.press.value(),
                disabled_amount: if self.enabled { 0.0 } else { 1.0 },
                ..SpriteMaterial::PLAIN
            },
        });
        let text = if self.enabled {
            palette::TEXT
        } else {
            palette::TEXT_DISABLED
        };
        let raise = (font_size * FLAT_LABEL_RAISE).round();
        let label_rect = Rect::new(rect.x, rect.y - raise, rect.w, rect.h);
        push_label(frame, self.label, label_rect, font_size, text, true);
    }
}

/// `rect` shrunk by `by` on every side.
fn inset(rect: Rect, by: f32) -> Rect {
    Rect::new(
        rect.x + by,
        rect.y + by,
        (rect.w - 2.0 * by).max(0.0),
        (rect.h - 2.0 * by).max(0.0),
    )
}

/// Texture rectangle of plain stone in the button art (inside the frame, clear of the
/// end ornaments), centered and cropped to the aspect of `size` so the stone is not
/// stretched.
fn stone_uv(art_size: [f32; 2], size: [f32; 2]) -> Rect {
    let left = BUTTON_BORDERS.left + STONE_ORNAMENT_CLEARANCE;
    let right = BUTTON_BORDERS.right + STONE_ORNAMENT_CLEARANCE;
    let plain = [
        art_size[0] - left - right,
        art_size[1] - BUTTON_BORDERS.top - BUTTON_BORDERS.bottom,
    ];
    let aspect = size[0] / size[1];
    if !(aspect.is_finite() && aspect > 0.0 && plain[0] > 0.0 && plain[1] > 0.0) {
        return FULL_UV;
    }
    let [w, h] = if plain[0] / plain[1] > aspect {
        [plain[1] * aspect, plain[1]]
    } else {
        [plain[0], plain[0] / aspect]
    };
    let x = left + (plain[0] - w) / 2.0;
    let y = BUTTON_BORDERS.top + (plain[1] - h) / 2.0;
    Rect::new(
        x / art_size[0],
        y / art_size[1],
        w / art_size[0],
        h / art_size[1],
    )
}

/// Blends two sRGB colors (`t` = 0 → `a`, 1 → `b`).
fn mix_srgb(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| {
        (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8
    })
}

/// Default size of a [`StoneButton`] (reference pixels, 1920x1080), of its label, and
/// the gap between stacked buttons: the main menu's buttons. Every screen uses these;
/// a button can be given its own through its `size` / `text_size` fields. Lower than the art's natural aspect (1410x342 →
/// 264x64): the 9-slice keeps the end ornaments' proportions (they scale with the
/// height) and stretches only the stone.
pub const STONE_BUTTON_SIZE: [f32; 2] = [336.0, 64.0];
pub const STONE_BUTTON_TEXT_SIZE: f32 = 26.0;
pub const STONE_BUTTON_GAP: f32 = 18.0;

/// The textured stone button with a crystal glow on hover.
pub struct StoneButton {
    label: LabelId,
    /// English text: the translation key for [`Self::relabel`].
    text: &'static str,
    layout: NineSliceLayout,
    /// Size and label size in reference pixels (1920x1080), scaled with the UI; start
    /// as [`STONE_BUTTON_SIZE`] / [`STONE_BUTTON_TEXT_SIZE`]. Screens lay the button out
    /// with `size`; change them for a button that needs its own.
    pub size: [f32; 2],
    pub text_size: f32,
    pub enabled: bool,
    /// Current tab: drawn darkened with unlit crystals, not clickable.
    pub selected: bool,
    /// Secondary style: darker stone.
    pub dark: bool,
    hover: Tween,
    press: Tween,
}

impl StoneButton {
    pub fn new(renderer: &mut UiRenderer, text: &'static str, enabled: bool) -> Self {
        Self {
            label: renderer.create_label(text, TextStyle::CENTERED),
            text,
            layout: NineSliceLayout::new(Rect::new(0.0, 0.0, 0.0, 0.0), [1.0, 1.0], BUTTON_BORDERS),
            size: STONE_BUTTON_SIZE,
            text_size: STONE_BUTTON_TEXT_SIZE,
            enabled,
            selected: false,
            dark: false,
            hover: Tween::new(0.0),
            press: Tween::new(0.0),
        }
    }

    /// `rect` in physical pixels, normally `size` scaled.
    pub fn place(&mut self, rect: Rect, art: &ButtonArt) {
        self.layout = NineSliceLayout::new(rect, art.size, BUTTON_BORDERS);
    }

    /// Shows the label in `language`.
    pub fn relabel(&self, renderer: &mut UiRenderer, language: Language) {
        renderer.set_label_text(self.label, language.tr(self.text));
    }

    pub fn is_interactive(&self) -> bool {
        self.enabled && !self.selected
    }

    pub fn hit_target<'a>(&'a self, id: usize, art: &'a ButtonArt) -> HitTarget<'a> {
        HitTarget {
            id,
            shape: HitShape::Alpha {
                layout: &self.layout,
                mask: &art.mask,
                threshold: DEFAULT_ALPHA_THRESHOLD,
            },
            interactive: self.is_interactive(),
        }
    }

    pub fn animate(&mut self, hovered: bool, pressed: bool, dt: f32) {
        let hovered = hovered && self.is_interactive();
        if hovered {
            self.hover.animate_to(1.0, HOVER_IN, Easing::EaseOutCubic);
        } else {
            self.hover.animate_to(0.0, HOVER_OUT, Easing::EaseOutCubic);
        }
        let press = if pressed && hovered { 1.0 } else { 0.0 };
        self.press.animate_to(press, PRESS_DURATION, Easing::Linear);
        self.hover.update(dt);
        self.press.update(dt);
    }

    pub fn reset_animation(&mut self) {
        self.hover.set(0.0);
        self.press.set(0.0);
    }

    /// `scale`: the UI scale (physical pixels per reference pixel), for the label size.
    pub fn draw(&self, frame: &mut UiFrame, art: &ButtonArt, scale: f32) {
        self.draw_to(
            &mut frame.group_sprites,
            &mut frame.group_labels,
            art,
            scale,
            1.0,
        );
    }

    /// Draws into the given layer; `opacity` fades both the stone and the label.
    pub fn draw_to(
        &self,
        sprites: &mut Vec<Sprite>,
        labels: &mut Vec<LabelDraw>,
        art: &ButtonArt,
        scale: f32,
        opacity: f32,
    ) {
        let font_size = (self.text_size * scale).round();
        let tint = if self.selected {
            SELECTED_TINT
        } else if self.dark {
            DARK_TINT
        } else {
            [1.0; 4]
        };
        // The selected tab is not interactive, so its crystals never light up.
        let hover_amount = if self.selected {
            0.0
        } else {
            self.hover.value()
        };
        sprites.push(Sprite {
            textures: art.textures,
            shape: SpriteShape::NineSlice(self.layout),
            material: SpriteMaterial {
                hover_amount,
                press_amount: self.press.value(),
                disabled_amount: if self.enabled { 0.0 } else { 1.0 },
                opacity,
                glow_color: BUTTON_GLOW_COLOR,
                glow_intensity: BUTTON_GLOW_INTENSITY,
                tint,
                ..SpriteMaterial::PLAIN
            },
        });
        let mut color = if self.selected {
            palette::TEXT_SELECTED
        } else if self.enabled {
            palette::TEXT
        } else {
            palette::TEXT_DISABLED
        };
        color[3] = (f32::from(color[3]) * opacity).round() as u8;
        push_label_to(
            labels,
            self.label,
            self.layout.rect(),
            font_size,
            color,
            !self.selected,
        );
    }
}

/// A horizontal value slider (0–[`MAX_VOLUME`]) with a crystal knob.
pub struct Slider {
    track: Rect,
    knob_size: f32,
    hover: Tween,
}

impl Default for Slider {
    fn default() -> Self {
        Self {
            track: Rect::new(0.0, 0.0, 0.0, 0.0),
            knob_size: 0.0,
            hover: Tween::new(0.0),
        }
    }
}

impl Slider {
    pub fn place(&mut self, track: Rect, knob_size: f32) {
        self.track = track;
        self.knob_size = knob_size;
    }

    /// The track plus room for the knob, so it is easy to grab.
    pub fn hit_rect(&self) -> Rect {
        let pad = self.knob_size / 2.0;
        let center_y = self.track.y + self.track.h / 2.0;
        Rect::new(
            self.track.x - pad,
            center_y - pad,
            self.track.w + 2.0 * pad,
            self.knob_size,
        )
    }

    pub fn hit_target(&self, id: usize) -> HitTarget<'static> {
        HitTarget {
            id,
            shape: HitShape::Rectangle(self.hit_rect()),
            interactive: true,
        }
    }

    pub fn value_at(&self, x: f32) -> u8 {
        slider_value(self.track, x)
    }

    /// `active` while hovered or dragged: the gem glows.
    pub fn animate(&mut self, active: bool, dt: f32) {
        if active {
            self.hover.animate_to(1.0, HOVER_IN, Easing::EaseOutCubic);
        } else {
            self.hover.animate_to(0.0, HOVER_OUT, Easing::EaseOutCubic);
        }
        self.hover.update(dt);
    }

    pub fn reset_animation(&mut self) {
        self.hover.set(0.0);
    }

    pub fn draw(&self, sprites: &mut Vec<Sprite>, kit: &UiKit, value: u8) {
        let t = f32::from(value.min(MAX_VOLUME)) / f32::from(MAX_VOLUME);
        let knob_x = (self.track.x + self.track.w * t).round();
        let track = self.track;
        push_flat(
            sprites,
            kit,
            Rect::new(track.x, track.y, knob_x - track.x, track.h),
            palette::SLIDER_FILL,
        );
        push_flat(
            sprites,
            kit,
            Rect::new(knob_x, track.y, track.x + track.w - knob_x, track.h),
            palette::SLIDER_REST,
        );
        let size = self.knob_size;
        let knob = Rect::new(
            (knob_x - size / 2.0).round(),
            (track.y + track.h / 2.0 - size / 2.0).round(),
            size,
            size,
        );
        push_image(
            sprites,
            kit.knob,
            knob,
            FULL_UV,
            SpriteMaterial {
                hover_amount: self.hover.value(),
                glow_color: KNOB_GLOW_COLOR,
                glow_intensity: KNOB_GLOW_INTENSITY,
                ..SpriteMaterial::PLAIN
            },
        );
    }
}

/// Arrow sides of an [`OptionSelector`].
pub const ARROW_LEFT: usize = 0;
pub const ARROW_RIGHT: usize = 1;

/// What an [`OptionSelector`] shows this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectorState {
    /// The highlighted segment (`None` for values outside the list or inactive rows).
    pub selected: Option<usize>,
    /// Number of choices, one segment each.
    pub count: usize,
    /// Which arrows can still move.
    pub enabled: [bool; 2],
    /// Value text color (normal, pending in gold, or disabled).
    pub value_color: [u8; 4],
}

/// Left/right arrows around a value, with one segment per choice underneath.
pub struct OptionSelector {
    value: LabelId,
    shown: Option<String>,
    arrows: [Rect; 2],
    value_rect: Rect,
    segments: Rect,
    hover: [Tween; 2],
}

impl OptionSelector {
    pub fn new(renderer: &mut UiRenderer) -> Self {
        Self::with_label(renderer.create_label("", TextStyle::CENTERED))
    }

    fn with_label(value: LabelId) -> Self {
        let empty = Rect::new(0.0, 0.0, 0.0, 0.0);
        Self {
            value,
            shown: None,
            arrows: [empty; 2],
            value_rect: empty,
            segments: empty,
            hover: [Tween::new(0.0); 2],
        }
    }

    /// `area` spans both arrows, vertically centered in it; the value text sits above
    /// the segment strip.
    pub fn place(&mut self, area: Rect, scale: f32) {
        let arrow_w = (22.0 * scale).round();
        let arrow_h = (34.0 * scale).round();
        let arrow_y = area.y + ((area.h - arrow_h) / 2.0).round();
        self.arrows = [
            Rect::new(area.x, arrow_y, arrow_w, arrow_h),
            Rect::new(area.x + area.w - arrow_w, arrow_y, arrow_w, arrow_h),
        ];
        let inset = arrow_w + (8.0 * scale).round();
        let inner_w = area.w - 2.0 * inset;
        let bands = value_bands(area, scale);
        self.value_rect = Rect::new(area.x + inset, bands.text.y, inner_w, bands.text.h);
        self.segments = Rect::new(area.x + inset, bands.strip.y, inner_w, bands.strip.h);
    }

    /// Hit areas for the two arrows (padded to be easy to click). Arrows at the end of
    /// the range are not interactive.
    pub fn hit_targets(&self, ids: [usize; 2], enabled: [bool; 2]) -> [HitTarget<'static>; 2] {
        std::array::from_fn(|side| {
            let arrow = self.arrows[side];
            let pad = (arrow.w * 0.4).round();
            HitTarget {
                id: ids[side],
                shape: HitShape::Rectangle(Rect::new(
                    arrow.x - pad,
                    arrow.y,
                    arrow.w + 2.0 * pad,
                    arrow.h,
                )),
                interactive: enabled[side],
            }
        })
    }

    pub fn animate(&mut self, hovered: [bool; 2], dt: f32) {
        for (tween, hovered) in self.hover.iter_mut().zip(hovered) {
            if hovered {
                tween.animate_to(1.0, HOVER_IN, Easing::EaseOutCubic);
            } else {
                tween.animate_to(0.0, HOVER_OUT, Easing::EaseOutCubic);
            }
            tween.update(dt);
        }
    }

    pub fn reset_animation(&mut self) {
        for tween in &mut self.hover {
            tween.set(0.0);
        }
    }

    /// Updates the value text; the label is re-shaped only when it changed.
    pub fn set_text(&mut self, renderer: &mut UiRenderer, text: &str) {
        if self.shown.as_deref() != Some(text) {
            renderer.set_label_text(self.value, text);
            self.shown = Some(text.to_owned());
        }
    }

    pub fn draw(&self, frame: &mut UiFrame, kit: &UiKit, font_size: f32, state: SelectorState) {
        let SelectorState {
            selected,
            count,
            enabled,
            value_color,
        } = state;
        for side in [ARROW_LEFT, ARROW_RIGHT] {
            let uv = if side == ARROW_LEFT {
                FULL_UV
            } else {
                Rect::new(1.0, 0.0, -1.0, 1.0)
            };
            let tint = if enabled[side] {
                let brightness = 1.0 + 0.8 * self.hover[side].value();
                [brightness, brightness, brightness, 1.0]
            } else {
                [1.0, 1.0, 1.0, 0.3]
            };
            push_image(
                &mut frame.group_sprites,
                kit.arrow,
                self.arrows[side],
                uv,
                SpriteMaterial {
                    tint,
                    ..SpriteMaterial::PLAIN
                },
            );
        }
        if count > MAX_SEGMENTS {
            // Long lists (resolutions, refresh rates): one strip with a position marker.
            push_flat(
                &mut frame.group_sprites,
                kit,
                self.segments,
                palette::SEGMENT_OFF,
            );
            if let Some(marker) =
                selected.and_then(|index| marker_rect(self.segments, count, index))
            {
                push_flat(&mut frame.group_sprites, kit, marker, palette::SEGMENT_ON);
            }
        } else {
            let gap = (4.0 * self.segments.h / 6.0).round().max(2.0);
            for (index, rect) in segment_rects(self.segments, count, gap).enumerate() {
                let color = if selected == Some(index) {
                    palette::SEGMENT_ON
                } else {
                    palette::SEGMENT_OFF
                };
                push_flat(&mut frame.group_sprites, kit, rect, color);
            }
        }
        push_label(
            frame,
            self.value,
            self.value_rect,
            font_size,
            value_color,
            false,
        );
    }
}

/// The two horizontal bands of a selector in `area`: the value text on top and the
/// segment strip along the bottom.
struct ValueBands {
    text: Rect,
    strip: Rect,
}

fn value_bands(area: Rect, scale: f32) -> ValueBands {
    let strip_h = (6.0 * scale).round().max(2.0);
    let gap = (3.0 * scale).round();
    ValueBands {
        text: Rect::new(area.x, area.y, area.w, area.h - strip_h - gap),
        strip: Rect::new(area.x, area.y + area.h - strip_h, area.w, strip_h),
    }
}

/// Above this many choices a selector draws a strip with a marker instead of segments.
const MAX_SEGMENTS: usize = 8;

/// The marker for choice `index` of `count` on a strip: an equal share of the width, at
/// least as wide as the strip is tall. `None` for an index outside the list.
pub fn marker_rect(strip: Rect, count: usize, index: usize) -> Option<Rect> {
    if index >= count {
        return None;
    }
    let width = (strip.w / count as f32).max(strip.h * 2.0).min(strip.w);
    let travel = strip.w - width;
    let t = if count > 1 {
        index as f32 / (count - 1) as f32
    } else {
        0.0
    };
    let left = (strip.x + travel * t).round();
    Some(Rect::new(left, strip.y, width.round(), strip.h))
}

/// `count` equal segments across `area` separated by `gap`, snapped to whole pixels.
pub fn segment_rects(area: Rect, count: usize, gap: f32) -> impl Iterator<Item = Rect> {
    let width = if count > 0 {
        (area.w - gap * (count as f32 - 1.0)) / count as f32
    } else {
        0.0
    };
    (0..count).map(move |index| {
        let start = area.x + index as f32 * (width + gap);
        let (left, right) = (start.round(), (start + width).round());
        Rect::new(left, area.y, right - left, area.h)
    })
}

/// Which of the two dialog buttons.
pub const DIALOG_CONFIRM: usize = 0;
pub const DIALOG_CANCEL: usize = 1;

/// Wide enough for two default-size stone buttons and the gap between them.
const DIALOG_PANEL: [f32; 4] = [560.0, 380.0, 800.0, 320.0];
const DIALOG_BUTTON_GAP: f32 = 40.0;
const _: () = assert!(2.0 * STONE_BUTTON_SIZE[0] + DIALOG_BUTTON_GAP < DIALOG_PANEL[2]);
const DIALOG_FADE: f32 = 0.15;
/// Backdrop that dims everything behind a dialog.
const DIALOG_BACKDROP: [u8; 4] = [0, 0, 0, 150];
const DIALOG_FRAME: [u8; 4] = [120, 88, 52, 255];
const DIALOG_FILL: [u8; 4] = [16, 13, 11, 250];

/// A modal dialog on the overlay layer: dimmed backdrop, framed panel, title, message,
/// and two stone buttons (confirm on the left, cancel on the right).
pub struct Dialog {
    title: LabelId,
    /// English title: the translation key for [`Self::relabel`].
    title_text: &'static str,
    message: LabelId,
    message_text: Option<String>,
    buttons: [StoneButton; 2],
    panel: Rect,
    title_rect: Rect,
    divider: Rect,
    message_rect: Rect,
    scale: f32,
    viewport: [f32; 2],
    opacity: Tween,
}

impl Dialog {
    pub fn new(renderer: &mut UiRenderer, title: &'static str, buttons: [&'static str; 2]) -> Self {
        let empty = Rect::new(0.0, 0.0, 0.0, 0.0);
        let bold = TextStyle {
            bold: true,
            align: TextAlign::Center,
        };
        Self {
            title: renderer.create_label(title, bold),
            title_text: title,
            message: renderer.create_label("", TextStyle::CENTERED),
            message_text: None,
            buttons: buttons.map(|text| StoneButton::new(renderer, text, true)),
            panel: empty,
            title_rect: empty,
            divider: empty,
            message_rect: empty,
            scale: 1.0,
            viewport: [0.0; 2],
            opacity: Tween::new(0.0),
        }
    }

    /// Centers the panel in the viewport (the reference layout is centered itself).
    pub fn layout(&mut self, scale: f32, viewport: [f32; 2], kit: &UiKit) {
        self.scale = scale;
        self.viewport = viewport;
        let origin = canvas_origin(scale, viewport);
        let at = |x, y, w, h| canvas_scaled(scale, origin, x, y, w, h);
        let [x, y, w, h] = DIALOG_PANEL;
        self.panel = at(x, y, w, h);
        self.title_rect = at(x, y + 34.0, w, 48.0);
        self.divider = at(x + w / 2.0 - 150.0, y + 88.0, 300.0, 22.0);
        self.message_rect = at(x, y + 122.0, w, 40.0);
        let sizes = self.buttons.each_ref().map(|button| button.size);
        let mut bx = x + (w - sizes[0][0] - sizes[1][0] - DIALOG_BUTTON_GAP) / 2.0;
        for (button, [bw, bh]) in self.buttons.iter_mut().zip(sizes) {
            button.place(at(bx, y + h - bh - 36.0, bw, bh), &kit.button);
            bx += bw + DIALOG_BUTTON_GAP;
        }
    }

    /// Starts fading the dialog in.
    pub fn show(&mut self) {
        self.opacity.set(0.0);
        self.opacity
            .animate_to(1.0, DIALOG_FADE, Easing::EaseOutCubic);
        for button in &mut self.buttons {
            button.reset_animation();
        }
    }

    /// Shows the title and buttons in `language` (the owner sets the message).
    pub fn relabel(&self, renderer: &mut UiRenderer, language: Language) {
        renderer.set_label_text(self.title, language.tr(self.title_text));
        for button in &self.buttons {
            button.relabel(renderer, language);
        }
    }

    pub fn set_message(&mut self, renderer: &mut UiRenderer, text: &str) {
        if self.message_text.as_deref() != Some(text) {
            renderer.set_label_text(self.message, text);
            self.message_text = Some(text.to_owned());
        }
    }

    /// Hit test for the two buttons; ids are `base_id + DIALOG_CONFIRM / DIALOG_CANCEL`.
    /// Everything behind the dialog is blocked by the caller.
    pub fn hit_test(&self, x: f32, y: f32, kit: &UiKit, base_id: usize) -> Hit {
        let targets = self
            .buttons
            .iter()
            .enumerate()
            .map(|(index, button)| button.hit_target(base_id + index, &kit.button));
        topmost_hit(targets, x, y)
    }

    pub fn animate(
        &mut self,
        hovered: Option<usize>,
        pressed: Option<usize>,
        base_id: usize,
        dt: f32,
    ) {
        self.opacity.update(dt);
        for (index, button) in self.buttons.iter_mut().enumerate() {
            let id = Some(base_id + index);
            button.animate(hovered == id, pressed == id, dt);
        }
    }

    pub fn draw(&self, frame: &mut UiFrame, kit: &UiKit) {
        let opacity = self.opacity.value();
        let scale = self.scale;
        let fade = |color: [u8; 4]| {
            let mut color = color;
            color[3] = (f32::from(color[3]) * opacity).round() as u8;
            color
        };
        let [width, height] = self.viewport;
        let sprites = &mut frame.overlay_sprites;
        push_flat(
            sprites,
            kit,
            Rect::new(0.0, 0.0, width, height),
            fade(DIALOG_BACKDROP),
        );
        let border = (2.0 * scale).round().max(1.0);
        let panel = self.panel;
        push_flat(
            sprites,
            kit,
            Rect::new(
                panel.x - border,
                panel.y - border,
                panel.w + 2.0 * border,
                panel.h + 2.0 * border,
            ),
            fade(DIALOG_FRAME),
        );
        push_flat(sprites, kit, panel, fade(DIALOG_FILL));
        push_image(
            sprites,
            kit.ornament_divider,
            self.divider,
            FULL_UV,
            SpriteMaterial {
                opacity,
                ..SpriteMaterial::PLAIN
            },
        );
        let labels = &mut frame.overlay_labels;
        push_label_to(
            labels,
            self.title,
            self.title_rect,
            (30.0 * scale).round(),
            fade(palette::TEXT),
            false,
        );
        push_label_to(
            labels,
            self.message,
            self.message_rect,
            (22.0 * scale).round(),
            fade(palette::TEXT),
            false,
        );
        for button in &self.buttons {
            button.draw_to(
                &mut frame.overlay_sprites,
                &mut frame.overlay_labels,
                &kit.button,
                scale,
                opacity,
            );
        }
    }
}

/// Maps a pointer x to a slider value, clamped to the track.
pub fn slider_value(track: Rect, x: f32) -> u8 {
    if track.w <= 0.0 {
        return 0;
    }
    let t = ((x - track.x) / track.w).clamp(0.0, 1.0);
    (t * f32::from(MAX_VOLUME)).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_button_stone_is_plain_and_unstretched() {
        let art = [1410.0, 342.0];
        for size in [[208.0, 28.0], [60.0, 40.0], [500.0, 20.0]] {
            let uv = stone_uv(art, size);
            // Inside the frame and between the ornaments.
            let clear = STONE_ORNAMENT_CLEARANCE;
            assert!(uv.x * art[0] >= BUTTON_BORDERS.left + clear - 1e-3);
            assert!((uv.x + uv.w) * art[0] <= art[0] - BUTTON_BORDERS.right - clear + 1e-3);
            assert!(uv.y * art[1] >= BUTTON_BORDERS.top - 1e-3);
            assert!((uv.y + uv.h) * art[1] <= art[1] - BUTTON_BORDERS.bottom + 1e-3);
            // Same shape as the button: no stretching.
            let shape = (uv.w * art[0]) / (uv.h * art[1]);
            assert!((shape - size[0] / size[1]).abs() < 1e-3, "{size:?}");
        }
        assert_eq!(stone_uv(art, [10.0, 0.0]), FULL_UV);
        assert_eq!(inset(Rect::new(0.0, 0.0, 3.0, 3.0), 2.0).w, 0.0);
    }

    #[test]
    fn slider_value_maps_and_clamps() {
        let track = Rect::new(100.0, 0.0, 200.0, 10.0);
        assert_eq!(slider_value(track, 100.0), 0);
        assert_eq!(slider_value(track, 200.0), 50);
        assert_eq!(slider_value(track, 300.0), 100);
        assert_eq!(slider_value(track, 5.0), 0);
        assert_eq!(slider_value(track, 999.0), 100);
        assert_eq!(slider_value(track, 101.0), 1, "rounds to nearest");
        assert_eq!(slider_value(Rect::new(0.0, 0.0, 0.0, 0.0), 10.0), 0);
    }

    #[test]
    fn slider_hit_area_covers_the_knob_at_both_ends() {
        let mut slider = Slider::default();
        slider.place(Rect::new(100.0, 50.0, 200.0, 10.0), 30.0);
        let HitShape::Rectangle(area) = slider.hit_target(0).shape else {
            panic!("slider uses a rectangle");
        };
        assert!(area.contains(86.0, 55.0), "left knob half");
        assert!(area.contains(314.0, 55.0), "right knob half");
        assert!(area.contains(200.0, 41.0) && !area.contains(200.0, 39.0));
    }

    #[test]
    fn segments_fill_the_strip_with_gaps() {
        let rects: Vec<Rect> = segment_rects(Rect::new(100.0, 10.0, 180.0, 6.0), 5, 4.0).collect();
        assert_eq!(rects.len(), 5);
        assert_eq!(rects[0], Rect::new(100.0, 10.0, 33.0, 6.0));
        assert_eq!(rects[1].x, 137.0);
        let last = rects[4];
        assert_eq!(
            last.x + last.w,
            280.0,
            "last segment ends at the strip's right edge"
        );
        assert_eq!(
            segment_rects(Rect::new(0.0, 0.0, 10.0, 1.0), 0, 2.0).count(),
            0
        );
    }

    #[test]
    fn canvas_origin_centers_the_reference_layout() {
        assert_eq!(canvas_origin(1.0, [1920.0, 1080.0]), [0.0, 0.0]);
        // Wider than 16:9: the leftover width is split evenly.
        assert_eq!(canvas_origin(1.0, [2200.0, 1080.0]), [140.0, 0.0]);
        // Taller than 16:9.
        assert_eq!(canvas_origin(1.0, [1920.0, 1300.0]), [0.0, 110.0]);
        // With the uniform scale the canvas fills the limiting axis exactly: a window
        // like 1911x853 is height-limited, so all the slack goes to the sides.
        let scale = 853.0 / 1080.0;
        assert_eq!(canvas_origin(scale, [1911.0, 853.0]), [197.0, 0.0]);
        // The canvas never shifts into the negative (a viewport smaller than the
        // smallest supported resolution just clips).
        assert_eq!(canvas_origin(1.0, [640.0, 360.0]), [0.0, 0.0]);
    }

    #[test]
    fn value_bands_put_text_above_the_strip() {
        let bands = value_bands(Rect::new(10.0, 100.0, 350.0, 40.0), 1.0);
        assert_eq!(bands.strip, Rect::new(10.0, 134.0, 350.0, 6.0));
        assert_eq!(bands.text, Rect::new(10.0, 100.0, 350.0, 31.0));
        assert!(
            bands.text.y + bands.text.h < bands.strip.y,
            "a gap between them"
        );
    }

    #[test]
    fn long_lists_use_a_marker_that_travels_the_strip() {
        let strip = Rect::new(100.0, 10.0, 200.0, 6.0);
        let first = marker_rect(strip, 20, 0).unwrap();
        let last = marker_rect(strip, 20, 19).unwrap();
        assert_eq!(first.x, 100.0);
        assert_eq!(
            last.x + last.w,
            300.0,
            "last marker ends at the strip's end"
        );
        assert_eq!(first.w, 12.0, "at least twice the strip height");
        assert_eq!(marker_rect(strip, 20, 20), None);
        assert_eq!(marker_rect(strip, 1, 0), Some(strip));
    }

    #[test]
    fn selector_layout_keeps_arrows_at_the_ends() {
        let mut selector = OptionSelector::with_label(LabelId::for_tests());
        selector.place(Rect::new(1000.0, 200.0, 350.0, 40.0), 1.0);
        assert_eq!(selector.arrows[0].x, 1000.0);
        assert_eq!(selector.arrows[1].x + selector.arrows[1].w, 1350.0);
        assert!(selector.segments.x > selector.arrows[0].x + selector.arrows[0].w);
        assert_eq!(selector.segments.y + selector.segments.h, 240.0);
        let [left, right] = selector.hit_targets([7, 8], [false, true]);
        assert!(!left.interactive && right.interactive);
        assert_eq!((left.id, right.id), (7, 8));
    }

    #[test]
    fn scaled_rects_snap_to_pixels() {
        assert_eq!(
            scaled(0.8333, 110.0, 430.0, 336.0, 80.0),
            Rect::new(92.0, 358.0, 280.0, 67.0)
        );
    }
}
