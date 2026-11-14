//! Screen-space UI renderer: textured sprites (stretched or 9-slice) with per-sprite
//! material parameters, GPU text, and one offscreen group with group opacity.
//!
//! Frame order:
//! 1. Group pass (offscreen, cleared to transparent): group sprites, then group labels.
//! 2. Surface pass: background sprites, then the group texture blended with the group
//!    opacity, then the overlay layer (sprites, then labels) on top of everything —
//!    dialogs and the FPS counter, which must not fade or be covered by group text.
//!
//! Applying the opacity once, after the group's children are composed, keeps labels from
//! double-blending over their buttons and lets masked children fade smoothly with their
//! parent instead of popping in. The offscreen target is reused across frames and only
//! recreated when the surface size changes.
//!
//! Alpha convention: premultiplied everywhere (textures, offscreen target, output).
//!
//! Clipping: a run of group sprites and labels can be clipped to a rectangle (scrolling
//! lists). Sprites carry the rectangle per vertex and the fragment shader zeroes what
//! falls outside; labels use glyphon's text bounds.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glyphon::cosmic_text::Align;
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, fontdb,
};

use super::FrameTarget;
use super::texture::create_texture;
use crate::engine::assets::{MaskImage, RgbaImage};
use crate::engine::ui::geometry::{NineSliceLayout, Rect};

/// Behind the background; only visible if the background fails to cover the screen.
const SURFACE_CLEAR: wgpu::Color = wgpu::Color {
    r: 0.010,
    g: 0.008,
    b: 0.016,
    a: 1.0,
};
const LINE_HEIGHT_FACTOR: f32 = 1.25;
/// Sans-serif families to fall back on when no bundled font loads, first installed one
/// wins.
const SANS_CANDIDATES: [&str; 7] = [
    "Roboto",
    "Segoe UI",
    "Noto Sans",
    "Open Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "Arial",
];

/// Handle to a base texture + glow mask pair registered with the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureSetId(usize);

/// Handle to a text label owned by the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelId(usize);

impl LabelId {
    /// A handle for layout tests that never reach the renderer.
    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// How a label is shaped; fixed per label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextStyle {
    pub bold: bool,
    pub align: TextAlign,
}

impl TextStyle {
    pub const CENTERED: Self = Self {
        bold: false,
        align: TextAlign::Center,
    };
}

/// Per-sprite material instance values (the material definition is the sprite shader).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteMaterial {
    pub hover_amount: f32,
    pub press_amount: f32,
    pub disabled_amount: f32,
    pub opacity: f32,
    /// Linear RGB.
    pub glow_color: [f32; 3],
    pub glow_intensity: f32,
    /// Linear RGBA multiplier with straight alpha (applied before glow). White = unchanged;
    /// used for flat-colored sprites and darkened states.
    pub tint: [f32; 4],
    /// Gamma adjustment in linear light: color^(1 / gamma). 1 = unchanged; above 1
    /// brightens mid-tones. Used for the game image's calibration, not for the interface.
    pub gamma: f32,
}

impl SpriteMaterial {
    pub const PLAIN: Self = Self {
        hover_amount: 0.0,
        press_amount: 0.0,
        disabled_amount: 0.0,
        opacity: 1.0,
        glow_color: [0.0; 3],
        glow_intensity: 0.0,
        tint: [1.0; 4],
        gamma: 1.0,
    };

    /// A flat color (sRGB RGBA) for sprites drawn with a white texture.
    pub fn tinted(srgb: [u8; 4]) -> Self {
        Self {
            tint: [
                srgb_to_linear(srgb[0]),
                srgb_to_linear(srgb[1]),
                srgb_to_linear(srgb[2]),
                f32::from(srgb[3]) / 255.0,
            ],
            ..Self::PLAIN
        }
    }
}

