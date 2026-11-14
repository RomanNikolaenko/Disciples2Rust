//! Menu controller: the Main, Settings and Credits screens over a shared background.
//!
//! Layout is authored for a 1920x1080 reference and scaled uniformly with the surface
//! (`min(width / 1920, height / 1080)`), so menus keep their proportions at any
//! resolution and DPI. All runtime geometry is in physical pixels.
//!
//! Art comes from `assets/ui/` (see [`UiKit`]); missing or broken files are logged and
//! replaced by flat colors. Button art contract:
//! - `button_base.webp`: 1410x342 RGBA, straight alpha. Its alpha is also the hit shape,
//!   so the transparent gaps between the end ornaments never take hover or clicks.
//!   9-slice borders: 240 texels left/right (ornaments + frame), 40 top/bottom.
//! - `button_glow_mask.webp`: same size, grayscale, linear; its solid part marks the
//!   crystals, which light up on hover. The light stays inside them (no halo).
//!
//! The background, panel shade, slider knob, ornaments and brush underline are
//! placeholders from `examples/generate_placeholder_assets.rs`.
//!
//! Screens switch through [`ScreenFlow`]: the whole group fades out, the screen changes,
//! then it fades back in. Settings and Credits darken the background with an overlay
//! that fades with the group.

use std::path::Path;
use std::time::Instant;

use super::credits_screen::CreditsScreen;
use super::screen_flow::{FlowEvent, ScreenFlow, Transition};
use super::settings_screen::{SettingsEvent, SettingsScreen, WHEEL_LINE};
use super::widgets::{
    self, DIALOG_CANCEL, DIALOG_CONFIRM, Dialog, FULL_UV, StoneButton, UiKit, palette, scaled,
};
use crate::engine::render::GpuContext;
use crate::engine::render::ui_renderer::{SpriteMaterial, TextureSetId, UiFrame, UiRenderer};
use crate::engine::ui::geometry::{Rect, cover_uv};
use crate::engine::ui::hit::{Hit, topmost_hit};
use crate::engine::ui::pointer::{PointerPosition, PointerRouter};
use crate::game::locale::Language;
use crate::game::settings::{DisplayInfo, DisplaySettings, Settings};

const REFERENCE_SIZE: [f32; 2] = [1920.0, 1080.0];
const MENU_LEFT: f32 = 110.0;
const MENU_TOP: f32 = 430.0;
/// Close to the art's natural aspect (1410x342 → 330x80), so the stone barely stretches.
const BUTTON_SIZE: [f32; 2] = [336.0, 80.0];
const BUTTON_PITCH: f32 = 90.0;
/// Extra space before buttons that start a new section (e.g. Exit).
const SECTION_GAP: f32 = 28.0;
const PANEL_WIDTH: f32 = 780.0;
const LABEL_SIZE: f32 = 26.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Settings,
    Credits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    NewGame,
    LoadGame,
    OpenSettings,
    OpenCredits,
    Exit,
}

struct ButtonSpec {
    label: &'static str,
    command: Command,
    enabled: bool,
    starts_section: bool,
}

/// New game / Load game stay disabled until there is a game to start or load.
const MAIN_BUTTONS: [ButtonSpec; 5] = [
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
        label: "Credits",
        command: Command::OpenCredits,
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

/// What the application has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEvent {
    Exit,
    /// The player applied settings: propagate them, and persist them when `save` (a
    /// window change is saved only once the player keeps it).
    SettingsApplied {
        settings: Settings,
        save: bool,
    },
}

/// Hit id of the exit confirmation dialog's base; the buttons add
/// [`DIALOG_CONFIRM`](super::widgets::DIALOG_CONFIRM) / `DIALOG_CANCEL`. Main buttons use
/// ids 0..4, so this stays clear of them.
const EXIT_DIALOG_ID: usize = 100;

