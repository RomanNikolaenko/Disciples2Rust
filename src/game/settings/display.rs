//! Display options and what the connected monitors offer.
//!
//! Monitor numbering: 0 is the primary monitor, then the others in the order the OS lists
//! them. Resolutions and refresh rates come from the selected monitor's video modes, so
//! they match the hardware; the application gathers them into a [`DisplayInfo`].

use serde::{Deserialize, Serialize};

use super::OptionValue;
use crate::game::locale::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DisplayMode {
    Windowed,
    /// A borderless window covering the monitor at the desktop resolution. Not offered
    /// any more; older settings files map it to [`DisplayMode::Fullscreen`] when loaded.
    #[serde(alias = "WindowedFullscreen")]
    Borderless,
    /// Exclusive fullscreen with a video-mode change.
    #[default]
    Fullscreen,
}

impl OptionValue for DisplayMode {
    const ALL: &'static [Self] = &[Self::Windowed, Self::Fullscreen];

    fn label(self) -> &'static str {
        match self {
            Self::Windowed => "Windowed",
            Self::Borderless => "Borderless",
            Self::Fullscreen => "Fullscreen",
        }
    }
}

/// Aspect ratio of the 3D view; Auto follows the resolution. Stored for the 3D scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AspectRatio {
    #[default]
    Auto,
    Wide16x9,
    Wide16x10,
    Ultrawide21x9,
    SuperUltrawide32x9,
}

impl AspectRatio {
    /// The preset whose aspect matches `size` within a small tolerance: video modes are
    /// not always exact (1366x768 is 16:9 within a fraction of a percent). `None` for a
    /// size no preset covers (e.g. 4:3).
    pub fn from_size(size: [u32; 2]) -> Option<Self> {
        if size[0] == 0 || size[1] == 0 {
            return None;
        }
        const TOLERANCE: f64 = 0.02;
        let aspect = f64::from(size[0]) / f64::from(size[1]);
        const PRESETS: [(AspectRatio, f64); 4] = [
            (AspectRatio::Wide16x9, 16.0 / 9.0),
            (AspectRatio::Wide16x10, 16.0 / 10.0),
            (AspectRatio::Ultrawide21x9, 21.0 / 9.0),
            (AspectRatio::SuperUltrawide32x9, 32.0 / 9.0),
        ];
        PRESETS
            .into_iter()
            .find(|&(_, target)| (aspect - target).abs() < TOLERANCE)
            .map(|(ratio, _)| ratio)
    }
}

impl OptionValue for AspectRatio {
    const ALL: &'static [Self] = &[
        Self::Auto,
        Self::Wide16x9,
        Self::Wide16x10,
        Self::Ultrawide21x9,
        Self::SuperUltrawide32x9,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Wide16x9 => "16:9",
            Self::Wide16x10 => "16:10",
            Self::Ultrawide21x9 => "21:9",
            Self::SuperUltrawide32x9 => "32:9",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FrameRateLimit {
    Fps30,
    Fps60,
    Fps90,
    Fps120,
    Fps144,
    Fps165,
    Fps240,
    #[default]
    Unlimited,
}

impl FrameRateLimit {
    pub fn fps(self) -> Option<u32> {
        match self {
            Self::Fps30 => Some(30),
            Self::Fps60 => Some(60),
            Self::Fps90 => Some(90),
            Self::Fps120 => Some(120),
            Self::Fps144 => Some(144),
            Self::Fps165 => Some(165),
            Self::Fps240 => Some(240),
            Self::Unlimited => None,
        }
    }
}

impl OptionValue for FrameRateLimit {
    const ALL: &'static [Self] = &[
        Self::Fps30,
        Self::Fps60,
        Self::Fps90,
        Self::Fps120,
        Self::Fps144,
        Self::Fps165,
        Self::Fps240,
        Self::Unlimited,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Fps30 => "30",
            Self::Fps60 => "60",
            Self::Fps90 => "90",
            Self::Fps120 => "120",
            Self::Fps144 => "144",
            Self::Fps165 => "165",
            Self::Fps240 => "240",
            Self::Unlimited => "Unlimited",
        }
    }
}

