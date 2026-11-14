//! Settings screen: Audio / Graphics / Display tabs and the Back to Menu / Reset / Apply /
//! Default row.
//!
//! Each tab shows a scrolling list of rows grouped under section headings (mouse wheel or
//! the scrollbar on the right). Graphics is split into two pages, switched on the header
//! line: Basic (the sections most players adjust) and Advanced (fine-tuning).
//!
//! Every control edits one draft of the whole [`Settings`]; values that differ from the
//! applied ones are shown in gold. Apply commits the draft (the application saves it and
//! applies display changes), Reset discards unapplied changes on all tabs, Default
//! restores the current tab's defaults (still to be applied).
//!
//! Reset, Apply and Default each ask for confirmation first.
//!
//! Safety nets: a change of window mode, monitor, size or fullscreen refresh rate must be
//! confirmed within [`CONFIRM_SECONDS`] or it is reverted, and leaving with unapplied
//! changes asks whether to apply or discard them. Such a change is applied at once but
//! saved only when kept, so quitting during the countdown keeps the last confirmed
//! settings. The countdown is a wall-clock deadline, independent of frame timing.
//!
//! Texts follow the applied interface language (Display Language): changing it in the
//! draft switches the menus on Apply, like every other setting.
//!
//! What takes effect now: display mode, monitor, resolution, refresh rate (exclusive
//! fullscreen), V-Sync, the frame rate limits, the FPS counter, brightness, gamma and the
//! HUD scale. Audio volumes, Aspect Ratio and the graphics options are stored for the
//! audio playback and the 3D scene that are still to come. HDR is listed as not
//! supported, and the upscaler list stays at Off until an upscaler is implemented.

use std::time::{Duration, Instant};

use super::widgets::{
    self, ARROW_LEFT, ARROW_RIGHT, DIALOG_CANCEL, DIALOG_CONFIRM, Dialog, FULL_UV, OptionSelector,
    STONE_BUTTON_GAP, STONE_BUTTON_SIZE, SelectorState, Slider, StoneButton, UiKit, palette,
    scaled,
};
use crate::engine::animation::{Easing, Tween};
use crate::engine::render::ui_renderer::{
    LabelId, SpriteMaterial, TextAlign, TextStyle, UiFrame, UiRenderer,
};
use crate::engine::ui::geometry::Rect;
use crate::engine::ui::hit::{Hit, HitShape, HitTarget, topmost_hit};
use crate::game::locale::Language;
use crate::game::settings::display::{
    MAX_BRIGHTNESS, MAX_GAMMA, MAX_UI_SCALE, MIN_GAMMA, MIN_UI_SCALE, fits_windowed, monitor_label,
    refresh_label, resolve_refresh, snap_ui_scale,
};
use crate::game::settings::graphics::MAX_SHARPENING;
use crate::game::settings::{
    AspectRatio, AudioChannel, BackgroundFrameRateLimit, DisplayInfo, DisplayMode, DisplaySettings,
    FrameRateLimit, GraphicsOption, MAX_VOLUME, OptionValue, Quality, Settings,
};

// Reference layout (1920x1080).
const LEFT: f32 = 230.0;
const TOP: f32 = 130.0;
const PANEL_LEFT: f32 = 610.0;
const HEADER: [f32; 4] = [PANEL_LEFT, 118.0, 760.0, 48.0];
const BRUSH: [f32; 4] = [PANEL_LEFT - 6.0, 166.0, 520.0, 14.0];
/// Basic / Advanced switch on the header line, right-aligned with the selectors.
const PAGE_SWITCH: [[f32; 4]; 2] = [[1400.0, 122.0, 120.0, 40.0], [1540.0, 122.0, 150.0, 40.0]];
/// The scrolling list's visible area (top and bottom).
const LIST_TOP: f32 = 200.0;
const LIST_BOTTOM: f32 = 912.0;
const LIST_HEIGHT: f32 = LIST_BOTTOM - LIST_TOP;
const ROW_PITCH: f32 = 56.0;
/// Section headings get extra space above them.
const HEADING_PITCH: f32 = 60.0;
const HEADING_TEXT_OFFSET: f32 = 16.0;
/// Row name text height; the underline sits just below it.
const ROW_TEXT_HEIGHT: f32 = 32.0;
const ROW_HEIGHT: f32 = 40.0;
const ROW_NAME_WIDTH: f32 = 380.0;
const UNDERLINE: [f32; 2] = [380.0, 3.0];
const SELECTOR: [f32; 2] = [1340.0, 350.0]; // x, width
/// Slider rows fill the selector block: the bar starts at the left arrow's edge and the
/// value ends at the right arrow's edge.
const SLIDER_TRACK_HEIGHT: f32 = 14.0;
const KNOB_SIZE: f32 = 30.0;
const SLIDER_VALUE_WIDTH: f32 = 64.0;
/// Room between the bar's end and the value, so the knob at 100 % stays clear of it.
const SLIDER_VALUE_GAP: f32 = 18.0;
/// Vertical center of a slider within its row (reference px from the row top): halfway
/// between the strip of the row above (bottom at `ROW_HEIGHT - ROW_PITCH` = -16) and the
/// arrows of the row below (top at `ROW_PITCH + 3` = 59), so both gaps are equal.
const SLIDER_CENTER: f32 = 22.0;
const SCROLLBAR: [f32; 2] = [1746.0, 8.0]; // x, width
/// Extra grab area on each side of the thin scrollbar.
const SCROLLBAR_GRAB: f32 = 10.0;
const SCROLLBAR_MIN_THUMB: f32 = 48.0;
/// Vertical center of the bottom buttons (Back, Reset, Apply, Default) and their left
/// edges. Tabs and buttons have the stone button's own size and gap.
const BOTTOM_CENTER: f32 = 986.0;
const BOTTOM_X: [f32; 4] = [LEFT, PANEL_LEFT, 982.0, 1354.0];
const HEADER_TEXT_SIZE: f32 = 30.0;
const ROW_TEXT_SIZE: f32 = 22.0;
const HEADING_TEXT_SIZE: f32 = 20.0;
const PAGE_SWITCH_TEXT_SIZE: f32 = 22.0;
/// One mouse-wheel notch scrolls this far (reference px).
pub const WHEEL_LINE: f32 = 84.0;
const SCROLL_DURATION: f32 = 0.18;
const HOVER_DURATION: f32 = 0.15;
/// Seconds to confirm a window change before it is reverted.
pub const CONFIRM_SECONDS: f32 = 15.0;
/// Shown for features the engine does not support.
const NOT_SUPPORTED: &str = "Not Supported";
const UNAPPLIED_MESSAGE: &str = "Some settings have not been applied.";

// Widget ids for hit testing.
const TAB_ID: usize = 0;
const BOTTOM_ID: usize = 10;
/// Basic / Advanced: `PAGE_ID + 0 / 1`.
const PAGE_ID: usize = 20;
const SCROLLBAR_ID: usize = 30;
/// Rows of the current page: `ROW_ID + ROW_PARTS * row + part`.
const ROW_ID: usize = 100;
const ROW_PARTS: usize = 3;
const PART_SLIDER: usize = 0;
/// Selector arrows: `PART_ARROW + ARROW_LEFT / ARROW_RIGHT`.
const PART_ARROW: usize = 1;
/// The right arrow's part (the last one).
const PART_ARROW_LAST: usize = PART_ARROW + ARROW_RIGHT;
const DIALOG_ID: usize = 1000;
const BACK: usize = 0;
const RESET: usize = 1;
const APPLY: usize = 2;
/// Knob steps per arrow-key press on a focused slider row.
const SLIDER_KEY_STEPS: isize = 5;

// Widget id ranges must not overlap (checked at compile time).
const _: () = assert!(TAB_ID + 3 <= BOTTOM_ID);
const _: () = assert!(BOTTOM_ID + 4 <= PAGE_ID);
const _: () = assert!(PAGE_ID + 2 <= SCROLLBAR_ID);
const _: () = assert!(SCROLLBAR_ID < ROW_ID);
const _: () = assert!(ROW_ID + ROW_PARTS * MAX_PAGE_ROWS <= DIALOG_ID);
const _: () = assert!(PART_ARROW + ARROW_RIGHT < ROW_PARTS);

// Layout invariants, checked at compile time.
const _: () = assert!(SELECTOR[0] + SELECTOR[1] < SCROLLBAR[0] - SCROLLBAR_GRAB);
const _: () = assert!(LIST_BOTTOM < BOTTOM_CENTER - STONE_BUTTON_SIZE[1] / 2.0);
// The bottom buttons, at their default width, neither overlap nor leave the screen.
const _: () = assert!(BOTTOM_X[1] - BOTTOM_X[0] > STONE_BUTTON_SIZE[0]);
const _: () = assert!(BOTTOM_X[2] - BOTTOM_X[1] > STONE_BUTTON_SIZE[0]);
const _: () = assert!(BOTTOM_X[3] - BOTTOM_X[2] > STONE_BUTTON_SIZE[0]);
const _: () = assert!(BOTTOM_X[3] + STONE_BUTTON_SIZE[0] < 1920.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Audio = 0,
    Graphics = 1,
    Display = 2,
}

const TABS: [(Tab, &str, &str); 3] = [
    (Tab::Audio, "Audio", "AUDIO OPTIONS"),
    (Tab::Graphics, "Graphics", "GRAPHICS OPTIONS"),
    (Tab::Display, "Display", "DISPLAY OPTIONS"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsEvent {
    BackToMenu,
    /// Make `settings` current. `save`: also write them to disk (false while a window
    /// change waits for confirmation). `leave`: return to the main menu afterwards.
    Apply {
        settings: Settings,
        save: bool,
        leave: bool,
    },
}

/// What Apply does with `applied` (previously `previous`): a window change is applied
/// unsaved and needs confirmation (`true`); anything else is applied and saved.
fn apply_outcome(previous: &Settings, applied: Settings, leave: bool) -> (SettingsEvent, bool) {
    if previous.display.window_changes(&applied.display) {
        let event = SettingsEvent::Apply {
            settings: applied,
            save: false,
            leave: false,
        };
        return (event, true);
    }
    let event = SettingsEvent::Apply {
        settings: applied,
        save: true,
        leave,
    };
    (event, false)
}

/// Gamma hundredths per slider step: 0–100 covers 0.50–2.50.
const GAMMA_PER_STEP: u16 = (MAX_GAMMA - MIN_GAMMA) / MAX_VOLUME as u16;

/// A slider row: a 0..=100 knob position mapped onto one setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliderKind {
    Volume(AudioChannel),
    Brightness,
    Gamma,
    UiScale,
    ImageSharpening,
}

impl SliderKind {
    fn label(self) -> &'static str {
        match self {
            Self::Volume(channel) => channel.label(),
            Self::Brightness => "Brightness",
            Self::Gamma => "Gamma",
            Self::UiScale => "UI Scale",
            Self::ImageSharpening => "Image Sharpening",
        }
    }

    fn position(self, s: &Settings) -> u8 {
        let d = &s.display;
        let position = match self {
            Self::Volume(channel) => u16::from(s.audio.get(channel)),
            Self::Brightness => u16::from(d.brightness),
            Self::Gamma => (d.gamma.clamp(MIN_GAMMA, MAX_GAMMA) - MIN_GAMMA) / GAMMA_PER_STEP,
            Self::UiScale => {
                let span = MAX_UI_SCALE - MIN_UI_SCALE;
                ((snap_ui_scale(d.ui_scale) - MIN_UI_SCALE) * 100 + span / 2) / span
            }
            Self::ImageSharpening => u16::from(s.graphics.image_sharpening),
        };
        position.min(u16::from(MAX_VOLUME)) as u8
    }

    fn set_position(self, s: &mut Settings, position: u8) {
        let position = position.min(MAX_VOLUME);
        let p = u16::from(position);
        match self {
            Self::Volume(channel) => s.audio.set(channel, position),
            Self::Brightness => s.display.brightness = position.min(MAX_BRIGHTNESS),
            Self::Gamma => s.display.gamma = MIN_GAMMA + p * GAMMA_PER_STEP,
            Self::UiScale => {
                let span = MAX_UI_SCALE - MIN_UI_SCALE;
                s.display.ui_scale = snap_ui_scale(MIN_UI_SCALE + (p * span + 50) / 100);
            }
            Self::ImageSharpening => s.graphics.image_sharpening = position.min(MAX_SHARPENING),
        }
    }

    fn text(self, s: &Settings) -> String {
        let d = &s.display;
        match self {
            Self::Volume(channel) => s.audio.get(channel).to_string(),
            Self::Brightness => format!("{}%", d.brightness),
            Self::Gamma => format!("{}.{:02}", d.gamma / 100, d.gamma % 100),
            Self::UiScale => format!("{}%", d.ui_scale),
            Self::ImageSharpening => format!("{}%", s.graphics.image_sharpening),
        }
    }
}