fn srgb_to_linear(value: u8) -> f32 {
    let c = f32::from(value) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpriteShape {
    /// Stretches the normalized `uv` sub-rectangle of the texture over `rect`.
    Stretched {
        rect: Rect,
        uv: Rect,
    },
    NineSlice(NineSliceLayout),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite {
    pub textures: TextureSetId,
    pub shape: SpriteShape,
    pub material: SpriteMaterial,
}

/// A label drawn in `rect`: aligned horizontally by its [`TextStyle`], centered
/// vertically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelDraw {
    pub label: LabelId,
    pub rect: Rect,
    /// Physical pixels.
    pub font_size: f32,
    /// sRGB RGBA.
    pub color: [u8; 4],
}

/// Everything the UI draws in one frame, in physical pixels. Reused between frames.
#[derive(Debug, Default)]
pub struct UiFrame {
    pub background: Vec<Sprite>,
    pub group_sprites: Vec<Sprite>,
    pub group_labels: Vec<LabelDraw>,
    pub group_opacity: f32,
    /// Drawn last, unaffected by the group opacity.
    pub overlay_sprites: Vec<Sprite>,
    pub overlay_labels: Vec<LabelDraw>,
    /// Runs of group sprites/labels clipped to a rectangle.
    group_clips: Vec<ClipSpan>,
}

/// A position in the group lists, taken before drawing content that will be clipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupMark {
    sprites: usize,
    labels: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct ClipSpan {
    sprites: Range<usize>,
    labels: Range<usize>,
    rect: Rect,
}

impl UiFrame {
    pub fn clear(&mut self) {
        self.background.clear();
        self.group_sprites.clear();
        self.group_labels.clear();
        self.group_opacity = 0.0;
        self.overlay_sprites.clear();
        self.overlay_labels.clear();
        self.group_clips.clear();
    }

    pub fn group_mark(&self) -> GroupMark {
        GroupMark {
            sprites: self.group_sprites.len(),
            labels: self.group_labels.len(),
        }
    }

    /// Clips the group sprites and labels added since `mark` to `rect` (physical pixels).
    /// Nested clips intersect.
    pub fn clip_group_since(&mut self, mark: GroupMark, rect: Rect) {
        self.group_clips.push(ClipSpan {
            sprites: mark.sprites..self.group_sprites.len(),
            labels: mark.labels..self.group_labels.len(),
            rect,
        });
    }

    fn sprite_clip(&self, index: usize) -> Option<Rect> {
        clip_for(&self.group_clips, |span| span.sprites.contains(&index))
    }
}

/// The intersection of all spans that contain an item; `None` when none does.
fn clip_for(spans: &[ClipSpan], contains: impl Fn(&ClipSpan) -> bool) -> Option<Rect> {
    spans
        .iter()
        .filter(|span| contains(span))
        .map(|span| span.rect)
        .reduce(|a, b| a.intersect(&b))
}

/// Clip rectangle for unclipped sprites: far larger than any surface.
const NO_CLIP: [f32; 4] = [-1.0e9, -1.0e9, 1.0e9, 1.0e9];

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct SpriteVertex {
    position: [f32; 2],
    uv: [f32; 2],
    glow: [f32; 4],
    params: [f32; 4],
    tint: [f32; 4],
    /// min x, min y, max x, max y in physical pixels.
    clip: [f32; 4],
    gamma: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen_size: [f32; 2],
    padding: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binding {
    Set(usize),
    Group,
}

#[derive(Debug, Clone)]
struct DrawCommand {
    binding: Binding,
    indices: Range<u32>,
}

struct GroupTarget {
    size: [u32; 2],
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

pub struct UiRenderer {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    no_glow: wgpu::TextureView,
    texture_sets: Vec<wgpu::BindGroup>,
    group: Option<GroupTarget>,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    vertices: Vec<SpriteVertex>,
    indices: Vec<u32>,
    group_commands: Vec<DrawCommand>,
    surface_commands: Vec<DrawCommand>,
    text: TextLayer,
}

impl UiRenderer {
    /// `format` is the surface format; the offscreen group target uses the same format so
    /// one pipeline and one glyph atlas serve both passes. `fonts` are the game's bundled
    /// font files: the family of the first one that loads becomes the sans-serif font for
    /// all labels (installed fonts remain as fallback for characters it lacks).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        fonts: &[PathBuf],
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui sprite shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/ui_sprite.wgsl").into(),
            ),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui texture set layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui sprite pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let vertex_attributes = wgpu::vertex_attr_array![
            0 => Float32x2,
            1 => Float32x2,
            2 => Float32x4,
            3 => Float32x4,
            4 => Float32x4,
            5 => Float32x4,
            6 => Float32,
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui sprite pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SpriteVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attributes,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui globals"),
            size: std::mem::size_of::<Globals>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui linear sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let no_glow = create_texture(
            device,
            queue,
            "no glow mask",
            [1, 1],
            wgpu::TextureFormat::R8Unorm,
            1,
            &[0],
        )
        .create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            format,
            pipeline,
            globals_buffer,
            globals_bind_group,
            texture_layout,
            sampler,
            no_glow,
            texture_sets: Vec::new(),
            group: None,
            vertex_buffer: create_dynamic_buffer(
                device,
                "ui vertices",
                wgpu::BufferUsages::VERTEX,
                0,
            ),
            index_buffer: create_dynamic_buffer(device, "ui indices", wgpu::BufferUsages::INDEX, 0),
            vertices: Vec::new(),
            indices: Vec::new(),
            group_commands: Vec::new(),
            surface_commands: Vec::new(),
            text: TextLayer::new(device, queue, format, fonts),
        }
    }

    /// Uploads a color texture (straight alpha, premultiplied here) and an optional glow
    /// mask as one texture set.
    pub fn create_texture_set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        base: &RgbaImage,
        glow: Option<&MaskImage>,
    ) -> TextureSetId {
        let mut pixels = base.pixels.clone();
        crate::engine::assets::premultiply_srgb(&mut pixels);
        let base_view = create_texture(
            device,
            queue,
            label,
            [base.width, base.height],
            wgpu::TextureFormat::Rgba8UnormSrgb,
            4,
            &pixels,
        )
        .create_view(&wgpu::TextureViewDescriptor::default());
        let glow_view = glow.map(|mask| {
            create_texture(
                device,
                queue,
                label,
                [mask.width, mask.height],
                wgpu::TextureFormat::R8Unorm,
                1,
                &mask.pixels,
            )
            .create_view(&wgpu::TextureViewDescriptor::default())
        });
        let bind_group = self.texture_bind_group(
            device,
            label,
            &base_view,
            glow_view.as_ref().unwrap_or(&self.no_glow),
        );
        self.texture_sets.push(bind_group);
        TextureSetId(self.texture_sets.len() - 1)
    }

    /// Registers a text label; it is shaped lazily at the size it is drawn with.
    pub fn create_label(&mut self, text: &str, style: TextStyle) -> LabelId {
        self.text.create_label(text, style)
    }

    /// Replaces a label's text; it is re-shaped on next use only if the text changed.
    pub fn set_label_text(&mut self, label: LabelId, text: &str) {
        self.text.set_label_text(label, text);
    }

    /// Encodes the UI passes for one frame into `encoder`.
    pub fn render(
        &mut self,
        target: &FrameTarget<'_>,
        encoder: &mut wgpu::CommandEncoder,
        frame: &UiFrame,
    ) {
        let size = [target.width, target.height];
        self.ensure_group_target(target.device, size);
        let draw_group = frame.group_opacity > 0.0;

        target.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen_size: [size[0] as f32, size[1] as f32],
                padding: [0.0; 2],
            }),
        );

        self.build_geometry(frame, size, draw_group);
        self.upload_geometry(target.device, target.queue);
        if draw_group {
            self.text.prepare(
                target.device,
                target.queue,
                size,
                TextPass::Group,
                &frame.group_labels,
                &frame.group_clips,
            );
        }
        self.text.prepare(
            target.device,
            target.queue,
            size,
            TextPass::Overlay,
            &frame.overlay_labels,
            &[],
        );

        if draw_group && let Some(group) = &self.group {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui group pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &group.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.draw_commands(&mut pass, &self.group_commands);
            self.text.render(TextPass::Group, &mut pass);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui surface pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SURFACE_CLEAR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.draw_commands(&mut pass, &self.surface_commands);
            self.text.render(TextPass::Overlay, &mut pass);
        }

        self.text.trim();
    }

    fn texture_bind_group(
        &self,
        device: &wgpu::Device,
        label: &str,
        base: &wgpu::TextureView,
        glow: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(base),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(glow),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    fn ensure_group_target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.group.as_ref().is_some_and(|group| group.size == size) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui group target"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.texture_bind_group(device, "ui group", &view, &self.no_glow);
        self.group = Some(GroupTarget {
            size,
            view,
            bind_group,
        });
    }

    fn build_geometry(&mut self, frame: &UiFrame, size: [u32; 2], draw_group: bool) {
        self.vertices.clear();
        self.indices.clear();
        self.group_commands.clear();
        self.surface_commands.clear();

        if draw_group {
            for (index, sprite) in frame.group_sprites.iter().enumerate() {
                let clip = frame.sprite_clip(index);
                let range = push_sprite(&mut self.vertices, &mut self.indices, sprite, clip);
                push_command(
                    &mut self.group_commands,
                    Binding::Set(sprite.textures.0),
                    range,
                );
            }
        }
        for sprite in &frame.background {
            let range = push_sprite(&mut self.vertices, &mut self.indices, sprite, None);
            push_command(
                &mut self.surface_commands,
                Binding::Set(sprite.textures.0),
                range,
            );
        }
        if draw_group {
            let composite = Sprite {
                textures: TextureSetId(usize::MAX), // Unused: bound via `Binding::Group`.
                shape: SpriteShape::Stretched {
                    rect: Rect::new(0.0, 0.0, size[0] as f32, size[1] as f32),
                    uv: Rect::new(0.0, 0.0, 1.0, 1.0),
                },
                material: SpriteMaterial {
                    opacity: frame.group_opacity.clamp(0.0, 1.0),
                    ..SpriteMaterial::PLAIN
                },
            };
            let range = push_sprite(&mut self.vertices, &mut self.indices, &composite, None);
            push_command(&mut self.surface_commands, Binding::Group, range);
        }
        for sprite in &frame.overlay_sprites {
            let range = push_sprite(&mut self.vertices, &mut self.indices, sprite, None);
            push_command(
                &mut self.surface_commands,
                Binding::Set(sprite.textures.0),
                range,
            );
        }
    }

    fn upload_geometry(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let vertex_bytes: &[u8] = bytemuck::cast_slice(&self.vertices);
        let index_bytes: &[u8] = bytemuck::cast_slice(&self.indices);
        if vertex_bytes.len() as u64 > self.vertex_buffer.size() {
            self.vertex_buffer = create_dynamic_buffer(
                device,
                "ui vertices",
                wgpu::BufferUsages::VERTEX,
                vertex_bytes.len() as u64,
            );
        }
        if index_bytes.len() as u64 > self.index_buffer.size() {
            self.index_buffer = create_dynamic_buffer(
                device,
                "ui indices",
                wgpu::BufferUsages::INDEX,
                index_bytes.len() as u64,
            );
        }
        if !vertex_bytes.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, vertex_bytes);
            queue.write_buffer(&self.index_buffer, 0, index_bytes);
        }
    }

    fn draw_commands(&self, pass: &mut wgpu::RenderPass<'_>, commands: &[DrawCommand]) {
        if commands.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for command in commands {
            let bind_group = match command.binding {
                Binding::Set(index) => self.texture_sets.get(index),
                Binding::Group => self.group.as_ref().map(|group| &group.bind_group),
            };
            let Some(bind_group) = bind_group else {
                continue;
            };
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw_indexed(command.indices.clone(), 0, 0..1);
        }
    }
}

