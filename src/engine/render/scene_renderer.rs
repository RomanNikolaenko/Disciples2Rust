//! The 3D layer: models drawn into an offscreen target that the UI composites as one
//! image (see `UiRenderer::create_view_texture`).
//!
//! Why offscreen: the 3D world needs a depth buffer and multisampling, and the UI passes
//! have neither. Drawing the world into its own target, cleared to transparent, gives a
//! premultiplied image that the UI places like any sprite, under the HUD and dialogs.
//!
//! Two kinds of draws share the target and its depth buffer:
//! - **figures**: skinned, animated models, one draw each, with their own block of a
//!   shared uniform buffer (matrices, light, joints) bound with a dynamic offset;
//! - **scenery**: static models (baked into their rest pose when uploaded, see
//!   [`SceneRenderer::create_static_model`]) drawn instanced: one draw per model and
//!   part for all its copies, each copy a matrix and a tint in an instance buffer. The
//!   scenery is lit in the same frame as the figures and fades to black at the map's
//!   edges.
//!
//! The ground is scenery too, drawn by its own pipeline when ground detail is set (see
//! [`GroundDetail`]): photographed surfaces (color and normal maps in texture arrays),
//! tiled in world space without visible repeats, mixed by weight maps over the map and
//! lit by the sun through their normals. Without detail it shows its painted texture.
//!
//! Each model has vertex and index buffers and, per part, the bind group of its texture
//! (mipmapped, sRGB). Textures are shared by key across models (every tree uses the same
//! bark), and parts without one use a 1x1 white texture.
//!
//! Scenery that would cost too many triangles (trees: thousands each) can be baked
//! into sprites once ([`SceneRenderer::bake_atlas`]): rendered by the same pipeline,
//! light and multisampling into the cells of an atlas, then drawn as camera-facing
//! quads. The camera never turns, so a sprite looks just like its model.
//!
//! The target starts at 1x1 and then matches the surface size (reallocated only when
//! that changes), so a frame without models costs no full-screen buffers. Uniform and
//! instance buffers grow when a frame needs more and are reused after.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};
use wgpu::util::DeviceExt as _;

use super::FrameTarget;
use super::texture::{create_texture_array, create_texture_with_mips, mip_chain};
use crate::engine::assets::RgbaImage;
use crate::engine::model::{MAX_JOINTS, ModelTexture, SkinnedModel, SkinnedVertex};

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Samples per pixel when the adapter supports it for the target formats; otherwise 1.
pub const PREFERRED_SAMPLES: u32 = 4;

/// Handle to a model uploaded with [`SceneRenderer::create_model`] or
/// [`SceneRenderer::create_static_model`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelId(usize);

/// Lighting shared by every draw of a frame, in the lighting frame (+Z towards the
/// viewer) the draws' matrices lead to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLight {
    /// Towards the light; normalized when used.
    pub direction: Vec3,
    /// 0..1: how lit the side away from the light stays.
    pub ambient: f32,
    /// Strength of the rim light on silhouettes.
    pub rim: f32,
}

impl Default for SceneLight {
    fn default() -> Self {
        Self {
            direction: Vec3::new(-0.5, 0.7, 0.5),
            ambient: 0.35,
            rim: 0.08,
        }
    }
}

/// The camera and map the scenery is drawn for. World space: x and z across the map (one
/// unit per tile), +Y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneView {
    pub clip_from_world: Mat4,
    /// World → lighting frame (a rotation).
    pub light_from_world: Mat4,
    /// The map's size along x and z; zero for no edge fade.
    pub map_size: Vec2,
    /// World units over which scenery fades to black towards the map's edges.
    pub edge_fade: f32,
    /// A square frame the ground draws onto itself, following its relief (the
    /// hovered tile); only the ground with detail draws it.
    pub ground_square: Option<GroundSquare>,
}

impl Default for SceneView {
    fn default() -> Self {
        Self {
            clip_from_world: Mat4::IDENTITY,
            light_from_world: Mat4::IDENTITY,
            map_size: Vec2::ZERO,
            edge_fade: 0.0,
            ground_square: None,
        }
    }
}

/// A square frame on the ground, axis-aligned in world x and z.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundSquare {
    /// World (x, z) of its corner with the smallest coordinates.
    pub corner: Vec2,
    /// Side, world units.
    pub side: f32,
    /// Width of its lines, world units.
    pub width: f32,
    /// Linear color, shown unlit.
    pub color: [f32; 3],
}

/// One placed copy of a static model.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct PropInstance {
    pub world_from_model: [[f32; 4]; 4],
    /// rgb: color multiplier; a: 0 lit, 1 shown in its own colors (ground art that
    /// already has its light painted in).
    pub tint: [f32; 4],
}

