//! Credits screen: a centered title with side ornaments, credit sections, and a Back to
//! Menu button (the mockup shows no way back; the button sits where Settings has it).

use super::widgets::{self, FULL_UV, StoneButton, UiKit, canvas_origin, canvas_scaled, palette};
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
/// Side ornaments: distance of the inner end from the center, and size.
const SIDE_GAP: f32 = 125.0;
const SIDE_SIZE: [f32; 2] = [220.0, 22.0];
const SUBTITLE_DIVIDER: [f32; 3] = [140.0, 300.0, 22.0]; // y, width, height
const SECTIONS_TOP: f32 = 196.0;
const HEADING_HEIGHT: f32 = 42.0;
const NAME_HEIGHT: f32 = 36.0;
const SECTION_GAP: f32 = 28.0;
const HEADING_TEXT_SIZE: f32 = 30.0;
const NAME_TEXT_SIZE: f32 = 26.0;
const BOTTOM_DIVIDER: [f32; 3] = [24.0, 520.0, 26.0]; // gap after content, width, height
const BACK_BUTTON: [f32; 4] = [230.0, 950.0, 336.0, 72.0];
const BUTTON_TEXT_SIZE: f32 = 24.0;

const BACK_ID: usize = 0;
const TITLE: &str = "CREDITS";

// Layout invariants, checked at compile time: "CREDITS" at 46 px is roughly 200 px
// wide, so the side ornaments must stay clear of it and on screen.
const _: () = assert!(SIDE_GAP > 110.0);
const _: () = assert!(CENTER_X - SIDE_GAP - SIDE_SIZE[0] > 0.0);

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
    back: StoneButton,
    scale: f32,
    title_rect: Rect,
    side_rects: [Rect; 2],
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
        let mut back = StoneButton::new(renderer, "Back to Menu", true);
        back.dark = true;
        Self {
            title: renderer.create_label(TITLE, TextStyle::CENTERED),
            lines,
            back,
            scale: 1.0,
            title_rect: empty,
            side_rects: [empty; 2],
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
        self.back.relabel(renderer, language);
    }

    pub fn layout(&mut self, scale: f32, viewport: [f32; 2], kit: &UiKit) {
        self.scale = scale;
        // The content is symmetric: center the whole reference canvas in the viewport,
        // not just use the reference coordinates (which center only in a 16:9 window).
        let origin = canvas_origin(scale, viewport);
        let at = |x, y, w, h| canvas_scaled(scale, origin, x, y, w, h);
        let left = CENTER_X - COLUMN_WIDTH / 2.0;
        self.title_rect = at(left, TITLE_Y, COLUMN_WIDTH, TITLE_HEIGHT);
        let side_y = TITLE_Y + (TITLE_HEIGHT - SIDE_SIZE[1]) / 2.0;
        let [side_w, side_h] = SIDE_SIZE;
        self.side_rects = [
            at(CENTER_X - SIDE_GAP - side_w, side_y, side_w, side_h),
            at(CENTER_X + SIDE_GAP, side_y, side_w, side_h),
        ];
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

        let [x, y, w, h] = BACK_BUTTON;
        self.back.place(at(x, y, w, h), &kit.button);
    }

    pub fn hit_test(&self, x: f32, y: f32, kit: &UiKit) -> Hit {
        topmost_hit([self.back.hit_target(BACK_ID, &kit.button)], x, y)
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
        // The side ornament art points left; the right one is mirrored through its UVs.
        let [left, right] = self.side_rects;
        let sprites = &mut frame.group_sprites;
        widgets::push_image(
            sprites,
            kit.ornament_side,
            left,
            FULL_UV,
            SpriteMaterial::PLAIN,
        );
        widgets::push_image(
            sprites,
            kit.ornament_side,
            right,
            Rect::new(1.0, 0.0, -1.0, 1.0),
            SpriteMaterial::PLAIN,
        );
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
            .draw(frame, &kit.button, (BUTTON_TEXT_SIZE * scale).round());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_and_headings_are_translated() {
        assert!(crate::game::locale::is_translated(TITLE));
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
        assert!(bottom < BACK_BUTTON[1], "credits end at {bottom}");
    }
}