/// Applied while the window is not focused, to save power. Higher limits from older
/// files read as Unlimited (the background limit never raises the main one anyway).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BackgroundFrameRateLimit {
    Fps15,
    #[default]
    Fps30,
    Fps60,
    #[serde(alias = "Fps90", alias = "Fps120", alias = "Fps144")]
    #[serde(alias = "Fps165", alias = "Fps240")]
    Unlimited,
}

impl BackgroundFrameRateLimit {
    pub fn fps(self) -> Option<u32> {
        match self {
            Self::Fps15 => Some(15),
            Self::Fps30 => Some(30),
            Self::Fps60 => Some(60),
            Self::Unlimited => None,
        }
    }
}

impl OptionValue for BackgroundFrameRateLimit {
    const ALL: &'static [Self] = &[Self::Fps15, Self::Fps30, Self::Fps60, Self::Unlimited];

    fn label(self) -> &'static str {
        match self {
            Self::Fps15 => "15",
            Self::Fps30 => "30",
            Self::Fps60 => "60",
            Self::Unlimited => "Unlimited",
        }
    }
}

pub const MAX_BRIGHTNESS: u8 = 100;
/// Brightness 50 % leaves the image unchanged.
const DEFAULT_BRIGHTNESS: u8 = 50;
/// Gamma in hundredths (0.50–2.50); 1.00 leaves the image unchanged.
pub const MIN_GAMMA: u16 = 50;
pub const MAX_GAMMA: u16 = 250;
const DEFAULT_GAMMA: u16 = 100;
/// HUD scale in percent, in steps of [`UI_SCALE_STEP`].
pub const MIN_UI_SCALE: u16 = 75;
pub const MAX_UI_SCALE: u16 = 200;
pub const UI_SCALE_STEP: u16 = 5;
const DEFAULT_UI_SCALE: u16 = 100;
/// Smallest supported resolution. The game does not target screens (or windows)
/// below 1280x720, so smaller sizes are never offered or accepted.
const MIN_RESOLUTION: [u32; 2] = [1280, 720];
/// Offered when the OS reports no video modes: common 16:9 sizes, largest first.
const FALLBACK_RESOLUTIONS: [[u32; 2]; 6] = [
    [3840, 2160],
    [2560, 1440],
    [1920, 1080],
    [1600, 900],
    [1366, 768],
    [1280, 720],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    /// Interface language. Kept by the Display tab's Default button, so resetting the
    /// display options never switches the menus to a language the player cannot read.
    pub language: Language,
    pub mode: DisplayMode,
    /// 0 = primary, then the other monitors (see the module docs).
    pub monitor: u8,
    /// Window size (Windowed) or exclusive video mode (Fullscreen), physical pixels.
    pub resolution: [u32; 2],
    /// Exclusive fullscreen refresh rate in millihertz; 0 = the highest available.
    pub refresh_rate: u32,
    pub aspect_ratio: AspectRatio,
    pub vsync: bool,
    pub frame_rate_limit: FrameRateLimit,
    pub background_frame_rate_limit: BackgroundFrameRateLimit,
    /// Shows a frames-per-second counter in the top-right corner.
    pub show_fps: bool,
    /// Percent (0–100, 50 = unchanged). Scales the game image — currently the menu
    /// background — but not the interface, so text stays readable at any setting.
    pub brightness: u8,
    /// Hundredths (50–250). Applied to the game image like brightness.
    pub gamma: u16,
    /// HUD scale in percent (75–200). Menus always fit the window; the HUD (now the
    /// FPS counter) follows this.
    pub ui_scale: u16,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            language: Language::default(),
            mode: DisplayMode::default(),
            monitor: 0,
            resolution: [1920, 1080],
            refresh_rate: 0,
            aspect_ratio: AspectRatio::default(),
            vsync: true,
            frame_rate_limit: FrameRateLimit::default(),
            background_frame_rate_limit: BackgroundFrameRateLimit::default(),
            show_fps: false,
            brightness: DEFAULT_BRIGHTNESS,
            gamma: DEFAULT_GAMMA,
            ui_scale: DEFAULT_UI_SCALE,
        }
    }
}