/// A selector row's option: how many choices it has, which is current, its text, and
/// how to change it. Monitor-dependent lists come from the [`DisplayInfo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    /// Sets every preset-controlled graphics option; shows "Custom" when they differ.
    Overall,
    Graphics(GraphicsOption),
    Language,
    DisplayMode,
    Resolution,
    AspectRatio,
    Monitor,
    RefreshRate,
    VSync,
    FrameRateLimit,
    BackgroundFrameRateLimit,
    ShowFps,
}

/// Sets an option to its value at `index`, ignoring out-of-range indices.
fn set_from<T: OptionValue>(value: &mut T, index: usize) {
    if let Some(&new) = T::ALL.get(index) {
        *value = new;
    }
}

/// `index` moved by `delta` (at most one lap) in a circular list of `count` values.
fn wrap_index(index: usize, delta: isize, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (index % count + count).saturating_add_signed(delta) % count
}

fn resolutions(s: &Settings, info: &DisplayInfo) -> Vec<[u32; 2]> {
    let sizes = info.resolutions(s.display.monitor);
    // In windowed mode the whole window (frame included) must stay on the screen, so
    // sizes larger than the screen minus the frame are not offered.
    if s.display.mode == DisplayMode::Windowed
        && let Some(max) = info.windowed_max(s.display.monitor)
        && sizes.iter().any(|size| fits_windowed(*size, max))
    {
        sizes
            .into_iter()
            .filter(|size| fits_windowed(*size, max))
            .collect()
    } else {
        sizes
    }
}

fn refresh_rates(s: &Settings, info: &DisplayInfo) -> Vec<u32> {
    info.refresh_rates(s.display.monitor, s.display.resolution)
}

/// The monitor that will be used: the stored one, or the primary when it is gone.
fn monitor_index(s: &Settings, info: &DisplayInfo) -> u8 {
    if usize::from(s.display.monitor) < info.monitor_count() {
        s.display.monitor
    } else {
        0
    }
}

/// Whether the monitor offers `size` as a video mode (true when it reports none).
fn has_video_mode(info: &DisplayInfo, monitor: u8, size: [u32; 2]) -> bool {
    info.monitor(monitor)
        .is_none_or(|m| m.modes.is_empty() || m.modes.iter().any(|mode| mode.size == size))
}

/// Moves a windowed resolution that no longer fits on the screen (frame included) to
/// the largest offered size that does; other modes keep the stored value.
fn snap_windowed(s: &mut Settings, info: &DisplayInfo) {
    if s.display.mode != DisplayMode::Windowed {
        return;
    }
    let Some(max) = info.windowed_max(s.display.monitor) else {
        return;
    };
    if fits_windowed(s.display.resolution, max) {
        return;
    }
    // `resolutions` is sorted largest first and already filtered for windowed mode.
    let sizes = resolutions(s, info);
    if let Some(&size) = sizes.first() {
        s.display.resolution = size;
    }
}

/// Aspect ratio options: Auto plus the presets the selected monitor's offered
/// resolutions actually match.
fn aspect_ratios(s: &Settings, info: &DisplayInfo) -> Vec<AspectRatio> {
    info.aspect_ratios(s.display.monitor)
}

impl Choice {
    fn label(self) -> &'static str {
        match self {
            Self::Overall => "Overall Quality",
            Self::Graphics(option) => option.label(),
            Self::Language => "Display Language",
            Self::DisplayMode => "Display Mode",
            Self::Resolution => "Resolution",
            Self::AspectRatio => "Aspect Ratio",
            Self::Monitor => "Monitor",
            Self::RefreshRate => "Refresh Rate",
            Self::VSync => "V-Sync",
            Self::FrameRateLimit => "Frame Rate Limit",
            Self::BackgroundFrameRateLimit => "Background FPS Limit",
            Self::ShowFps => "Show FPS",
        }
    }

    fn count(self, s: &Settings, info: &DisplayInfo) -> usize {
        match self {
            Self::Overall => Quality::ALL.len(),
            Self::Graphics(option) => option.count(),
            Self::Language => Language::ALL.len(),
            Self::DisplayMode => DisplayMode::ALL.len(),
            Self::Resolution => resolutions(s, info).len(),
            Self::AspectRatio => aspect_ratios(s, info).len(),
            Self::Monitor => info.monitor_count().max(1),
            Self::RefreshRate => refresh_rates(s, info).len(),
            Self::VSync | Self::ShowFps => bool::ALL.len(),
            Self::FrameRateLimit => FrameRateLimit::ALL.len(),
            Self::BackgroundFrameRateLimit => BackgroundFrameRateLimit::ALL.len(),
        }
    }

    /// Resolution only matters for a window or a video mode; the refresh rate only for
    /// exclusive fullscreen; the monitor only when there is more than one.
    fn is_active(self, s: &Settings, info: &DisplayInfo) -> bool {
        match self {
            Self::Graphics(option) => option.is_active(&s.graphics),
            Self::Monitor => info.monitor_count() > 1,
            Self::RefreshRate => {
                s.display.mode == DisplayMode::Fullscreen && !refresh_rates(s, info).is_empty()
            }
            _ => true,
        }
    }

    /// Index of the current value; `None` for "Custom" and an unlisted resolution.
    fn current(self, s: &Settings, info: &DisplayInfo) -> Option<usize> {
        let d = &s.display;
        match self {
            Self::Overall => s.graphics.overall().map(Quality::index),
            Self::Graphics(option) => Some(option.index(&s.graphics)),
            Self::Language => Some(d.language.index()),
            Self::DisplayMode => Some(d.mode.index()),
            Self::Resolution => resolutions(s, info).iter().position(|&r| r == d.resolution),
            Self::AspectRatio => aspect_ratios(s, info)
                .iter()
                .position(|&ratio| ratio == d.aspect_ratio),
            Self::Monitor => Some(usize::from(monitor_index(s, info))),
            Self::RefreshRate => {
                let rates = refresh_rates(s, info);
                resolve_refresh(&rates, d.refresh_rate)
                    .and_then(|rate| rates.iter().position(|&r| r == rate))
            }
            Self::VSync => Some(d.vsync.index()),
            Self::FrameRateLimit => Some(d.frame_rate_limit.index()),
            Self::BackgroundFrameRateLimit => Some(d.background_frame_rate_limit.index()),
            Self::ShowFps => Some(d.show_fps.index()),
        }
    }

    /// Where the arrows go from a value outside the list: the closest preset for
    /// "Custom", the largest resolution for an unlisted one.
    fn fallback(self, s: &Settings) -> usize {
        match self {
            Self::Overall => s.graphics.closest_preset().index(),
            _ => 0,
        }
    }

    /// The value as shown, in the interface `language`.
    fn text(self, s: &Settings, info: &DisplayInfo, language: Language) -> String {
        let d = &s.display;
        let tr = |english: &'static str| language.tr(english).to_owned();
        match self {
            Self::Overall => tr(s.graphics.overall().map_or("Custom", Quality::label)),
            Self::Graphics(option) => tr(option.value_label(&s.graphics)),
            // Language names are always written in their own language.
            Self::Language => d.language.label().to_owned(),
            Self::DisplayMode => tr(d.mode.label()),
            Self::Resolution => format!("{} x {}", d.resolution[0], d.resolution[1]),
            Self::AspectRatio => tr(d.aspect_ratio.label()),
            Self::Monitor => monitor_label(monitor_index(s, info), language),
            Self::RefreshRate => {
                // Outside exclusive fullscreen the desktop's rate applies.
                let rate = if d.mode == DisplayMode::Fullscreen {
                    resolve_refresh(&refresh_rates(s, info), d.refresh_rate)
                } else {
                    info.monitor(d.monitor).and_then(|m| m.refresh_millihertz)
                };
                rate.map_or_else(|| "—".to_owned(), |rate| refresh_label(rate, language))
            }
            Self::VSync => tr(d.vsync.label()),
            Self::FrameRateLimit => tr(d.frame_rate_limit.label()),
            Self::BackgroundFrameRateLimit => tr(d.background_frame_rate_limit.label()),
            Self::ShowFps => tr(d.show_fps.label()),
        }
    }

    fn select(self, s: &mut Settings, index: usize, info: &DisplayInfo) {
        match self {
            Self::Overall => {
                if let Some(&level) = Quality::ALL.get(index) {
                    s.graphics.apply_preset(level);
                }
            }
            Self::Graphics(option) => option.select(&mut s.graphics, index),
            Self::Language => set_from(&mut s.display.language, index),
            Self::DisplayMode => {
                set_from(&mut s.display.mode, index);
                // Exclusive fullscreen needs a real video mode: start from the native one.
                let d = &mut s.display;
                if d.mode == DisplayMode::Fullscreen
                    && !has_video_mode(info, d.monitor, d.resolution)
                {
                    d.resolution = info.native_resolution(d.monitor);
                }
                // A windowed window may keep the size only while it fits the screen.
                snap_windowed(s, info);
            }
            Self::Resolution => {
                if let Some(&size) = resolutions(s, info).get(index) {
                    s.display.resolution = size;
                }
            }
            Self::AspectRatio => {
                if let Some(&ratio) = aspect_ratios(s, info).get(index) {
                    s.display.aspect_ratio = ratio;
                }
            }
            Self::Monitor => {
                let d = &mut s.display;
                d.monitor = u8::try_from(index).unwrap_or(0);
                if !has_video_mode(info, d.monitor, d.resolution) {
                    d.resolution = info.native_resolution(d.monitor);
                }
                // The new monitor may be smaller: stay within its screen when windowed.
                snap_windowed(s, info);
            }
            Self::RefreshRate => {
                if let Some(&rate) = refresh_rates(s, info).get(index) {
                    s.display.refresh_rate = rate;
                }
            }
            Self::VSync => set_from(&mut s.display.vsync, index),
            Self::FrameRateLimit => set_from(&mut s.display.frame_rate_limit, index),
            Self::BackgroundFrameRateLimit => {
                set_from(&mut s.display.background_frame_rate_limit, index);
            }
            Self::ShowFps => set_from(&mut s.display.show_fps, index),
        }
    }

    /// Whether the arrows can move: active rows with more than one value. The list is
    /// circular, so both arrows always work; a value outside the list moves to
    /// [`Self::fallback`].
    fn can_step(self, s: &Settings, info: &DisplayInfo) -> bool {
        if !self.is_active(s, info) {
            return false;
        }
        let count = self.count(s, info);
        match self.current(s, info) {
            Some(_) => count > 1,
            None => count > 0,
        }
    }

    /// Moves one choice left (`-1`) or right (`+1`), wrapping around at the ends.
    fn step(self, s: &mut Settings, delta: isize, info: &DisplayInfo) {
        if !self.can_step(s, info) {
            return;
        }
        let index = match self.current(s, info) {
            Some(index) => wrap_index(index, delta, self.count(s, info)),
            None => self.fallback(s),
        };
        self.select(s, index, info);
    }
}