pub struct MainMenu {
    kit: UiKit,
    background: TextureSetId,
    background_size: [f32; 2],
    panel: TextureSetId,
    /// Same order as [`MAIN_BUTTONS`].
    main_buttons: Vec<StoneButton>,
    settings: SettingsScreen,
    credits: CreditsScreen,
    /// "Exit the game?" confirmation; opens instead of quitting right away.
    exit_dialog: Dialog,
    exit_dialog_open: bool,
    flow: ScreenFlow<Screen>,
    router: PointerRouter,
    pointer: PointerPosition,
    /// Keyboard-focused widget (same id space as the hit test); `None` when nothing
    /// is focused. Mouse presses and screen changes move or clear it.
    focus: Option<usize>,
    /// An event produced by input, reported by the next [`MainMenu::update`].
    pending: Option<MenuEvent>,
    viewport: [f32; 2],
    scale: f32,
    /// Language the labels currently show; `None` until the first frame sets them.
    shown_language: Option<Language>,
}

impl MainMenu {
    pub fn new(
        gpu: &GpuContext,
        renderer: &mut UiRenderer,
        assets_dir: &Path,
        viewport: [u32; 2],
        settings: Settings,
        display: DisplayInfo,
    ) -> Self {
        let (device, queue) = (gpu.device(), gpu.queue());
        let ui_dir = assets_dir.join("ui");
        let kit = UiKit::load(gpu, renderer, &ui_dir);

        let background_image = widgets::load_color(
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
        let panel_image = widgets::load_color(
            &ui_dir.join("menu_panel_shade.webp"),
            [1, 1],
            [0, 0, 0, 190],
        );
        let panel =
            renderer.create_texture_set(device, queue, "menu panel shade", &panel_image, None);

        let main_buttons = MAIN_BUTTONS
            .iter()
            .map(|spec| StoneButton::new(renderer, spec.label, spec.enabled))
            .collect();
        let settings = SettingsScreen::new(renderer, settings, display);
        let credits = CreditsScreen::new(renderer);
        let exit_dialog = Dialog::new(renderer, "Exit the game?", ["Exit", "Cancel"]);

        let mut menu = Self {
            kit,
            background,
            background_size,
            panel,
            main_buttons,
            settings,
            credits,
            exit_dialog,
            exit_dialog_open: false,
            flow: ScreenFlow::new(Screen::Main),
            router: PointerRouter::default(),
            pointer: PointerPosition::default(),
            focus: None,
            pending: None,
            viewport: [0.0; 2],
            scale: 1.0,
            shown_language: None,
        };
        menu.resize(viewport);
        menu
    }

    /// Recomputes all layouts for a new surface size (physical pixels).
    pub fn resize(&mut self, viewport: [u32; 2]) {
        if viewport[0] == 0 || viewport[1] == 0 {
            return;
        }
        self.viewport = [viewport[0] as f32, viewport[1] as f32];
        self.scale = ui_scale(self.viewport);
        let rects = layout_column(
            MAIN_BUTTONS.iter().map(|spec| spec.starts_section),
            self.scale,
        );
        for (button, rect) in self.main_buttons.iter_mut().zip(rects) {
            button.place(rect, &self.kit.button);
        }
        self.settings.layout(self.scale, self.viewport, &self.kit);
        self.credits.layout(self.scale, self.viewport, &self.kit);
        self.exit_dialog
            .layout(self.scale, self.viewport, &self.kit);
        self.refresh_hover();
    }

    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        self.pointer.moved([x, y]);
        if self.flow.current() == Screen::Settings
            && let Some(id) = self.router.captured()
        {
            self.settings.drag(id, x, y);
        }
        self.refresh_hover();
    }

    pub fn pointer_left(&mut self) {
        self.pointer.left();
        self.refresh_hover();
    }

    /// The pointer came back without a move (see [`PointerPosition`]).
    pub fn pointer_entered(&mut self) {
        self.pointer.entered();
        self.refresh_hover();
    }

    /// The window lost focus: drop any press in progress, so a drag does not continue
    /// after Alt+Tab or a release outside the window.
    pub fn focus_lost(&mut self) {
        self.router.reset();
        self.refresh_hover();
    }

    pub fn pointer_pressed(&mut self) {
        // A press proves the pointer is over the window, even after a spurious leave.
        if self.pointer.pressed() {
            self.refresh_hover();
        }
        tracing::debug!(hovered = ?self.router.hovered(), pointer = ?self.pointer.get(), "pointer pressed");
        if !self.flow.is_interactive() {
            return;
        }
        // A mouse press moves the keyboard focus onto the pressed widget; a press on
        // the background clears it.
        self.focus = self.router.hovered();
        self.router.press();
        if self.flow.current() == Screen::Settings
            && let (Some(id), Some([x, y])) = (self.router.captured(), self.pointer.get())
        {
            self.settings.press(id, x, y);
        }
    }

