//! Credits screen: a centered title, credit sections, and a plain Back to Menu button
//! like the main menu's Update button (the mockup shows no way back; the button sits
//! where Settings has its own).

use super::widgets::{
    self, FLAT_BUTTON_SIZE, FLAT_BUTTON_TEXT_SIZE, FULL_UV, FlatButton, UiKit, canvas_origin,
    canvas_scaled, palette,
};
use crate::engine::render::ui_renderer::{
    LabelId, SpriteMaterial, TextAlign, TextStyle, UiFrame, UiRenderer,
};
use crate::engine::ui::geometry::Rect;
use crate::engine::ui::hit::{Hit, topmost_hit};
use crate::game::locale::Language;

/// Credits content, as given in the design mockup. Headings are translated; names are
/// proper names and stay as written.
const CREDITS: [(&str, &[&str]); 4] = [
    ("GAME DESIGN", &["Roman Nikolaienko"]),
    ("PROGRAMMING", &["Roman Nikolaienko"]),
    ("ART", &["Roman Nikolaienko", "Quixel Megascans"]),
    ("SOUND DESIGN", &["Roman Nikolaienko", "Freesound.org"]),
];

// Reference layout (1920x1080), centered on x = 960.
const CENTER_X: f32 = 960.0;
const COLUMN_WIDTH: f32 = 800.0;
const TITLE_Y: f32 = 66.0;
const TITLE_HEIGHT: f32 = 64.0;
const TITLE_TEXT_SIZE: f32 = 46.0;
const SUBTITLE_DIVIDER: [f32; 3] = [140.0, 300.0, 22.0]; // y, width, height
const SECTIONS_TOP: f32 = 196.0;
const HEADING_HEIGHT: f32 = 42.0;
const NAME_HEIGHT: f32 = 36.0;
const SECTION_GAP: f32 = 28.0;
const HEADING_TEXT_SIZE: f32 = 30.0;
const NAME_TEXT_SIZE: f32 = 26.0;
const BOTTOM_DIVIDER: [f32; 3] = [24.0, 520.0, 26.0]; // gap after content, width, height
/// Back button: left edge and vertical center (where the earlier 72 px high stone
/// button at y 950 had its center).
const BACK_BUTTON: [f32; 2] = [230.0, 986.0];
const BACK_TEXT: &str = "Back to Menu";

const BACK_ID: usize = 0;
const TITLE: &str = "CREDITS";

struct Line {
    label: LabelId,
    /// English text (the translation key for headings).
    text: &'static str,
    heading: bool,
    rect: Rect,
}

pub struct CreditsScreen {
    title: LabelId,
    lines: Vec<Line>,
    back: FlatButton,
    scale: f32,
    title_rect: Rect,
    subtitle_divider: Rect,
    bottom_divider: Rect,
}

impl CreditsScreen {
    pub fn new(renderer: &mut UiRenderer) -> Self {
        let heading_style = TextStyle {
            bold: true,
            align: TextAlign::Center,
        };
        let mut lines = Vec::new();
        for (heading, names) in CREDITS {
            lines.push(Line {
                label: renderer.create_label(heading, heading_style),
                text: heading,
                heading: true,
                rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            });
            for name in names {
                lines.push(Line {
                    label: renderer.create_label(name, TextStyle::CENTERED),
                    text: name,
                    heading: false,
                    rect: Rect::new(0.0, 0.0, 0.0, 0.0),
                });
            }
        }
        let empty = Rect::new(0.0, 0.0, 0.0, 0.0);
        let mut back = FlatButton::new(renderer);
        back.enabled = true;
        Self {
            title: renderer.create_label(TITLE, TextStyle::CENTERED),
            lines,
            back,
            scale: 1.0,
            title_rect: empty,
            subtitle_divider: empty,
            bottom_divider: empty,
        }
    }

    pub fn open(&mut self) {
        self.back.reset_animation();
    }

    /// Shows the title, headings and button in `language`.
    pub fn relabel(&self, renderer: &mut UiRenderer, language: Language) {
        renderer.set_label_text(self.title, language.tr(TITLE));
        for line in self.lines.iter().filter(|line| line.heading) {
            renderer.set_label_text(line.label, language.tr(line.text));
        }
        self.back.set_text(renderer, language.tr(BACK_TEXT));
    }

