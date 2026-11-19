//! GPU texture creation from CPU pixel data.

use crate::engine::assets::RgbaImage;

/// Creates a sampled 2D texture (single mip level) and uploads `pixels`.
///
/// `bytes_per_pixel` must match `format` (4 for RGBA8 formats, 1 for `R8Unorm`).
/// No mipmaps are generated: UI textures are drawn close to their native size.
pub fn create_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    bytes_per_pixel: u32,
    pixels: &[u8],
) -> wgpu::Texture {
    let extent = wgpu::Extent3d {
        width: size[0].max(1),
        height: size[1].max(1),
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(extent.width * bytes_per_pixel),
            rows_per_image: Some(extent.height),
        },
        extent,
    );
    texture
}

/// Creates a sampled RGBA8 texture from a mip chain (level 0 first, as
/// [`mip_chain`] makes it) and uploads every level.
pub fn create_texture_with_mips(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    levels: &[RgbaImage],
) -> wgpu::Texture {
    let base = levels.first();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: base.map_or(1, |image| image.width.max(1)),
            height: base.map_or(1, |image| image.height.max(1)),
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len().max(1) as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, image) in levels.iter().enumerate() {
        let extent = wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            extent,
        );
    }
    texture
}

/// Creates a sampled RGBA8 2D texture array, one layer per mip chain in `layers` (each
/// level 0 first, as [`mip_chain`] makes it; every chain the first one's sizes), and
/// uploads every level of every layer. Views of it must ask for
/// [`wgpu::TextureViewDimension::D2Array`]: a single layer would default to plain 2D.
pub fn create_texture_array(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    layers: &[Vec<RgbaImage>],
) -> wgpu::Texture {
    let base = layers.first().and_then(|chain| chain.first());
    let levels = layers.first().map_or(1, |chain| chain.len().max(1));
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: base.map_or(1, |image| image.width.max(1)),
            height: base.map_or(1, |image| image.height.max(1)),
            depth_or_array_layers: layers.len().max(1) as u32,
        },
        mip_level_count: levels as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, chain) in layers.iter().enumerate() {
        for (level, image) in chain.iter().take(levels).enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &image.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(image.width * 4),
                    rows_per_image: Some(image.height),
                },
                wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    texture
}

/// `image` and its successive halvings (rounded down, at least 1 pixel) down to 1x1, as
/// a GPU mip chain expects: each level averages 2x2 blocks of the one before. The
/// averaging is done on the stored (sRGB) values, which darkens fine contrast slightly
/// in the small levels; acceptable for model textures.
pub fn mip_chain(image: &RgbaImage) -> Vec<RgbaImage> {
    let mut levels = vec![image.clone()];
    let Some(buffer) = image::RgbaImage::from_raw(image.width, image.height, image.pixels.clone())
    else {
        return levels;
    };
    // `DynamicImage`'s own method is compiled optimized with the image crate; the
    // generic `imageops` functions would build unoptimized here in debug builds.
    let mut current = image::DynamicImage::ImageRgba8(buffer);
    while current.width() > 1 || current.height() > 1 {
        current =
            current.thumbnail_exact((current.width() / 2).max(1), (current.height() / 2).max(1));
        let level = current.to_rgba8();
        levels.push(RgbaImage {
            width: level.width(),
            height: level.height(),
            pixels: level.into_raw(),
        });
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chains_halve_down_to_one_pixel() {
        let image = RgbaImage::solid(8, 2, [200, 100, 50, 255]);
        let sizes: Vec<(u32, u32)> = mip_chain(&image)
            .iter()
            .map(|level| (level.width, level.height))
            .collect();
        assert_eq!(sizes, [(8, 2), (4, 1), (2, 1), (1, 1)]);
        // A flat color stays that color at every level.
        assert!(
            mip_chain(&image)
                .iter()
                .all(|level| level.pixels[..4] == [200, 100, 50, 255])
        );
        assert_eq!(mip_chain(&RgbaImage::solid(1, 1, [0; 4])).len(), 1);
    }
}
