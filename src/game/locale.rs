//! Interface languages and the menu translations.
//!
//! The English text doubles as the translation key: `language.tr("Apply")`. A missing
//! translation falls back to English; tests list the texts that must have one. Language
//! names are always shown in their own language; setting names ("Anti-Aliasing",
//! "V-Sync", ...) and proper names (credits) are never translated.

use serde::{Deserialize, Serialize};

use super::settings::OptionValue;

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
}
