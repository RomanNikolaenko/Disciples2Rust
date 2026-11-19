//! GPU device and main-window surface.
//!
//! One `wgpu::Device` is shared by every render path (2D, 3D, UI). This module owns the
//! instance, device, queue and the presentation surface of the main window, and handles
//! resize, zero-sized (minimized) windows, surface loss and device loss.
//!
//! Color space: the surface uses an sRGB format when one is available, so shaders write
//! *linear* color and the hardware encodes to sRGB on store. Clear colors are linear too.
//! If the platform only offers non-sRGB formats a warning is logged; passes that write to
//! the surface would then have to encode manually.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;
use winit::dpi::PhysicalSize;
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

#[derive(Debug, Error)]
pub enum RenderInitError {
    #[error("failed to create the window surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("no compatible GPU adapter: {0}")]
    RequestAdapter(#[from] wgpu::RequestAdapterError),
    #[error("failed to create the GPU device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("the window surface reports no supported texture formats on adapter `{adapter}`")]
    NoSurfaceFormat { adapter: String },
}

/// GPU handles and the surface view for encoding one frame (sizes in physical pixels).
pub struct FrameTarget<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

/// What happened to a frame requested via [`GpuContext::render_frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStatus {
    /// The frame was submitted and presented.
    Presented,
    /// The surface was temporarily unavailable (timeout, outdated or lost surface);
    /// retrying on the next redraw is expected to succeed.
    Skipped,
    /// The window has a zero-sized client area (e.g. minimized). Nothing can be drawn
    /// until the next non-zero resize.
    SurfaceUnavailable,
}

pub struct GpuContext {
    window: Arc<Window>,
    instance: wgpu::Instance,
    /// Kept to answer capability queries (texture format features).
    adapter: wgpu::Adapter,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// False while the client area is zero-sized: a zero-sized surface cannot be configured.
    surface_configured: bool,
    /// Set after a `Suboptimal` frame. Configuring while a `SurfaceTexture` is alive panics,
    /// so the reconfiguration is deferred to the start of the next frame.
    reconfigure_pending: bool,
    /// Set by the device-lost callback (which may run on another thread).
    device_lost: Arc<AtomicBool>,
}

