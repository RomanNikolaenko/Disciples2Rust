//! Persisted user settings: the single source of truth for options shown in Settings.
//!
//! Stored as TOML in the per-user config directory:
//! - Windows: `%APPDATA%\Disciples2Rust\settings.toml`
//! - Linux: `$XDG_CONFIG_HOME/disciples2rust/settings.toml` (default `~/.config/...`)
//!
//! Missing fields fall back to defaults and out-of-range values are clamped, so older or
//! hand-edited files still load. A corrupt file is reported and ignored (defaults are
//! used) without being overwritten until the player applies new settings. Saving writes
//! a temporary file and renames it over the old one, so a crash never leaves a
//! half-written file.
//!
//! Format history:
//! - 1: brightness was 50–150 %; brightness / Show FPS were briefly under `[graphics]`
//!   and Resolution Scale under `[display]`.
//! - 2: brightness is 0–100 % (50 = unchanged); the fields above live where they are now.
//!   Renamed graphics fields and values are read through serde aliases.

pub mod display;
pub mod graphics;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use display::{
    AspectRatio, BackgroundFrameRateLimit, DisplayInfo, DisplayMode, DisplaySettings,
    FrameRateLimit, MonitorInfo, VideoMode,
};
pub use graphics::{GraphicsOption, GraphicsSettings, Quality, ResolutionScale};

/// Current file format version, written on save.
pub const SETTINGS_VERSION: u32 = 2;
const FILE_NAME: &str = "settings.toml";
pub const MAX_VOLUME: u8 = 100;
const DEFAULT_VOLUME: u8 = 50;

/// A setting with a fixed list of named values, shown as a selector.
pub trait OptionValue: Copy + PartialEq + 'static {
    /// Every value in display order (left to right).
    const ALL: &'static [Self];

    fn label(self) -> &'static str;

    /// Position in [`Self::ALL`] (0 for a value missing from it).
    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|&value| value == self)
            .unwrap_or(0)
    }
}

impl OptionValue for bool {
    const ALL: &'static [Self] = &[false, true];

    fn label(self) -> &'static str {
        if self { "On" } else { "Off" }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("failed to read `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse `{path}`: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize settings: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("failed to write `{path}`: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Volume channels shown on the Audio tab, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChannel {
    Master,
    Music,
    Sfx,
    Ui,
    Voice,
}

impl AudioChannel {
    pub const ALL: [Self; 5] = [Self::Master, Self::Music, Self::Sfx, Self::Ui, Self::Voice];

    pub fn label(self) -> &'static str {
        match self {
            Self::Master => "Master",
            Self::Music => "Music",
            Self::Sfx => "SFX",
            Self::Ui => "UI",
            Self::Voice => "Voice",
        }
    }
}

/// Volumes in percent (0–100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub master: u8,
    pub music: u8,
    pub sfx: u8,
    pub ui: u8,
    pub voice: u8,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            master: DEFAULT_VOLUME,
            music: DEFAULT_VOLUME,
            sfx: DEFAULT_VOLUME,
            ui: DEFAULT_VOLUME,
            voice: DEFAULT_VOLUME,
        }
    }
}

impl AudioSettings {
    pub fn get(&self, channel: AudioChannel) -> u8 {
        match channel {
            AudioChannel::Master => self.master,
            AudioChannel::Music => self.music,
            AudioChannel::Sfx => self.sfx,
            AudioChannel::Ui => self.ui,
            AudioChannel::Voice => self.voice,
        }
    }

    pub fn set(&mut self, channel: AudioChannel, volume: u8) {
        let volume = volume.min(MAX_VOLUME);
        match channel {
            AudioChannel::Master => self.master = volume,
            AudioChannel::Music => self.music = volume,
            AudioChannel::Sfx => self.sfx = volume,
            AudioChannel::Ui => self.ui = volume,
            AudioChannel::Voice => self.voice = volume,
        }
    }

    fn clamped(mut self) -> Self {
        for channel in AudioChannel::ALL {
            self.set(channel, self.get(channel));
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub audio: AudioSettings,
    pub graphics: GraphicsSettings,
    pub display: DisplaySettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            audio: AudioSettings::default(),
            graphics: GraphicsSettings::default(),
            display: DisplaySettings::default(),
        }
    }
}