/// One line of a settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowDef {
    Heading(&'static str),
    Slider(SliderKind),
    Choice(Choice),
    /// A feature the engine does not support: listed, never changeable.
    Unavailable(&'static str),
}

const fn graphics(option: GraphicsOption) -> RowDef {
    RowDef::Choice(Choice::Graphics(option))
}

const fn volume(channel: AudioChannel) -> RowDef {
    RowDef::Slider(SliderKind::Volume(channel))
}

const AUDIO_ROWS: [RowDef; 5] = [
    volume(AudioChannel::Master),
    volume(AudioChannel::Music),
    volume(AudioChannel::Sfx),
    volume(AudioChannel::Ui),
    volume(AudioChannel::Voice),
];

const GRAPHICS_BASIC_ROWS: [RowDef; 26] = {
    use GraphicsOption as G;
    [
        RowDef::Heading("General"),
        RowDef::Choice(Choice::Overall),
        graphics(G::ViewDistance),
        RowDef::Heading("Rendering & Upscaling"),
        graphics(G::ResolutionScale),
        graphics(G::AntiAliasing),
        graphics(G::UpscalingMethod),
        graphics(G::UpscalingQuality),
        RowDef::Slider(SliderKind::ImageSharpening),
        graphics(G::DynamicResolution),
        graphics(G::DynamicResolutionTarget),
        RowDef::Heading("Textures & Materials"),
        graphics(G::TextureQuality),
        graphics(G::MaterialQuality),
        graphics(G::TextureFiltering),
        RowDef::Heading("Lighting & Shadows"),
        graphics(G::ShadowQuality),
        graphics(G::ShadowDistance),
        graphics(G::GlobalIllumination),
        graphics(G::AmbientOcclusion),
        graphics(G::ContactShadows),
        RowDef::Heading("Effects & Particles"),
        graphics(G::EffectsQuality),
        graphics(G::ParticleQuality),
        graphics(G::SpellEffectsQuality),
        graphics(G::ParticleDensity),
    ]
};

const GRAPHICS_ADVANCED_ROWS: [RowDef; 24] = {
    use GraphicsOption as G;
    [
        RowDef::Heading("Geometry & Environment"),
        graphics(G::TerrainQuality),
        graphics(G::FoliageQuality),
        graphics(G::GeometryQuality),
        graphics(G::ObjectDetail),
        graphics(G::VegetationDensity),
        RowDef::Heading("Reflections & Water"),
        graphics(G::ReflectionQuality),
        graphics(G::WaterQuality),
        graphics(G::WaterReflections),
        RowDef::Heading("Atmosphere"),
        graphics(G::FogQuality),
        graphics(G::VolumetricFog),
        graphics(G::LightShafts),
        graphics(G::WeatherEffectsQuality),
        RowDef::Heading("Post Processing"),
        graphics(G::PostProcessingQuality),
        graphics(G::Bloom),
        graphics(G::DepthOfField),
        graphics(G::MotionBlur),
        graphics(G::ColorGrading),
        graphics(G::Vignette),
        graphics(G::FilmGrain),
        graphics(G::ChromaticAberration),
    ]
};

const DISPLAY_ROWS: [RowDef; 18] = [
    RowDef::Heading("Display"),
    RowDef::Choice(Choice::Language),
    RowDef::Choice(Choice::DisplayMode),
    RowDef::Choice(Choice::Resolution),
    RowDef::Choice(Choice::AspectRatio),
    RowDef::Choice(Choice::Monitor),
    RowDef::Choice(Choice::RefreshRate),
    RowDef::Heading("Frame Rate & Synchronization"),
    RowDef::Choice(Choice::VSync),
    RowDef::Choice(Choice::FrameRateLimit),
    RowDef::Choice(Choice::BackgroundFrameRateLimit),
    RowDef::Choice(Choice::ShowFps),
    RowDef::Heading("Display Calibration"),
    RowDef::Slider(SliderKind::Brightness),
    RowDef::Slider(SliderKind::Gamma),
    RowDef::Unavailable("HDR"),
    RowDef::Unavailable("HDR Peak Brightness"),
    RowDef::Slider(SliderKind::UiScale),
];

/// Pages in [`page_index`] order.
const PAGES: [&[RowDef]; 4] = [
    &AUDIO_ROWS,
    &GRAPHICS_BASIC_ROWS,
    &GRAPHICS_ADVANCED_ROWS,
    &DISPLAY_ROWS,
];

const MAX_PAGE_ROWS: usize = {
    let mut max = 0;
    let mut i = 0;
    while i < PAGES.len() {
        if PAGES[i].len() > max {
            max = PAGES[i].len();
        }
        i += 1;
    }
    max
};

fn row_visible(def: RowDef, s: &Settings) -> bool {
    match def {
        RowDef::Choice(Choice::Graphics(option)) => option.is_visible(&s.graphics),
        _ => true,
    }
}

fn row_pitch(def: RowDef) -> f32 {
    match def {
        RowDef::Heading(_) => HEADING_PITCH,
        _ => ROW_PITCH,
    }
}

/// A slider row's bar and value text (physical px) for a row whose top is at `y`
/// (reference px): together they span the selector block, centered on [`SLIDER_CENTER`].
fn slider_rects(scale: f32, y: f32) -> (Rect, Rect) {
    let block = scaled(scale, SELECTOR[0], y, SELECTOR[1], ROW_HEIGHT);
    let value_w = (SLIDER_VALUE_WIDTH * scale).round();
    let gap = (SLIDER_VALUE_GAP * scale).round();
    let track_h = (SLIDER_TRACK_HEIGHT * scale).round().max(2.0);
    let text_h = (ROW_TEXT_HEIGHT * scale).round();
    let center = ((y + SLIDER_CENTER) * scale).round();
    let track = Rect::new(
        block.x,
        (center - track_h / 2.0).round(),
        block.w - value_w - gap,
        track_h,
    );
    let value = Rect::new(
        block.x + block.w - value_w,
        (center - text_h / 2.0).round(),
        value_w,
        text_h,
    );
    (track, value)
}

/// The text shown for a row's name. Section headings follow the interface language;
/// setting names keep their original (English) names in every language, as players know
/// them from other games and hardware tools.
fn row_name(def: RowDef, language: Language) -> String {
    match def {
        RowDef::Heading(title) => language.tr(title).to_uppercase(),
        RowDef::Slider(kind) => kind.label().to_owned(),
        RowDef::Choice(choice) => choice.label().to_owned(),
        RowDef::Unavailable(label) => label.to_owned(),
    }
}

/// Stacks the visible rows: each row's top within the list (`None` when hidden) and the
/// total content height, in reference px.
fn stack_rows(
    defs: impl Iterator<Item = RowDef>,
    visible: impl Fn(RowDef) -> bool,
) -> (Vec<Option<f32>>, f32) {
    let mut y = 0.0;
    let tops = defs
        .map(|def| {
            visible(def).then(|| {
                let top = y;
                y += row_pitch(def);
                top
            })
        })
        .collect();
    (tops, y)
}

/// Scrollbar thumb top and height for a track of `track_h` at `track_top` showing
/// `content_h` of content scrolled by `offset` (all in the same units).
fn thumb_geometry(track_top: f32, track_h: f32, content_h: f32, offset: f32) -> (f32, f32) {
    let max = (content_h - track_h).max(0.0);
    if max <= 0.0 {
        return (track_top, track_h);
    }
    let h = (track_h * track_h / content_h).clamp(SCROLLBAR_MIN_THUMB.min(track_h), track_h);
    let t = (offset / max).clamp(0.0, 1.0);
    (track_top + (track_h - h) * t, h)
}

/// The scroll offset that puts the thumb's top at `thumb_top`.
fn offset_for_thumb(track_top: f32, track_h: f32, content_h: f32, thumb_top: f32) -> f32 {
    let max = (content_h - track_h).max(0.0);
    let (_, h) = thumb_geometry(track_top, track_h, content_h, 0.0);
    let travel = track_h - h;
    if max <= 0.0 || travel <= 0.0 {
        return 0.0;
    }
    ((thumb_top - track_top) / travel).clamp(0.0, 1.0) * max
}

/// Applied settings plus the draft the controls edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SettingsDraft {
    applied: Settings,
    draft: Settings,
}

impl SettingsDraft {
    fn new(applied: Settings) -> Self {
        Self {
            applied,
            draft: applied,
        }
    }

    fn has_changes(&self) -> bool {
        self.draft != self.applied
    }

    /// Discards unapplied changes on every tab.
    fn reset(&mut self) {
        self.draft = self.applied;
    }

    /// The draft with one tab's defaults (Graphics covers both its pages); the default
    /// resolution is the primary monitor's native size, and the interface language stays.
    fn defaults_for(&self, tab: Tab, native: [u32; 2]) -> Settings {
        let mut settings = self.draft;
        match tab {
            Tab::Audio => settings.audio = Default::default(),
            Tab::Graphics => settings.graphics = Default::default(),
            Tab::Display => {
                settings.display = DisplaySettings {
                    language: settings.display.language,
                    resolution: native,
                    ..DisplaySettings::default()
                };
            }
        }
        settings
    }

    fn apply(&mut self) -> Settings {
        self.applied = self.draft;
        self.applied
    }

    /// Goes back to `previous` for both the applied values and the draft.
    fn revert_to(&mut self, previous: Settings) {
        self.applied = previous;
        self.draft = previous;
    }
}

/// What the screen is waiting for, if anything.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Modal {
    None,
    /// A window change was applied (not saved yet); keep it or revert to `previous`.
    ConfirmDisplay {
        previous: Settings,
        /// When it reverts by itself.
        deadline: Instant,
        leave_after: bool,
    },
    /// Leaving with unapplied changes.
    Unapplied,
    /// Reset / Apply / Default was pressed; waiting for the player to confirm.
    Confirm(ConfirmAction),
}

/// A bottom-row action that asks for confirmation before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmAction {
    Reset = 0,
    Apply = 1,
    Default = 2,
}

impl ConfirmAction {
    const ALL: [Self; 3] = [Self::Reset, Self::Apply, Self::Default];

    /// Dialog title, message and confirm button (English keys).
    fn texts(self) -> [&'static str; 3] {
        match self {
            Self::Reset => [
                "Reset changes?",
                "Unapplied changes on all tabs will be discarded.",
                "Reset",
            ],
            Self::Apply => [
                "Apply settings?",
                "The new settings will be saved.",
                "Apply",
            ],
            Self::Default => [
                "Restore defaults?",
                "Settings on this tab will return to their default values.",
                "Restore",
            ],
        }
    }
}

const CANCEL: &str = "Cancel";

impl Modal {
    /// Checks a display confirmation against the clock. Once its deadline has passed,
    /// closes it and returns the settings to restore.
    fn tick(&mut self, now: Instant) -> Option<Settings> {
        let Self::ConfirmDisplay {
            previous, deadline, ..
        } = self
        else {
            return None;
        };
        if now < *deadline {
            return None;
        }
        let previous = *previous;
        *self = Self::None;
        Some(previous)
    }
}

enum RowWidget {
    Heading,
    Slider {
        slider: Slider,
        value: LabelId,
        /// Text currently shown by `value` (to update the label only on change).
        shown: Option<String>,
    },
    Selector(OptionSelector),
    Info(LabelId),
}

struct Row {
    def: RowDef,
    name: LabelId,
    widget: RowWidget,
    /// Top within the list content (reference px); `None` while hidden.
    top: Option<f32>,
    // Physical rects at the current scroll position.
    bounds: Rect,
    name_rect: Rect,
    underline: Rect,
    value_rect: Rect,
}

impl Row {
    fn new(renderer: &mut UiRenderer, def: RowDef) -> Self {
        let left = TextStyle {
            bold: false,
            align: TextAlign::Left,
        };
        let (name, widget) = match def {
            RowDef::Heading(title) => {
                let bold = TextStyle {
                    bold: true,
                    align: TextAlign::Left,
                };
                (
                    renderer.create_label(&title.to_uppercase(), bold),
                    RowWidget::Heading,
                )
            }
            RowDef::Slider(kind) => (
                renderer.create_label(kind.label(), left),
                RowWidget::Slider {
                    slider: Slider::default(),
                    value: renderer.create_label(
                        "",
                        TextStyle {
                            bold: false,
                            align: TextAlign::Right,
                        },
                    ),
                    shown: None,
                },
            ),
            RowDef::Choice(choice) => (
                renderer.create_label(choice.label(), left),
                RowWidget::Selector(OptionSelector::new(renderer)),
            ),
            RowDef::Unavailable(label) => (
                renderer.create_label(label, left),
                RowWidget::Info(renderer.create_label(NOT_SUPPORTED, TextStyle::CENTERED)),
            ),
        };
        let empty = Rect::new(0.0, 0.0, 0.0, 0.0);
        Self {
            def,
            name,
            widget,
            top: None,
            bounds: empty,
            name_rect: empty,
            underline: empty,
            value_rect: empty,
        }
    }