impl GpuContext {
    pub async fn new(
        window: Arc<Window>,
        display: OwnedDisplayHandle,
        vsync: bool,
    ) -> Result<Self, RenderInitError> {
        // `_from_env` lets `WGPU_BACKEND`, `WGPU_DEBUG`, etc. override defaults when debugging.
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display)),
        );
        let surface = instance.create_surface(Arc::clone(&window))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;

        let info = adapter.get_info();
        tracing::info!(
            adapter = %info.name,
            backend = ?info.backend,
            device_type = ?info.device_type,
            driver = %info.driver,
            "GPU adapter selected"
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("main device"),
                required_features: wgpu::Features::empty(),
                // WebGPU default limits, but allow surfaces/textures as large as the adapter
                // supports (large or multi-monitor windows can exceed the default 8192).
                required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await?;

        // The default handler panics; log instead so a single bad call does not kill the game.
        device.on_uncaptured_error(Arc::new(|error: wgpu::Error| {
            tracing::error!("uncaptured wgpu error: {error}");
        }));
        let device_lost = Arc::new(AtomicBool::new(false));
        let lost_flag = Arc::clone(&device_lost);
        device.set_device_lost_callback(move |reason, message| {
            if matches!(reason, wgpu::DeviceLostReason::Destroyed) {
                tracing::debug!("GPU device destroyed: {message}");
            } else {
                tracing::error!("GPU device lost ({reason:?}): {message}");
                lost_flag.store(true, Ordering::Release);
            }
        });

        let caps = surface.get_capabilities(&adapter);
        let format = choose_surface_format(&caps.formats).ok_or_else(|| {
            RenderInitError::NoSurfaceFormat {
                adapter: info.name.clone(),
            }
        })?;
        if !format.is_srgb() {
            tracing::warn!(
                ?format,
                "no sRGB surface format available; output will look too dark"
            );
        }

        let size = window.inner_size();
        let max_dimension = device.limits().max_texture_dimension_2d;
        let extent = surface_extent(size.width, size.height, max_dimension);
        let (width, height) = extent.unwrap_or((1, 1));
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: present_mode(vsync),
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
        };
        tracing::info!(?format, width, height, "surface format chosen");

        let mut context = Self {
            window,
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            surface_configured: false,
            reconfigure_pending: false,
            device_lost,
        };
        if extent.is_some() {
            context.configure_surface();
        }
        Ok(context)
    }

    /// True after the GPU device was lost; the whole context must then be recreated.
    pub fn is_device_lost(&self) -> bool {
        self.device_lost.load(Ordering::Acquire)
    }

    /// Applies a new client-area size in physical pixels. A zero size (minimized window)
    /// leaves the surface unconfigured until the next non-zero resize.
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let max_dimension = self.device.limits().max_texture_dimension_2d;
        match surface_extent(size.width, size.height, max_dimension) {
            Some((width, height)) => {
                self.config.width = width;
                self.config.height = height;
                self.configure_surface();
            }
            None => self.surface_configured = false,
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Format of the presentation surface (sRGB when available, see module docs).
    /// Switches between vsync and uncapped presentation; takes effect on the next frame.
    pub fn set_vsync(&mut self, vsync: bool) {
        let mode = present_mode(vsync);
        if self.config.present_mode == mode {
            return;
        }
        self.config.present_mode = mode;
        if self.surface_configured {
            self.configure_surface();
        }
        tracing::info!(vsync, "present mode changed");
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Whether render targets of every format in `formats` can be multisampled with
    /// `samples` samples (and color formats resolved), on this adapter.
    pub fn supports_multisampling(&self, formats: &[wgpu::TextureFormat], samples: u32) -> bool {
        formats.iter().all(|&format| {
            let flags = self.adapter.get_texture_format_features(format).flags;
            let resolvable = format.is_depth_stencil_format()
                || flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
            flags.sample_count_supported(samples) && resolvable
        })
    }

    /// Acquires the next surface texture, lets `encode` record the frame's passes, then
    /// submits and presents. Nothing is encoded when the surface is unavailable.
    pub fn render_frame(
        &mut self,
        encode: impl FnOnce(&FrameTarget<'_>, &mut wgpu::CommandEncoder),
    ) -> FrameStatus {
        if !self.surface_configured {
            return FrameStatus::SurfaceUnavailable;
        }
        if self.reconfigure_pending {
            self.configure_surface();
        }
        let Some(frame) = self.acquire_frame() else {
            return FrameStatus::Skipped;
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        let target = FrameTarget {
            device: &self.device,
            queue: &self.queue,
            view: &view,
            width: frame.texture.width(),
            height: frame.texture.height(),
        };
        encode(&target, &mut encoder);

        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        FrameStatus::Presented
    }

    /// Acquires the next surface texture, repairing an outdated or lost surface and
    /// retrying once so that a resize does not cost a visible frame.
    fn acquire_frame(&mut self) -> Option<wgpu::SurfaceTexture> {
        for _attempt in 0..2 {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => return Some(frame),
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    self.reconfigure_pending = true;
                    return Some(frame);
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return None;
                }
                wgpu::CurrentSurfaceTexture::Outdated => self.configure_surface(),
                wgpu::CurrentSurfaceTexture::Lost => {
                    tracing::warn!("surface lost; recreating it");
                    self.recreate_surface();
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    tracing::warn!("validation error while acquiring a surface texture");
                    return None;
                }
            }
            if !self.surface_configured {
                return None;
            }
        }
        None
    }

    fn configure_surface(&mut self) {
        self.surface.configure(&self.device, &self.config);
        self.surface_configured = true;
        self.reconfigure_pending = false;
    }

    fn recreate_surface(&mut self) {
        match self.instance.create_surface(Arc::clone(&self.window)) {
            Ok(surface) => {
                self.surface = surface;
                self.configure_surface();
            }
            Err(error) => {
                tracing::error!("failed to recreate the window surface: {error}");
                self.surface_configured = false;
            }
        }
    }
}

/// `AutoVsync` (no tearing) falls back to FIFO and `AutoNoVsync` to FIFO as well when the
/// platform has no uncapped mode, so both are always valid.
fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync {
        wgpu::PresentMode::AutoVsync
    } else {
        wgpu::PresentMode::AutoNoVsync
    }
}

/// Converts a client-area size into a configurable surface extent: `None` for a zero-sized
/// area, otherwise each side clamped to the device's maximum 2D texture dimension.
fn surface_extent(width: u32, height: u32, max_dimension: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    Some((width.min(max_dimension), height.min(max_dimension)))
}

/// Prefers an sRGB format so linear shader output is encoded on store.
fn choose_surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| formats.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sized_area_has_no_extent() {
        assert_eq!(surface_extent(0, 720, 8192), None);
        assert_eq!(surface_extent(1280, 0, 8192), None);
        assert_eq!(surface_extent(0, 0, 8192), None);
    }

    #[test]
    fn extent_is_clamped_per_side() {
        assert_eq!(surface_extent(1280, 720, 8192), Some((1280, 720)));
        assert_eq!(surface_extent(10_000, 720, 8192), Some((8192, 720)));
        assert_eq!(surface_extent(1280, 9_000, 8192), Some((1280, 8192)));
    }

    #[test]
    fn srgb_format_is_preferred() {
        let formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ];
        assert_eq!(
            choose_surface_format(&formats),
            Some(wgpu::TextureFormat::Rgba8UnormSrgb)
        );
    }

    #[test]
    fn falls_back_to_first_format_without_srgb() {
        let formats = [
            wgpu::TextureFormat::Rgb10a2Unorm,
            wgpu::TextureFormat::Bgra8Unorm,
        ];
        assert_eq!(
            choose_surface_format(&formats),
            Some(wgpu::TextureFormat::Rgb10a2Unorm)
        );
        assert_eq!(choose_surface_format(&[]), None);
    }
}