    /// Mouse wheel in notches (positive = away from the player, i.e. scroll up).
    pub fn scroll_lines(&mut self, lines: f32) {
        self.scroll_by(-lines * WHEEL_LINE);
    }

    /// Touchpad scrolling in physical pixels (positive = content moves down).
    pub fn scroll_pixels(&mut self, pixels: f32) {
        if self.scale > 0.0 {
            self.scroll_by(-pixels / self.scale);
        }
    }

    /// Scrolls the Settings list by `delta` reference px.
    fn scroll_by(&mut self, delta: f32) {
        if self.flow.is_interactive() && self.flow.current() == Screen::Settings {
            self.settings.scroll(delta);
            self.refresh_hover();
        }
    }

    pub fn pointer_released(&mut self) {
        let Some(id) = self.router.release() else {
            return;
        };
        if !self.flow.is_interactive() {
            return;
        }
        self.click(id);
        // A transition makes the menu non-interactive: let hover glows fade out.
        self.refresh_hover();
    }

    /// Runs the action of widget `id` (a mouse click or a keyboard activation).
    fn click(&mut self, id: usize) {
        match self.flow.current() {
            Screen::Main => {
                if self.exit_dialog_open {
                    // The dialog is modal: only its buttons answer clicks.
                    match id.checked_sub(EXIT_DIALOG_ID) {
                        Some(DIALOG_CONFIRM) => {
                            self.exit_dialog_open = false;
                            self.focus = None;
                            tracing::info!("exit confirmed");
                            self.flow.request(Transition::Exit);
                        }
                        Some(DIALOG_CANCEL) => {
                            self.exit_dialog_open = false;
                            self.focus = None;
                            tracing::debug!("exit canceled");
                        }
                        _ => {}
                    }
                } else {
                    match MAIN_BUTTONS.get(id).map(|spec| spec.command) {
                        Some(Command::OpenSettings) => {
                            self.flow.request(Transition::To(Screen::Settings))
                        }
                        Some(Command::OpenCredits) => {
                            self.flow.request(Transition::To(Screen::Credits))
                        }
                        // The exit asks for confirmation first; its buttons take the focus.
                        Some(Command::Exit) => {
                            self.exit_dialog_open = true;
                            self.exit_dialog.show();
                            self.focus = Some(EXIT_DIALOG_ID + DIALOG_CONFIRM);
                            tracing::debug!("exit confirmation requested");
                        }
                        Some(command @ (Command::NewGame | Command::LoadGame)) => {
                            tracing::info!(?command, "not available yet");
                        }
                        None => {}
                    }
                }
            }
            Screen::Settings => {
                let was_modal = self.settings.modal_open();
                let event = self.settings.click(id);
                self.handle_settings_event(event);
                if self.settings.modal_open() {
                    // A dialog just opened: focus its confirm button.
                    self.focus = self.settings.focus_stops().first().copied();
                } else if was_modal {
                    // The dialog the click went to just closed; its ids are gone.
                    self.focus = None;
                }
            }
            Screen::Credits => {
                if self.credits.click(id) {
                    self.flow.request(Transition::To(Screen::Main));
                }
            }
        }
    }

    /// Tab / Shift+Tab: moves the keyboard focus to the next (previous) stop,
    /// wrapping around; starting from nothing focuses the first (last) stop.
    pub fn tab_focus(&mut self, backwards: bool) {
        if !self.flow.is_interactive() {
            return;
        }
        let stops = self.focus_stops();
        self.focus = next_focus(&stops, self.focus, backwards);
    }

    /// Enter / Space: activates the focused element like a mouse click on it.
    pub fn activate_focused(&mut self) {
        if !self.flow.is_interactive() {
            return;
        }
        if let Some(id) = self.focus {
            tracing::debug!(id, "keyboard activation");
            self.click(id);
        }
    }

    /// Arrow keys on a focused Settings slider or choice value.
    pub fn adjust_focused(&mut self, delta: isize) {
        if !self.flow.is_interactive() || self.flow.current() != Screen::Settings {
            return;
        }
        if let Some(id) = self.focus {
            self.settings.adjust(id, delta);
        }
    }

