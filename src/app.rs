//! Application lifecycle: main window, GPU context and per-frame flow.
//!
//! Per-frame order (AGENTS.md): input events are dispatched to the menu as they arrive;
//! on redraw: frame timing → menu update/animation → build UI frame → encode/submit.
//!
//! Frame pacing: with V-Sync the present call paces frames; a frame rate limit (from
//! Settings) additionally waits until the next frame deadline via `ControlFlow::WaitUntil`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::{MonitorHandle, VideoModeHandle};
use winit::window::{Fullscreen, Window, WindowId};

use crate::engine::assets::assets_dir;
use crate::engine::render::ui_renderer::{
    LabelDraw, LabelId, TextAlign, TextStyle, UiFrame, UiRenderer,
};
use crate::engine::render::{FrameStatus, GpuContext};
use crate::engine::time::{FpsCounter, FrameClock, FrameStats, MAX_ANIMATION_STEP};
use crate::engine::ui::geometry::Rect;
use crate::game::locale::UI_FONTS;
use crate::game::menus::main_menu::{MainMenu, MenuEvent};
use crate::game::settings::{
    AspectRatio, DisplayInfo, DisplayMode, DisplaySettings, MonitorInfo, Settings, VideoMode,
    settings_path,
};
use crate::game::update::{self, Updater};

const WINDOW_TITLE: &str = "Disciples2Rust";
/// Smallest supported window client area, matching [`MIN_RESOLUTION`] in the display
/// settings: smaller screens are out of scope.
const MIN_WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 720.0);
const FRAME_STATS_INTERVAL: Duration = Duration::from_secs(5);
const FPS_COUNTER_WINDOW: Duration = Duration::from_millis(500);
/// sRGB colors of the FPS counter and its shadow.
const FPS_TEXT: [u8; 4] = [242, 240, 235, 255];
const FPS_SHADOW: [u8; 4] = [0, 0, 0, 170];

pub struct App {
    // Field order matters for drop order: UI resources and the GPU context (and its
    // surface) are dropped before the window they present to.
    menu: Option<MainMenu>,
    renderer: Option<UiRenderer>,
    gpu: Option<GpuContext>,
    window: Option<Arc<Window>>,
    /// Render list rebuilt every frame; kept to reuse its allocations.
    ui_frame: UiFrame,
    clock: FrameClock,
    stats: FrameStats,
    /// True while drawing is paused because the window has no drawable area.
    paused: bool,
    modifiers: ModifiersState,
    /// Whether the window has keyboard focus (the background FPS limit applies otherwise).
    focused: bool,
    fps_counter: FpsCounter,
    /// Last measured FPS and the overlay label showing it (created on first use).
    fps: Option<u32>,
    fps_label: Option<LabelId>,
    /// Single source of truth for user settings; saved when the player applies changes.
    settings: Settings,
    settings_path: Option<PathBuf>,
    /// Checks for and installs game updates in the background.
    updater: Updater,
    /// Extra size the window frame adds over the client area, so a windowed window is
    /// never larger than its screen; see [`DisplayInfo::windowed_max`].
    window_frame: [u32; 2],
    fatal_error: Option<anyhow::Error>,
}

impl App {
    pub fn new() -> Self {
        let settings_path = settings_path();
        let settings = Settings::load_or_default(settings_path.as_deref());
        // The previous version has exited by now: its leftovers can go.
        update::clean_up_previous_update();
        let updater = Updater::start();
        Self {
            menu: None,
            renderer: None,
            gpu: None,
            window: None,
            ui_frame: UiFrame::default(),
            clock: FrameClock::default(),
            stats: FrameStats::new(FRAME_STATS_INTERVAL),
            paused: false,
            modifiers: ModifiersState::default(),
            focused: true,
            fps_counter: FpsCounter::new(FPS_COUNTER_WINDOW),
            fps: None,
            fps_label: None,
            settings,
            settings_path,
            updater,
            window_frame: [0, 0],
            fatal_error: None,
        }
    }

    /// The error that stopped the event loop, if any.
    pub fn take_fatal_error(&mut self) -> Option<anyhow::Error> {
        self.fatal_error.take()
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        tracing::error!("{error:#}");
        self.fatal_error = Some(error);
        event_loop.exit();
    }