impl PropInstance {
    pub fn new(world_from_model: Mat4, tint: [f32; 4]) -> Self {
        Self {
            world_from_model: world_from_model.to_cols_array_2d(),
            tint,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SceneDraw {
    model: ModelId,
    clip_from_model: Mat4,
    light_from_model: Mat4,
    /// Into [`SceneFrame::joints`].
    joints: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
struct PropBatch {
    model: ModelId,
    /// Into [`SceneFrame::instances`].
    instances: Range<u32>,
    /// Drawn with the ground's detail.
    ground: bool,
}

/// Most surface layers the ground mixes: two RGBA weight maps of four.
pub const MAX_GROUND_LAYERS: usize = 8;

/// One photographed surface of the ground.
#[derive(Debug, Clone, Copy)]
pub struct GroundLayer<'a> {
    /// Albedo, sRGB.
    pub color: &'a RgbaImage,
    /// Tangent-space normal, OpenGL convention (green up the image), stored 0.5·n + 0.5;
    /// alpha: a height (what sticks out wins where layers meet).
    pub normal: &'a RgbaImage,
    /// World units one repeat covers.
    pub size: f32,
    /// Light its lowest places give off (linear color; zero for none), unlit by the sun.
    pub glow: [f32; 3],
    /// 0..1: up to which height those places glow.
    pub glow_depth: f32,
}

/// The ground's surfaces and where each of them lies (see `shaders/ground.wgsl`).
#[derive(Debug, Clone, Copy)]
pub struct GroundDetail<'a> {
    /// 1..=[`MAX_GROUND_LAYERS`] layers, every image the size of the first layer's color.
    pub layers: &'a [GroundLayer<'a>],
    /// Linear weights over the map (texel (0, 0) at its north-west corner): layers 0..4
    /// in the first image's r, g, b, a, layers 4..8 in the second's (the same size).
    /// Where they sum to zero (water) the ground shows its own painted texture.
    pub weights: [&'a RgbaImage; 2],
    /// Red: a brightness multiplier over the ground's own texture area (contact
    /// shadows), mapped like that texture.
    pub shade: &'a RgbaImage,
    /// Multiplies the lit surfaces' colors.
    pub brightness: f32,
    /// Scales the slopes of the normal maps (1: as photographed).
    pub relief: f32,
}

/// Group 2 of the ground pipeline's uniforms; mirrors `Detail` in `shaders/ground.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct DetailUniforms {
    /// World units per repeat, four layers per entry.
    sizes: [[f32; 4]; 2],
    /// x: layer count; y: brightness; z: relief.
    params: [f32; 4],
    /// Each layer's glow: rgb, and a: the height up to which it glows.
    glows: [[f32; 4]; MAX_GROUND_LAYERS],
}

impl DetailUniforms {
    fn new(detail: &GroundDetail<'_>) -> Self {
        let mut sizes = [[1.0; 4]; 2];
        let mut glows = [[0.0; 4]; MAX_GROUND_LAYERS];
        for (index, layer) in detail.layers.iter().take(MAX_GROUND_LAYERS).enumerate() {
            sizes[index / 4][index % 4] = layer.size.max(1e-3);
            let [r, g, b] = layer.glow.map(|c| c.max(0.0));
            glows[index] = [r, g, b, layer.glow_depth.clamp(0.01, 1.0)];
        }
        Self {
            sizes,
            glows,
            params: [
                detail.layers.len().min(MAX_GROUND_LAYERS) as f32,
                detail.brightness.max(0.0),
                detail.relief.max(0.0),
                0.0,
            ],
        }
    }
}

/// Why `detail` cannot be uploaded, if it cannot.
fn check_ground_detail(detail: &GroundDetail<'_>) -> Result<(), String> {
    let Some(first) = detail.layers.first() else {
        return Err("no ground layers".into());
    };
    if detail.layers.len() > MAX_GROUND_LAYERS {
        return Err(format!(
            "{} ground layers, at most {MAX_GROUND_LAYERS}",
            detail.layers.len()
        ));
    }
    let side = (first.color.width, first.color.height);
    for (index, layer) in detail.layers.iter().enumerate() {
        for image in [layer.color, layer.normal] {
            if (image.width, image.height) != side {
                return Err(format!(
                    "ground layer {index} is {}x{}, the first {}x{}",
                    image.width, image.height, side.0, side.1
                ));
            }
        }
    }
    let [low, high] = detail.weights;
    if (low.width, low.height) != (high.width, high.height) {
        return Err("the two ground weight maps differ in size".into());
    }
    Ok(())
}

/// Everything the 3D layer draws in one frame. Reused between frames.
#[derive(Debug, Default)]
pub struct SceneFrame {
    pub light: SceneLight,
    pub view: SceneView,
    draws: Vec<SceneDraw>,
    joints: Vec<Mat4>,
    batches: Vec<PropBatch>,
    instances: Vec<PropInstance>,
}

impl SceneFrame {
    pub fn clear(&mut self) {
        self.draws.clear();
        self.joints.clear();
        self.batches.clear();
        self.instances.clear();
        self.light = SceneLight::default();
        self.view = SceneView::default();
    }

    pub fn is_empty(&self) -> bool {
        self.draws.is_empty() && self.batches.is_empty()
    }

    /// Queues a skinned `model` posed by `joints` (model space; at most [`MAX_JOINTS`]
    /// are used). `clip_from_model` places it on screen (clip space of the scene
    /// target); `light_from_model` turns its normals into the lighting frame.
    pub fn push(
        &mut self,
        model: ModelId,
        clip_from_model: Mat4,
        light_from_model: Mat4,
        joints: &[Mat4],
    ) {
        let start = self.joints.len();
        self.joints
            .extend_from_slice(&joints[..joints.len().min(MAX_JOINTS)]);
        self.draws.push(SceneDraw {
            model,
            clip_from_model,
            light_from_model,
            joints: start..self.joints.len(),
        });
    }

    /// Queues copies of a static model (one uploaded with
    /// [`SceneRenderer::create_static_model`]), drawn with [`Self::view`].
    pub fn push_props(
        &mut self,
        model: ModelId,
        instances: impl IntoIterator<Item = PropInstance>,
    ) {
        let start = self.instances.len() as u32;
        self.instances.extend(instances);
        let end = self.instances.len() as u32;
        if end == start {
            return;
        }
        match self.batches.last_mut() {
            Some(batch)
                if batch.model == model && batch.instances.end == start && !batch.ground =>
            {
                batch.instances.end = end;
            }
            _ => self.batches.push(PropBatch {
                model,
                instances: start..end,
                ground: false,
            }),
        }
    }