impl DisplaySettings {
    pub(super) fn sanitized(mut self) -> Self {
        // Borderless fullscreen is not offered any more: old settings fall back to
        // exclusive fullscreen.
        if self.mode == DisplayMode::Borderless {
            self.mode = DisplayMode::Fullscreen;
        }
        self.resolution = [
            self.resolution[0].max(MIN_RESOLUTION[0]),
            self.resolution[1].max(MIN_RESOLUTION[1]),
        ];
        self.brightness = self.brightness.min(MAX_BRIGHTNESS);
        self.gamma = self.gamma.clamp(MIN_GAMMA, MAX_GAMMA);
        self.ui_scale = snap_ui_scale(self.ui_scale);
        self
    }

    /// Whether switching to `next` changes the window itself: the mode, the monitor, the
    /// client size (Windowed) or the video mode (Fullscreen), or the refresh rate in
    /// exclusive fullscreen. Such changes are applied right away and must be confirmed.
    pub fn window_changes(&self, next: &Self) -> bool {
        self.mode != next.mode
            || self.monitor != next.monitor
            || self.resolution != next.resolution
            || (self.refresh_rate != next.refresh_rate && next.mode == DisplayMode::Fullscreen)
    }

    /// The frame rate cap for the current focus state.
    pub fn frame_limit(&self, focused: bool) -> Option<u32> {
        let foreground = self.frame_rate_limit.fps();
        if focused {
            return foreground;
        }
        match (foreground, self.background_frame_rate_limit.fps()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Linear-light multiplier for the game image: 0.5 at 0 %, 1 at 50 %, 1.5 at 100 %.
    pub fn brightness_factor(&self) -> f32 {
        0.5 + f32::from(self.brightness.min(MAX_BRIGHTNESS)) / 100.0
    }

    pub fn gamma_value(&self) -> f32 {
        f32::from(self.gamma.clamp(MIN_GAMMA, MAX_GAMMA)) / 100.0
    }

    pub fn ui_scale_factor(&self) -> f32 {
        f32::from(snap_ui_scale(self.ui_scale)) / 100.0
    }
}

/// Clamps to the UI scale range and rounds to the nearest step.
pub fn snap_ui_scale(percent: u16) -> u16 {
    let clamped = percent.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
    let steps = (clamped - MIN_UI_SCALE + UI_SCALE_STEP / 2) / UI_SCALE_STEP;
    (MIN_UI_SCALE + steps * UI_SCALE_STEP).min(MAX_UI_SCALE)
}

/// A video mode a monitor supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoMode {
    pub size: [u32; 2],
    pub refresh_millihertz: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// Current desktop size, physical pixels.
    pub size: [u32; 2],
    /// Current desktop refresh rate, when the OS reports it.
    pub refresh_millihertz: Option<u32>,
    pub modes: Vec<VideoMode>,
}

/// The connected monitors, primary first. Empty when the OS reports none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisplayInfo {
    pub monitors: Vec<MonitorInfo>,
    /// Extra size the window frame (title bar and borders) adds over the client area,
    /// measured on the real window; `[0, 0]` when unknown (e.g. the window has so far
    /// only been fullscreen, where the frame is hidden). Runtime-only, never saved.
    pub window_frame: [u32; 2],
}

impl DisplayInfo {
    /// The monitor at `index`, or the primary one when that monitor is gone.
    pub fn monitor(&self, index: u8) -> Option<&MonitorInfo> {
        self.monitors
            .get(usize::from(index))
            .or_else(|| self.monitors.first())
    }

    pub fn monitor_count(&self) -> usize {
        self.monitors.len()
    }

    /// Largest client area a windowed window may have on `monitor`: the screen size
    /// minus the window frame, so the whole window stays on the screen. `None` when the
    /// frame is unknown or nothing above [`MIN_RESOLUTION`] would fit.
    pub fn windowed_max(&self, monitor: u8) -> Option<[u32; 2]> {
        let info = self.monitor(monitor)?;
        let max = [
            info.size[0].saturating_sub(self.window_frame[0]),
            info.size[1].saturating_sub(self.window_frame[1]),
        ];
        (max[0] >= MIN_RESOLUTION[0] && max[1] >= MIN_RESOLUTION[1]).then_some(max)
    }