    /// Every focusable widget of the current screen, in visual order.
    fn focus_stops(&self) -> Vec<usize> {
        match self.flow.current() {
            Screen::Main => {
                if self.exit_dialog_open {
                    return vec![
                        EXIT_DIALOG_ID + DIALOG_CONFIRM,
                        EXIT_DIALOG_ID + DIALOG_CANCEL,
                    ];
                }
                (0..MAIN_BUTTONS.len())
                    .filter(|&index| self.main_buttons[index].is_interactive())
                    .collect()
            }
            Screen::Settings => self.settings.focus_stops(),
            Screen::Credits => self.credits.focus_stops(),
        }
    }

    /// When an unconfirmed display change reverts by itself, if one is waiting.
    pub fn confirmation_deadline(&self) -> Option<Instant> {
        self.settings.confirmation_deadline()
    }

    /// Reverts an unconfirmed display change whose deadline has passed, also while no
    /// frames are drawn (minimized window).
    pub fn expire_confirmation(&mut self, now: Instant) -> Option<MenuEvent> {
        let event = self.settings.expire_confirmation(now);
        self.handle_settings_event(event);
        self.pending.take()
    }

    /// Settings were changed outside the menu (e.g. the F11 display-mode toggle).
    pub fn set_applied_settings(&mut self, settings: Settings) {
        self.settings.set_applied(settings);
    }

    /// The connected monitors were enumerated again.
    pub fn set_display_info(&mut self, display: DisplayInfo) {
        self.settings.set_display_info(display);
    }

    /// Escape: leave Settings / Credits for the main screen.
    pub fn back(&mut self) {
        if !self.flow.is_interactive() {
            return;
        }
        match self.flow.current() {
            // The exit dialog closes on Escape; the main screen has nothing to leave.
            Screen::Main => {
                if self.exit_dialog_open {
                    self.exit_dialog_open = false;
                    self.focus = None;
                    self.router.reset();
                    self.refresh_hover();
                }
                return;
            }
            // Settings may close a dialog or ask about unapplied changes instead.
            Screen::Settings => {
                let was_modal = self.settings.modal_open();
                let event = self.settings.back();
                self.handle_settings_event(event);
                if was_modal && !self.settings.modal_open() {
                    // The dialog's button ids are no longer valid stops.
                    self.focus = None;
                }
            }
            Screen::Credits => self.flow.request(Transition::To(Screen::Main)),
        }
        self.router.reset();
        self.refresh_hover();
    }

    fn handle_settings_event(&mut self, event: Option<SettingsEvent>) {
        match event {
            Some(SettingsEvent::BackToMenu) => self.flow.request(Transition::To(Screen::Main)),
            Some(SettingsEvent::Apply {
                settings,
                save,
                leave,
            }) => {
                self.pending = Some(MenuEvent::SettingsApplied { settings, save });
                if leave {
                    self.flow.request(Transition::To(Screen::Main));
                }
            }
            None => {}
        }
    }

    /// Advances animations by `dt` seconds and reports what the application must do.
    pub fn update(&mut self, dt: f32) -> Option<MenuEvent> {
        match self.flow.update(dt) {
            FlowEvent::None => {}
            FlowEvent::ScreenChanged => {
                let screen = self.flow.current();
                tracing::debug!(?screen, "menu screen changed");
                match screen {
                    Screen::Main => self
                        .main_buttons
                        .iter_mut()
                        .for_each(StoneButton::reset_animation),
                    Screen::Settings => self.settings.open(),
                    Screen::Credits => self.credits.open(),
                }
                self.router.reset();
                self.focus = None;
                self.refresh_hover();
            }
            FlowEvent::ExitReady => return Some(MenuEvent::Exit),
        }
        if self.flow.current() == Screen::Settings {
            let event = self.settings.update(dt);
            if event.is_some() {
                self.handle_settings_event(event);
            }
            // The list moves under a resting pointer while it scrolls.
            if event.is_some() || self.settings.is_scrolling() {
                self.refresh_hover();
            }
        }

        // The focused widget glows like a hovered one.
        let hovered = self.router.hovered().or(self.focus);
        let pressed = self.router.pressed_visual();
        match self.flow.current() {
            Screen::Main => {
                for (index, button) in self.main_buttons.iter_mut().enumerate() {
                    button.animate(
                        !self.exit_dialog_open && hovered == Some(index),
                        !self.exit_dialog_open && pressed == Some(index),
                        dt,
                    );
                }
                if self.exit_dialog_open {
                    self.exit_dialog
                        .animate(hovered, pressed, EXIT_DIALOG_ID, dt);
                }
            }
            Screen::Settings => {
                self.settings
                    .animate(hovered, pressed, self.router.captured(), dt);
            }
            Screen::Credits => self.credits.animate(hovered, pressed, dt),
        }
        self.pending.take()
    }