    /// Queues the ground (a static model) with the detail set by
    /// [`SceneRenderer::set_ground_detail`]; without detail it is drawn like scenery.
    pub fn push_ground(&mut self, model: ModelId, instance: PropInstance) {
        let start = self.instances.len() as u32;
        self.instances.push(instance);
        self.batches.push(PropBatch {
            model,
            instances: start..start + 1,
            ground: true,
        });
    }
}

/// One view of a static model to bake into a cell of an atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasCell {
    /// Uploaded with [`SceneRenderer::create_static_model`].
    pub model: ModelId,
    /// Model space → the cell's view frame: x and y from -1 to 1 fill the cell (y up),
    /// z towards the viewer within -2..2. It also turns the normals into the lighting
    /// frame, so it should rotate and scale uniformly only.
    pub cell_from_model: Mat4,
}

/// Depth of a baked cell: view-frame z in -2..2 (towards the viewer) to 1..0. Cells
/// frame their model by width and height; its depth may reach past 1.
const CELL_DEPTH: Mat4 = Mat4::from_cols(
    glam::Vec4::X,
    glam::Vec4::Y,
    glam::Vec4::new(0.0, 0.0, -0.25, 0.0),
    glam::Vec4::new(0.0, 0.0, 0.5, 1.0),
);

/// One figure draw's uniform block; mirrors `Draw` in `shaders/skinned.wgsl` (no
/// implicit padding: every member is a multiple of 16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct DrawUniforms {
    clip_from_model: [[f32; 4]; 4],
    light_from_model: [[f32; 4]; 4],
    light: [f32; 4],
    surface: [f32; 4],
    joints: [[[f32; 4]; 4]; MAX_JOINTS],
}

impl DrawUniforms {
    fn new(draw: &SceneDraw, light: &SceneLight, joints: &[Mat4]) -> Self {
        let mut uniforms = Self {
            clip_from_model: draw.clip_from_model.to_cols_array_2d(),
            light_from_model: draw.light_from_model.to_cols_array_2d(),
            light: light_vector(light),
            // No per-draw color change yet: white keeps the materials' colors.
            surface: [1.0, 1.0, 1.0, light.rim],
            joints: [Mat4::IDENTITY.to_cols_array_2d(); MAX_JOINTS],
        };
        for (slot, joint) in uniforms.joints.iter_mut().zip(joints) {
            *slot = joint.to_cols_array_2d();
        }
        uniforms
    }
}

/// The scenery's per-frame uniforms; mirrors `Frame` in `shaders/props.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PropsUniforms {
    clip_from_world: [[f32; 4]; 4],
    light_from_world: [[f32; 4]; 4],
    light: [f32; 4],
    surface: [f32; 4],
    map: [f32; 4],
    /// The ground square: corner x, z, side, line width; and its color, a 1 when shown.
    square: [f32; 4],
    square_color: [f32; 4],
}

impl PropsUniforms {
    fn new(view: &SceneView, light: &SceneLight) -> Self {
        Self {
            clip_from_world: view.clip_from_world.to_cols_array_2d(),
            light_from_world: view.light_from_world.to_cols_array_2d(),
            light: light_vector(light),
            surface: [1.0, 1.0, 1.0, light.rim],
            map: [view.map_size.x, view.map_size.y, view.edge_fade, 0.0],
            square: view.ground_square.map_or([0.0; 4], |square| {
                [square.corner.x, square.corner.y, square.side, square.width]
            }),
            square_color: view.ground_square.map_or([0.0; 4], |square| {
                let [r, g, b] = square.color;
                [r, g, b, 1.0]
            }),
        }
    }
}

/// Unit vector towards the light, and the ambient level.
fn light_vector(light: &SceneLight) -> [f32; 4] {
    let direction = light.direction.normalize_or(Vec3::Z);
    [direction.x, direction.y, direction.z, light.ambient]
}

struct GpuPart {
    indices: Range<u32>,
    texture: Rc<wgpu::BindGroup>,
}

struct GpuModel {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    parts: Vec<GpuPart>,
    /// Baked into its rest pose: drawn as scenery, not as a figure.
    baked: bool,
}

struct SceneTarget {
    size: [u32; 2],
    /// Multisampled color, resolved into `resolve`; `None` without multisampling.
    multisampled: Option<wgpu::TextureView>,
    resolve: wgpu::TextureView,
    resolve_texture: wgpu::Texture,
    depth: wgpu::TextureView,
}

pub struct SceneRenderer {
    color_format: wgpu::TextureFormat,
    samples: u32,
    skinned_pipeline: wgpu::RenderPipeline,
    props_pipeline: wgpu::RenderPipeline,
    ground_pipeline: wgpu::RenderPipeline,
    detail_layout: wgpu::BindGroupLayout,
    /// The ground's detail, once set.
    detail: Option<wgpu::BindGroup>,
    uniform_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    /// Bytes per figure draw in the uniform buffer (aligned for dynamic offsets).
    stride: u64,
    /// Figure draws the uniform buffer holds.
    capacity: usize,
    staging: Vec<u8>,
    props_uniforms: wgpu::Buffer,
    props_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    /// Instances the instance buffer holds.
    instance_capacity: usize,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// For parts without a texture.
    white: Rc<wgpu::BindGroup>,
    /// Textures by key; an entry nobody else holds is dropped when a model is released.
    textures: HashMap<String, Rc<wgpu::BindGroup>>,
    models: Vec<Option<GpuModel>>,
    target: SceneTarget,
}