    /// Resolutions to offer on a monitor, largest first: exactly the sizes its video
    /// modes report (at least [`MIN_RESOLUTION`]), so every option is supported. When
    /// the OS reports no modes, common sizes that fit the screen are offered instead.
    pub fn resolutions(&self, monitor: u8) -> Vec<[u32; 2]> {
        let info = self.monitor(monitor);
        let mut sizes: Vec<[u32; 2]> = info
            .map(|info| {
                info.modes
                    .iter()
                    .map(|mode| mode.size)
                    .filter(|size| size[0] >= MIN_RESOLUTION[0] && size[1] >= MIN_RESOLUTION[1])
                    .collect()
            })
            .unwrap_or_default();
        if sizes.is_empty() {
            let fits = |size: &[u32; 2]| {
                info.is_none_or(|info| size[0] <= info.size[0] && size[1] <= info.size[1])
            };
            sizes = FALLBACK_RESOLUTIONS.into_iter().filter(fits).collect();
            if sizes.is_empty() {
                sizes.push(info.map_or(FALLBACK_RESOLUTIONS[5], |info| info.size));
            }
        }
        sizes.sort_by(|a, b| (b[0] * b[1], b[0]).cmp(&(a[0] * a[1], a[0])));
        sizes.dedup();
        sizes
    }

    /// Aspect ratios to offer on a monitor: [`AspectRatio::Auto`] plus the presets whose
    /// aspect matches at least one offered resolution. A ratio the hardware cannot
    /// display is not offered. Ordered as in [`AspectRatio::ALL`].
    pub fn aspect_ratios(&self, monitor: u8) -> Vec<AspectRatio> {
        let mut ratios: Vec<AspectRatio> = self
            .resolutions(monitor)
            .iter()
            .filter_map(|&size| AspectRatio::from_size(size))
            .collect();
        ratios.sort_by_key(|ratio| ratio.index());
        ratios.dedup();
        let mut all = vec![AspectRatio::Auto];
        all.extend(ratios);
        all
    }

    /// The monitor's largest resolution (its native size, normally).
    pub fn native_resolution(&self, monitor: u8) -> [u32; 2] {
        let largest = self.monitor(monitor).and_then(|info| {
            info.modes
                .iter()
                .map(|mode| mode.size)
                .max_by_key(|s| s[0] * s[1])
        });
        largest
            .or_else(|| self.monitor(monitor).map(|info| info.size))
            .unwrap_or([1920, 1080])
    }

    /// Refresh rates the monitor offers at `size`, lowest first (millihertz).
    pub fn refresh_rates(&self, monitor: u8, size: [u32; 2]) -> Vec<u32> {
        let mut rates: Vec<u32> = self
            .monitor(monitor)
            .map(|info| {
                info.modes
                    .iter()
                    .filter(|mode| mode.size == size)
                    .map(|mode| mode.refresh_millihertz)
                    .collect()
            })
            .unwrap_or_default();
        rates.sort_unstable();
        rates.dedup();
        rates
    }
}

/// Whether a windowed client area of `size` fits on a screen that allows `max`
/// ([`DisplayInfo::windowed_max`]).
pub fn fits_windowed(size: [u32; 2], max: [u32; 2]) -> bool {
    size[0] <= max[0] && size[1] <= max[1]
}

/// Name shown for a monitor index.
pub fn monitor_label(index: u8, language: Language) -> String {
    match index {
        0 => language.tr("Primary").to_owned(),
        1 => language.tr("Secondary").to_owned(),
        n => format!("{} {}", language.tr("Monitor"), u16::from(n) + 1),
    }
}

/// "144 Hz", or "59.94 Hz" for fractional rates.
pub fn refresh_label(millihertz: u32, language: Language) -> String {
    let unit = language.tr("Hz");
    if millihertz.is_multiple_of(1000) {
        format!("{} {unit}", millihertz / 1000)
    } else {
        format!("{:.2} {unit}", f64::from(millihertz) / 1000.0)
    }
}

