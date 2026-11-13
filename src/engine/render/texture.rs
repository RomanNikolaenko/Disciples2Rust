//! GPU texture creation from CPU pixel data.

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