    /// Places the row with its top at `y` (reference px, already scrolled).
    fn place(&mut self, scale: f32, y: f32) {
        self.bounds = scaled(
            scale,
            PANEL_LEFT,
            y,
            SCROLLBAR[0] - PANEL_LEFT,
            row_pitch(self.def),
        );
        if matches!(self.widget, RowWidget::Heading) {
            self.name_rect = scaled(
                scale,
                PANEL_LEFT,
                y + HEADING_TEXT_OFFSET,
                HEADER[2],
                ROW_TEXT_HEIGHT,
            );
            return;
        }
        self.name_rect = scaled(scale, PANEL_LEFT, y, ROW_NAME_WIDTH, ROW_TEXT_HEIGHT);
        self.underline = scaled(
            scale,
            PANEL_LEFT,
            y + ROW_TEXT_HEIGHT + 4.0,
            UNDERLINE[0],
            UNDERLINE[1],
        );
        match &mut self.widget {
            RowWidget::Heading => {}
            RowWidget::Slider { slider, .. } => {
                let (track, value) = slider_rects(scale, y);
                slider.place(track, (KNOB_SIZE * scale).round());
                self.value_rect = value;
            }
            RowWidget::Selector(selector) => {
                selector.place(
                    scaled(scale, SELECTOR[0], y, SELECTOR[1], ROW_HEIGHT),
                    scale,
                );
            }
            RowWidget::Info(_) => {
                self.value_rect = scaled(scale, SELECTOR[0], y, SELECTOR[1], ROW_TEXT_HEIGHT);
            }
        }
    }

    /// Shows the row's name (and fixed value text) in `language`.
    fn relabel(&self, renderer: &mut UiRenderer, language: Language) {
        renderer.set_label_text(self.name, &row_name(self.def, language));
        if let RowWidget::Info(value) = self.widget {
            renderer.set_label_text(value, language.tr(NOT_SUPPORTED));
        }
    }

    fn reset_animation(&mut self) {
        match &mut self.widget {
            RowWidget::Slider { slider, .. } => slider.reset_animation(),
            RowWidget::Selector(selector) => selector.reset_animation(),
            RowWidget::Heading | RowWidget::Info(_) => {}
        }
    }
}

/// One scrolling list of rows.
struct Page {
    rows: Vec<Row>,
    content_height: f32,
    /// Current scroll offset (reference px), animated toward `scroll_target`.
    scroll: Tween,
    scroll_target: f32,
}

impl Page {
    fn new(renderer: &mut UiRenderer, defs: &[RowDef]) -> Self {
        Self {
            rows: defs.iter().map(|&def| Row::new(renderer, def)).collect(),
            content_height: 0.0,
            scroll: Tween::new(0.0),
            scroll_target: 0.0,
        }
    }

    fn max_scroll(&self) -> f32 {
        (self.content_height - LIST_HEIGHT).max(0.0)
    }

    /// Re-stacks the rows for the current settings (some rows hide, e.g. Upscaling
    /// Quality without an upscaler) and keeps the scroll offset in range.
    fn arrange(&mut self, s: &Settings) {
        let (tops, height) = stack_rows(self.rows.iter().map(|row| row.def), |def| {
            row_visible(def, s)
        });
        for (row, top) in self.rows.iter_mut().zip(tops) {
            row.top = top;
        }
        self.content_height = height;
        if self.scroll_target > self.max_scroll() {
            self.scroll_to(self.max_scroll());
        }
    }

    /// Smoothly scrolls by `delta` reference px.
    fn scroll_by(&mut self, delta: f32) {
        let target = (self.scroll_target + delta).clamp(0.0, self.max_scroll());
        if target != self.scroll_target {
            self.scroll_target = target;
            self.scroll
                .animate_to(target, SCROLL_DURATION, Easing::EaseOutCubic);
        }
    }

    /// Jumps to `offset` (dragging the scrollbar, opening the screen).
    fn scroll_to(&mut self, offset: f32) {
        self.scroll_target = offset.clamp(0.0, self.max_scroll());
        self.scroll.set(self.scroll_target);
    }

    fn position(&mut self, scale: f32) {
        let offset = self.scroll.value();
        for row in &mut self.rows {
            if let Some(top) = row.top {
                row.place(scale, LIST_TOP + top - offset);
            }
        }
    }
}

/// Basic / Advanced text switch.
struct PageSwitchItem {
    label: LabelId,
    /// English text (translation key).
    text: &'static str,
    rect: Rect,
    hover: Tween,
}

pub struct SettingsScreen {
    tabs: [StoneButton; 3],
    headers: [LabelId; 3],
    page_switch: [PageSwitchItem; 2],
    /// Same order as [`PAGES`].
    pages: [Page; 4],
    bottom: [StoneButton; 4],
    tab: Tab,
    /// Graphics shows its Advanced page.
    advanced: bool,
    settings: SettingsDraft,
    modal: Modal,
    confirm_dialog: Dialog,
    unapplied_dialog: Dialog,
    /// Confirmation dialogs in [`ConfirmAction`] order.
    action_dialogs: [Dialog; 3],
    /// Monitors, resolutions and refresh rates offered on the Display tab.
    display: DisplayInfo,
    scale: f32,
    header_rect: Rect,
    brush_rect: Rect,
    /// The list's visible area (physical px); rows are clipped to it.
    list_rect: Rect,
    scrollbar_hover: Tween,
    /// While dragging the thumb: where it was grabbed, from its top (reference px).
    scrollbar_grab: Option<f32>,
}

impl SettingsScreen {
    pub fn new(renderer: &mut UiRenderer, applied: Settings, display: DisplayInfo) -> Self {
        let header = TextStyle {
            bold: true,
            align: TextAlign::Left,
        };
        let empty = Rect::new(0.0, 0.0, 0.0, 0.0);
        let pages = PAGES.map(|defs| Page::new(renderer, defs));
        let page_switch = ["Basic", "Advanced"].map(|text| PageSwitchItem {
            label: renderer.create_label(
                text,
                TextStyle {
                    bold: true,
                    align: TextAlign::Center,
                },
            ),
            text,
            rect: empty,
            hover: Tween::new(0.0),
        });

        let mut bottom = ["Back to Menu", "Reset", "Apply", "Default"]
            .map(|name| StoneButton::new(renderer, name, true));
        bottom[BACK].dark = true;
        let action_dialogs = ConfirmAction::ALL.map(|action| {
            let [title, message, confirm] = action.texts();
            let mut dialog = Dialog::new(renderer, title, [confirm, CANCEL]);
            dialog.set_message(renderer, message);
            dialog
        });
        let mut unapplied_dialog = Dialog::new(renderer, "Apply changes?", ["Apply", "Discard"]);
        unapplied_dialog.set_message(renderer, UNAPPLIED_MESSAGE);
        let confirm_dialog =
            Dialog::new(renderer, "Keep these display settings?", ["Keep", "Revert"]);
        let mut screen = Self {
            tabs: TABS.map(|(_, name, _)| StoneButton::new(renderer, name, true)),
            headers: TABS.map(|(_, _, title)| renderer.create_label(title, header)),
            page_switch,
            pages,
            bottom,
            tab: Tab::Audio,
            advanced: false,
            settings: SettingsDraft::new(applied),
            modal: Modal::None,
            confirm_dialog,
            unapplied_dialog,
            action_dialogs,
            display,
            scale: 1.0,
            header_rect: empty,
            brush_rect: empty,
            list_rect: empty,
            scrollbar_hover: Tween::new(0.0),
            scrollbar_grab: None,
        };
        screen.select_tab(Tab::Audio);
        screen
    }

    /// Prepares the screen for showing: Audio tab, Basic graphics page, lists at the
    /// top, draft = applied values.
    pub fn open(&mut self) {
        self.settings.reset();
        self.modal = Modal::None;
        self.advanced = false;
        self.scrollbar_grab = None;
        self.select_tab(Tab::Audio);
        for button in self.tabs.iter_mut().chain(&mut self.bottom) {
            button.reset_animation();
        }
        for item in &mut self.page_switch {
            item.hover.set(0.0);
        }
        self.scrollbar_hover.set(0.0);
        for page in &mut self.pages {
            page.scroll_to(0.0);
            page.rows.iter_mut().for_each(Row::reset_animation);
        }
        self.sync_buttons();
    }

    /// Settings were changed outside this screen (e.g. the F11 display-mode toggle).
    pub fn set_applied(&mut self, applied: Settings) {
        self.settings.applied = applied;
        let (draft, applied) = (&mut self.settings.draft.display, applied.display);
        draft.mode = applied.mode;
        draft.monitor = applied.monitor;
        draft.resolution = applied.resolution;
        self.sync_buttons();
    }

    /// The interface language in effect (the applied one, not the draft).
    pub fn language(&self) -> Language {
        self.settings.applied.display.language
    }

    /// Shows every fixed text of the screen in `language`.
    pub fn relabel(&mut self, renderer: &mut UiRenderer, language: Language) {
        for button in self.tabs.iter().chain(&self.bottom) {
            button.relabel(renderer, language);
        }
        for (label, (_, _, title)) in self.headers.iter().zip(TABS) {
            renderer.set_label_text(*label, language.tr(title));
        }
        for item in &self.page_switch {
            renderer.set_label_text(item.label, language.tr(item.text));
        }
        for row in self.pages.iter().flat_map(|page| &page.rows) {
            row.relabel(renderer, language);
        }
        self.confirm_dialog.relabel(renderer, language);
        self.unapplied_dialog.relabel(renderer, language);
        self.unapplied_dialog
            .set_message(renderer, language.tr(UNAPPLIED_MESSAGE));
        for (dialog, action) in self.action_dialogs.iter_mut().zip(ConfirmAction::ALL) {
            dialog.relabel(renderer, language);
            dialog.set_message(renderer, language.tr(action.texts()[1]));
        }
    }

    /// The connected monitors were enumerated again.
    pub fn set_display_info(&mut self, display: DisplayInfo) {
        self.display = display;
        self.sync_buttons();
    }

    /// Display calibration to show: the draft while the player is adjusting it, else the
    /// applied values.
    pub fn display_preview(&self, previewing: bool) -> DisplaySettings {
        if previewing {
            self.settings.draft.display
        } else {
            self.settings.applied.display
        }
    }

    pub fn layout(&mut self, scale: f32, viewport: [f32; 2], kit: &UiKit) {
        self.scale = scale;
        let mut y = TOP;
        for tab in &mut self.tabs {
            let [w, h] = tab.size;
            tab.place(scaled(scale, LEFT, y, w, h), &kit.button);
            y += h + STONE_BUTTON_GAP;
        }
        for (button, x) in self.bottom.iter_mut().zip(BOTTOM_X) {
            let [w, h] = button.size;
            let y = BOTTOM_CENTER - h / 2.0;
            button.place(scaled(scale, x, y, w, h), &kit.button);
        }
        let [x, y, w, h] = HEADER;
        self.header_rect = scaled(scale, x, y, w, h);
        let [x, y, w, h] = BRUSH;
        self.brush_rect = scaled(scale, x, y, w, h);
        for (item, [x, y, w, h]) in self.page_switch.iter_mut().zip(PAGE_SWITCH) {
            item.rect = scaled(scale, x, y, w, h);
        }
        let top = (LIST_TOP * scale).round();
        self.list_rect = Rect::new(0.0, top, viewport[0], (LIST_BOTTOM * scale).round() - top);
        self.position_rows();
        self.confirm_dialog.layout(scale, viewport, kit);
        self.unapplied_dialog.layout(scale, viewport, kit);
        for dialog in &mut self.action_dialogs {
            dialog.layout(scale, viewport, kit);
        }
    }