    fn init_graphics(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        // Created hidden and shown after the first presented frame, so the OS never
        // shows an unpainted (white) client area. The saved display mode decides the
        // window kind; exclusive fullscreen needs the window's monitor, so it starts
        // borderless and switches right after creation.
        let display = self.settings.display;
        let monitors = ordered_monitors(
            event_loop.primary_monitor(),
            event_loop.available_monitors(),
        );
        let monitor = target_monitor(&monitors, &display, None);
        let attributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_min_inner_size(MIN_WINDOW_SIZE)
            .with_visible(false);
        let attributes = match display.mode {
            DisplayMode::Windowed => {
                let attributes = attributes.with_inner_size(PhysicalSize::new(
                    display.resolution[0],
                    display.resolution[1],
                ));
                match &monitor {
                    Some(monitor) => {
                        attributes.with_position(centered_on(monitor, display.resolution))
                    }
                    None => attributes,
                }
            }
            DisplayMode::Borderless | DisplayMode::Fullscreen => {
                attributes.with_fullscreen(Some(Fullscreen::Borderless(monitor)))
            }
        };
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("failed to create the main window")?,
        );
        // The frame size is measured on a windowed window; in fullscreen it is hidden.
        self.window_frame = frame_overhead(&window);
        tracing::info!(
            scale_factor = window.scale_factor(),
            size = ?window.inner_size(),
            frame = ?self.window_frame,
            "main window created"
        );
        if display.mode == DisplayMode::Fullscreen {
            apply_window_mode(&window, &display, self.window_frame);
        } else if display.mode == DisplayMode::Windowed {
            // The saved size may be larger than the screen (older settings file): shrink
            // the still hidden window to fit before anything sees it.
            let monitor = window.current_monitor();
            let size = windowed_clamp(display.resolution, monitor.as_ref(), self.window_frame);
            if size != display.resolution {
                tracing::info!(?size, "windowed size clamped to the screen");
                let _ = window.request_inner_size(PhysicalSize::new(size[0], size[1]));
            }
        }

        self.init_rendering(event_loop, &window)?;
        // The menu starts fully transparent, so this first frame shows the background.
        self.draw();
        window.set_visible(true);

