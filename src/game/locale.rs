//! Interface languages and the menu translations.
//!
//! The English text doubles as the translation key: `language.tr("Apply")`. A missing
//! translation falls back to English; tests list the texts that must have one. Language
//! names are always shown in their own language; setting names ("Anti-Aliasing",
//! "V-Sync", ...) and proper names (credits) are never translated.

use serde::{Deserialize, Serialize};

use super::settings::OptionValue;

/// The interface font (Inter, SIL Open Font License, `assets/fonts/Inter-LICENSE.txt`),
/// relative to the assets directory: regular and bold. It must cover every language's
/// text (tested below).
pub const UI_FONTS: [&str; 2] = ["fonts/Inter-Regular.ttf", "fonts/Inter-Bold.ttf"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    English,
    Ukrainian,
}

impl OptionValue for Language {
    const ALL: &'static [Self] = &[Self::English, Self::Ukrainian];

    fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Ukrainian => "Українська",
        }
    }
}

impl Language {
    /// `english` in this language.
    pub fn tr(self, english: &'static str) -> &'static str {
        match self {
            Self::English => english,
            Self::Ukrainian => ukrainian(english).unwrap_or(english),
        }
    }

    /// Update button: an installable newer version.
    pub fn update_to(self, version: &str) -> String {
        match self {
            Self::English => format!("Update to {version}"),
            Self::Ukrainian => format!("Оновити до {version}"),
        }
    }

    /// Update button: a newer version this build cannot install (development run).
    pub fn update_available(self, version: &str) -> String {
        match self {
            Self::English => format!("{version} available"),
            Self::Ukrainian => format!("Доступна {version}"),
        }
    }

    /// Update button while downloading.
    pub fn downloading(self, percent: u32) -> String {
        match self {
            Self::English => format!("Downloading {percent}%"),
            Self::Ukrainian => format!("Завантаження {percent}%"),
        }
    }

    /// The update confirmation's message.
    pub fn update_message(self, version: &str) -> String {
        match self {
            Self::English => format!("{version} will be downloaded and the game will restart."),
            Self::Ukrainian => format!("{version} буде завантажено, і гра перезапуститься."),
        }
    }

    /// The display-change countdown: "Reverting in 12 s".
    pub fn reverting_in(self, seconds: u32) -> String {
        match self {
            Self::English => format!("Reverting in {seconds} s"),
            Self::Ukrainian => format!("Повернення через {seconds} с"),
        }
    }
}

/// Whether `english` has a Ukrainian translation (for coverage tests).
#[cfg(test)]
pub fn is_translated(english: &str) -> bool {
    ukrainian(english).is_some()
}

fn ukrainian(english: &str) -> Option<&'static str> {
    Some(match english {
        // Main menu.
        "New game" => "Нова гра",
        "Load game" => "Завантажити гру",
        "Settings" => "Налаштування",
        "Credits" => "Автори",
        "Exit" => "Вихід",
        // Credits.
        "CREDITS" => "АВТОРИ",
        "GAME DESIGN" => "ГЕЙМДИЗАЙН",
        "PROGRAMMING" => "ПРОГРАМУВАННЯ",
        "ART" => "ГРАФІКА",
        "SOUND DESIGN" => "ЗВУКОВИЙ ДИЗАЙН",
        "SPECIAL THANKS" => "ОСОБЛИВА ПОДЯКА",
        // Settings frame.
        "Audio" => "Звук",
        "Graphics" => "Графіка",
        "Display" => "Екран",
        "AUDIO OPTIONS" => "НАЛАШТУВАННЯ ЗВУКУ",
        "GRAPHICS OPTIONS" => "НАЛАШТУВАННЯ ГРАФІКИ",
        "DISPLAY OPTIONS" => "НАЛАШТУВАННЯ ЕКРАНА",
        "Basic" => "Основні",
        "Advanced" => "Розширені",
        "Back to Menu" => "Назад до меню",
        "Reset" => "Скинути",
        "Apply" => "Застосувати",
        "Default" => "Типові",
        "Apply changes?" => "Застосувати зміни?",
        "Some settings have not been applied." => "Деякі налаштування не застосовано.",
        "Discard" => "Відхилити",
        "Keep these display settings?" => "Залишити ці налаштування екрана?",
        "Keep" => "Залишити",
        "Revert" => "Повернути",
        "Cancel" => "Скасувати",
        "Reset changes?" => "Скинути зміни?",
        "Unapplied changes on all tabs will be discarded." => {
            "Незастосовані зміни на всіх вкладках буде скасовано."
        }
        "Apply settings?" => "Застосувати налаштування?",
        "The new settings will be saved." => "Нові налаштування буде збережено.",
        "Exit the game?" => "Вийти з гри?",
        // World map.
        "Return to the main menu?" => "Повернутися до головного меню?",
        "Main menu" => "Головне меню",
        "This game will not be saved." => "Цю гру не буде збережено.",
        "Water" => "Вода",
        "Plains" => "Рівнина",
        "Forest" => "Ліс",
        "Hills" => "Пагорби",
        "Mountains" => "Гори",
        "Road" => "Дорога",
        "Bridge" => "Міст",
        "Fields" => "Поля",
        "Village" => "Село",
        // Updates.
        "Checking…" => "Перевірка…",
        "Up to date" => "Актуальна версія",
        "Updates" => "Оновлення",
        "Restarting…" => "Перезапуск…",
        "Retry update" => "Повторити",
        "Install the update?" => "Встановити оновлення?",
        "Update" => "Оновити",
        "Restore defaults?" => "Відновити типові налаштування?",
        "Restore" => "Відновити",
        "Settings on this tab will return to their default values." => {
            "Налаштування цієї вкладки повернуться до типових значень."
        }
        // Section headings.
        "General" => "Загальні",
        "Rendering & Upscaling" => "Рендеринг і масштабування",
        "Textures & Materials" => "Текстури та матеріали",
        "Lighting & Shadows" => "Освітлення та тіні",
        "Effects & Particles" => "Ефекти та частинки",
        "Geometry & Environment" => "Геометрія та оточення",
        "Reflections & Water" => "Відображення та вода",
        "Atmosphere" => "Атмосфера",
        "Post Processing" => "Постобробка",
        "Frame Rate & Synchronization" => "Частота кадрів",
        "Display Calibration" => "Калібрування",
        // Values. (Setting names themselves stay in English.)
        "Monitor" => "Монітор",
        "Not Supported" => "Не підтримується",
        "Low" => "Низька",
        "Medium" => "Середня",
        "High" => "Висока",
        "Ultra" => "Ультра",
        "Custom" => "Власна",
        "Off" => "Вимк.",
        "On" => "Увімк.",
        "Native AA" => "Нативне AA",
        "Quality" => "Якість",
        "Balanced" => "Баланс",
        "Performance" => "Продуктивність",
        "Bilinear" => "Білінійна",
        "Trilinear" => "Трилінійна",
        "Anisotropic 2×" => "Анізотропна 2×",
        "Anisotropic 4×" => "Анізотропна 4×",
        "Anisotropic 8×" => "Анізотропна 8×",
        "Anisotropic 16×" => "Анізотропна 16×",
        "Near" => "Близька",
        "Far" => "Далека",
        "Windowed" => "Віконний",
        "Borderless" => "Без рамки",
        "Fullscreen" => "Повноекранний",
        "Auto" => "Авто",
        "Unlimited" => "Без обмежень",
        "Primary" => "Основний",
        "Secondary" => "Другий",
        "Hz" => "Гц",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::settings::graphics::{
        EffectLevel, OptionalQuality, ShadowDistance, TextureFiltering, UpscalingQuality,
    };
    use crate::game::settings::{BackgroundFrameRateLimit, DisplayMode, Quality};

    fn labels<T: OptionValue>() -> impl Iterator<Item = &'static str> {
        T::ALL.iter().map(|value| value.label())
    }

    #[test]
    fn english_is_the_key_and_the_fallback() {
        assert_eq!(Language::English.tr("Apply"), "Apply");
        assert_eq!(Language::Ukrainian.tr("Apply"), "Застосувати");
        assert_eq!(
            Language::Ukrainian.tr("FXAA"),
            "FXAA",
            "untranslated stays English"
        );
        assert_eq!(Language::Ukrainian.reverting_in(7), "Повернення через 7 с");
        assert_eq!(Language::English.reverting_in(7), "Reverting in 7 s");
    }

    #[test]
    fn language_names_are_in_their_own_language() {
        assert_eq!(Language::English.label(), "English");
        assert_eq!(Language::Ukrainian.label(), "Українська");
        assert_eq!(Language::default(), Language::English);
    }

    #[test]
    fn every_named_option_value_is_translated() {
        let values = labels::<Quality>()
            .chain(labels::<OptionalQuality>())
            .chain(labels::<EffectLevel>())
            .chain(labels::<ShadowDistance>())
            .chain(labels::<TextureFiltering>())
            .chain(labels::<UpscalingQuality>())
            .chain(labels::<DisplayMode>())
            .chain(labels::<bool>())
            .chain(["Custom", "Auto", "Primary", "Secondary", "Hz"])
            .chain([BackgroundFrameRateLimit::Unlimited.label()]);
        for value in values {
            assert!(is_translated(value), "{value}");
        }
    }

    /// Every character the interface can show (all menu texts, translations and the
    /// Ukrainian alphabet) has a glyph in both bundled font files.
    #[test]
    fn the_interface_font_covers_every_text() {
        use glyphon::{FontSystem, fontdb};
        use std::path::PathBuf;

        let assets = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"));
        let paths: Vec<PathBuf> = UI_FONTS.iter().map(|font| assets.join(font)).collect();
        let mut db = fontdb::Database::new();
        let families = crate::engine::render::ui_renderer::load_bundled_fonts(&mut db, &paths);
        assert_eq!(families, ["Inter"]);
        let faces: Vec<(fontdb::ID, fontdb::Weight)> =
            db.faces().map(|face| (face.id, face.weight)).collect();
        assert_eq!(faces.len(), UI_FONTS.len(), "one face per file");

        let sources = [
            include_str!("locale.rs"),
            include_str!("menus/main_menu.rs"),
            include_str!("menus/settings_screen.rs"),
            include_str!("menus/credits_screen.rs"),
            include_str!("settings/mod.rs"),
            include_str!("settings/graphics.rs"),
            include_str!("settings/display.rs"),
            "АБВГҐДЕЄЖЗИІЇЙКЛМНОПРСТУФХЦЧШЩЬЮЯабвгґдеєжзиіїйклмнопрстуфхцчшщьюяʼ’«»№",
        ];
        let mut needed: Vec<char> = sources
            .iter()
            .flat_map(|text| text.chars())
            .filter(|c| !c.is_control() && !c.is_whitespace())
            .collect();
        needed.sort_unstable();
        needed.dedup();

        let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
        for (id, weight) in faces {
            let font = fonts.get_font(id, weight).expect("bundled face loads");
            let charmap = font.as_swash().charmap();
            assert_eq!(
                charmap.map('\u{4E2D}'),
                0,
                "the check can tell a missing glyph"
            );
            let missing: String = needed.iter().filter(|&&c| charmap.map(c) == 0).collect();
            assert!(
                missing.is_empty(),
                "{weight:?} lacks glyphs for {missing:?}"
            );
        }
    }
}