    pub fn layout(&mut self, scale: f32, viewport: [f32; 2]) {
        self.scale = scale;
        // The content is symmetric: center the whole reference canvas in the viewport,
        // not just use the reference coordinates (which center only in a 16:9 window).
        let origin = canvas_origin(scale, viewport);
        let at = |x, y, w, h| canvas_scaled(scale, origin, x, y, w, h);
        let left = CENTER_X - COLUMN_WIDTH / 2.0;
        self.title_rect = at(left, TITLE_Y, COLUMN_WIDTH, TITLE_HEIGHT);
        let [y, w, h] = SUBTITLE_DIVIDER;
        self.subtitle_divider = at(CENTER_X - w / 2.0, y, w, h);

        let mut y = SECTIONS_TOP;
        let mut first = true;
        for line in &mut self.lines {
            if line.heading {
                if !first {
                    y += SECTION_GAP;
                }
                first = false;
                line.rect = at(left, y, COLUMN_WIDTH, HEADING_HEIGHT);
                y += HEADING_HEIGHT;
            } else {
                line.rect = at(left, y, COLUMN_WIDTH, NAME_HEIGHT);
                y += NAME_HEIGHT;
            }
        }
        let [gap, w, h] = BOTTOM_DIVIDER;
        self.bottom_divider = at(CENTER_X - w / 2.0, y + gap, w, h);

        let [x, center_y] = BACK_BUTTON;
        let [w, h] = FLAT_BUTTON_SIZE;
        self.back.place(at(x, center_y - h / 2.0, w, h));
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Hit {
        topmost_hit([self.back.hit_target(BACK_ID)], x, y)
    }

    /// Returns true when the click asks to go back to the main menu.
    pub fn click(&mut self, id: usize) -> bool {
        id == BACK_ID
    }

    /// Keyboard focus stops: the single Back button.
    pub fn focus_stops(&self) -> Vec<usize> {
        vec![BACK_ID]
    }

    pub fn animate(&mut self, hovered: Option<usize>, pressed: Option<usize>, dt: f32) {
        self.back
            .animate(hovered == Some(BACK_ID), pressed == Some(BACK_ID), dt);
    }

    pub fn build(&self, frame: &mut UiFrame, kit: &UiKit) {
        let scale = self.scale;
        widgets::push_label(
            frame,
            self.title,
            self.title_rect,
            (TITLE_TEXT_SIZE * scale).round(),
            palette::TEXT,
            false,
        );
        let sprites = &mut frame.group_sprites;
        for divider in [self.subtitle_divider, self.bottom_divider] {
            widgets::push_image(
                sprites,
                kit.ornament_divider,
                divider,
                FULL_UV,
                SpriteMaterial::PLAIN,
            );
        }

        for line in &self.lines {
            let (size, color) = if line.heading {
                (HEADING_TEXT_SIZE, palette::HEADING)
            } else {
                (NAME_TEXT_SIZE, palette::TEXT)
            };
            widgets::push_label(
                frame,
                line.label,
                line.rect,
                (size * scale).round(),
                color,
                false,
            );
        }
        self.back
            .draw(frame, kit, (FLAT_BUTTON_TEXT_SIZE * scale).round());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_and_headings_are_translated() {
        assert!(crate::game::locale::is_translated(TITLE));
        assert!(crate::game::locale::is_translated(BACK_TEXT));
        for (heading, _) in CREDITS {
            assert!(crate::game::locale::is_translated(heading), "{heading}");
        }
    }

    #[test]
    fn content_fits_above_the_back_button() {
        let lines: f32 = CREDITS
            .iter()
            .map(|(_, names)| HEADING_HEIGHT + NAME_HEIGHT * names.len() as f32)
            .sum();
        let gaps = SECTION_GAP * (CREDITS.len() - 1) as f32;
        let bottom = SECTIONS_TOP + lines + gaps + BOTTOM_DIVIDER[0] + BOTTOM_DIVIDER[2];
        let back_top = BACK_BUTTON[1] - FLAT_BUTTON_SIZE[1] / 2.0;
        assert!(bottom < back_top, "credits end at {bottom}");
    }
}
