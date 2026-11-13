mod app;
mod engine;
mod game;

use anyhow::Context as _;
use tracing_subscriber::EnvFilter;
use winit::event_loop::EventLoop;

/// Used when `RUST_LOG` is not set. wgpu internals are noisy at `info`.
const DEFAULT_LOG_FILTER: &str = "info,wgpu_core=warn,wgpu_hal=warn,naga=warn";

fn main() -> anyhow::Result<()> {
    init_logging();

    let event_loop = EventLoop::new().context("failed to create the event loop")?;
    let mut app = app::App::new();
    event_loop
        .run_app(&mut app)
        .context("event loop terminated with an error")?;

    match app.take_fatal_error() {
        Some(error) => Err(error),
        None => {
            tracing::info!("clean shutdown");
            Ok(())
        }
    }
}

fn init_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
