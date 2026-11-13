//! Application lifecycle: main window, GPU context and per-frame flow.
//!
//! Per-frame order (AGENTS.md): input events are dispatched to the menu as they arrive;
//! on redraw: frame timing → menu update/animation → build UI frame → encode/submit.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::engine::assets::assets_dir;
use crate::engine::render::ui_renderer::{UiFrame, UiRenderer};
use crate::engine::render::{FrameStatus, GpuContext};
use crate::engine::time::{FrameClock, FrameStats, MAX_ANIMATION_STEP};
use crate::game::menus::main_menu::{MainMenu, MenuEvent};

const WINDOW_TITLE: &str = "Disciples2Rust";
/// Windowed-mode size, used after leaving the default fullscreen (F11 / Alt+Enter).
const INITIAL_WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 720.0);
const MIN_WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(640.0, 360.0);
const FRAME_STATS_INTERVAL: Duration = Duration::from_secs(5);

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
    fatal_error: Option<anyhow::Error>,
}

impl App {
    pub fn new() -> Self {
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
        // shows an unpainted (white) client area. Borderless fullscreen on the current
        // monitor by default: no video-mode switch, fast Alt+Tab.
        let attributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_inner_size(INITIAL_WINDOW_SIZE)
            .with_min_inner_size(MIN_WINDOW_SIZE)
            .with_fullscreen(Some(Fullscreen::Borderless(None)))
            .with_visible(false);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("failed to create the main window")?,
        );
        tracing::info!(
            scale_factor = window.scale_factor(),
            size = ?window.inner_size(),
            "main window created"
        );

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
        let gpu = create_gpu_context(event_loop, window)?;
        let mut renderer = UiRenderer::new(gpu.device(), gpu.queue(), gpu.surface_format());
        let size = window.inner_size();
        let menu = MainMenu::new(
            &gpu,
            &mut renderer,
            &assets_dir(),
            [size.width, size.height],
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
            self.menu.as_ref(),
        ) else {
            return None;
        };
        self.ui_frame.clear();
        menu.build_frame(&mut self.ui_frame);
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

        let frame_time = self.clock.tick(Instant::now());
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

        if let Some(menu) = self.menu.as_mut()
            && let Some(MenuEvent::Exit) = menu.update(dt.as_secs_f32())
        {
            event_loop.exit();
            return;
        }

        match self.draw() {
            Some(FrameStatus::Presented | FrameStatus::Skipped) => window.request_redraw(),
            // Stop the redraw loop until a non-zero resize; see `resized`.
            Some(FrameStatus::SurfaceUnavailable) => self.paused = true,
            None => {}
        }
    }

    fn toggle_fullscreen(&self) {
        let Some(window) = &self.window else {
            return;
        };
        let next = match window.fullscreen() {
            Some(_) => None,
            None => Some(Fullscreen::Borderless(None)),
        };
        tracing::info!(fullscreen = next.is_some(), "toggling fullscreen");
        window.set_fullscreen(next);
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
            WindowEvent::Resized(size) => self.resized(size),
            // winit follows a scale change with `Resized` when the physical size changes.
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                tracing::info!(scale_factor, "window scale factor changed");
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(menu) = self.menu.as_mut() {
                    menu.pointer_moved(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(menu) = self.menu.as_mut() {
                    menu.pointer_left();
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
                if !is_synthetic
                    && is_fullscreen_toggle(
                        event.physical_key,
                        event.state,
                        event.repeat,
                        self.modifiers.alt_key(),
                    )
                {
                    self.toggle_fullscreen();
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
) -> anyhow::Result<GpuContext> {
    pollster::block_on(GpuContext::new(
        Arc::clone(window),
        event_loop.owned_display_handle(),
    ))
    .context("failed to initialize GPU rendering")
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