/// Buffers grow to the next power of two (min 4 KiB) and are reused afterwards.
fn create_dynamic_buffer(
    device: &wgpu::Device,
    label: &str,
    usage: wgpu::BufferUsages,
    min_size: u64,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: min_size.max(4096).next_power_of_two(),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Appends a sprite's vertices/indices and returns its index range.
fn push_sprite(
    vertices: &mut Vec<SpriteVertex>,
    indices: &mut Vec<u32>,
    sprite: &Sprite,
    clip: Option<Rect>,
) -> Range<u32> {
    let clip = clip.map_or(NO_CLIP, |r| [r.x, r.y, r.x + r.w, r.y + r.h]);
    let m = sprite.material;
    let glow = [
        m.glow_color[0],
        m.glow_color[1],
        m.glow_color[2],
        m.glow_intensity,
    ];
    let params = [m.hover_amount, m.opacity, m.press_amount, m.disabled_amount];
    let first_index = indices.len() as u32;
    let base = vertices.len() as u32;
    let vertex = |position: [f32; 2], uv: [f32; 2]| SpriteVertex {
        position,
        uv,
        glow,
        params,
        tint: m.tint,
        clip,
        gamma: m.gamma,
    };

    match sprite.shape {
        SpriteShape::Stretched { rect, uv } => {
            for (column, row) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                vertices.push(vertex(
                    [rect.x + rect.w * column, rect.y + rect.h * row],
                    [uv.x + uv.w * column, uv.y + uv.h * row],
                ));
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
        }
        SpriteShape::NineSlice(layout) => {
            for row in 0..4 {
                for column in 0..4 {
                    let (position, uv) = layout.grid_vertex(column, row);
                    vertices.push(vertex(position, uv));
                }
            }
            for row in 0..3u32 {
                for column in 0..3u32 {
                    let top_left = base + row * 4 + column;
                    let bottom_left = top_left + 4;
                    indices.extend_from_slice(&[
                        top_left,
                        top_left + 1,
                        bottom_left,
                        bottom_left,
                        top_left + 1,
                        bottom_left + 1,
                    ]);
                }
            }
        }
    }
    first_index..indices.len() as u32
}

/// Adds an index range, merging it into the previous command when the binding matches.
fn push_command(commands: &mut Vec<DrawCommand>, binding: Binding, indices: Range<u32>) {
    if let Some(last) = commands.last_mut()
        && last.binding == binding
        && last.indices.end == indices.start
    {
        last.indices.end = indices.end;
        return;
    }
    commands.push(DrawCommand { binding, indices });
}

struct Label {
    buffer: Buffer,
    text: String,
    style: TextStyle,
    /// Font size and width the buffer was last shaped for; `None` = needs shaping.
    shaped_size: Option<[f32; 2]>,
}

/// Which text renderer a label list goes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextPass {
    Group,
    Overlay,
}