impl SceneRenderer {
    /// `color_format` is the composited target's format (the surface's, so the UI can
    /// sample it like its other textures); `samples` is 1 or a count the adapter supports
    /// for `color_format` and [`DEPTH_FORMAT`].
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let block_size = std::mem::size_of::<DrawUniforms>() as u64;
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skinned draw layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(block_size),
                },
                count: None,
            }],
        });
        let props_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scenery frame layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<PropsUniforms>() as u64
                    ),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let vertex_stride = std::mem::size_of::<SkinnedVertex>() as wgpu::BufferAddress;
        let skinned_attributes = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32x3,
            2 => Float32x3,
            3 => Float32x2,
            4 => Float32,
            5 => Uint8x4,
            6 => Float32x4,
        ];
        // The scenery reads the same vertices without their joints, plus one instance
        // (four matrix columns and a tint) per copy.
        let static_attributes = wgpu::vertex_attr_array![
            0 => Float32x3,
            1 => Float32x3,
            2 => Float32x3,
            3 => Float32x2,
            4 => Float32,
        ];
        let instance_attributes = wgpu::vertex_attr_array![
            5 => Float32x4,
            6 => Float32x4,
            7 => Float32x4,
            8 => Float32x4,
            9 => Float32x4,
        ];
        let skinned_pipeline = create_pipeline(
            device,
            "skinned",
            include_str!("../../../shaders/skinned.wgsl"),
            &[&uniform_layout, &texture_layout],
            &[wgpu::VertexBufferLayout {
                array_stride: vertex_stride,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &skinned_attributes,
            }],
            color_format,
            samples,
        );
        let props_pipeline = create_pipeline(
            device,
            "scenery",
            include_str!("../../../shaders/props.wgsl"),
            &[&props_layout, &texture_layout],
            &[
                wgpu::VertexBufferLayout {
                    array_stride: vertex_stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &static_attributes,
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PropInstance>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instance_attributes,
                },
            ],
            color_format,
            samples,
        );

        let detail_layout = create_detail_layout(device);
        let ground_pipeline = create_pipeline(
            device,
            "ground",
            include_str!("../../../shaders/ground.wgsl"),
            &[&props_layout, &texture_layout, &detail_layout],
            &[
                wgpu::VertexBufferLayout {
                    array_stride: vertex_stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &static_attributes,
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PropInstance>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instance_attributes,
                },
            ],
            color_format,
            samples,
        );

        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let stride = block_size.div_ceil(alignment) * alignment;
        let capacity = 1;
        let (uniform_buffer, uniform_bind_group) =
            create_uniforms(device, &uniform_layout, stride, capacity);
        let props_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scenery frame uniforms"),
            size: std::mem::size_of::<PropsUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let props_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scenery frame uniforms"),
            layout: &props_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: props_uniforms.as_entire_binding(),
            }],
        });
        let instance_capacity = 256;
        let instance_buffer = create_instance_buffer(device, instance_capacity);
        let target = SceneTarget::new(device, color_format, samples, [1, 1]);
        // Texture coordinates outside 0..1 repeat, as FBX textures wrap by default.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("model texture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let white = Rc::new(texture_bind_group(
            device,
            queue,
            &texture_layout,
            &sampler,
            "white model texture",
            &RgbaImage::solid(1, 1, [255; 4]),
        ));
        tracing::info!(samples, ?color_format, "3D layer ready");
        Self {
            color_format,
            samples,
            skinned_pipeline,
            props_pipeline,
            ground_pipeline,
            detail_layout,
            detail: None,
            uniform_layout,
            uniform_buffer,
            uniform_bind_group,
            stride,
            capacity,
            staging: Vec::new(),
            props_uniforms,
            props_bind_group,
            instance_buffer,
            instance_capacity,
            texture_layout,
            sampler,
            white,
            textures: HashMap::new(),
            models: Vec::new(),
            target,
        }
    }

    /// Uploads a skinned model (a figure); the slot of a released model is reused.
    pub fn create_model(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &SkinnedModel,
    ) -> ModelId {
        self.upload(device, queue, model, model.vertices(), false)
    }

    /// Uploads a model baked into its rest pose, to draw as scenery with
    /// [`SceneFrame::push_props`].
    pub fn create_static_model(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &SkinnedModel,
    ) -> ModelId {
        self.upload(device, queue, model, &model.rest_pose_vertices(), true)
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &SkinnedModel,
        vertices: &[SkinnedVertex],
        baked: bool,
    ) -> ModelId {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model indices"),
            contents: bytemuck::cast_slice(model.indices()),
            usage: wgpu::BufferUsages::INDEX,
        });
        let parts = model
            .parts()
            .iter()
            .map(|part| GpuPart {
                indices: part.indices.clone(),
                texture: match part.texture.and_then(|index| model.textures().get(index)) {
                    Some(texture) => self.texture(device, queue, texture),
                    None => Rc::clone(&self.white),
                },
            })
            .collect();
        let gpu = GpuModel {
            vertices: vertex_buffer,
            indices: index_buffer,
            parts,
            baked,
        };
        match self.models.iter().position(Option::is_none) {
            Some(slot) => {
                self.models[slot] = Some(gpu);
                ModelId(slot)
            }
            None => {
                self.models.push(Some(gpu));
                ModelId(self.models.len() - 1)
            }
        }
    }

    /// The bind group of `texture`, shared with every model that uses the same key.
    fn texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &ModelTexture,
    ) -> Rc<wgpu::BindGroup> {
        if let Some(existing) = self.textures.get(&texture.key) {
            return Rc::clone(existing);
        }
        let bind_group = Rc::new(texture_bind_group(
            device,
            queue,
            &self.texture_layout,
            &self.sampler,
            &texture.key,
            &texture.image,
        ));
        self.textures
            .insert(texture.key.clone(), Rc::clone(&bind_group));
        bind_group
    }

    /// Uploads the ground's detail (replacing any before); the ground drawn with
    /// [`SceneFrame::push_ground`] uses it from then on. Nothing changes if `detail` is
    /// unusable (see [`GroundDetail`]).
    pub fn set_ground_detail(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        detail: &GroundDetail<'_>,
    ) -> Result<(), String> {
        check_ground_detail(detail)?;
        let started = std::time::Instant::now();
        // Mip chains for every color and normal map, built side by side.
        let (colors, normals): (Vec<Vec<RgbaImage>>, Vec<Vec<RgbaImage>>) =
            std::thread::scope(|scope| {
                let workers: Vec<_> = detail
                    .layers
                    .iter()
                    .map(|layer| {
                        scope.spawn(move || (mip_chain(layer.color), mip_chain(layer.normal)))
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|worker| worker.join().map_err(|_| "a mip chain failed".to_string()))
                    .collect::<Result<Vec<_>, String>>()
            })?
            .into_iter()
            .unzip();
        let array_view = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        // Colors are sRGB; normals, heights and weights are data (linear).
        let colors = array_view(&create_texture_array(
            device,
            queue,
            "ground colors",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &colors,
        ));
        let normals = array_view(&create_texture_array(
            device,
            queue,
            "ground normals",
            wgpu::TextureFormat::Rgba8Unorm,
            &normals,
        ));
        let weights = array_view(&create_texture_array(
            device,
            queue,
            "ground weights",
            wgpu::TextureFormat::Rgba8Unorm,
            &detail.weights.map(|image| vec![image.clone()]),
        ));
        let red: Vec<u8> = detail.shade.pixels.iter().step_by(4).copied().collect();
        let shade = super::texture::create_texture(
            device,
            queue,
            "ground shade",
            [detail.shade.width, detail.shade.height],
            wgpu::TextureFormat::R8Unorm,
            1,
            &red,
        )
        .create_view(&wgpu::TextureViewDescriptor::default());
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ground detail uniforms"),
            contents: bytemuck::bytes_of(&DetailUniforms::new(detail)),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let map_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ground map sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // The camera sees the ground at a slant: anisotropic filtering keeps it sharp
        // into the distance (adapters without it ignore the clamp).
        let surface_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ground surface sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        self.detail = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ground detail"),
            layout: &self.detail_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&weights),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&map_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&colors),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&normals),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&surface_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&shade),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: buffer.as_entire_binding(),
                },
            ],
        }));
        tracing::info!(
            layers = detail.layers.len(),
            ms = started.elapsed().as_millis(),
            "ground surfaces uploaded"
        );
        Ok(())
    }

    /// Renders `cells` into a `size` x `size` atlas of `columns` x `columns` square
    /// cells (row by row from the top left), lit by `light` in each cell's view frame,
    /// and reads it back: RGBA8, sRGB, premultiplied alpha (transparent around the
    /// models). Blocks until the GPU is done; meant for load time.
    pub fn bake_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        columns: u32,
        light: &SceneLight,
        cells: &[AtlasCell],
    ) -> Result<RgbaImage, String> {
        let columns = columns.max(1);
        if cells.len() > (columns * columns) as usize {
            return Err(format!(
                "{} cells do not fit {columns}x{columns}",
                cells.len()
            ));
        }
        let cell = (size / columns).max(1) as f32;
        let target = SceneTarget::new(device, self.color_format, self.samples, [size, size]);
        let uniforms = PropsUniforms::new(
            &SceneView {
                clip_from_world: CELL_DEPTH,
                ..SceneView::default()
            },
            light,
        );
        queue.write_buffer(&self.props_uniforms, 0, bytemuck::bytes_of(&uniforms));
        let instances: Vec<PropInstance> = cells
            .iter()
            .map(|cell| PropInstance::new(cell.cell_from_model, [1.0, 1.0, 1.0, 0.0]))
            .collect();
        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instance_buffer = create_instance_buffer(device, self.instance_capacity);
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("atlas bake"),
        });
        {
            let (view, resolve_target) = match &target.multisampled {
                Some(multisampled) => (multisampled, Some(&target.resolve)),
                None => (&target.resolve, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("atlas bake pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.props_pipeline);
            pass.set_bind_group(0, &self.props_bind_group, &[]);
            pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            for (index, atlas_cell) in cells.iter().enumerate() {
                let Some(Some(model)) = self.models.get(atlas_cell.model.0) else {
                    continue;
                };
                let (column, row) = (index as u32 % columns, index as u32 / columns);
                pass.set_viewport(
                    column as f32 * cell,
                    row as f32 * cell,
                    cell,
                    cell,
                    0.0,
                    1.0,
                );
                pass.set_vertex_buffer(0, model.vertices.slice(..));
                pass.set_index_buffer(model.indices.slice(..), wgpu::IndexFormat::Uint32);
                let instance = index as u32..index as u32 + 1;
                for part in &model.parts {
                    pass.set_bind_group(1, part.texture.as_ref(), &[]);
                    pass.draw_indexed(part.indices.clone(), 0, instance.clone());
                }
            }
        }
        let image = read_back(device, queue, encoder, &target, size)?;
        Ok(match self.color_format {
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
                let mut image = image;
                for pixel in image.pixels.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                image
            }
            _ => image,
        })
    }

    /// Drops the ground's detail.
    pub fn clear_ground_detail(&mut self) {
        self.detail = None;
    }

    /// Frees a model's GPU memory (and textures no other model uses); drawing it
    /// afterwards draws nothing.
    pub fn release_model(&mut self, model: ModelId) {
        if let Some(slot) = self.models.get_mut(model.0) {
            *slot = None;
        }
        self.textures
            .retain(|_, bind_group| Rc::strong_count(bind_group) > 1);
    }

    /// Makes the target `size` (physical pixels). True when it was (re)created, so views
    /// of it held elsewhere must be refreshed.
    pub fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) -> bool {
        let size = size.map(|side| side.max(1));
        if self.target.size == size {
            return false;
        }
        self.target = SceneTarget::new(device, self.color_format, self.samples, size);
        true
    }

    /// The composited image (single-sampled, premultiplied alpha).
    pub fn target_view(&self) -> &wgpu::TextureView {
        &self.target.resolve
    }

    /// Clears the target and draws `frame` into it: the scenery, then the figures. Call
    /// [`Self::ensure_target`] first with the surface size.
    pub fn render(
        &mut self,
        target: &FrameTarget<'_>,
        encoder: &mut wgpu::CommandEncoder,
        frame: &SceneFrame,
    ) {
        self.upload_draws(target.device, target.queue, frame);
        self.upload_props(target.device, target.queue, frame);
        let scene = &self.target;
        let (view, resolve_target) = match &scene.multisampled {
            Some(multisampled) => (multisampled, Some(&scene.resolve)),
            None => (&scene.resolve, None),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3D layer pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    // Only the resolved image is needed afterwards.
                    store: if resolve_target.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &scene.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        if !frame.batches.is_empty() {
            let mut ground_bound = None;
            for batch in &frame.batches {
                let Some(Some(model)) = self.models.get(batch.model.0) else {
                    continue;
                };
                if !model.baked {
                    continue;
                }
                let detail = self.detail.as_ref().filter(|_| batch.ground);
                if ground_bound != Some(detail.is_some()) {
                    match detail {
                        Some(detail) => {
                            pass.set_pipeline(&self.ground_pipeline);
                            pass.set_bind_group(2, detail, &[]);
                        }
                        None => pass.set_pipeline(&self.props_pipeline),
                    }
                    pass.set_bind_group(0, &self.props_bind_group, &[]);
                    pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
                    ground_bound = Some(detail.is_some());
                }
                pass.set_vertex_buffer(0, model.vertices.slice(..));
                pass.set_index_buffer(model.indices.slice(..), wgpu::IndexFormat::Uint32);
                for part in &model.parts {
                    pass.set_bind_group(1, part.texture.as_ref(), &[]);
                    pass.draw_indexed(part.indices.clone(), 0, batch.instances.clone());
                }
            }
        }

        pass.set_pipeline(&self.skinned_pipeline);
        for (index, draw) in frame.draws.iter().enumerate() {
            let Some(Some(model)) = self.models.get(draw.model.0) else {
                continue;
            };
            if model.baked {
                continue;
            }
            let offset = (index as u64 * self.stride) as u32;
            pass.set_bind_group(0, &self.uniform_bind_group, &[offset]);
            pass.set_vertex_buffer(0, model.vertices.slice(..));
            pass.set_index_buffer(model.indices.slice(..), wgpu::IndexFormat::Uint32);
            for part in &model.parts {
                pass.set_bind_group(1, part.texture.as_ref(), &[]);
                pass.draw_indexed(part.indices.clone(), 0, 0..1);
            }
        }
    }

    /// Writes every figure draw's uniform block, growing the buffer if needed.
    fn upload_draws(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &SceneFrame) {
        if frame.draws.is_empty() {
            return;
        }
        if frame.draws.len() > self.capacity {
            self.capacity = frame.draws.len().next_power_of_two();
            (self.uniform_buffer, self.uniform_bind_group) =
                create_uniforms(device, &self.uniform_layout, self.stride, self.capacity);
        }
        self.staging.clear();
        self.staging
            .resize(frame.draws.len() * self.stride as usize, 0);
        for (index, draw) in frame.draws.iter().enumerate() {
            let joints = frame.joints.get(draw.joints.clone()).unwrap_or_default();
            let uniforms = DrawUniforms::new(draw, &frame.light, joints);
            let start = index * self.stride as usize;
            let bytes = bytemuck::bytes_of(&uniforms);
            self.staging[start..start + bytes.len()].copy_from_slice(bytes);
        }
        queue.write_buffer(&self.uniform_buffer, 0, &self.staging);
    }

    /// Writes the scenery's frame uniforms and instances, growing the buffer if needed.
    fn upload_props(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &SceneFrame) {
        if frame.batches.is_empty() {
            return;
        }
        let uniforms = PropsUniforms::new(&frame.view, &frame.light);
        queue.write_buffer(&self.props_uniforms, 0, bytemuck::bytes_of(&uniforms));
        if frame.instances.len() > self.instance_capacity {
            self.instance_capacity = frame.instances.len().next_power_of_two();
            self.instance_buffer = create_instance_buffer(device, self.instance_capacity);
        }
        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&frame.instances),
        );
    }
}