/// The rate `stored` resolves to among `rates`: itself when offered, else the highest.
pub fn resolve_refresh(rates: &[u32], stored: u32) -> Option<u32> {
    if rates.contains(&stored) {
        Some(stored)
    } else {
        rates.last().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(w: u32, h: u32, hz: u32) -> VideoMode {
        VideoMode {
            size: [w, h],
            refresh_millihertz: hz * 1000,
        }
    }

    fn two_monitors() -> DisplayInfo {
        DisplayInfo {
            monitors: vec![
                MonitorInfo {
                    size: [1920, 1080],
                    refresh_millihertz: Some(60_000),
                    modes: vec![
                        mode(1920, 1080, 60),
                        mode(1920, 1080, 144),
                        mode(1280, 720, 60),
                        mode(800, 600, 60),
                        mode(640, 480, 60),
                        mode(320, 200, 60),
                    ],
                },
                MonitorInfo {
                    size: [2560, 1440],
                    refresh_millihertz: Some(165_000),
                    modes: vec![mode(2560, 1440, 165), mode(1920, 1080, 120)],
                },
            ],
            window_frame: [0, 0],
        }
    }

    #[test]
    fn windowed_max_subtracts_the_frame_from_the_screen() {
        let mut info = two_monitors();
        assert_eq!(info.windowed_max(0), Some([1920, 1080]));
        info.window_frame = [8, 46];
        assert_eq!(info.windowed_max(0), Some([1912, 1034]));
        // An unknown monitor index falls back to the primary one.
        assert_eq!(info.windowed_max(9), Some([1912, 1034]));
        // No monitors at all: no maximum.
        assert_eq!(DisplayInfo::default().windowed_max(0), None);
        // A frame that leaves too little room disables the limit instead of offering
        // sizes below the minimum.
        info.window_frame = [10_000, 10_000];
        assert_eq!(info.windowed_max(0), None);
    }

    #[test]
    fn fits_windowed_requires_both_axes() {
        assert!(fits_windowed([1912, 1034], [1912, 1034]));
        assert!(!fits_windowed([1912, 1035], [1912, 1034]));
        assert!(!fits_windowed([1913, 1034], [1912, 1034]));
    }

    #[test]
    fn resolutions_come_from_video_modes_largest_first() {
        let info = two_monitors();
        // Only sizes the monitor's video modes report, and never below the supported
        // floor of 1280x720 (the smaller modes of the fixture are dropped).
        assert_eq!(info.resolutions(0), [[1920, 1080], [1280, 720]]);
        assert_eq!(info.resolutions(1)[0], [2560, 1440]);
        // A saved size the monitor does not list is not offered as an option.
        assert!(!info.resolutions(0).contains(&[1024, 768]));
        // A missing monitor falls back to the primary one.
        assert_eq!(info.resolutions(7)[0], [1920, 1080]);
        assert_eq!(info.native_resolution(1), [2560, 1440]);
    }

    #[test]
    fn aspect_ratios_follow_the_supported_sizes() {
        let info = two_monitors();
        // Primary: 1920x1080, 1280x720 (smaller modes are below the floor) — only 16:9
        // presets match.
        assert_eq!(
            info.aspect_ratios(0),
            [AspectRatio::Auto, AspectRatio::Wide16x9]
        );
        // The second monitor offers 1920x1080 too; no other ratio appears.
        assert_eq!(
            info.aspect_ratios(1),
            [AspectRatio::Auto, AspectRatio::Wide16x9]
        );
        // 1366x768 is not exactly 16:9 but matches within the tolerance.
        let mut odd = two_monitors();
        odd.monitors[0].modes.push(super::VideoMode {
            size: [1366, 768],
            refresh_millihertz: 60_000,
        });
        assert!(odd.aspect_ratios(0).contains(&AspectRatio::Wide16x9));
        // A 4:3-only monitor offers no preset besides Auto.
        let mut square = two_monitors();
        square.monitors[0].size = [1024, 768];
        square.monitors[0].modes = vec![super::VideoMode {
            size: [1024, 768],
            refresh_millihertz: 60_000,
        }];
        assert_eq!(square.aspect_ratios(0), [AspectRatio::Auto]);
    }

    #[test]
    fn without_video_modes_common_sizes_are_offered() {
        let info = DisplayInfo {
            monitors: vec![MonitorInfo {
                size: [1600, 900],
                refresh_millihertz: None,
                modes: Vec::new(),
            }],
            window_frame: [0, 0],
        };
        assert_eq!(info.resolutions(0), [[1600, 900], [1366, 768], [1280, 720]]);
        assert!(!DisplayInfo::default().resolutions(0).is_empty());
    }

    #[test]
    fn refresh_rates_depend_on_the_resolution() {
        let info = two_monitors();
        assert_eq!(info.refresh_rates(0, [1920, 1080]), [60_000, 144_000]);
        assert_eq!(info.refresh_rates(0, [1280, 720]), [60_000]);
        assert!(info.refresh_rates(0, [1600, 900]).is_empty());
        assert_eq!(resolve_refresh(&[60_000, 144_000], 0), Some(144_000));
        assert_eq!(resolve_refresh(&[60_000, 144_000], 60_000), Some(60_000));
        assert_eq!(resolve_refresh(&[], 60_000), None);
        let (en, uk) = (Language::English, Language::Ukrainian);
        assert_eq!(refresh_label(144_000, en), "144 Hz");
        assert_eq!(refresh_label(59_940, en), "59.94 Hz");
        assert_eq!(refresh_label(60_000, uk), "60 Гц");
        assert_eq!(monitor_label(0, en), "Primary");
        assert_eq!(monitor_label(1, en), "Secondary");
        assert_eq!(monitor_label(2, en), "Monitor 3");
        assert_eq!(monitor_label(0, uk), "Основний");
        assert_eq!(monitor_label(2, uk), "Монітор 3");
    }

    #[test]
    fn background_limit_only_lowers_the_frame_rate() {
        let mut display = DisplaySettings::default();
        assert_eq!(display.frame_limit(true), None, "unlimited when focused");
        assert_eq!(display.frame_limit(false), Some(30));
        display.frame_rate_limit = FrameRateLimit::Fps60;
        display.background_frame_rate_limit = BackgroundFrameRateLimit::Unlimited;
        assert_eq!(
            display.frame_limit(false),
            Some(60),
            "never raises the limit"
        );
        display.background_frame_rate_limit = BackgroundFrameRateLimit::Fps15;
        assert_eq!(display.frame_limit(false), Some(15));
    }

    #[test]
    fn window_changes_cover_mode_monitor_size_and_refresh() {
        let base = DisplaySettings::default();
        assert_eq!(
            base.mode,
            DisplayMode::Fullscreen,
            "the default mode is exclusive fullscreen"
        );
        // In fullscreen the resolution selects the video mode: a change matters.
        let resized = DisplaySettings {
            resolution: [1280, 720],
            ..base
        };
        assert!(base.window_changes(&resized));
        let windowed = DisplaySettings {
            mode: DisplayMode::Windowed,
            ..base
        };
        assert!(windowed.window_changes(&resized), "windowed size change");
        assert!(base.window_changes(&windowed));
        assert!(base.window_changes(&DisplaySettings { monitor: 1, ..base }));
        assert!(base.window_changes(&DisplaySettings {
            refresh_rate: 60_000,
            ..base
        }));
        assert!(!base.window_changes(&DisplaySettings {
            vsync: false,
            brightness: 80,
            gamma: 150,
            ..base
        }));
    }

    #[test]
    fn calibration_values_map_and_clamp() {
        let mut display = DisplaySettings::default();
        assert_eq!(display.brightness_factor(), 1.0);
        assert_eq!(display.gamma_value(), 1.0);
        assert_eq!(display.ui_scale_factor(), 1.0);
        display.brightness = 0;
        assert_eq!(display.brightness_factor(), 0.5);
        display.brightness = 100;
        assert_eq!(display.brightness_factor(), 1.5);
        let display = DisplaySettings {
            brightness: 200,
            gamma: 10,
            ui_scale: 333,
            resolution: [100, 50],
            ..display
        }
        .sanitized();
        assert_eq!(display.brightness, MAX_BRIGHTNESS);
        assert_eq!(display.gamma, MIN_GAMMA);
        assert_eq!(display.ui_scale, MAX_UI_SCALE);
        assert_eq!(display.resolution, MIN_RESOLUTION);
        assert_eq!(snap_ui_scale(103), 105);
        assert_eq!(snap_ui_scale(102), 100);
        assert_eq!(snap_ui_scale(10), MIN_UI_SCALE);
    }
}