    /// Appends this frame's sprites and labels. Updates label text that changed (e.g.
    /// slider values), hence the renderer.
    pub fn build_frame(&mut self, renderer: &mut UiRenderer, frame: &mut UiFrame) {
        let language = self.settings.language();
        if self.shown_language != Some(language) {
            self.relabel(renderer, language);
        }
        let [width, height] = self.viewport;
        let full = Rect::new(0.0, 0.0, width, height);
        widgets::push_image(
            &mut frame.background,
            self.background,
            full,
            cover_uv(self.background_size, self.viewport),
            background_material(
                &self
                    .settings
                    .display_preview(self.flow.current() == Screen::Settings),
            ),
        );
        widgets::push_image(
            &mut frame.background,
            self.panel,
            Rect::new(0.0, 0.0, (PANEL_WIDTH * self.scale).round(), height),
            FULL_UV,
            SpriteMaterial::PLAIN,
        );

        frame.group_opacity = self.flow.opacity();
        let label_size = (LABEL_SIZE * self.scale).round();
        match self.flow.current() {
            Screen::Main => {
                for button in &self.main_buttons {
                    button.draw(frame, &self.kit.button, label_size);
                }
                if self.exit_dialog_open {
                    self.exit_dialog.draw(frame, &self.kit);
                }
            }
            Screen::Settings => {
                widgets::push_flat(&mut frame.group_sprites, &self.kit, full, palette::OVERLAY);
                self.settings.build(frame, renderer, &self.kit);
            }
            Screen::Credits => {
                widgets::push_flat(&mut frame.group_sprites, &self.kit, full, palette::OVERLAY);
                self.credits.build(frame, &self.kit);
            }
        }
    }

    /// Shows every screen's fixed texts in `language` (on start and after the player
    /// applies another Display Language).
    fn relabel(&mut self, renderer: &mut UiRenderer, language: Language) {
        for button in &self.main_buttons {
            button.relabel(renderer, language);
        }
        self.settings.relabel(renderer, language);
        self.credits.relabel(renderer, language);
        self.exit_dialog.relabel(renderer, language);
        self.shown_language = Some(language);
        tracing::info!(?language, "interface language set");
    }

    fn hit_test(&self, x: f32, y: f32) -> Hit {
        match self.flow.current() {
            Screen::Main => {
                // The exit confirmation is modal: it blocks the buttons behind it.
                if self.exit_dialog_open {
                    return self.exit_dialog.hit_test(x, y, &self.kit, EXIT_DIALOG_ID);
                }
                topmost_hit(
                    self.main_buttons
                        .iter()
                        .enumerate()
                        .map(|(index, button)| button.hit_target(index, &self.kit.button)),
                    x,
                    y,
                )
            }
            Screen::Settings => self.settings.hit_test(x, y, &self.kit),
            Screen::Credits => self.credits.hit_test(x, y, &self.kit),
        }
    }

    /// Re-runs hit testing for the last pointer position.
    fn refresh_hover(&mut self) {
        let hovered = match self.pointer.get() {
            Some([x, y]) if self.flow.is_interactive() => self.hit_test(x, y).interactive(),
            _ => None,
        };
        self.router.set_hovered(hovered);
    }
}

/// The stop after (or before, `backwards`) `current` in `stops`, wrapping around;
/// from nothing (or a stop that is gone) it starts at the first (last) stop.
fn next_focus(stops: &[usize], current: Option<usize>, backwards: bool) -> Option<usize> {
    let count = stops.len();
    if count == 0 {
        return None;
    }
    let index = match current.and_then(|id| stops.iter().position(|&stop| stop == id)) {
        Some(index) if backwards => (index + count - 1) % count,
        Some(index) => (index + 1) % count,
        None if backwards => count - 1,
        None => 0,
    };
    Some(stops[index])
}