/// Keys whose place or meaning changed (see the format history), as found in a file.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LegacyFields {
    graphics: LegacyKeys,
    display: LegacyKeys,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LegacyKeys {
    brightness: Option<u8>,
    show_fps: Option<bool>,
    resolution_scale: Option<u8>,
}

/// Version 1 brightness range.
const LEGACY_BRIGHTNESS: [u8; 2] = [50, 150];

impl Settings {
    /// Parses settings text, migrating older formats and clamping out-of-range values.
    fn parse(text: &str, path: &Path) -> Result<Self, SettingsError> {
        let mut settings: Self = toml::from_str(text).map_err(|source| SettingsError::Parse {
            path: path.to_owned(),
            source,
        })?;
        if settings.version > SETTINGS_VERSION {
            tracing::warn!(
                version = settings.version,
                "settings were written by a newer version; reading known fields only"
            );
        }
        // The main parse succeeded, so the same text parses here too; a type mismatch in
        // one legacy key only skips the migration.
        let legacy: LegacyFields = toml::from_str(text).unwrap_or_default();
        settings.migrate(&legacy);
        Ok(Self {
            version: SETTINGS_VERSION,
            audio: settings.audio.clamped(),
            graphics: settings.graphics.sanitized(),
            display: settings.display.sanitized(),
        })
    }

    /// Moves relocated keys to their current place (unless that place has a value) and
    /// converts version 1 brightness. Runs before sanitizing.
    fn migrate(&mut self, legacy: &LegacyFields) {
        if legacy.display.show_fps.is_none()
            && let Some(show_fps) = legacy.graphics.show_fps
        {
            self.display.show_fps = show_fps;
        }
        if legacy.graphics.resolution_scale.is_none()
            && let Some(scale) = legacy.display.resolution_scale
        {
            self.graphics.resolution_scale = ResolutionScale(scale);
        }
        if self.version < 2 {
            let [low, high] = LEGACY_BRIGHTNESS;
            self.display.brightness = match legacy.display.brightness.or(legacy.graphics.brightness)
            {
                Some(old) => old.clamp(low, high) - low,
                None => DisplaySettings::default().brightness,
            };
        }
    }

    /// Reads settings from `path`; `Ok(None)` when the file does not exist yet.
    pub fn read(path: &Path) -> Result<Option<Self>, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, path).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SettingsError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Loads settings for startup: defaults when the file is missing, unreadable or
    /// corrupt (the problem is logged, the file is left untouched).
    pub fn load_or_default(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            tracing::warn!("no config directory found; using default settings");
            return Self::default();
        };
        match Self::read(path) {
            Ok(Some(settings)) => {
                tracing::info!(path = %path.display(), "settings loaded");
                settings
            }
            Ok(None) => {
                tracing::info!(path = %path.display(), "no settings file yet; using defaults");
                Self::default()
            }
            Err(error) => {
                tracing::error!("{error}; using default settings");
                Self::default()
            }
        }
    }

    /// Writes settings atomically: a temporary file in the same directory is renamed
    /// over the target.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let text = toml::to_string_pretty(self)?;
        let write_error = |source| SettingsError::Write {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(write_error)?;
        }
        let temp = path.with_extension("toml.tmp");
        std::fs::write(&temp, text).map_err(write_error)?;
        std::fs::rename(&temp, path).map_err(write_error)?;
        Ok(())
    }
}