impl SceneTarget {
    fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        samples: u32,
        size: [u32; 2],
    ) -> Self {
        let size = size.map(|side| side.max(1));
        let texture = |label, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let view =
            |texture: &wgpu::Texture| texture.create_view(&wgpu::TextureViewDescriptor::default());
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let resolve_texture = texture(
            "3D layer",
            color_format,
            1,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        Self {
            size,
            multisampled: (samples > 1).then(|| {
                view(&texture(
                    "3D layer (multisampled)",
                    color_format,
                    samples,
                    attachment,
                ))
            }),
            resolve: view(&resolve_texture),
            resolve_texture,
            depth: view(&texture(
                "3D layer depth",
                DEPTH_FORMAT,
                samples,
                attachment,
            )),
        }
    }
}

/// An opaque, depth-tested pipeline for one of the 3D layer's shaders (no culling: the
/// winding of imported meshes is not guaranteed, leaves are seen from both sides, and
/// the depth test hides the back faces of closed meshes anyway).
fn create_pipeline(
    device: &wgpu::Device,
    label: &str,
    source: &str,
    bind_group_layouts: &[&wgpu::BindGroupLayout],
    buffers: &[wgpu::VertexBufferLayout<'_>],
    color_format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layouts: Vec<Option<&wgpu::BindGroupLayout>> = bind_group_layouts
        .iter()
        .map(|layout| Some(*layout))
        .collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &layouts,
        immediate_size: 0,
    });
    let buffers: Vec<Option<wgpu::VertexBufferLayout<'_>>> =
        buffers.iter().cloned().map(Some).collect();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &buffers,
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: color_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Copies a target's resolved image into a buffer, waits for the GPU and returns it
/// (`size` x `size`, four bytes per texel in the target's own channel order).
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    target: &SceneTarget,
    size: u32,
) -> Result<RgbaImage, String> {
    let row = size * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("atlas readback"),
        size: u64::from(padded) * u64::from(size),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.resolve_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| format!("waiting for the atlas: {error}"))?;
    receiver
        .recv()
        .map_err(|error| format!("atlas readback lost: {error}"))?
        .map_err(|error| format!("atlas readback failed: {error}"))?;
    let mut pixels = Vec::with_capacity((row * size) as usize);
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| format!("atlas readback range: {error}"))?;
        for line in mapped.chunks_exact(padded as usize) {
            pixels.extend_from_slice(&line[..row as usize]);
        }
    }
    buffer.unmap();
    Ok(RgbaImage {
        width: size,
        height: size,
        pixels,
    })
}