/// glyphon-based text. Labels are shaped once per size and cached. The group and the
/// overlay are drawn in different passes, so each has its own renderer; both share one
/// glyph atlas.
struct TextLayer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    group_renderer: TextRenderer,
    overlay_renderer: TextRenderer,
    labels: Vec<Label>,
}

impl TextLayer {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        fonts: &[PathBuf],
    ) -> Self {
        let started = std::time::Instant::now();
        // Installed fonts first: they cover characters the bundled font lacks.
        let mut font_system = FontSystem::new();
        let bundled = load_bundled_fonts(font_system.db_mut(), fonts);
        let family = bundled.first().map(String::as_str).or_else(|| {
            SANS_CANDIDATES.into_iter().find(|candidate| {
                font_system
                    .db()
                    .faces()
                    .any(|face| face.families.iter().any(|(name, _)| name == candidate))
            })
        });
        match family {
            Some(family) => font_system.db_mut().set_sans_serif_family(family),
            None => tracing::warn!("no preferred sans-serif font installed; using the default"),
        }
        tracing::info!(
            sans_serif = family.unwrap_or("default"),
            bundled = !bundled.is_empty(),
            faces = font_system.db().len(),
            elapsed_ms = started.elapsed().as_millis(),
            "fonts loaded"
        );

        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let color_mode = if format.is_srgb() {
            ColorMode::Accurate
        } else {
            ColorMode::Web
        };
        let mut atlas = TextAtlas::with_color_mode(device, queue, &cache, format, color_mode);
        let group_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let overlay_renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            viewport,
            atlas,
            group_renderer,
            overlay_renderer,
            labels: Vec::new(),
        }
    }

    fn create_label(&mut self, text: &str, style: TextStyle) -> LabelId {
        let buffer = Buffer::new(&mut self.font_system, Metrics::new(16.0, 20.0));
        self.labels.push(Label {
            buffer,
            text: text.to_owned(),
            style,
            shaped_size: None,
        });
        LabelId(self.labels.len() - 1)
    }

    fn set_label_text(&mut self, label: LabelId, text: &str) {
        if let Some(label) = self.labels.get_mut(label.0)
            && label.text != text
        {
            text.clone_into(&mut label.text);
            label.shaped_size = None;
        }
    }

    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        pass: TextPass,
        draws: &[LabelDraw],
        clips: &[ClipSpan],
    ) {
        for draw in draws {
            let Some(label) = self.labels.get_mut(draw.label.0) else {
                continue;
            };
            let font_size = draw.font_size.max(1.0);
            if label.shaped_size == Some([font_size, draw.rect.w]) {
                continue;
            }
            let line_height = (font_size * LINE_HEIGHT_FACTOR).ceil();
            label.buffer.set_metrics_and_size(
                Metrics::new(font_size, line_height),
                Some(draw.rect.w),
                Some(line_height),
            );
            let weight = if label.style.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            };
            let align = match label.style.align {
                TextAlign::Left => Align::Left,
                TextAlign::Center => Align::Center,
                TextAlign::Right => Align::Right,
            };
            label.buffer.set_text(
                &label.text,
                &Attrs::new().family(Family::SansSerif).weight(weight),
                Shaping::Advanced,
                Some(align),
            );
            label
                .buffer
                .shape_until_scroll(&mut self.font_system, false);
            label.shaped_size = Some([font_size, draw.rect.w]);
        }

        self.viewport.update(
            queue,
            Resolution {
                width: size[0],
                height: size[1],
            },
        );
        let labels = &self.labels;
        let areas = draws.iter().enumerate().filter_map(|(index, draw)| {
            let label = labels.get(draw.label.0)?;
            let line_height = label.buffer.metrics().line_height;
            let [r, g, b, a] = draw.color;
            let bounds = match clip_for(clips, |span| span.labels.contains(&index)) {
                Some(clip) => draw.rect.intersect(&clip),
                None => draw.rect,
            };
            if bounds.w <= 0.0 || bounds.h <= 0.0 {
                return None;
            }
            Some(TextArea {
                buffer: &label.buffer,
                left: draw.rect.x,
                top: draw.rect.y + ((draw.rect.h - line_height) / 2.0).round(),
                scale: 1.0,
                bounds: TextBounds {
                    left: bounds.x.floor() as i32,
                    top: bounds.y.floor() as i32,
                    right: (bounds.x + bounds.w).ceil() as i32,
                    bottom: (bounds.y + bounds.h).ceil() as i32,
                },
                default_color: Color::rgba(r, g, b, a),
                custom_glyphs: &[],
            })
        });
        let renderer = match pass {
            TextPass::Group => &mut self.group_renderer,
            TextPass::Overlay => &mut self.overlay_renderer,
        };
        if let Err(error) = renderer.prepare(
            device,
            queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash_cache,
        ) {
            tracing::error!("failed to prepare text: {error}");
        }
    }

    fn render(&self, which: TextPass, pass: &mut wgpu::RenderPass<'_>) {
        let renderer = match which {
            TextPass::Group => &self.group_renderer,
            TextPass::Overlay => &self.overlay_renderer,
        };
        if let Err(error) = renderer.render(&self.atlas, &self.viewport, pass) {
            tracing::error!("failed to render text: {error}");
        }
    }

    fn trim(&mut self) {
        self.atlas.trim();
    }
}

