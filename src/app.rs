//! Application lifecycle: main window, GPU context and per-frame flow.
//!
//! Per-frame order (AGENTS.md): collect input → dispatch UI/input → update game state →
//! fixed simulation → animation → prepare render data → encode/submit. Only frame timing
//! and the final clear/present exist so far.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::{Window, WindowId};

use crate::engine::render::{FrameStatus, GpuContext};
use crate::engine::time::{FrameClock, FrameStats};

const WINDOW_TITLE: &str = "Disciples2Rust";
const INITIAL_WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 720.0);
const MIN_WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(640.0, 360.0);
/// Linear RGB (the surface is sRGB): a near-black, slightly violet background.
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.010,
    g: 0.008,
    b: 0.016,
    a: 1.0,
};
const FRAME_STATS_INTERVAL: Duration = Duration::from_secs(5);

pub struct App {
    // Field order matters for drop order: the GPU context (and its surface) is dropped
    // before the window it presents to.
    gpu: Option<GpuContext>,
    window: Option<Arc<Window>>,
    clock: FrameClock,
    stats: FrameStats,
    /// True while drawing is paused because the window has no drawable area.
    paused: bool,
    fatal_error: Option<anyhow::Error>,
}

impl App {
    pub fn new() -> Self {
        Self {
            gpu: None,
            window: None,
            clock: FrameClock::default(),
            stats: FrameStats::new(FRAME_STATS_INTERVAL),
            paused: false,
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
        // shows an unpainted (white) client area.
        let attributes = Window::default_attributes()
            .with_title(WINDOW_TITLE)
            .with_inner_size(INITIAL_WINDOW_SIZE)
            .with_min_inner_size(MIN_WINDOW_SIZE)
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

        let mut gpu = create_gpu_context(event_loop, &window)?;
        gpu.draw_frame(CLEAR_COLOR);
        window.set_visible(true);

        event_loop.set_control_flow(ControlFlow::Wait);
        window.request_redraw();
        self.window = Some(window);
        self.gpu = Some(gpu);
        Ok(())
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
            // No other GPU resources exist yet, so recovery is a full context rebuild.
            tracing::warn!("recreating the GPU context after device loss");
            self.gpu = None;
            match create_gpu_context(event_loop, &window) {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(error) => return self.fail(event_loop, error),
            }
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        if let Some(frame_time) = self.clock.tick(Instant::now())
            && let Some(summary) = self.stats.record(frame_time)
        {
            tracing::debug!(
                frames = summary.frames,
                avg_ms = summary.average.as_secs_f64() * 1000.0,
                worst_ms = summary.worst.as_secs_f64() * 1000.0,
                "frame timing"
            );
        }

        match gpu.draw_frame(CLEAR_COLOR) {
            FrameStatus::Presented | FrameStatus::Skipped => window.request_redraw(),
            // Stop the redraw loop until a non-zero resize; see `resized`.
            FrameStatus::SurfaceUnavailable => self.paused = true,
        }
    }

    fn resized(&mut self, size: winit::dpi::PhysicalSize<u32>) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.resize(size);
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