    pub fn hit_test(&self, x: f32, y: f32, kit: &UiKit) -> Hit {
        // A dialog blocks everything behind it.
        if let Some(dialog) = self.active_dialog() {
            return dialog.hit_test(x, y, kit, DIALOG_ID);
        }
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| tab.hit_target(TAB_ID + i, &kit.button));
        let switch = self
            .page_switch
            .iter()
            .enumerate()
            .filter(|_| self.tab == Tab::Graphics)
            .map(|(i, item)| HitTarget {
                id: PAGE_ID + i,
                shape: HitShape::Rectangle(item.rect),
                interactive: self.advanced != (i == 1),
            });
        let draft = &self.settings.draft;
        let info = &self.display;
        // Rows only take the pointer inside the visible part of the list.
        let in_list = self.list_rect.contains(x, y);
        let rows = self
            .page()
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| in_list && row.top.is_some() && row.bounds.overlaps(&self.list_rect))
            .flat_map(|(i, row)| {
                let mut targets = Vec::new();
                match (&row.widget, row.def) {
                    (RowWidget::Slider { slider, .. }, _) => {
                        targets.push(slider.hit_target(row_id(i, PART_SLIDER)));
                    }
                    (RowWidget::Selector(selector), RowDef::Choice(choice)) => {
                        let enabled = [choice.can_step(draft, info); 2];
                        let ids =
                            [ARROW_LEFT, ARROW_RIGHT].map(|side| row_id(i, PART_ARROW + side));
                        targets.extend(selector.hit_targets(ids, enabled));
                    }
                    _ => {}
                }
                targets
            });
        let scrollbar = self.scrollbar_target();
        let bottom = self
            .bottom
            .iter()
            .enumerate()
            .map(|(i, button)| button.hit_target(BOTTOM_ID + i, &kit.button));
        topmost_hit(
            tabs.chain(switch)
                .chain(rows)
                .chain(scrollbar)
                .chain(bottom),
            x,
            y,
        )
    }

    /// Pointer pressed on widget `id` at (`x`, `y`): sliders jump to the pointer, the
    /// scrollbar starts dragging (from the grab point on the thumb, or centered on the
    /// pointer when the track was hit).
    pub fn press(&mut self, id: usize, x: f32, y: f32) {
        if id == SCROLLBAR_ID && self.scale > 0.0 {
            let page = self.page();
            let pointer = y / self.scale;
            let (top, h) = thumb_geometry(
                LIST_TOP,
                LIST_HEIGHT,
                page.content_height,
                page.scroll.value(),
            );
            let on_thumb = (top..top + h).contains(&pointer);
            self.scrollbar_grab = Some(if on_thumb { pointer - top } else { h / 2.0 });
        }
        self.drag(id, x, y);
    }

    /// Pointer moved while widget `id` holds the capture.
    pub fn drag(&mut self, id: usize, x: f32, y: f32) {
        if self.modal != Modal::None {
            return;
        }
        if id == SCROLLBAR_ID {
            if let Some(grab) = self.scrollbar_grab
                && self.scale > 0.0
            {
                let scale = self.scale;
                let page = self.page_mut();
                let offset =
                    offset_for_thumb(LIST_TOP, LIST_HEIGHT, page.content_height, y / scale - grab);
                page.scroll_to(offset);
                page.position(scale);
            }
            return;
        }
        let page = &self.pages[self.page_index()];
        if let Some((row, PART_SLIDER)) = split_row_id(id)
            && let Some(Row {
                def: RowDef::Slider(kind),
                widget: RowWidget::Slider { slider, .. },
                ..
            }) = page.rows.get(row)
        {
            let kind = *kind;
            kind.set_position(&mut self.settings.draft, slider.value_at(x));
            self.sync_buttons();
        }
    }

    /// Mouse wheel: scrolls the current list by `delta` reference px (positive = down).
    pub fn scroll(&mut self, delta: f32) {
        if self.modal == Modal::None {
            self.page_mut().scroll_by(delta);
        }
    }

    /// Whether the list is still moving (hover must follow the content).
    pub fn is_scrolling(&self) -> bool {
        !self.page().scroll.is_settled()
    }

    pub fn click(&mut self, id: usize) -> Option<SettingsEvent> {
        let event = self.handle_click(id);
        self.sync_buttons();
        event
    }

    /// Whether a modal dialog is open (keyboard focus stays inside it).
    pub fn modal_open(&self) -> bool {
        self.modal != Modal::None
    }

    /// Keyboard focus stops of the current view, in visual order: a modal dialog's
    /// two buttons, or the tabs, the page switch, the visible interactive rows and
    /// the enabled bottom buttons.
    pub fn focus_stops(&self) -> Vec<usize> {
        if self.modal_open() {
            return vec![DIALOG_ID + DIALOG_CONFIRM, DIALOG_ID + DIALOG_CANCEL];
        }
        let mut stops: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.is_interactive())
            .map(|(i, _)| TAB_ID + i)
            .collect();
        if self.tab == Tab::Graphics {
            // Only the switch away from the current page is interactive.
            stops.push(PAGE_ID + usize::from(!self.advanced));
        }
        let draft = &self.settings.draft;
        let info = &self.display;
        for (i, row) in self.page().rows.iter().enumerate() {
            // Scrolled-out rows are skipped like in the hit test.
            if row.top.is_none() || !row.bounds.overlaps(&self.list_rect) {
                continue;
            }
            match row.def {
                RowDef::Slider(_) => stops.push(row_id(i, PART_SLIDER)),
                RowDef::Choice(choice) if choice.can_step(draft, info) => {
                    stops.push(row_id(i, PART_ARROW + ARROW_LEFT));
                    stops.push(row_id(i, PART_ARROW + ARROW_RIGHT));
                }
                _ => {}
            }
        }
        stops.extend(
            self.bottom
                .iter()
                .enumerate()
                .filter(|(_, button)| button.is_interactive())
                .map(|(i, _)| BOTTOM_ID + i),
        );
        stops
    }

    /// Arrow keys on a focused row: moves a slider by `SLIDER_KEY_STEPS` knob steps
    /// or a choice by one value (`delta` = -1 left / +1 right). A modal dialog or a
    /// stop without a value ignores the keys.
    pub fn adjust(&mut self, id: usize, delta: isize) {
        if self.modal != Modal::None {
            return;
        }
        let Some((row, part)) = split_row_id(id) else {
            return;
        };
        let index = self.page_index();
        let Some(def) = self.pages[index].rows.get(row).map(|row| row.def) else {
            return;
        };
        match (def, part) {
            (RowDef::Slider(kind), PART_SLIDER) => {
                let position = isize::from(kind.position(&self.settings.draft));
                let stepped =
                    (position + SLIDER_KEY_STEPS * delta).clamp(0, isize::from(MAX_VOLUME)) as u8;
                kind.set_position(&mut self.settings.draft, stepped);
                self.sync_buttons();
            }
            (RowDef::Choice(choice), PART_ARROW..=PART_ARROW_LAST) => {
                choice.step(&mut self.settings.draft, delta, &self.display);
                self.sync_buttons();
            }
            _ => {}
        }
    }

    /// Escape: answers an open dialog the safe way, or leaves the screen.
    pub fn back(&mut self) -> Option<SettingsEvent> {
        let event = match self.modal {
            Modal::ConfirmDisplay { previous, .. } => Some(self.revert(previous)),
            Modal::Unapplied | Modal::Confirm(_) => {
                self.modal = Modal::None;
                None
            }
            Modal::None => self.request_back(),
        };
        self.sync_buttons();
        event
    }

    /// Advances scrolling and counts down an open display confirmation (reverting when
    /// it runs out).
    pub fn update(&mut self, dt: f32) -> Option<SettingsEvent> {
        let scale = self.scale;
        let page = self.page_mut();
        if !page.scroll.is_settled() {
            page.scroll.update(dt);
            page.position(scale);
        }
        self.expire_confirmation(Instant::now())
    }

    pub fn animate(
        &mut self,
        hovered: Option<usize>,
        pressed: Option<usize>,
        captured: Option<usize>,
        dt: f32,
    ) {
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            let id = Some(TAB_ID + i);
            tab.animate(hovered == id, pressed == id, dt);
        }
        for (i, button) in self.bottom.iter_mut().enumerate() {
            let id = Some(BOTTOM_ID + i);
            button.animate(hovered == id, pressed == id, dt);
        }
        for (i, item) in self.page_switch.iter_mut().enumerate() {
            animate_hover(&mut item.hover, hovered == Some(PAGE_ID + i), dt);
        }
        let scrollbar_active = hovered == Some(SCROLLBAR_ID) || captured == Some(SCROLLBAR_ID);
        animate_hover(&mut self.scrollbar_hover, scrollbar_active, dt);
        let index = self.page_index();
        for (i, row) in self.pages[index].rows.iter_mut().enumerate() {
            match &mut row.widget {
                RowWidget::Slider { slider, .. } => {
                    let id = Some(row_id(i, PART_SLIDER));
                    slider.animate(hovered == id || captured == id, dt);
                }
                RowWidget::Selector(selector) => {
                    let hovered_side = [ARROW_LEFT, ARROW_RIGHT]
                        .map(|side| hovered == Some(row_id(i, PART_ARROW + side)));
                    selector.animate(hovered_side, dt);
                }
                RowWidget::Heading | RowWidget::Info(_) => {}
            }
        }
        self.confirm_dialog.animate(hovered, pressed, DIALOG_ID, dt);
        self.unapplied_dialog
            .animate(hovered, pressed, DIALOG_ID, dt);
        for dialog in &mut self.action_dialogs {
            dialog.animate(hovered, pressed, DIALOG_ID, dt);
        }
    }

    pub fn build(&mut self, frame: &mut UiFrame, renderer: &mut UiRenderer, kit: &UiKit) {
        let scale = self.scale;
        for tab in &self.tabs {
            tab.draw(frame, &kit.button, scale);
        }
        for button in &self.bottom {
            button.draw(frame, &kit.button, scale);
        }
        widgets::push_label(
            frame,
            self.headers[self.tab as usize],
            self.header_rect,
            (HEADER_TEXT_SIZE * scale).round(),
            palette::TEXT,
            false,
        );
        widgets::push_image(
            &mut frame.group_sprites,
            kit.brush,
            self.brush_rect,
            FULL_UV,
            SpriteMaterial::PLAIN,
        );
        if self.tab == Tab::Graphics {
            self.draw_page_switch(frame, kit);
        }

        let mark = frame.group_mark();
        self.draw_rows(frame, renderer, kit);
        frame.clip_group_since(mark, self.list_rect);
        self.draw_scrollbar(frame, kit);

        if let Modal::ConfirmDisplay { deadline, .. } = self.modal {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let seconds = remaining.as_secs_f32().ceil().max(1.0) as u32;
            let text = self.language().reverting_in(seconds);
            self.confirm_dialog.set_message(renderer, &text);
        }
        if let Some(dialog) = self.active_dialog() {
            dialog.draw(frame, kit);
        }
    }

    fn draw_page_switch(&self, frame: &mut UiFrame, kit: &UiKit) {
        let scale = self.scale;
        let size = (PAGE_SWITCH_TEXT_SIZE * scale).round();
        for (i, item) in self.page_switch.iter().enumerate() {
            let selected = self.advanced == (i == 1);
            let color = if selected {
                palette::HEADING
            } else {
                mix_color(palette::TEXT_DISABLED, palette::TEXT, item.hover.value())
            };
            widgets::push_label(frame, item.label, item.rect, size, color, false);
            if selected {
                let line_h = (2.0 * scale).round().max(1.0);
                let inset = (20.0 * scale).round();
                let rect = item.rect;
                widgets::push_flat(
                    &mut frame.group_sprites,
                    kit,
                    Rect::new(
                        rect.x + inset,
                        rect.y + rect.h,
                        rect.w - 2.0 * inset,
                        line_h,
                    ),
                    palette::HEADING,
                );
            }
        }
    }

    fn draw_rows(&mut self, frame: &mut UiFrame, renderer: &mut UiRenderer, kit: &UiKit) {
        let scale = self.scale;
        let row_text = (ROW_TEXT_SIZE * scale).round();
        let heading_text = (HEADING_TEXT_SIZE * scale).round();
        let SettingsDraft { applied, draft } = self.settings;
        let language = applied.display.language;
        let info = &self.display;
        let list = self.list_rect;
        let page = &mut self.pages[page_index(self.tab, self.advanced)];
        let visible = page
            .rows
            .iter_mut()
            .filter(|row| row.top.is_some() && row.bounds.overlaps(&list));
        for row in visible {
            let name = RowName {
                label: row.name,
                rect: row.name_rect,
                underline: row.underline,
                font_size: row_text,
            };
            match (&mut row.widget, row.def) {
                (RowWidget::Heading, _) => {
                    widgets::push_label(
                        frame,
                        row.name,
                        row.name_rect,
                        heading_text,
                        palette::HEADING,
                        false,
                    );
                }
                (
                    RowWidget::Slider {
                        slider,
                        value,
                        shown,
                    },
                    RowDef::Slider(kind),
                ) => {
                    let text = kind.text(&draft);
                    let color = if text == kind.text(&applied) {
                        palette::TEXT
                    } else {
                        palette::TEXT_PENDING
                    };
                    if shown.as_deref() != Some(text.as_str()) {
                        renderer.set_label_text(*value, &text);
                        *shown = Some(text);
                    }
                    name.push(frame, kit, palette::TEXT);
                    slider.draw(&mut frame.group_sprites, kit, kind.position(&draft));
                    widgets::push_label(frame, *value, row.value_rect, row_text, color, false);
                }
                (RowWidget::Selector(selector), RowDef::Choice(choice)) => {
                    let text = choice.text(&draft, info, language);
                    selector.set_text(renderer, &text);
                    name.push(frame, kit, palette::TEXT);
                    let active = choice.is_active(&draft, info);
                    let enabled = [choice.can_step(&draft, info); 2];
                    let color = if !active {
                        palette::TEXT_DISABLED
                    } else if text == choice.text(&applied, info, language) {
                        palette::TEXT
                    } else {
                        palette::TEXT_PENDING
                    };
                    let selected = if active {
                        choice.current(&draft, info)
                    } else {
                        None
                    };
                    selector.draw(
                        frame,
                        kit,
                        row_text,
                        SelectorState {
                            selected,
                            count: choice.count(&draft, info),
                            enabled,
                            value_color: color,
                        },
                    );
                }
                (RowWidget::Info(value), _) => {
                    name.push(frame, kit, palette::TEXT_DISABLED);
                    widgets::push_label(
                        frame,
                        *value,
                        row.value_rect,
                        row_text,
                        palette::TEXT_DISABLED,
                        false,
                    );
                }
                _ => {}
            }
        }
    }

    fn draw_scrollbar(&self, frame: &mut UiFrame, kit: &UiKit) {
        let Some((track, thumb)) = self.scrollbar_rects() else {
            return;
        };
        widgets::push_flat(&mut frame.group_sprites, kit, track, palette::SEGMENT_OFF);
        let color = mix_color(
            palette::SCROLL_THUMB,
            palette::SLIDER_FILL,
            self.scrollbar_hover.value(),
        );
        widgets::push_flat(&mut frame.group_sprites, kit, thumb, color);
    }

    /// Track and thumb (physical px) when the current list scrolls.
    fn scrollbar_rects(&self) -> Option<(Rect, Rect)> {
        let page = self.page();
        if page.max_scroll() <= 0.0 {
            return None;
        }
        let scale = self.scale;
        let [x, w] = SCROLLBAR;
        let (top, h) = thumb_geometry(
            LIST_TOP,
            LIST_HEIGHT,
            page.content_height,
            page.scroll.value(),
        );
        Some((
            scaled(scale, x, LIST_TOP, w, LIST_HEIGHT),
            scaled(scale, x, top, w, h),
        ))
    }

    fn scrollbar_target(&self) -> Option<HitTarget<'static>> {
        let (track, _) = self.scrollbar_rects()?;
        let pad = (SCROLLBAR_GRAB * self.scale).round();
        Some(HitTarget {
            id: SCROLLBAR_ID,
            shape: HitShape::Rectangle(Rect::new(
                track.x - pad,
                track.y,
                track.w + 2.0 * pad,
                track.h,
            )),
            interactive: true,
        })
    }

    fn handle_click(&mut self, id: usize) -> Option<SettingsEvent> {
        if let Some(offset) = id.checked_sub(DIALOG_ID) {
            return self.dialog_click(offset);
        }
        if let Some((row, part)) = split_row_id(id) {
            if let Some(side) = part.checked_sub(PART_ARROW)
                && let Some(RowDef::Choice(choice)) = self.page().rows.get(row).map(|row| row.def)
            {
                let delta = if side == ARROW_LEFT { -1 } else { 1 };
                choice.step(&mut self.settings.draft, delta, &self.display);
            }
            return None; // Slider values change on press/drag.
        }
        if id == SCROLLBAR_ID {
            self.scrollbar_grab = None;
            return None;
        }
        if let Some(page) = id.checked_sub(PAGE_ID).filter(|&page| page < 2) {
            self.set_advanced(page == 1);
            return None;
        }
        if let Some(offset) = id.checked_sub(BOTTOM_ID).filter(|&offset| offset < 4) {
            return match offset {
                BACK => self.request_back(),
                RESET => self.ask(ConfirmAction::Reset),
                APPLY => self.ask(ConfirmAction::Apply),
                _ => self.ask(ConfirmAction::Default),
            };
        }
        if let Some(&(tab, _, _)) = TABS.get(id - TAB_ID) {
            self.select_tab(tab);
        }
        None
    }

    fn dialog_click(&mut self, button: usize) -> Option<SettingsEvent> {
        match (self.modal, button) {
            // Kept: now it is saved.
            (Modal::ConfirmDisplay { leave_after, .. }, DIALOG_CONFIRM) => {
                self.modal = Modal::None;
                Some(SettingsEvent::Apply {
                    settings: self.settings.applied,
                    save: true,
                    leave: leave_after,
                })
            }
            (Modal::ConfirmDisplay { previous, .. }, DIALOG_CANCEL) => Some(self.revert(previous)),
            (Modal::Unapplied, DIALOG_CONFIRM) => {
                self.modal = Modal::None;
                self.apply(true)
            }
            (Modal::Unapplied, DIALOG_CANCEL) => {
                self.modal = Modal::None;
                self.settings.reset();
                Some(SettingsEvent::BackToMenu)
            }
            (Modal::Confirm(action), DIALOG_CONFIRM) => {
                self.modal = Modal::None;
                self.run(action)
            }
            (Modal::Confirm(_), DIALOG_CANCEL) => {
                self.modal = Modal::None;
                None
            }
            _ => None,
        }
    }

    /// Opens the confirmation for `action`.
    fn ask(&mut self, action: ConfirmAction) -> Option<SettingsEvent> {
        self.modal = Modal::Confirm(action);
        self.action_dialogs[action as usize].show();
        None
    }

    /// Runs a confirmed action. Apply may open the display confirmation next.
    fn run(&mut self, action: ConfirmAction) -> Option<SettingsEvent> {
        match action {
            ConfirmAction::Reset => {
                self.settings.reset();
                None
            }
            ConfirmAction::Apply => self.apply(false),
            ConfirmAction::Default => {
                self.settings.draft = self.settings.defaults_for(self.tab, self.native());
                None
            }
        }
    }

    /// Commits the draft. A window change is applied unsaved and opens the confirmation;
    /// otherwise the settings are saved, and the screen is left when asked.
    fn apply(&mut self, leave_after: bool) -> Option<SettingsEvent> {
        let previous = self.settings.applied;
        let applied = self.settings.apply();
        let (event, confirm) = apply_outcome(&previous, applied, leave_after);
        if confirm {
            self.modal = Modal::ConfirmDisplay {
                previous,
                deadline: Instant::now() + Duration::from_secs_f32(CONFIRM_SECONDS),
                leave_after,
            };
            self.confirm_dialog.show();
        }
        Some(event)
    }

    /// Goes back to the last confirmed settings (already on disk; saved again so the file
    /// matches whatever is current).
    fn revert(&mut self, previous: Settings) -> SettingsEvent {
        self.settings.revert_to(previous);
        self.modal = Modal::None;
        SettingsEvent::Apply {
            settings: previous,
            save: true,
            leave: false,
        }
    }

    /// When an unconfirmed window change reverts by itself, if one is waiting.
    pub fn confirmation_deadline(&self) -> Option<Instant> {
        match self.modal {
            Modal::ConfirmDisplay { deadline, .. } => Some(deadline),
            _ => None,
        }
    }

    /// Reverts an unconfirmed window change whose deadline has passed at `now`.
    pub fn expire_confirmation(&mut self, now: Instant) -> Option<SettingsEvent> {
        let previous = self.modal.tick(now)?;
        tracing::info!("display change not confirmed; reverting");
        let event = self.revert(previous);
        self.sync_buttons();
        Some(event)
    }

    fn request_back(&mut self) -> Option<SettingsEvent> {
        if self.settings.has_changes() {
            self.modal = Modal::Unapplied;
            self.unapplied_dialog.show();
            return None;
        }
        Some(SettingsEvent::BackToMenu)
    }

    /// Enables Reset / Apply only with unapplied changes and Default only when the tab
    /// differs from its defaults; re-stacks rows whose visibility depends on values.
    fn sync_buttons(&mut self) {
        let changed = self.settings.has_changes();
        self.bottom[RESET].enabled = changed;
        self.bottom[APPLY].enabled = changed;
        self.bottom[BOTTOM_DEFAULT].enabled =
            self.settings.defaults_for(self.tab, self.native()) != self.settings.draft;
        let draft = self.settings.draft;
        for page in &mut self.pages {
            page.arrange(&draft);
        }
        self.position_rows();
    }

    fn position_rows(&mut self) {
        let scale = self.scale;
        for page in &mut self.pages {
            page.position(scale);
        }
    }

    fn native(&self) -> [u32; 2] {
        self.display.native_resolution(0)
    }

    fn active_dialog(&self) -> Option<&Dialog> {
        match self.modal {
            Modal::None => None,
            Modal::ConfirmDisplay { .. } => Some(&self.confirm_dialog),
            Modal::Unapplied => Some(&self.unapplied_dialog),
            Modal::Confirm(action) => Some(&self.action_dialogs[action as usize]),
        }
    }

    fn page_index(&self) -> usize {
        page_index(self.tab, self.advanced)
    }

    fn page(&self) -> &Page {
        &self.pages[self.page_index()]
    }

    fn page_mut(&mut self) -> &mut Page {
        let index = self.page_index();
        &mut self.pages[index]
    }

    fn select_tab(&mut self, tab: Tab) {
        self.tab = tab;
        for (button, (candidate, _, _)) in self.tabs.iter_mut().zip(TABS) {
            button.selected = candidate == tab;
        }
        self.scrollbar_grab = None;
        self.sync_buttons();
    }

    fn set_advanced(&mut self, advanced: bool) {
        self.advanced = advanced;
        self.scrollbar_grab = None;
        self.page_mut()
            .rows
            .iter_mut()
            .for_each(Row::reset_animation);
    }
}

