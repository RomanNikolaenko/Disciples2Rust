//! Loading of image assets from the `assets` directory.
//!
//! Color conventions: color images are sRGB-encoded RGBA8. [`load_rgba`] returns straight
//! (non-premultiplied) alpha; [`premultiply_srgb`] converts to premultiplied alpha for
//! upload, doing the multiplication in linear space. Masks (e.g. glow masks) are linear
//! single-channel data and are loaded with [`load_mask`].

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("failed to read `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode `{path}`: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
}

/// RGBA8 pixels, row-major, sRGB-encoded color.
#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// A single-color image (used as a fallback for missing assets).
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let pixels = rgba.repeat((width * height) as usize);
        Self {
            width,
            height,
            pixels,
        }
    }
}

/// Single-channel (linear) image, row-major.
#[derive(Debug, Clone)]
pub struct MaskImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The `assets` directory: next to the executable for shipped builds, otherwise the
/// current directory (`cargo run` from the repository root).
pub fn assets_dir() -> PathBuf {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("assets")));
    match beside_exe {
        Some(dir) if dir.is_dir() => dir,
        _ => PathBuf::from("assets"),
    }
}

/// Loads a color image with straight alpha.
pub fn load_rgba(path: &Path) -> Result<RgbaImage, AssetError> {
    let image = decode(path)?.into_rgba8();
    Ok(RgbaImage {
        width: image.width(),
        height: image.height(),
        pixels: image.into_raw(),
    })
}

/// Loads a mask (white = full effect). Color is reduced to luma and multiplied by alpha,
/// so "white on opaque black" and "white with an alpha falloff" mean the same thing.
pub fn load_mask(path: &Path) -> Result<MaskImage, AssetError> {
    let image = decode(path)?.into_luma_alpha8();
    let (width, height) = image.dimensions();
    Ok(MaskImage {
        width,
        height,
        pixels: luma_alpha_to_mask(&image.into_raw()),
    })
}

fn luma_alpha_to_mask(luma_alpha: &[u8]) -> Vec<u8> {
    luma_alpha
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[luma, alpha]| ((u16::from(luma) * u16::from(alpha) + 127) / 255) as u8)
        .collect()
}

fn decode(path: &Path) -> Result<image::DynamicImage, AssetError> {
    let bytes = std::fs::read(path).map_err(|source| AssetError::Read {
        path: path.to_owned(),
        source,
    })?;
    image::load_from_memory(&bytes).map_err(|source| AssetError::Decode {
        path: path.to_owned(),
        source,
    })
}

/// Converts sRGB-encoded straight-alpha RGBA8 pixels to premultiplied alpha in place.
///
/// The color is decoded to linear, multiplied by alpha and re-encoded, so the result is
/// correct when sampled from an `*Srgb` texture (which decodes back to linear). Doing the
/// multiplication on encoded values instead would darken semi-transparent edges.
pub fn premultiply_srgb(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        match pixel[3] {
            255 => {}
            0 => pixel[..3].fill(0),
            alpha => {
                let alpha = f32::from(alpha) / 255.0;
                for channel in &mut pixel[..3] {
                    let linear = srgb_to_linear(f32::from(*channel) / 255.0) * alpha;
                    *channel = (linear_to_srgb(linear) * 255.0).round() as u8;
                }
            }
        }
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_and_transparent_pixels() {
        let mut pixels = [200, 100, 50, 255, 200, 100, 50, 0];
        premultiply_srgb(&mut pixels);
        assert_eq!(pixels, [200, 100, 50, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn half_alpha_halves_linear_light() {
        let mut pixels = [255, 255, 255, 128];
        premultiply_srgb(&mut pixels);
        // Linear 1.0 * (128/255) ≈ 0.502 encodes to ≈ 0.736 → 188, not 128.
        assert_eq!(&pixels[..3], &[188, 188, 188]);
        assert_eq!(pixels[3], 128);
    }

    #[test]
    fn transfer_functions_round_trip() {
        for value in [0.0, 0.002, 0.2, 0.5, 0.9, 1.0] {
            let back = linear_to_srgb(srgb_to_linear(value));
            assert!((back - value).abs() < 1e-5, "{value} -> {back}");
        }
    }

    #[test]
    fn mask_strength_is_luma_times_alpha() {
        // Opaque white, opaque black, white at half alpha, transparent white.
        let mask = luma_alpha_to_mask(&[255, 255, 0, 255, 255, 128, 255, 0]);
        assert_eq!(mask, [255, 0, 128, 0]);
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let error = load_rgba(Path::new("does/not/exist.webp")).unwrap_err();
        assert!(matches!(error, AssetError::Read { .. }), "{error}");
    }

    #[test]
    fn solid_image_has_expected_size() {
        let image = RgbaImage::solid(2, 3, [1, 2, 3, 4]);
        assert_eq!(image.pixels.len(), 2 * 3 * 4);
        assert_eq!(&image.pixels[4..8], &[1, 2, 3, 4]);
    }
}