/// Where settings live for the current user, if a config directory can be determined.
pub fn settings_path() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Disciples2Rust"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|dir| dir.join("disciples2rust"))
    }?;
    Some(dir.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::graphics::{AntiAliasing, OptionalQuality};
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("disciples2rust-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn parse(text: &str) -> Settings {
        Settings::parse(text, Path::new("t")).unwrap()
    }

    #[test]
    fn defaults_are_fifty_percent() {
        let audio = AudioSettings::default();
        assert!(AudioChannel::ALL.iter().all(|&c| audio.get(c) == 50));
    }

    #[test]
    fn set_clamps_to_max() {
        let mut audio = AudioSettings::default();
        audio.set(AudioChannel::Music, 250);
        assert_eq!(audio.music, 100);
        audio.set(AudioChannel::Voice, 7);
        assert_eq!(audio.get(AudioChannel::Voice), 7);
    }

    #[test]
    fn missing_fields_default_and_out_of_range_values_clamp() {
        let settings = parse("[audio]\nmaster = 80\nmusic = 200\n");
        assert_eq!(settings.audio.master, 80);
        assert_eq!(settings.audio.music, 100);
        assert_eq!(settings.audio.sfx, 50);
        assert_eq!(settings.version, SETTINGS_VERSION);
        assert_eq!(settings.graphics, GraphicsSettings::default());
        assert_eq!(settings.display, DisplaySettings::default());
    }

    #[test]
    fn corrupt_text_is_a_parse_error() {
        let error = Settings::parse("[audio\nmaster = ", Path::new("bad.toml")).unwrap_err();
        assert!(matches!(error, SettingsError::Parse { .. }), "{error}");
    }

    #[test]
    fn save_then_read_round_trips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("nested").join(FILE_NAME);
        let mut settings = Settings::default();
        settings.audio.set(AudioChannel::Sfx, 73);
        settings.graphics.apply_preset(Quality::Ultra);
        settings.display.mode = DisplayMode::Fullscreen;
        settings.display.refresh_rate = 144_000;
        settings.display.language = crate::game::locale::Language::Ukrainian;
        settings.save(&path).unwrap();
        assert!(
            !path.with_extension("toml.tmp").exists(),
            "temp file renamed away"
        );
        assert_eq!(Settings::read(&path).unwrap(), Some(settings));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_reads_as_none_and_loads_defaults() {
        let dir = temp_dir("missing");
        let path = dir.join(FILE_NAME);
        assert_eq!(Settings::read(&path).unwrap(), None);
        assert_eq!(Settings::load_or_default(Some(&path)), Settings::default());
    }

    #[test]
    fn version_one_brightness_is_rescaled() {
        // 50–150 % becomes 0–100 %, whichever section held it.
        let settings = parse("version = 1\n[display]\nbrightness = 130\n");
        assert_eq!(settings.display.brightness, 80);
        let settings = parse("version = 1\n[graphics]\nbrightness = 100\nshow_fps = true\n");
        assert_eq!(settings.display.brightness, 50);
        assert!(settings.display.show_fps);
        // [display] wins over the interim [graphics] place.
        let settings =
            parse("version = 1\n[graphics]\nbrightness = 150\n[display]\nbrightness = 60\n");
        assert_eq!(settings.display.brightness, 10);
        // No stored brightness: the default, not a shifted one.
        assert_eq!(parse("version = 1\n").display.brightness, 50);
        // Current files keep their value.
        assert_eq!(
            parse("version = 2\n[display]\nbrightness = 30\n")
                .display
                .brightness,
            30
        );
    }

    #[test]
    fn version_one_graphics_and_display_values_carry_over() {
        let text = "version = 1\n\
            [graphics]\nview_distance = \"Cinematic\"\nanti_aliasing = \"Medium\"\n\
            shadows = \"Epic\"\nshading = \"Low\"\nlandscape = \"Medium\"\n\
            textures = \"Low\"\npost_processing = \"Epic\"\n\
            [display]\nmode = \"WindowedFullscreen\"\nresolution_scale = 80\n\
            background_frame_rate_limit = \"Fps144\"\naspect_ratio = \"Wide16x10\"\n";
        let settings = parse(text);
        let g = settings.graphics;
        assert_eq!(g.view_distance, Quality::Ultra);
        assert_eq!(g.anti_aliasing, AntiAliasing::Smaa);
        assert_eq!(g.shadow_quality, OptionalQuality::Ultra);
        assert_eq!(g.material_quality, Quality::Low);
        assert_eq!(g.terrain_quality, Quality::Medium);
        assert_eq!(g.texture_quality, Quality::Low);
        assert_eq!(g.post_processing_quality, Quality::Ultra);
        assert_eq!(g.resolution_scale, ResolutionScale(80));
        let d = settings.display;
        assert_eq!(d.mode, DisplayMode::Fullscreen, "borderless is mapped away");
        assert_eq!(
            d.background_frame_rate_limit,
            BackgroundFrameRateLimit::Unlimited
        );
        assert_eq!(d.aspect_ratio, AspectRatio::Wide16x10);
    }

    #[test]
    fn option_values_index_and_label() {
        assert_eq!(true.label(), "On");
        assert_eq!(false.index(), 0);
        assert_eq!(Quality::Ultra.index(), 3);
        assert_eq!(DisplayMode::Fullscreen.label(), "Fullscreen");
        assert_eq!(FrameRateLimit::ALL.len(), 8);
    }
}