        event_loop.set_control_flow(ControlFlow::Wait);
        window.request_redraw();
        self.window = Some(window);
        Ok(())
    }

    /// (Re)creates the GPU context and everything that owns GPU resources.
    fn init_rendering(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Arc<Window>,
    ) -> anyhow::Result<()> {
        self.menu = None;
        self.renderer = None;
        self.gpu = None;
        self.fps_label = None;
        let gpu = create_gpu_context(event_loop, window, self.settings.display.vsync)?;
        let fonts: Vec<PathBuf> = UI_FONTS
            .iter()
            .map(|font| assets_dir().join(font))
            .collect();
        let mut renderer = UiRenderer::new(gpu.device(), gpu.queue(), gpu.surface_format(), &fonts);
        let size = window.inner_size();
        // An older settings file may keep an aspect ratio this monitor cannot display.
        snap_aspect_ratio(
            &mut self.settings,
            &display_info(&window_monitors(window), self.window_frame),
        );
        let menu = MainMenu::new(
            &gpu,
            &mut renderer,
            &assets_dir(),
            [size.width, size.height],
            self.settings,
            display_info(&window_monitors(window), self.window_frame),
            self.updater.clone(),
        );
        self.menu = Some(menu);
        self.renderer = Some(renderer);
        self.gpu = Some(gpu);
        Ok(())
    }

    /// Builds this frame's UI and renders it. `None` if rendering is not initialized.
    fn draw(&mut self) -> Option<FrameStatus> {
        let (Some(gpu), Some(renderer), Some(menu)) = (
            self.gpu.as_mut(),
            self.renderer.as_mut(),
            self.menu.as_mut(),
        ) else {
            return None;
        };
        self.ui_frame.clear();
        menu.build_frame(renderer, &mut self.ui_frame);
        if self.settings.display.show_fps
            && let (Some(window), Some(fps)) = (&self.window, self.fps)
        {
            let label = *self.fps_label.get_or_insert_with(|| {
                renderer.create_label(
                    "",
                    TextStyle {
                        bold: true,
                        align: TextAlign::Right,
                    },
                )
            });
            renderer.set_label_text(label, &format!("{fps} FPS"));
            push_fps_counter(
                &mut self.ui_frame.overlay_labels,
                label,
                window.inner_size(),
                self.settings.display.ui_scale_factor(),
            );
        }
        let frame = &self.ui_frame;
        Some(gpu.render_frame(|target, encoder| renderer.render(target, encoder, frame)))
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        // On restore, Windows can deliver a redraw before the non-zero `Resized`; only
        // `resized` ends the pause, so this frame must not tick the clock.
        if self.paused {
            return;
        }
        let Some(window) = self.window.clone() else {
            return;
        };

        if self.gpu.as_ref().is_some_and(GpuContext::is_device_lost) {
            // All GPU resources are gone: rebuild the context, renderer and menu.
            tracing::warn!("recreating rendering after GPU device loss");
            if let Err(error) = self.init_rendering(event_loop, &window) {
                return self.fail(event_loop, error);
            }
        }

        let frame_started = Instant::now();
        let frame_time = self.clock.tick(frame_started);
        if let Some(frame_time) = frame_time
            && let Some(fps) = self.fps_counter.record(frame_time)
        {
            self.fps = Some(fps);
        }
        if let Some(frame_time) = frame_time
            && let Some(summary) = self.stats.record(frame_time)
        {
            tracing::debug!(
                frames = summary.frames,
                avg_ms = summary.average.as_secs_f64() * 1000.0,
                worst_ms = summary.worst.as_secs_f64() * 1000.0,
                "frame timing"
            );
        }
        let dt = frame_time.unwrap_or_default().min(MAX_ANIMATION_STEP);

        match self
            .menu
            .as_mut()
            .and_then(|menu| menu.update(dt.as_secs_f32()))
        {
            Some(MenuEvent::Exit) => {
                event_loop.exit();
                return;
            }
            Some(MenuEvent::Restart) => match update::restart() {
                Ok(()) => {
                    tracing::info!("restarting into the updated game");
                    event_loop.exit();
                    return;
                }
                Err(error) => tracing::error!("{error:#}"),
            },
            Some(MenuEvent::SettingsApplied { settings, save }) => {
                self.apply_settings(settings, save);
            }
            None => {}
        }

        match self.draw() {
            Some(FrameStatus::Presented | FrameStatus::Skipped) => {
                self.schedule_next_frame(event_loop, &window, frame_started);
            }
            // Stop the redraw loop until a non-zero resize; see `resized`.
            Some(FrameStatus::SurfaceUnavailable) => self.paused = true,
            None => {}
        }
    }

    /// Makes `settings` current: applies display changes and, when `save`, writes them
    /// to disk. Audio and graphics values are only stored (no audio playback or 3D scene
    /// exists yet).
    fn apply_settings(&mut self, mut settings: Settings, save: bool) {
        // A value the monitor cannot display (e.g. loaded from an older file) falls
        // back to Auto before anything sees it.
        if let Some(window) = &self.window {
            let info = display_info(&window_monitors(window), self.window_frame);
            snap_aspect_ratio(&mut settings, &info);
        }
        let previous = self.settings.display;
        self.settings = settings;
        let display = settings.display;
        if previous.window_changes(&display) {
            if let Some(window) = &self.window {
                apply_window_mode(window, &display, self.window_frame);
            }
            // Once windowed, re-measure the frame in case it was unknown before, and
            // give the Display tab the corrected maximum window size.
            self.refresh_window_frame();
            if let Some(window) = &self.window {
                let info = display_info(&window_monitors(window), self.window_frame);
                if let Some(menu) = self.menu.as_mut() {
                    menu.set_display_info(info);
                }
            }
        }
        if previous.vsync != display.vsync
            && let Some(gpu) = self.gpu.as_mut()
        {
            gpu.set_vsync(display.vsync);
        }
        if save {
            self.save_settings();
        } else {
            tracing::info!("display change applied; saved once the player keeps it");
        }
    }

    /// Re-measures the window frame and refreshes the Display tab data; only meaningful
    /// while the window is windowed (in fullscreen the frame is hidden).
    fn refresh_window_frame(&mut self) {
        if self.settings.display.mode != DisplayMode::Windowed {
            return;
        }
        let Some(window) = self.window.clone() else {
            return;
        };
        let frame = frame_overhead(&window);
        if frame == self.window_frame {
            return;
        }
        self.window_frame = frame;
        if let Some(menu) = self.menu.as_mut() {
            let info = display_info(&window_monitors(&window), frame);
            menu.set_display_info(info);
        }
    }

    fn save_settings(&self) {
        let Some(path) = &self.settings_path else {
            tracing::warn!("no config directory; settings are not saved");
            return;
        };
        match self.settings.save(path) {
            Ok(()) => tracing::info!(path = %path.display(), "settings saved"),
            Err(error) => tracing::error!("{error}"),
        }
    }

    /// F11 / Alt+Enter: switches between a window and fullscreen through the settings,
    /// so the Display tab and the saved file stay in sync.
    fn toggle_fullscreen(&mut self) {
        let mut settings = self.settings;
        settings.display.mode = toggled_mode(settings.display.mode);
        if settings.display.mode == DisplayMode::Windowed
            && let Some(window) = &self.window
        {
            let display = &settings.display;
            let options = display_info(&window_monitors(window), self.window_frame)
                .resolutions(display.monitor);
            settings.display.resolution = windowed_fit(
                display.resolution,
                windowed_size_max(window, self.window_frame),
                &options,
            );
        }
        tracing::info!(mode = ?settings.display.mode, "display mode toggled");
        self.apply_settings(settings, true);
        // The window may just have become windowed: re-measure the frame for the cap.
        self.refresh_window_frame();
        if let Some(menu) = self.menu.as_mut() {
            menu.set_applied_settings(settings);
        }
    }

    /// Requests the next redraw now, or at the frame-rate-limit deadline.
    fn schedule_next_frame(&self, event_loop: &ActiveEventLoop, window: &Window, started: Instant) {
        let fps = self.settings.display.frame_limit(self.focused);
        match next_frame_deadline(started, fps) {
            Some(deadline) if deadline > Instant::now() => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
            _ => window.request_redraw(),
        }
    }

    fn resized(&mut self, size: PhysicalSize<u32>) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.resize(size);
        if let Some(menu) = self.menu.as_mut() {
            menu.resize([size.width, size.height]);
        }
        let has_area = size.width > 0 && size.height > 0;
        if !has_area {
            // Pause here rather than waiting for a redraw: Windows delivers no redraws
            // while minimized, and the pause must not be measured as one long frame.
            if !self.paused {
                tracing::debug!("window has no drawable area; rendering paused");
            }
            self.paused = true;
        } else if self.paused {
            tracing::debug!(?size, "rendering resumed");
            self.paused = false;
            self.clock.reset();
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        // A deadline passed: the next frame of the frame rate limit or, while no frames
        // are drawn, an unconfirmed display change that must revert.
        if let StartCause::ResumeTimeReached { .. } = cause {
            event_loop.set_control_flow(ControlFlow::Wait);
            if self.paused {
                let event = self
                    .menu
                    .as_mut()
                    .and_then(|menu| menu.expire_confirmation(Instant::now()));
                if let Some(MenuEvent::SettingsApplied { settings, save }) = event {
                    self.apply_settings(settings, save);
                }
            } else if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Frames stop while the window has no drawable area (minimized); still wake up to
        // revert an unconfirmed display change on time.
        if self.paused
            && let Some(deadline) = self.menu.as_ref().and_then(MainMenu::confirmation_deadline)
        {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Desktop platforms resume once at startup; keep the existing window afterwards.
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.init_graphics(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                tracing::debug!(?size, "window resized");
                self.resized(size);
            }
            // winit follows a scale change with `Resized` when the physical size changes.
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                tracing::info!(scale_factor, "window scale factor changed");
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(focused) => {
                tracing::debug!(focused, "window focus changed");
                self.focused = focused;
                if !focused && let Some(menu) = self.menu.as_mut() {
                    menu.focus_lost();
                }
                // Leave a long background wait right away when focus returns.
                if focused && let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorEntered { .. } => {
                tracing::debug!("cursor entered");
                if let Some(menu) = self.menu.as_mut() {
                    menu.pointer_entered();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                tracing::trace!(x = position.x, y = position.y, "cursor moved");
                if let Some(menu) = self.menu.as_mut() {
                    menu.pointer_moved(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                tracing::debug!("cursor left");
                if let Some(menu) = self.menu.as_mut() {
                    menu.pointer_left();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(menu) = self.menu.as_mut() {
                    match delta {
                        MouseScrollDelta::LineDelta(_, lines) => menu.scroll_lines(lines),
                        MouseScrollDelta::PixelDelta(offset) => {
                            menu.scroll_pixels(offset.y as f32);
                        }
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(menu) = self.menu.as_mut() {
                    match state {
                        ElementState::Pressed => menu.pointer_pressed(),
                        ElementState::Released => menu.pointer_released(),
                    }
                }
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // Synthetic presses replay keys held while focus was elsewhere.
                if is_synthetic || event.state != ElementState::Pressed || event.repeat {
                    return;
                }
                if is_fullscreen_toggle(
                    event.physical_key,
                    event.state,
                    event.repeat,
                    self.modifiers.alt_key(),
                ) {
                    self.toggle_fullscreen();
                } else if let (Some(key), Some(menu)) = (
                    menu_key(event.physical_key, self.modifiers.shift_key()),
                    self.menu.as_mut(),
                ) {
                    match key {
                        MenuKey::Back => menu.back(),
                        MenuKey::Next => menu.tab_focus(false),
                        MenuKey::Previous => menu.tab_focus(true),
                        MenuKey::Activate => menu.activate_focused(),
                        MenuKey::Decrease => menu.adjust_focused(-1),
                        MenuKey::Increase => menu.adjust_focused(1),
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }
}

fn create_gpu_context(
    event_loop: &ActiveEventLoop,
    window: &Arc<Window>,
    vsync: bool,
) -> anyhow::Result<GpuContext> {
    pollster::block_on(GpuContext::new(
        Arc::clone(window),
        event_loop.owned_display_handle(),
        vsync,
    ))
    .context("failed to initialize GPU rendering")
}

fn monitor_size(window: &Window) -> Option<[u32; 2]> {
    window.current_monitor().map(|monitor| {
        let size = monitor.size();
        [size.width, size.height]
    })
}

/// Monitors in settings order: the primary first, then the others as the OS lists them.
fn ordered_monitors(
    primary: Option<MonitorHandle>,
    all: impl Iterator<Item = MonitorHandle>,
) -> Vec<MonitorHandle> {
    let mut monitors: Vec<MonitorHandle> = primary.into_iter().collect();
    for monitor in all {
        if !monitors.contains(&monitor) {
            monitors.push(monitor);
        }
    }
    monitors
}

fn window_monitors(window: &Window) -> Vec<MonitorHandle> {
    ordered_monitors(window.primary_monitor(), window.available_monitors())
}

/// The monitor the settings ask for: the stored index, else `current`, else the primary.
fn target_monitor(
    monitors: &[MonitorHandle],
    display: &DisplaySettings,
    current: Option<MonitorHandle>,
) -> Option<MonitorHandle> {
    monitors
        .get(usize::from(display.monitor))
        .cloned()
        .or(current)
        .or_else(|| monitors.first().cloned())
}

/// Drops a stored aspect ratio the selected monitor cannot display (e.g. a 16:10 value
/// saved on a 16:9 screen): Auto follows the resolution instead, so the Display tab
/// only ever shows ratios the hardware supports.
fn snap_aspect_ratio(settings: &mut Settings, info: &DisplayInfo) {
    let supported = info.aspect_ratios(settings.display.monitor);
    if !supported.contains(&settings.display.aspect_ratio) {
        tracing::info!(
            stored = ?settings.display.aspect_ratio,
            supported = ?supported,
            "aspect ratio is not supported by the monitor; resetting to Auto"
        );
        settings.display.aspect_ratio = AspectRatio::Auto;
    }
}

/// What the monitors offer, for the Display tab.
fn display_info(monitors: &[MonitorHandle], window_frame: [u32; 2]) -> DisplayInfo {
    let monitors = monitors
        .iter()
        .map(|monitor| MonitorInfo {
            size: [monitor.size().width, monitor.size().height],
            refresh_millihertz: monitor.refresh_rate_millihertz(),
            modes: monitor
                .video_modes()
                .map(|mode| VideoMode {
                    size: [mode.size().width, mode.size().height],
                    refresh_millihertz: mode.refresh_rate_millihertz(),
                })
                .collect(),
        })
        .collect();
    DisplayInfo {
        monitors,
        window_frame,
    }
}

/// Top-left position that centers a window of `size` on `monitor` (physical pixels).
fn centered_on(monitor: &MonitorHandle, size: [u32; 2]) -> PhysicalPosition<i32> {
    let origin = monitor.position();
    let area = monitor.size();
    let offset = |area: u32, size: u32| (i64::from(area) - i64::from(size)).max(0) / 2;
    let x = i64::from(origin.x) + offset(area.width, size[0]);
    let y = i64::from(origin.y) + offset(area.height, size[1]);
    PhysicalPosition::new(
        i32::try_from(x).unwrap_or(origin.x),
        i32::try_from(y).unwrap_or(origin.y),
    )
}

/// A window size for F11's windowed mode: the saved resolution if it fits on the screen
/// with room for the window frame, else the largest offered size that does, else the
/// smallest offered size.
fn windowed_fit(resolution: [u32; 2], max: Option<[u32; 2]>, options: &[[u32; 2]]) -> [u32; 2] {
    let Some(max) = max else {
        return resolution;
    };
    let fits = |size: &[u32; 2]| size[0] <= max[0] && size[1] <= max[1];
    if fits(&resolution) {
        return resolution;
    }
    options
        .iter()
        .copied()
        .find(fits)
        .or_else(|| options.last().copied())
        .unwrap_or(resolution)
}

/// Puts the window into the display mode of `display`, on the monitor it names.
/// Returns `true` when the windowed branch ran (its caller may then re-measure the
/// window frame, which fullscreen hides).
fn apply_window_mode(window: &Window, display: &DisplaySettings, frame: [u32; 2]) -> bool {
    let [width, height] = display.resolution;
    let current = window.current_monitor();
    let monitor = target_monitor(&window_monitors(window), display, current.clone());
    match display.mode {
        DisplayMode::Windowed => {
            window.set_fullscreen(None);
            // Re-measure the restored frame here: the caller's measurement may be from a
            // fullscreen window (no frame), yet the windowed window must fit the screen.
            let frame = match frame_overhead(window) {
                [0, 0] => frame,
                measured => measured,
            };
            let size = windowed_clamp([width, height], monitor.as_ref(), frame);
            let _ = window.request_inner_size(PhysicalSize::new(size[0], size[1]));
            // Only move between monitors; a size change keeps the window where it is.
            if let Some(monitor) = &monitor
                && current.as_ref() != Some(monitor)
            {
                window.set_outer_position(centered_on(monitor, size));
            }
        }
        DisplayMode::Borderless => {
            window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
        }
        DisplayMode::Fullscreen => {
            let video_mode = monitor
                .as_ref()
                .and_then(|m| exclusive_video_mode(m, display.resolution, display.refresh_rate));
            match video_mode {
                Some(video_mode) => {
                    let refresh_millihertz = video_mode.refresh_rate_millihertz();
                    tracing::info!(refresh_millihertz, "exclusive video mode chosen");
                    window.set_fullscreen(Some(Fullscreen::Exclusive(video_mode)));
                }
                None => {
                    tracing::warn!(
                        width,
                        height,
                        "the monitor has no video mode of this size; using borderless fullscreen"
                    );
                    window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
                }
            }
        }
    }
    let (mode, monitor_index) = (display.mode, display.monitor);
    tracing::info!(?mode, monitor_index, width, height, "window mode applied");
    display.mode == DisplayMode::Windowed
}

/// Extra size the window frame (title bar and borders) adds over the client area;
/// zeros while the window is fullscreen or the OS has not laid the window out yet.
fn frame_overhead(window: &Window) -> [u32; 2] {
    let outer = window.outer_size();
    let inner = window.inner_size();
    [
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    ]
}

/// Clamps a windowed client size so the whole window (frame included) stays inside
/// `monitor`. Without a monitor the size is returned unchanged.
fn windowed_clamp(size: [u32; 2], monitor: Option<&MonitorHandle>, frame: [u32; 2]) -> [u32; 2] {
    let Some(monitor) = monitor else {
        return size;
    };
    let screen = monitor.size();
    let max = [
        screen.width.saturating_sub(frame[0]),
        screen.height.saturating_sub(frame[1]),
    ];
    [size[0].min(max[0]), size[1].min(max[1])]
}

/// Largest windowed client area on the window's current monitor: the screen minus the
/// frame. `None` when the monitor is unknown.
fn windowed_size_max(window: &Window, frame: [u32; 2]) -> Option<[u32; 2]> {
    let screen = monitor_size(window)?;
    let max = [
        screen[0].saturating_sub(frame[0]),
        screen[1].saturating_sub(frame[1]),
    ];
    (max[0] > 0 && max[1] > 0).then_some(max)
}

/// `monitor`'s video mode for exclusive fullscreen at `resolution` and `refresh`
/// (millihertz; 0 = the highest available).
fn exclusive_video_mode(
    monitor: &MonitorHandle,
    resolution: [u32; 2],
    refresh: u32,
) -> Option<VideoModeHandle> {
    let modes: Vec<VideoModeHandle> = monitor.video_modes().collect();
    let keys: Vec<VideoModeKey> = modes
        .iter()
        .map(|mode| VideoModeKey {
            size: [mode.size().width, mode.size().height],
            bit_depth: mode.bit_depth(),
            refresh_millihertz: mode.refresh_rate_millihertz(),
        })
        .collect();
    let index = best_video_mode(&keys, resolution, refresh)?;
    modes.into_iter().nth(index)
}

/// The parts of a video mode that matter for choosing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VideoModeKey {
    size: [u32; 2],
    bit_depth: u16,
    refresh_millihertz: u32,
}

/// Index of the mode with exactly `size`: the `refresh` rate (millihertz) when offered,
/// else the highest rate, then the highest bit depth. `None` when no mode has that size.
fn best_video_mode(modes: &[VideoModeKey], size: [u32; 2], refresh: u32) -> Option<usize> {
    modes
        .iter()
        .enumerate()
        .filter(|(_, mode)| mode.size == size)
        .max_by_key(|(_, mode)| {
            let wanted = refresh != 0 && mode.refresh_millihertz == refresh;
            (wanted, mode.refresh_millihertz, mode.bit_depth)
        })
        .map(|(index, _)| index)
}

/// F11 goes from any fullscreen mode to a window, and from a window to exclusive
/// fullscreen.
fn toggled_mode(mode: DisplayMode) -> DisplayMode {
    match mode {
        DisplayMode::Windowed => DisplayMode::Fullscreen,
        DisplayMode::Borderless | DisplayMode::Fullscreen => DisplayMode::Windowed,
    }
}

/// Top-right FPS counter on the overlay (not faded with the menu), with a soft shadow.
/// It is part of the HUD, so it follows the UI Scale setting (`hud_scale`).
fn push_fps_counter(
    labels: &mut Vec<LabelDraw>,
    label: LabelId,
    size: PhysicalSize<u32>,
    hud_scale: f32,
) {
    let scale = (size.width as f32 / 1920.0).min(size.height as f32 / 1080.0) * hud_scale;
    let rect = Rect::new(
        size.width as f32 - (220.0 * scale).round(),
        (12.0 * scale).round(),
        (200.0 * scale).round(),
        (32.0 * scale).round(),
    );
    let font_size = (20.0 * scale).round().max(10.0);
    let offset = (font_size / 16.0).max(1.0).round();
    labels.push(LabelDraw {
        label,
        rect: Rect::new(rect.x + offset, rect.y + offset, rect.w, rect.h),
        font_size,
        color: FPS_SHADOW,
    });
    labels.push(LabelDraw {
        label,
        rect,
        font_size,
        color: FPS_TEXT,
    });
}

fn next_frame_deadline(started: Instant, fps: Option<u32>) -> Option<Instant> {
    fps.filter(|&fps| fps > 0)
        .map(|fps| started + Duration::from_secs_f64(1.0 / f64::from(fps)))
}

/// What a key does in the menus (keyboard navigation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuKey {
    Back,
    Next,
    Previous,
    Activate,
    Decrease,
    Increase,
}

/// Escape goes back; Tab / Shift+Tab and Down / Up move the focus; Enter and Space
/// activate the focused element; Left / Right change a focused value. (Alt+Enter is
/// the fullscreen toggle and is checked first.)
fn menu_key(key: PhysicalKey, shift: bool) -> Option<MenuKey> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    Some(match code {
        KeyCode::Escape => MenuKey::Back,
        KeyCode::Tab if shift => MenuKey::Previous,
        KeyCode::Tab | KeyCode::ArrowDown => MenuKey::Next,
        KeyCode::ArrowUp => MenuKey::Previous,
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => MenuKey::Activate,
        KeyCode::ArrowLeft => MenuKey::Decrease,
        KeyCode::ArrowRight => MenuKey::Increase,
        _ => return None,
    })
}

/// F11, or Alt+Enter (the Windows convention), toggles fullscreen. Key repeats are
/// ignored so holding the keys does not flip between modes.
fn is_fullscreen_toggle(key: PhysicalKey, state: ElementState, repeat: bool, alt: bool) -> bool {
    if state != ElementState::Pressed || repeat {
        return false;
    }
    match key {
        PhysicalKey::Code(KeyCode::F11) => true,
        PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => alt,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESSED: ElementState = ElementState::Pressed;
    const F11: PhysicalKey = PhysicalKey::Code(KeyCode::F11);
    const ENTER: PhysicalKey = PhysicalKey::Code(KeyCode::Enter);

    #[test]
    fn windowed_fit_leaves_room_for_the_frame() {
        let options = [[1920, 1080], [1600, 900], [1366, 768], [1280, 720]];
        // A 1920x1080 screen minus the window frame: full HD does not fit windowed.
        let max = Some([1912, 1034]);
        assert_eq!(windowed_fit([1920, 1080], max, &options), [1600, 900]);
        assert_eq!(windowed_fit([1600, 900], max, &options), [1600, 900]);
        assert_eq!(windowed_fit([1280, 720], max, &options), [1280, 720]);
        // Without a known maximum the saved size is kept.
        assert_eq!(windowed_fit([1920, 1080], None, &options), [1920, 1080]);
        assert_eq!(
            windowed_fit([1280, 720], Some([1280, 720]), &[[1280, 720]]),
            [1280, 720]
        );
    }

    #[test]
    fn toggle_switches_between_window_and_fullscreen() {
        assert_eq!(toggled_mode(DisplayMode::Windowed), DisplayMode::Fullscreen);
        assert_eq!(toggled_mode(DisplayMode::Fullscreen), DisplayMode::Windowed);
    }

    #[test]
    fn exclusive_mode_matches_the_size_with_the_best_refresh_rate() {
        let mode = |w, h, bit_depth, hz: u32| VideoModeKey {
            size: [w, h],
            bit_depth,
            refresh_millihertz: hz * 1000,
        };
        let modes = [
            mode(1920, 1080, 32, 60),
            mode(1920, 1080, 32, 144),
            mode(1920, 1080, 16, 144),
            mode(1280, 720, 32, 165),
        ];
        assert_eq!(best_video_mode(&modes, [1920, 1080], 0), Some(1), "highest");
        assert_eq!(
            best_video_mode(&modes, [1920, 1080], 60_000),
            Some(0),
            "asked for"
        );
        assert_eq!(
            best_video_mode(&modes, [1920, 1080], 75_000),
            Some(1),
            "unavailable rate: highest"
        );
        assert_eq!(best_video_mode(&modes, [1280, 720], 0), Some(3));
        assert_eq!(best_video_mode(&modes, [1600, 900], 0), None);
        assert_eq!(best_video_mode(&[], [1920, 1080], 0), None);
    }

    #[test]
    fn frame_deadline_follows_the_limit() {
        let start = Instant::now();
        assert_eq!(next_frame_deadline(start, None), None);
        assert_eq!(next_frame_deadline(start, Some(0)), None);
        let deadline = next_frame_deadline(start, Some(60)).unwrap();
        let period = deadline - start;
        assert!(period > Duration::from_micros(16_600) && period < Duration::from_micros(16_700));
    }

    #[test]
    fn menu_keys_map_to_navigation() {
        let key = |code| PhysicalKey::Code(code);
        assert_eq!(menu_key(key(KeyCode::Escape), false), Some(MenuKey::Back));
        assert_eq!(menu_key(key(KeyCode::Tab), false), Some(MenuKey::Next));
        assert_eq!(menu_key(key(KeyCode::Tab), true), Some(MenuKey::Previous));
        assert_eq!(
            menu_key(key(KeyCode::ArrowUp), false),
            Some(MenuKey::Previous)
        );
        assert_eq!(
            menu_key(key(KeyCode::Enter), false),
            Some(MenuKey::Activate)
        );
        assert_eq!(
            menu_key(key(KeyCode::Space), false),
            Some(MenuKey::Activate)
        );
        assert_eq!(
            menu_key(key(KeyCode::ArrowLeft), false),
            Some(MenuKey::Decrease)
        );
        assert_eq!(
            menu_key(key(KeyCode::ArrowRight), false),
            Some(MenuKey::Increase)
        );
        assert_eq!(menu_key(key(KeyCode::KeyA), false), None);
    }

    #[test]
    fn f11_press_toggles_with_or_without_alt() {
        assert!(is_fullscreen_toggle(F11, PRESSED, false, false));
        assert!(is_fullscreen_toggle(F11, PRESSED, false, true));
    }

    #[test]
    fn enter_toggles_only_with_alt() {
        assert!(!is_fullscreen_toggle(ENTER, PRESSED, false, false));
        assert!(is_fullscreen_toggle(ENTER, PRESSED, false, true));
        let numpad_enter = PhysicalKey::Code(KeyCode::NumpadEnter);
        assert!(is_fullscreen_toggle(numpad_enter, PRESSED, false, true));
    }

    #[test]
    fn releases_repeats_and_other_keys_are_ignored() {
        assert!(!is_fullscreen_toggle(
            F11,
            ElementState::Released,
            false,
            false
        ));
        assert!(!is_fullscreen_toggle(F11, PRESSED, true, false));
        assert!(!is_fullscreen_toggle(ENTER, PRESSED, true, true));
        let space = PhysicalKey::Code(KeyCode::Space);
        assert!(!is_fullscreen_toggle(space, PRESSED, false, true));
    }
}