/// Index of the Default button in the bottom row.
const BOTTOM_DEFAULT: usize = 3;

fn page_index(tab: Tab, advanced: bool) -> usize {
    match tab {
        Tab::Audio => 0,
        Tab::Graphics if advanced => 2,
        Tab::Graphics => 1,
        Tab::Display => 3,
    }
}

fn row_id(row: usize, part: usize) -> usize {
    ROW_ID + ROW_PARTS * row + part
}

/// The row and part of a row widget id.
fn split_row_id(id: usize) -> Option<(usize, usize)> {
    let offset = id.checked_sub(ROW_ID)?;
    (offset < ROW_PARTS * MAX_PAGE_ROWS).then_some((offset / ROW_PARTS, offset % ROW_PARTS))
}

fn animate_hover(tween: &mut Tween, hovered: bool, dt: f32) {
    let target = if hovered { 1.0 } else { 0.0 };
    tween.animate_to(target, HOVER_DURATION, Easing::EaseOutCubic);
    tween.update(dt);
}

/// Blends two sRGB colors (`t` = 0 → `a`, 1 → `b`); enough for small hover accents.
fn mix_color(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| {
        (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8
    })
}

/// A row's name with its underline.
struct RowName {
    label: LabelId,
    rect: Rect,
    underline: Rect,
    font_size: f32,
}