/// The ground detail's bind group layout (group 2 of `shaders/ground.wgsl`): the weight
/// maps (array) and the sampler for maps over the whole map, the color and normal arrays
/// and their tiling sampler, the shade map, and the uniforms.
fn create_detail_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let texture = |binding, view_dimension| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension,
            multisampled: false,
        },
        count: None,
    };
    let array = wgpu::TextureViewDimension::D2Array;
    let sampler = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ground detail layout"),
        entries: &[
            texture(0, array),
            sampler(1),
            texture(2, array),
            texture(3, array),
            sampler(4),
            texture(5, wgpu::TextureViewDimension::D2),
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<DetailUniforms>() as u64
                    ),
                },
                count: None,
            },
        ],
    })
}

/// A mipmapped sRGB texture of `image` and its bind group.
fn texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    label: &str,
    image: &RgbaImage,
) -> wgpu::BindGroup {
    let texture = create_texture_with_mips(
        device,
        queue,
        label,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        &mip_chain(image),
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn create_uniforms(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    stride: u64,
    capacity: usize,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("skinned draw uniforms"),
        size: stride * capacity as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("skinned draw uniforms"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(std::mem::size_of::<DrawUniforms>() as u64),
            }),
        }],
    });
    (buffer, bind_group)
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scenery instances"),
        size: (capacity * std::mem::size_of::<PropInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_blocks_match_the_shader_layouts() {
        // WGSL: two mat4x4 (64 each), two vec4 (16 each), array<mat4x4, MAX_JOINTS>.
        assert_eq!(
            std::mem::size_of::<DrawUniforms>(),
            64 * 2 + 16 * 2 + 64 * MAX_JOINTS
        );
        assert_eq!(std::mem::offset_of!(DrawUniforms, light), 128);
        assert_eq!(std::mem::offset_of!(DrawUniforms, joints), 160);
        let source = include_str!("../../../shaders/skinned.wgsl");
        assert!(source.contains(&format!("const MAX_JOINTS: u32 = {MAX_JOINTS}u;")));
        // The scenery: two mat4x4 and five vec4 (props.wgsl and ground.wgsl alike).
        assert_eq!(std::mem::size_of::<PropsUniforms>(), 64 * 2 + 16 * 5);
        assert_eq!(std::mem::offset_of!(PropsUniforms, map), 160);
        assert_eq!(std::mem::offset_of!(PropsUniforms, square_color), 192);
        let square = GroundSquare {
            corner: Vec2::new(3.0, 4.0),
            side: 1.0,
            width: 0.05,
            color: [0.5, 0.25, 0.125],
        };
        let view = SceneView {
            ground_square: Some(square),
            ..SceneView::default()
        };
        let uniforms = PropsUniforms::new(&view, &SceneLight::default());
        assert_eq!(uniforms.square, [3.0, 4.0, 1.0, 0.05]);
        assert_eq!(uniforms.square_color, [0.5, 0.25, 0.125, 1.0]);
        let hidden = PropsUniforms::new(&SceneView::default(), &SceneLight::default());
        assert_eq!(hidden.square_color[3], 0.0);
    }

    #[test]
    fn vertex_and_instance_layouts_match_the_attributes() {
        // Locations 0..6: position, normal, color, uv, texture weight, joints, weights.
        assert_eq!(
            std::mem::size_of::<SkinnedVertex>(),
            12 * 3 + 8 + 4 + 4 + 16
        );
        assert_eq!(std::mem::offset_of!(SkinnedVertex, uv), 36);
        assert_eq!(std::mem::offset_of!(SkinnedVertex, texture_weight), 44);
        assert_eq!(std::mem::offset_of!(SkinnedVertex, joints), 48);
        assert_eq!(std::mem::offset_of!(SkinnedVertex, weights), 52);
        // Locations 5..9 of the scenery: four matrix columns, then the tint.
        assert_eq!(std::mem::size_of::<PropInstance>(), 16 * 5);
        assert_eq!(std::mem::offset_of!(PropInstance, tint), 64);
        // The uniform block fits the smallest binding size every adapter allows.
        assert!(std::mem::size_of::<DrawUniforms>() <= 16 * 1024);
    }

    #[test]
    fn draws_keep_their_own_joints_and_unused_ones_are_identity() {
        let mut frame = SceneFrame::default();
        let a = [Mat4::from_translation(Vec3::X); 2];
        let b = [Mat4::from_translation(Vec3::Y); 3];
        frame.push(ModelId(0), Mat4::IDENTITY, Mat4::IDENTITY, &a);
        frame.push(ModelId(1), Mat4::IDENTITY, Mat4::IDENTITY, &b);
        assert_eq!(frame.draws[0].joints, 0..2);
        assert_eq!(frame.draws[1].joints, 2..5);
        let second = &frame.draws[1];
        let uniforms =
            DrawUniforms::new(second, &frame.light, &frame.joints[second.joints.clone()]);
        assert_eq!(uniforms.joints[2], b[2].to_cols_array_2d());
        assert_eq!(uniforms.joints[3], Mat4::IDENTITY.to_cols_array_2d());
        let light = Vec3::from_slice(&uniforms.light[..3]);
        assert!((light.length() - 1.0).abs() < 1e-6);
        // More joints than the shader holds are cut off, not overrun.
        frame.push(
            ModelId(0),
            Mat4::IDENTITY,
            Mat4::IDENTITY,
            &[Mat4::IDENTITY; MAX_JOINTS + 5],
        );
        assert_eq!(frame.draws[2].joints.len(), MAX_JOINTS);
        frame.clear();
        assert!(frame.is_empty() && frame.joints.is_empty());
    }

    #[test]
    fn consecutive_copies_of_a_model_share_a_batch() {
        let mut frame = SceneFrame::default();
        let copy = PropInstance::new(Mat4::IDENTITY, [1.0; 4]);
        frame.push_props(ModelId(3), [copy; 2]);
        frame.push_props(ModelId(3), [copy]);
        frame.push_props(ModelId(4), [copy]);
        frame.push_props(ModelId(5), []);
        frame.push_props(ModelId(3), [copy]);
        let batches: Vec<(usize, Range<u32>)> = frame
            .batches
            .iter()
            .map(|batch| (batch.model.0, batch.instances.clone()))
            .collect();
        assert_eq!(batches, [(3, 0..3), (4, 3..4), (3, 4..5)]);
        assert!(!frame.is_empty());
    }

    fn validate(source: &str) {
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
    fn the_shaders_parse_and_validate() {
        validate(include_str!("../../../shaders/skinned.wgsl"));
        validate(include_str!("../../../shaders/props.wgsl"));
        validate(include_str!("../../../shaders/ground.wgsl"));
    }

    #[test]
    fn the_ground_draws_alone_and_detail_uniforms_match_the_shader() {
        let mut frame = SceneFrame::default();
        let copy = PropInstance::new(Mat4::IDENTITY, [1.0; 4]);
        frame.push_ground(ModelId(0), copy);
        frame.push_props(ModelId(0), [copy]);
        assert_eq!(
            frame.batches.len(),
            2,
            "scenery never joins the ground's batch"
        );
        assert!(frame.batches[0].ground && !frame.batches[1].ground);
    }

    #[test]
    fn ground_detail_packs_its_layers_and_refuses_what_cannot_be_drawn() {
        // WGSL: array<vec4, 2>, vec4, array<vec4, 8>.
        assert_eq!(
            std::mem::size_of::<DetailUniforms>(),
            16 * (3 + MAX_GROUND_LAYERS)
        );
        assert_eq!(std::mem::offset_of!(DetailUniforms, glows), 48);
        let source = include_str!("../../../shaders/ground.wgsl");
        assert!(source.contains(&format!("const MAX_LAYERS: i32 = {MAX_GROUND_LAYERS};")));
        let big = RgbaImage::solid(4, 4, [128; 4]);
        let small = RgbaImage::solid(2, 2, [128; 4]);
        let layer = |size| GroundLayer {
            color: &big,
            normal: &big,
            size,
            glow: [size, 0.0, 0.0],
            glow_depth: 0.5,
        };
        let layers: Vec<GroundLayer<'_>> = (1..=6).map(|i| layer(i as f32)).collect();
        let detail = GroundDetail {
            layers: &layers,
            weights: [&small, &small],
            shade: &small,
            brightness: 1.5,
            relief: 2.0,
        };
        assert_eq!(check_ground_detail(&detail), Ok(()));
        let uniforms = DetailUniforms::new(&detail);
        assert_eq!(uniforms.sizes, [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 1.0, 1.0]]);
        assert_eq!(uniforms.params, [6.0, 1.5, 2.0, 0.0]);
        assert_eq!(uniforms.glows[2], [3.0, 0.0, 0.0, 0.5]);
        assert_eq!(uniforms.glows[7], [0.0; 4]);

        let odd = [
            layer(1.0),
            GroundLayer {
                normal: &small,
                ..layer(1.0)
            },
        ];
        let too_many = [layer(1.0); MAX_GROUND_LAYERS + 1];
        for layers in [&[][..], &odd[..], &too_many[..]] {
            assert!(check_ground_detail(&GroundDetail { layers, ..detail }).is_err());
        }
        let mismatched = GroundDetail {
            weights: [&small, &big],
            ..detail
        };
        assert!(check_ground_detail(&mismatched).is_err());
    }

    /// The ground shader's group 2 matches [`create_detail_layout`]: arrays where the
    /// layout has arrays.
    #[test]
    fn the_ground_shader_binds_what_the_layout_provides() {
        let module = naga::front::wgsl::parse_str(include_str!("../../../shaders/ground.wgsl"))
            .unwrap_or_else(|error| panic!("{error}"));
        let mut found = Vec::new();
        for (_, variable) in module.global_variables.iter() {
            let Some(binding) = &variable.binding else {
                continue;
            };
            if binding.group != 2 {
                continue;
            }
            let kind = match &module.types[variable.ty].inner {
                naga::TypeInner::Image { arrayed: true, .. } => "array",
                naga::TypeInner::Image { arrayed: false, .. } => "texture",
                naga::TypeInner::Sampler { .. } => "sampler",
                _ => "uniform",
            };
            found.push((binding.binding, kind));
        }
        found.sort();
        assert_eq!(
            found,
            [
                (0, "array"),
                (1, "sampler"),
                (2, "array"),
                (3, "array"),
                (4, "sampler"),
                (5, "texture"),
                (6, "uniform"),
            ]
        );
    }
}