/// Loads font files into `db` and returns the families they add, in file order. A file
/// that cannot be read or parsed is logged and skipped, leaving installed fonts in use.
pub fn load_bundled_fonts(db: &mut fontdb::Database, paths: &[PathBuf]) -> Vec<String> {
    let mut families: Vec<String> = Vec::new();
    for path in paths {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::error!(path = %path.display(), %error, "cannot read bundled font");
                continue;
            }
        };
        let ids = db.load_font_source(fontdb::Source::Binary(Arc::new(bytes)));
        if ids.is_empty() {
            tracing::error!(path = %path.display(), "bundled font has no usable faces");
        }
        for id in ids {
            // The first name is the face's (typographic) family, e.g. "Inter".
            let family = db.face(id).and_then(|face| face.families.first());
            if let Some((name, _)) = family
                && !families.contains(name)
            {
                families.push(name.clone());
            }
        }
    }
    families
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ui::geometry::NineSlice;

    #[test]
    fn unreadable_fonts_are_skipped() {
        let mut db = fontdb::Database::new();
        let missing = PathBuf::from("no/such/font.ttf");
        let not_a_font = PathBuf::from(file!());
        assert!(load_bundled_fonts(&mut db, &[missing, not_a_font]).is_empty());
        assert_eq!(db.len(), 0);
    }

    fn sprite(textures: usize, shape: SpriteShape) -> Sprite {
        Sprite {
            textures: TextureSetId(textures),
            shape,
            material: SpriteMaterial::PLAIN,
        }
    }

    #[test]
    fn stretched_sprite_is_one_quad() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        let shape = SpriteShape::Stretched {
            rect: Rect::new(10.0, 20.0, 30.0, 40.0),
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
        };
        let range = push_sprite(&mut vertices, &mut indices, &sprite(0, shape), None);
        assert_eq!(range, 0..6);
        assert_eq!(vertices[0].clip, NO_CLIP);
        assert_eq!(vertices[0].gamma, 1.0);
        assert_eq!(vertices.len(), 4);
        assert_eq!(vertices[3].position, [40.0, 60.0]);
        assert_eq!(vertices[3].uv, [1.0, 1.0]);
    }

    #[test]
    fn nine_slice_sprite_is_nine_quads() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        let borders = NineSlice {
            left: 8.0,
            right: 8.0,
            top: 8.0,
            bottom: 8.0,
        };
        let layout = NineSliceLayout::new(Rect::new(0.0, 0.0, 100.0, 32.0), [64.0, 32.0], borders);
        let range = push_sprite(
            &mut vertices,
            &mut indices,
            &sprite(0, SpriteShape::NineSlice(layout)),
            Some(Rect::new(5.0, 6.0, 10.0, 20.0)),
        );
        assert_eq!(vertices[0].clip, [5.0, 6.0, 15.0, 26.0]);
        assert_eq!(range, 0..54);
        assert_eq!(vertices.len(), 16);
        assert!(indices.iter().all(|&index| index < 16));
    }

    #[test]
    fn sprite_shader_parses_and_validates() {
        let source = include_str!("../../../shaders/ui_sprite.wgsl");
        let module = match naga::front::wgsl::parse_str(source) {
            Ok(module) => module,
            Err(error) => panic!("{}", error.emit_to_string(source)),
        };
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        if let Err(error) = validator.validate(&module) {
            panic!("{}", error.emit_to_string(source));
        }
    }

    #[test]
    fn clips_cover_the_marked_run_and_nest() {
        let mut frame = UiFrame::default();
        let plain = sprite(
            0,
            SpriteShape::Stretched {
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            },
        );
        frame.group_sprites.push(plain);
        let outer = frame.group_mark();
        frame.group_sprites.push(plain);
        let inner = frame.group_mark();
        frame.group_sprites.push(plain);
        frame.clip_group_since(inner, Rect::new(50.0, 0.0, 100.0, 100.0));
        frame.clip_group_since(outer, Rect::new(0.0, 0.0, 100.0, 100.0));
        frame.group_sprites.push(plain);
        assert_eq!(frame.sprite_clip(0), None, "before the mark");
        assert_eq!(
            frame.sprite_clip(1),
            Some(Rect::new(0.0, 0.0, 100.0, 100.0))
        );
        assert_eq!(
            frame.sprite_clip(2),
            Some(Rect::new(50.0, 0.0, 50.0, 100.0))
        );
        assert_eq!(frame.sprite_clip(3), None, "after the span");
        frame.clear();
        assert!(frame.group_clips.is_empty());
    }

    #[test]
    fn adjacent_commands_with_same_binding_merge() {
        let mut commands = Vec::new();
        push_command(&mut commands, Binding::Set(1), 0..6);
        push_command(&mut commands, Binding::Set(1), 6..12);
        push_command(&mut commands, Binding::Set(2), 12..18);
        push_command(&mut commands, Binding::Group, 18..24);
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].indices, 0..12);
    }
}