impl RowName {
    fn push(&self, frame: &mut UiFrame, kit: &UiKit, color: [u8; 4]) {
        widgets::push_label(frame, self.label, self.rect, self.font_size, color, false);
        widgets::push_image(
            &mut frame.group_sprites,
            kit.fade_right,
            self.underline,
            FULL_UV,
            SpriteMaterial::tinted(palette::ROW_LINE),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::locale;
    use crate::game::settings::graphics::{OptionalQuality, UpscalingMethod};
    use crate::game::settings::{MonitorInfo, VideoMode};

    const EN: Language = Language::English;

    fn mode(w: u32, h: u32, hz: u32) -> VideoMode {
        VideoMode {
            size: [w, h],
            refresh_millihertz: hz * 1000,
        }
    }

    fn monitor(size: [u32; 2], modes: Vec<VideoMode>) -> MonitorInfo {
        MonitorInfo {
            size,
            refresh_millihertz: Some(60_000),
            modes,
        }
    }

    /// One 1920x1080 monitor (60/144 Hz) with a few smaller modes.
    fn one_monitor() -> DisplayInfo {
        DisplayInfo {
            monitors: vec![monitor(
                [1920, 1080],
                vec![
                    mode(1920, 1080, 60),
                    mode(1920, 1080, 144),
                    mode(1600, 900, 60),
                    mode(1280, 720, 60),
                ],
            )],
            window_frame: [0, 0],
        }
    }

    fn two_monitors() -> DisplayInfo {
        let mut info = one_monitor();
        info.monitors
            .push(monitor([2560, 1440], vec![mode(2560, 1440, 165)]));
        info
    }

    #[test]
    fn row_ids_round_trip() {
        assert_eq!(row_id(0, PART_SLIDER), ROW_ID);
        assert_eq!(
            split_row_id(row_id(7, PART_ARROW + ARROW_RIGHT)),
            Some((7, 2))
        );
        assert_eq!(
            split_row_id(row_id(7, PART_ARROW + ARROW_LEFT)),
            Some((7, 1))
        );
        assert_eq!(split_row_id(ROW_ID - 1), None);
        assert_eq!(split_row_id(DIALOG_ID), None);
    }

    #[test]
    fn windowed_resolution_options_stay_within_the_screen() {
        let mut info = one_monitor();
        info.window_frame = [8, 46];
        let mut s = Settings::default();
        s.display.mode = DisplayMode::Windowed;
        s.display.resolution = [1920, 1080];
        // Full HD does not fit a windowed window on a 1080p screen; smaller sizes do.
        assert_eq!(resolutions(&s, &info), [[1600, 900], [1280, 720]]);
        // Fullscreen keeps every size the monitor offers.
        s.display.mode = DisplayMode::Fullscreen;
        assert!(resolutions(&s, &info).contains(&[1920, 1080]));
    }

    #[test]
    fn switching_to_windowed_snaps_an_oversized_resolution() {
        let mut info = one_monitor();
        info.window_frame = [8, 46];
        let mut s = Settings::default();
        s.display.mode = DisplayMode::Fullscreen;
        s.display.resolution = [1920, 1080];
        Choice::DisplayMode.select(&mut s, DisplayMode::Windowed.index(), &info);
        // The largest offered size that still fits, not the unlisted screen size.
        assert_eq!(s.display.resolution, [1600, 900]);
        // Switching back to fullscreen keeps the snapped size (it is a real video mode).
        Choice::DisplayMode.select(&mut s, DisplayMode::Fullscreen.index(), &info);
        assert_eq!(s.display.resolution, [1600, 900]);
    }

    #[test]
    fn changing_monitor_snaps_a_windowed_resolution_to_the_new_screen() {
        let mut info = one_monitor();
        info.window_frame = [8, 46];
        // A 1600x900 monitor: the frame leaves 1592x854, so 1600x900 no longer fits.
        info.monitors.push(monitor(
            [1600, 900],
            vec![mode(1600, 900, 60), mode(1280, 720, 60)],
        ));
        let mut s = Settings::default();
        s.display.mode = DisplayMode::Windowed;
        s.display.resolution = [1600, 900];
        Choice::Monitor.select(&mut s, 1, &info);
        assert_eq!(s.display.resolution, [1280, 720]);
    }

    #[test]
    fn graphics_pages_list_every_option_once() {
        let listed: Vec<GraphicsOption> = GRAPHICS_BASIC_ROWS
            .iter()
            .chain(&GRAPHICS_ADVANCED_ROWS)
            .filter_map(|def| match def {
                RowDef::Choice(Choice::Graphics(option)) => Some(*option),
                _ => None,
            })
            .collect();
        for option in GraphicsOption::ALL {
            let count = listed.iter().filter(|&&o| o == option).count();
            assert_eq!(count, 1, "{option:?}");
        }
        assert_eq!(listed.len(), GraphicsOption::ALL.len());
    }

    #[test]
    fn hidden_rows_take_no_space() {
        let s = Settings::default();
        let (tops, height) =
            stack_rows(GRAPHICS_BASIC_ROWS.into_iter(), |def| row_visible(def, &s));
        let quality = GRAPHICS_BASIC_ROWS
            .iter()
            .position(|&d| d == graphics(GraphicsOption::UpscalingQuality))
            .unwrap();
        assert_eq!(tops[quality], None, "no upscaler: Upscaling Quality hidden");
        assert_eq!(tops[0], Some(0.0));
        assert_eq!(tops[1], Some(HEADING_PITCH));
        assert_eq!(
            tops[quality + 1],
            tops[quality - 1].map(|top| top + ROW_PITCH)
        );
        let headings = GRAPHICS_BASIC_ROWS
            .iter()
            .filter(|d| matches!(d, RowDef::Heading(_)))
            .count();
        let rows = GRAPHICS_BASIC_ROWS.len() - headings - 1;
        assert_eq!(
            height,
            headings as f32 * HEADING_PITCH + rows as f32 * ROW_PITCH
        );
    }

    #[test]
    fn scrollbar_thumb_maps_offsets_both_ways() {
        let (top, h) = thumb_geometry(100.0, 400.0, 800.0, 0.0);
        assert_eq!((top, h), (100.0, 200.0));
        let (top, _) = thumb_geometry(100.0, 400.0, 800.0, 400.0);
        assert_eq!(top, 300.0, "fully scrolled: thumb at the bottom");
        assert_eq!(offset_for_thumb(100.0, 400.0, 800.0, 200.0), 200.0);
        assert_eq!(offset_for_thumb(100.0, 400.0, 800.0, -50.0), 0.0);
        assert_eq!(offset_for_thumb(100.0, 400.0, 800.0, 999.0), 400.0);
        // Short content: the thumb fills the track and nothing scrolls.
        assert_eq!(thumb_geometry(0.0, 400.0, 300.0, 50.0), (0.0, 400.0));
        assert_eq!(offset_for_thumb(0.0, 400.0, 300.0, 100.0), 0.0);
        // Very long content keeps a grabbable thumb.
        assert_eq!(
            thumb_geometry(0.0, 400.0, 100_000.0, 0.0).1,
            SCROLLBAR_MIN_THUMB
        );
    }

    #[test]
    fn reset_discards_unapplied_changes_on_all_tabs() {
        let mut settings = SettingsDraft::new(Settings::default());
        settings.draft.audio.set(AudioChannel::Music, 80);
        settings.draft.display.vsync = false;
        settings.draft.graphics.motion_blur = true;
        assert!(settings.has_changes());
        settings.reset();
        assert_eq!(settings.draft, Settings::default());
        assert!(!settings.has_changes());
    }

    #[test]
    fn defaults_cover_only_the_current_tab() {
        let mut applied = Settings::default();
        applied.audio.set(AudioChannel::Voice, 5);
        applied.graphics.shadow_quality = OptionalQuality::Low;
        applied.graphics.film_grain = true;
        applied.display.brightness = 70;
        let settings = SettingsDraft::new(applied);
        let defaults = settings.defaults_for(Tab::Graphics, [1920, 1080]);
        assert_eq!(defaults.graphics, Default::default(), "both graphics pages");
        assert_eq!(defaults.display.brightness, 70, "display untouched");
        assert_eq!(
            defaults.audio.get(AudioChannel::Voice),
            5,
            "audio untouched"
        );
        let mut windowed = settings;
        windowed.draft.display.resolution = [1280, 720];
        let display = windowed.defaults_for(Tab::Display, [2560, 1440]);
        assert_eq!(
            display.display.resolution,
            [2560, 1440],
            "native resolution"
        );
        assert_eq!(display.display.brightness, 50);
        assert!(display.graphics.film_grain, "graphics untouched");
        let mut ukrainian = settings;
        ukrainian.draft.display.language = Language::Ukrainian;
        let display = ukrainian.defaults_for(Tab::Display, [1920, 1080]);
        assert_eq!(
            display.display.language,
            Language::Ukrainian,
            "language stays"
        );
    }

    #[test]
    fn every_menu_text_is_translated() {
        // Headings and the screen's own texts; setting names stay in English.
        let mut texts: Vec<&str> = PAGES
            .iter()
            .flat_map(|page| page.iter())
            .filter_map(|def| match *def {
                RowDef::Heading(title) => Some(title),
                _ => None,
            })
            .collect();
        texts.extend(TABS.iter().flat_map(|&(_, name, title)| [name, title]));
        texts.extend([
            "Basic",
            "Advanced",
            "Back to Menu",
            "Reset",
            "Apply",
            "Default",
        ]);
        texts.extend(["Apply changes?", "Discard", "Keep these display settings?"]);
        texts.extend(["Keep", "Revert", UNAPPLIED_MESSAGE, NOT_SUPPORTED, CANCEL]);
        texts.extend(ConfirmAction::ALL.iter().flat_map(|action| action.texts()));
        for text in texts {
            assert!(locale::is_translated(text), "{text}");
        }
    }

    #[test]
    fn values_follow_the_interface_language() {
        let info = one_monitor();
        let mut s = Settings::default();
        let uk = Language::Ukrainian;
        assert_eq!(Choice::Overall.text(&s, &info, uk), "Висока");
        assert_eq!(Choice::DisplayMode.text(&s, &info, uk), "Повноекранний");
        assert_eq!(Choice::RefreshRate.text(&s, &info, uk), "144 Гц");
        assert_eq!(Choice::Monitor.text(&s, &info, uk), "Основний");
        assert_eq!(Choice::VSync.text(&s, &info, uk), "Увімк.");
        assert_eq!(Choice::Resolution.text(&s, &info, uk), "1920 x 1080");
        // The language row names each language in that language.
        assert_eq!(Choice::Language.text(&s, &info, EN), "English");
        Choice::Language.step(&mut s, 1, &info);
        assert_eq!(s.display.language, uk);
        assert_eq!(Choice::Language.text(&s, &info, EN), "Українська");
        Choice::Language.step(&mut s, 1, &info);
        assert_eq!(s.display.language, EN, "wraps to the first language");
    }

    #[test]
    fn revert_restores_applied_and_draft() {
        let previous = Settings::default();
        let mut settings = SettingsDraft::new(previous);
        settings.draft.display.mode = DisplayMode::Windowed;
        settings.apply();
        settings.revert_to(previous);
        assert_eq!(settings.applied, previous);
        assert_eq!(settings.draft, previous);
    }

    #[test]
    fn unconfirmed_display_change_reverts_at_the_deadline() {
        let mut previous = Settings::default();
        previous.display.resolution = [1600, 900];
        let start = Instant::now();
        let mut modal = Modal::ConfirmDisplay {
            previous,
            deadline: start + Duration::from_secs_f32(CONFIRM_SECONDS),
            leave_after: false,
        };
        // Frame timing plays no part: only the clock does.
        assert_eq!(modal.tick(start), None);
        assert_eq!(modal.tick(start + Duration::from_secs(14)), None);
        assert!(matches!(modal, Modal::ConfirmDisplay { .. }));
        // Past the deadline (e.g. after a long freeze) it reverts at once, exactly once.
        assert_eq!(modal.tick(start + Duration::from_secs(60)), Some(previous));
        assert_eq!(modal, Modal::None);
        assert_eq!(modal.tick(start + Duration::from_secs(61)), None);
        assert_eq!(
            Modal::Unapplied.tick(start + Duration::from_secs(100)),
            None,
            "only the confirmation counts down"
        );
    }

    #[test]
    fn window_changes_are_saved_only_once_kept() {
        let previous = Settings::default();
        let mut windowed = previous;
        windowed.display.mode = DisplayMode::Windowed;
        let (event, confirm) = apply_outcome(&previous, windowed, true);
        assert!(confirm);
        assert_eq!(
            event,
            SettingsEvent::Apply {
                settings: windowed,
                save: false,
                leave: false
            },
            "applied, not saved, and the screen stays for the confirmation"
        );
        let mut louder = previous;
        louder.audio.set(AudioChannel::Music, 90);
        let (event, confirm) = apply_outcome(&previous, louder, true);
        assert!(!confirm);
        assert_eq!(
            event,
            SettingsEvent::Apply {
                settings: louder,
                save: true,
                leave: true
            }
        );
    }

    #[test]
    fn calibration_sliders_cover_their_ranges() {
        let mut s = Settings::default();
        assert_eq!(SliderKind::Brightness.position(&s), 50);
        assert_eq!(SliderKind::Brightness.text(&s), "50%");
        SliderKind::Brightness.set_position(&mut s, 255);
        assert_eq!(s.display.brightness, 100);

        assert_eq!(SliderKind::Gamma.text(&s), "1.00");
        assert_eq!(SliderKind::Gamma.position(&s), 25);
        SliderKind::Gamma.set_position(&mut s, 0);
        assert_eq!(s.display.gamma, MIN_GAMMA);
        SliderKind::Gamma.set_position(&mut s, 100);
        assert_eq!(SliderKind::Gamma.text(&s), "2.50");

        assert_eq!(SliderKind::UiScale.position(&s), 20);
        SliderKind::UiScale.set_position(&mut s, 20);
        assert_eq!(s.display.ui_scale, 100, "round trip keeps 100%");
        SliderKind::UiScale.set_position(&mut s, 0);
        assert_eq!(s.display.ui_scale, MIN_UI_SCALE);
        SliderKind::UiScale.set_position(&mut s, 100);
        assert_eq!(SliderKind::UiScale.text(&s), "200%");

        SliderKind::ImageSharpening.set_position(&mut s, 30);
        assert_eq!(s.graphics.image_sharpening, 30);
        assert_eq!(SliderKind::ImageSharpening.text(&s), "30%");
    }

    #[test]
    fn resolution_follows_the_display_mode() {
        let info = one_monitor();
        let mut s = Settings::default();
        assert!(
            Choice::Resolution.is_active(&s, &info),
            "fullscreen needs a video mode size"
        );
        assert!(Choice::Resolution.can_step(&s, &info));
        Choice::DisplayMode.step(&mut s, -1, &info);
        assert_eq!(s.display.mode, DisplayMode::Windowed);
        assert!(Choice::Resolution.can_step(&s, &info));
        Choice::Resolution.step(&mut s, 1, &info);
        assert_eq!(s.display.resolution, [1600, 900]);
        assert_eq!(Choice::DisplayMode.text(&s, &info, EN), "Windowed");
        assert_eq!(Choice::DisplayMode.count(&s, &info), 2);
    }

    #[test]
    fn refresh_rate_applies_to_exclusive_fullscreen_only() {
        let info = one_monitor();
        let mut s = Settings::default();
        assert!(
            Choice::RefreshRate.is_active(&s, &info),
            "exclusive fullscreen"
        );
        assert_eq!(
            Choice::RefreshRate.text(&s, &info, EN),
            "144 Hz",
            "highest by default"
        );
        s.display.mode = DisplayMode::Windowed;
        assert!(!Choice::RefreshRate.is_active(&s, &info));
        assert_eq!(
            Choice::RefreshRate.text(&s, &info, EN),
            "60 Hz",
            "desktop rate"
        );
        s.display.mode = DisplayMode::Fullscreen;
        assert!(Choice::RefreshRate.is_active(&s, &info));
        assert_eq!(
            Choice::RefreshRate.text(&s, &info, EN),
            "144 Hz",
            "highest by default"
        );
        Choice::RefreshRate.step(&mut s, -1, &info);
        assert_eq!(s.display.refresh_rate, 60_000);
        Choice::RefreshRate.step(&mut s, -1, &info);
        assert_eq!(s.display.refresh_rate, 144_000, "wraps to the highest");
        // A size without video modes offers no refresh rates.
        s.display.resolution = [1366, 768];
        assert!(!Choice::RefreshRate.is_active(&s, &info));
    }

    #[test]
    fn switching_to_fullscreen_snaps_to_a_real_video_mode() {
        let info = one_monitor();
        let mut s = Settings::default();
        s.display.mode = DisplayMode::Windowed;
        s.display.resolution = [1500, 800];
        Choice::DisplayMode.step(&mut s, 1, &info); // Fullscreen
        assert_eq!(s.display.resolution, [1920, 1080]);
    }

    #[test]
    fn monitor_choice_needs_a_second_monitor() {
        let mut s = Settings::default();
        let single = one_monitor();
        assert!(!Choice::Monitor.is_active(&s, &single));
        assert_eq!(Choice::Monitor.text(&s, &single, EN), "Primary");
        let dual = two_monitors();
        assert!(Choice::Monitor.is_active(&s, &dual));
        s.display.mode = DisplayMode::Fullscreen;
        Choice::Monitor.step(&mut s, 1, &dual);
        assert_eq!(s.display.monitor, 1);
        assert_eq!(Choice::Monitor.text(&s, &dual, EN), "Secondary");
        assert_eq!(
            s.display.resolution,
            [2560, 1440],
            "native size of that monitor"
        );
        // The monitor was unplugged: the primary is used and shown.
        assert_eq!(Choice::Monitor.text(&s, &single, EN), "Primary");
    }

    #[test]
    fn overall_sets_presets_and_shows_custom() {
        let info = one_monitor();
        let mut s = Settings::default();
        assert_eq!(Choice::Overall.text(&s, &info, EN), "High");
        Choice::Overall.step(&mut s, 1, &info);
        assert_eq!(s.graphics.overall(), Some(Quality::Ultra));
        Choice::Overall.step(&mut s, 1, &info);
        assert_eq!(
            s.graphics.overall(),
            Some(Quality::Low),
            "past Ultra comes Low"
        );
        Choice::Overall.step(&mut s, -1, &info);
        assert_eq!(s.graphics.overall(), Some(Quality::Ultra));
        let shadows = Choice::Graphics(GraphicsOption::ShadowQuality);
        shadows.step(&mut s, -1, &info);
        assert_eq!(s.graphics.shadow_quality, OptionalQuality::High);
        assert_eq!(Choice::Overall.text(&s, &info, EN), "Custom");
        assert_eq!(Choice::Overall.current(&s, &info), None);
        // From Custom either arrow snaps to the closest preset.
        Choice::Overall.step(&mut s, -1, &info);
        assert_eq!(s.graphics.overall(), Some(Quality::Ultra));
    }

    #[test]
    fn locked_graphics_rows_do_not_step() {
        let info = one_monitor();
        let mut s = Settings::default();
        let upscaler = Choice::Graphics(GraphicsOption::UpscalingMethod);
        assert!(!upscaler.can_step(&s, &info));
        upscaler.step(&mut s, 1, &info);
        assert_eq!(s.graphics.upscaling_method, UpscalingMethod::Off);
        let target = Choice::Graphics(GraphicsOption::DynamicResolutionTarget);
        assert!(!target.is_active(&s, &info));
        Choice::Graphics(GraphicsOption::DynamicResolution).step(&mut s, 1, &info);
        assert!(target.is_active(&s, &info));
        assert!(!Choice::Graphics(GraphicsOption::ResolutionScale).is_active(&s, &info));
    }

    #[test]
    fn display_choices_read_and_write_settings() {
        let info = one_monitor();
        let mut s = Settings::default();
        assert_eq!(Choice::VSync.text(&s, &info, EN), "On");
        Choice::VSync.step(&mut s, -1, &info);
        assert!(!s.display.vsync);
        assert_eq!(Choice::FrameRateLimit.text(&s, &info, EN), "Unlimited");
        Choice::FrameRateLimit.step(&mut s, -1, &info);
        assert_eq!(s.display.frame_rate_limit.fps(), Some(240));
        assert_eq!(Choice::BackgroundFrameRateLimit.text(&s, &info, EN), "30");
        Choice::BackgroundFrameRateLimit.step(&mut s, -1, &info);
        assert_eq!(s.display.background_frame_rate_limit.fps(), Some(15));
        Choice::ShowFps.step(&mut s, 1, &info);
        assert!(s.display.show_fps);
        assert_eq!(Choice::AspectRatio.text(&s, &info, EN), "Auto");
        Choice::AspectRatio.step(&mut s, 1, &info);
        assert_eq!(Choice::AspectRatio.text(&s, &info, EN), "16:9");
    }

    #[test]
    fn unlisted_resolution_is_not_offered_but_stays_selectable() {
        let info = one_monitor();
        let mut s = Settings::default();
        s.display.mode = DisplayMode::Windowed;
        s.display.resolution = [1000, 700];
        // Only video-mode sizes are offered; the saved odd size is not among them.
        let options = resolutions(&s, &info);
        assert!(!options.contains(&[1000, 700]));
        assert_eq!(options[0], [1920, 1080]);
        assert!(Choice::Resolution.current(&s, &info).is_none());
        // The arrows still work from an unlisted value: they go to the largest offered.
        Choice::Resolution.step(&mut s, 1, &info);
        assert_eq!(s.display.resolution, [1920, 1080]);
    }

    #[test]
    fn aspect_ratio_options_match_the_monitor() {
        let info = one_monitor();
        let mut s = Settings::default();
        // The 1920x1080 monitor only offers 16:9 sizes: Auto and 16:9, nothing else.
        assert_eq!(
            aspect_ratios(&s, &info),
            [AspectRatio::Auto, AspectRatio::Wide16x9]
        );
        assert_eq!(Choice::AspectRatio.count(&s, &info), 2);
        Choice::AspectRatio.step(&mut s, 1, &info);
        assert_eq!(s.display.aspect_ratio, AspectRatio::Wide16x9);
        Choice::AspectRatio.step(&mut s, 1, &info);
        assert_eq!(s.display.aspect_ratio, AspectRatio::Auto, "wraps around");
        // A saved ratio the monitor cannot display is not in the list: arrows start
        // from Auto instead.
        s.display.aspect_ratio = AspectRatio::Ultrawide21x9;
        assert!(Choice::AspectRatio.current(&s, &info).is_none());
        Choice::AspectRatio.step(&mut s, -1, &info);
        assert_eq!(s.display.aspect_ratio, AspectRatio::Auto);
    }

    #[test]
    fn slider_and_value_fill_the_selector_block() {
        let (track, value) = slider_rects(1.0, 300.0);
        assert_eq!(track.x, SELECTOR[0], "bar starts at the left arrow's edge");
        assert_eq!(
            value.x + value.w,
            SELECTOR[0] + SELECTOR[1],
            "value ends at the right arrow's edge"
        );
        assert!(
            track.x + track.w + KNOB_SIZE / 2.0 < value.x,
            "knob at 100 % clears the value"
        );
        assert_eq!(
            track.y + track.h / 2.0,
            value.y + value.h / 2.0,
            "bar and value share a center"
        );
        // Equal gaps to the row above (its strip ends 16 px above this row) and below (its
        // arrows start 3 px into the next row).
        let above = 300.0 + ROW_HEIGHT - ROW_PITCH;
        let below = 300.0 + ROW_PITCH + (ROW_HEIGHT - 34.0) / 2.0;
        let gap_up = track.y - above;
        let gap_down = below - (track.y + track.h);
        assert!((gap_up - gap_down).abs() <= 1.0, "{gap_up} vs {gap_down}");
    }

    #[test]
    fn setting_names_keep_their_original_names() {
        let uk = Language::Ukrainian;
        let overall = RowDef::Choice(Choice::Overall);
        assert_eq!(row_name(overall, uk), "Overall Quality");
        assert_eq!(
            row_name(graphics(GraphicsOption::AntiAliasing), uk),
            "Anti-Aliasing"
        );
        assert_eq!(
            row_name(RowDef::Slider(SliderKind::Brightness), uk),
            "Brightness"
        );
        assert_eq!(row_name(volume(AudioChannel::Master), uk), "Master");
        assert_eq!(row_name(RowDef::Unavailable("HDR"), uk), "HDR");
        // Section headings are still translated (and upper-cased).
        assert_eq!(row_name(RowDef::Heading("General"), uk), "ЗАГАЛЬНІ");
        assert_eq!(row_name(RowDef::Heading("General"), EN), "GENERAL");
    }

    #[test]
    fn every_bottom_action_has_its_own_confirmation() {
        // Dialog order follows the enum values used to index them.
        for (index, action) in ConfirmAction::ALL.into_iter().enumerate() {
            assert_eq!(action as usize, index);
        }
        let titles: Vec<&str> = ConfirmAction::ALL.iter().map(|a| a.texts()[0]).collect();
        assert_eq!(
            titles,
            ["Reset changes?", "Apply settings?", "Restore defaults?"]
        );
        assert_eq!(ConfirmAction::Apply.texts()[2], "Apply");
    }

    #[test]
    fn selectors_cycle_in_both_directions() {
        let info = one_monitor();
        let mut s = Settings::default();
        assert!(Choice::FrameRateLimit.can_step(&s, &info));
        Choice::FrameRateLimit.step(&mut s, 1, &info);
        assert_eq!(
            s.display.frame_rate_limit,
            FrameRateLimit::Fps30,
            "Unlimited -> 30"
        );
        Choice::FrameRateLimit.step(&mut s, -1, &info);
        assert_eq!(s.display.frame_rate_limit, FrameRateLimit::Unlimited);
        s.display.mode = DisplayMode::Windowed;
        Choice::DisplayMode.step(&mut s, -1, &info);
        assert_eq!(s.display.mode, DisplayMode::Fullscreen, "first -> last");
        assert_eq!(wrap_index(0, -1, 5), 4);
        assert_eq!(wrap_index(4, 1, 5), 0);
        assert_eq!(wrap_index(2, 1, 5), 3);
        assert_eq!(wrap_index(0, 1, 0), 0);
        // A single value has nowhere to go.
        assert!(!Choice::Monitor.can_step(&s, &info));
    }

    #[test]
    fn colors_mix_linearly_enough() {
        assert_eq!(
            mix_color([0, 0, 0, 255], [200, 100, 50, 255], 0.5),
            [100, 50, 25, 255]
        );
        assert_eq!(mix_color([10; 4], [20; 4], 2.0), [20; 4]);
    }
}