/// The background stands in for the game image: brightness and gamma calibrate it
/// (linear light), while the interface keeps its colors.
fn background_material(display: &DisplaySettings) -> SpriteMaterial {
    let factor = display.brightness_factor();
    SpriteMaterial {
        tint: [factor, factor, factor, 1.0],
        gamma: display.gamma_value(),
        ..SpriteMaterial::PLAIN
    }
}

/// Uniform UI scale relative to the 1920x1080 reference.
fn ui_scale(viewport: [f32; 2]) -> f32 {
    (viewport[0] / REFERENCE_SIZE[0]).min(viewport[1] / REFERENCE_SIZE[1])
}

/// Places the main column of buttons, snapped to whole pixels.
fn layout_column(section_starts: impl Iterator<Item = bool>, scale: f32) -> Vec<Rect> {
    let mut y = MENU_TOP;
    section_starts
        .enumerate()
        .map(|(index, starts_section)| {
            if starts_section && index > 0 {
                y += SECTION_GAP;
            }
            let rect = scaled(scale, MENU_LEFT, y, BUTTON_SIZE[0], BUTTON_SIZE[1]);
            y += BUTTON_PITCH;
            rect
        })
        .collect()
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
        let rects = layout_column(MAIN_BUTTONS.iter().map(|spec| spec.starts_section), 1.0);
        let ys: Vec<f32> = rects.iter().map(|r| r.y).collect();
        assert_eq!(ys, [430.0, 520.0, 610.0, 700.0, 818.0]);
        assert!(
            rects
                .iter()
                .all(|r| r.x == 110.0 && r.w == 336.0 && r.h == 80.0)
        );
        assert!(rects[4].y + rects[4].h <= 1080.0, "Exit stays on screen");
    }

    #[test]
    fn column_scales() {
        let rects = layout_column([false].into_iter(), 0.5);
        assert_eq!(rects[0], Rect::new(55.0, 215.0, 168.0, 40.0));
    }

    #[test]
    fn calibration_changes_only_the_color() {
        let mut display = DisplaySettings::default();
        assert_eq!(background_material(&display).tint, [1.0; 4]);
        assert_eq!(background_material(&display).gamma, 1.0);
        display.brightness = 0;
        display.gamma = 220;
        let material = background_material(&display);
        assert_eq!(material.tint, [0.5, 0.5, 0.5, 1.0], "coverage unchanged");
        assert!((material.gamma - 2.2).abs() < 1e-6);
    }

    #[test]
    fn main_buttons_are_translated() {
        for spec in &MAIN_BUTTONS {
            assert!(
                crate::game::locale::is_translated(spec.label),
                "{}",
                spec.label
            );
        }
    }

    #[test]
    fn button_specs_match_the_menu_design() {
        let enabled: Vec<&str> = MAIN_BUTTONS
            .iter()
            .filter(|b| b.enabled)
            .map(|b| b.label)
            .collect();
        assert_eq!(enabled, ["Settings", "Credits", "Exit"]);
    }

    #[test]
    fn focus_moves_through_stops_and_wraps() {
        let stops = [3, 5, 9];
        assert_eq!(next_focus(&stops, None, false), Some(3));
        assert_eq!(next_focus(&stops, None, true), Some(9));
        assert_eq!(next_focus(&stops, Some(5), false), Some(9));
        assert_eq!(next_focus(&stops, Some(9), false), Some(3), "wraps forward");
        assert_eq!(next_focus(&stops, Some(3), true), Some(9), "wraps backward");
        assert_eq!(
            next_focus(&stops, Some(42), false),
            Some(3),
            "a vanished stop restarts"
        );
        assert_eq!(next_focus(&[], Some(1), false), None);
    }

    #[test]
    fn exit_dialog_ids_stay_clear_of_the_main_buttons() {
        assert!(EXIT_DIALOG_ID >= MAIN_BUTTONS.len());
    }

    #[test]
    fn exit_dialog_is_translated() {
        assert!(crate::game::locale::is_translated("Exit the game?"));
    }
}
