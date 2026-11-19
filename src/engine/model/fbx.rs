//! FBX loading through ufbx: the meshes of a file, their skeleton, a base color texture
//! and every animation stack, converted to a [`SkinnedModel`].
//!
//! Conversion: ufbx turns the scene right-handed, +Y up, meters. It does so through the
//! root node's transform, which is part of the skeleton here, so nothing else changes.
//! Animation stacks are sampled at [`SAMPLE_RATE`]; their names lose the `Armature|`
//! prefix Blender adds, and Mixamo's stacks (all called `mixamo.com`) take the file's
//! name instead. Stacks that move nothing (such as the bind-pose `Take 001` Mixamo adds)
//! are dropped.
//!
//! Meshes: skinned meshes follow their clusters' bones; a mesh without a skin is a rigid
//! part of its node (one joint for the node, full weight). Identical corners are welded
//! into one vertex of the indexed triangle list.
//!
//! Joints: one per distinct node and bind matrix among the clusters that weigh any
//! vertex and the rigid meshes' nodes. A character split into several meshes bound in
//! one space (Mixamo's body and joints, for instance) so shares one set of joints, and
//! end bones without weights cost none.
//!
//! Materials: a color is the FBX diffuse color × factor (linear RGB, Blender's
//! convention). A material's diffuse texture is found embedded in the file, or by its
//! file name in the model's folder, its `textures` subfolder or the folder above's
//! `Textures` (never at the absolute path the file records). Textured materials are
//! drawn as texture × diffuse factor, and the model is split into one part per texture
//! (up to [`MAX_TEXTURES`]); texture alpha below a quarter cuts the surface away
//! (leaves). Textures that cannot be found are reported, and their materials keep their
//! color, or the one [`LoadOptions::material_colors`] gives them. Textures are scaled
//! down to at most [`MAX_TEXTURE_SIDE`]; a [`TextureCache`] shared by several loads
//! decodes each file once, and [`LoadOptions::texture_substitutes`] can swap a file for
//! another (art meant to be recolored by its author's shader).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use glam::{Mat4, Quat, Vec3, Vec4};

use super::{
    Clip, Joint, MeshPart, ModelError, ModelTexture, NodeTransform, SkeletonNode, SkinnedModel,
    SkinnedVertex, Track, strongest_influences,
};
use crate::engine::assets::RgbaImage;

/// Animation samples per second.
pub const SAMPLE_RATE: f32 = 30.0;
/// Longest side of a base color texture; larger ones are scaled down (models on the map
/// cover a few hundred pixels at most).
pub const MAX_TEXTURE_SIDE: u32 = 1024;
/// Textures one model may use; materials past them keep their colors.
pub const MAX_TEXTURES: usize = 8;
/// Corners whose material is missing.
const FALLBACK_COLOR: [f32; 3] = [0.5, 0.5, 0.5];
/// Samples this close to the rest transform do not need a track.
const CONSTANT_EPSILON: f32 = 1e-5;
/// Clusters of the same bone whose inverse bind matrices differ by less than this (per
/// element, file units) share a joint: well under a millimeter or a tenth of a degree.
const SHARED_BIND_EPSILON: f32 = 1e-3;
/// The name Mixamo gives every animation stack.
const MIXAMO_STACK: &str = "mixamo.com";

/// Decoded textures by key, shared by loads that use the same files.
pub type TextureCache = RefCell<HashMap<String, RgbaImage>>;

/// What the game adds to a file.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoadOptions<'a> {
    /// Linear RGB colors by material name, for materials without a usable texture (a
    /// file whose textures were not shipped).
    pub material_colors: &'a [(&'a str, [f32; 3])],
    /// Texture file names to load instead of the ones the file names: `(named, used)`.
    pub texture_substitutes: &'a [(&'a str, &'a str)],
    /// Textures decoded by earlier loads; new ones are added.
    pub cache: Option<&'a TextureCache>,
}

pub fn load_with(path: &Path, options: LoadOptions<'_>) -> Result<SkinnedModel, ModelError> {
    let bytes = std::fs::read(path).map_err(|source| ModelError::Read {
        path: path.to_owned(),
        source,
    })?;
    let scene = ufbx::load_memory(&bytes, load_options()).map_err(|error| ModelError::Parse {
        path: path.to_owned(),
        message: format!("{error:?}"),
    })?;
    let source = Source {
        file_name: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
        dir: path.parent().unwrap_or(Path::new("")),
        options,
    };
    let model = convert(&scene, &source).map_err(|problem| ModelError::Invalid {
        path: path.to_owned(),
        problem,
    })?;
    tracing::info!(
        path = %path.display(),
        triangles = model.indices().len() / 3,
        vertices = model.vertices().len(),
        joints = model.joints().len(),
        clips = model.clips().len(),
        textures = model.textures().len(),
        "model loaded"
    );
    Ok(model)
}

/// Right-handed, +Y up, meters; normals generated where the file has none.
fn load_options() -> ufbx::LoadOpts<'static> {
    ufbx::LoadOpts {
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        generate_missing_normals: true,
        ..Default::default()
    }
}

/// Where a scene came from: names Mixamo's clips and locates textures.
struct Source<'a> {
    /// Without extension.
    file_name: String,
    dir: &'a Path,
    options: LoadOptions<'a>,
}

fn convert(scene: &ufbx::Scene, source: &Source<'_>) -> Result<SkinnedModel, String> {
    let mut meshes = Vec::new();
    for mesh in scene.meshes.iter().map(|mesh| &**mesh) {
        if mesh.skin_deformers.len() > 1 {
            return Err(format!(
                "mesh {:?} has {} skins",
                &*mesh.element.name,
                mesh.skin_deformers.len()
            ));
        }
        if mesh.element.instances.is_empty() {
            tracing::warn!(mesh = %mesh.element.name, "mesh is not placed in the scene; skipped");
            continue;
        }
        meshes.push(mesh);
    }
    if meshes.is_empty() {
        return Err("no mesh".into());
    }

    let (nodes, sources) = skeleton(scene, &meshes)?;
    let node_index: HashMap<u32, usize> = sources
        .iter()
        .enumerate()
        .map(|(index, node)| (node.element.typed_id, index))
        .collect();
    let find_node = |node: &ufbx::Node| {
        node_index
            .get(&node.element.typed_id)
            .copied()
            .ok_or_else(|| format!("node {:?} is not in the skeleton", &*node.element.name))
    };

    let mut joints: Vec<Joint> = Vec::new();
    let mut textures = Textures {
        source,
        loaded: Vec::new(),
        reported: HashSet::new(),
    };
    let mut out = Welder::default();
    for mesh in &meshes {
        let binding = match mesh.skin_deformers.first() {
            Some(skin) => {
                // A skin without any weights still needs one joint for its corners to follow.
                let weightless = skin.clusters.iter().all(|cluster| cluster.num_weights == 0);
                let mut cluster_joints = Vec::with_capacity(skin.clusters.len());
                for (index, cluster) in skin.clusters.iter().enumerate() {
                    if cluster.num_weights == 0 && !(weightless && index == 0) {
                        cluster_joints.push(None);
                        continue;
                    }
                    let bone = cluster.bone_node.as_ref().ok_or_else(|| {
                        format!("cluster {:?} has no bone", &*cluster.element.name)
                    })?;
                    let node = find_node(bone)?;
                    let joint = joint_for(&mut joints, node, matrix(&cluster.geometry_to_bone))?;
                    cluster_joints.push(Some(joint));
                }
                Binding::Skin {
                    skin,
                    cluster_joints,
                }
            }
            None => {
                let instances = &mesh.element.instances;
                if instances.len() > 1 {
                    tracing::warn!(
                        mesh = %mesh.element.name,
                        count = instances.len(),
                        "mesh is placed several times; only the first is used"
                    );
                }
                let node = &instances[0];
                let joint = joint_for(
                    &mut joints,
                    find_node(node)?,
                    matrix(&node.geometry_to_node),
                )?;
                Binding::Rigid(joint)
            }
        };
        // Materials no face uses get no look (and no texture lookup or warning).
        let looks: Vec<Look> = mesh
            .materials
            .iter()
            .enumerate()
            .map(|(index, material)| {
                let used = mesh
                    .material_parts
                    .get(index)
                    .is_none_or(|part| part.num_triangles > 0);
                if used {
                    textures.look(material)
                } else {
                    Look {
                        color: FALLBACK_COLOR,
                        texture: None,
                        uv_set: String::new(),
                    }
                }
            })
            .collect();
        append_mesh(mesh, &binding, &looks, &mut out)?;
    }

    let mut clips = Vec::new();
    for stack in &scene.anim_stacks {
        let clip = sample_stack(stack, &nodes, &sources, &source.file_name);
        if clip.tracks.is_empty() {
            tracing::debug!(clip = %clip.name, "animation moves nothing; dropped");
        } else {
            clips.push(clip);
        }
    }
    let (vertices, indices, parts) = out.finish();
    SkinnedModel::new(vertices, indices, nodes, joints, clips)?
        .with_textures(textures.loaded, parts)
}

/// The joint binding `node` with `inverse_bind`, shared with an equal existing one.
fn joint_for(joints: &mut Vec<Joint>, node: usize, inverse_bind: Mat4) -> Result<u8, String> {
    let shared = joints.iter().position(|joint| {
        joint.node == node
            && joint
                .inverse_bind
                .abs_diff_eq(inverse_bind, SHARED_BIND_EPSILON)
    });
    let index = shared.unwrap_or_else(|| {
        joints.push(Joint { node, inverse_bind });
        joints.len() - 1
    });
    u8::try_from(index).map_err(|_| format!("more than {} joints", u8::MAX))
}

/// Every node the meshes hang from (their clusters' bones, or the nodes of rigid
/// meshes) with its ancestors, parents first.
fn skeleton<'a>(
    scene: &'a ufbx::Scene,
    meshes: &[&ufbx::Mesh],
) -> Result<(Vec<SkeletonNode>, Vec<&'a ufbx::Node>), String> {
    let mut needed = vec![false; scene.nodes.len()];
    let mut mark = |mut node: Option<&ufbx::Node>| {
        while let Some(current) = node {
            let index = current.element.typed_id as usize;
            if needed.get(index).copied().unwrap_or(true) {
                break;
            }
            needed[index] = true;
            node = current.parent.as_deref();
        }
    };
    for mesh in meshes {
        match mesh.skin_deformers.first() {
            Some(skin) => {
                for cluster in &skin.clusters {
                    mark(cluster.bone_node.as_deref());
                }
            }
            None => mark(mesh.element.instances.first().map(|node| &**node)),
        }
    }
    let mut sources: Vec<&ufbx::Node> = scene
        .nodes
        .iter()
        .map(|node| &**node)
        .filter(|node| needed.get(node.element.typed_id as usize) == Some(&true))
        .collect();
    // A node is deeper than its parent, so sorting by depth puts parents first.
    sources.sort_by_key(|node| (node.node_depth, node.element.typed_id));
    let position: HashMap<u32, usize> = sources
        .iter()
        .enumerate()
        .map(|(index, node)| (node.element.typed_id, index))
        .collect();

    let mut nodes = Vec::with_capacity(sources.len());
    for node in &sources {
        let ignore_parent_scale = match node.inherit_mode {
            ufbx::InheritMode::Normal => false,
            ufbx::InheritMode::IgnoreParentScale => true,
            mode => {
                // Per-component scale inheritance is not implemented; the pose may be
                // slightly off where scales differ from 1.
                tracing::warn!(
                    node = %node.element.name,
                    ?mode,
                    "unsupported scale inheritance; treated as normal"
                );
                false
            }
        };
        let parent = match node.parent.as_ref() {
            Some(parent) => Some(
                *position
                    .get(&parent.element.typed_id)
                    .ok_or_else(|| format!("node {:?} lost its parent", &*node.element.name))?,
            ),
            None => None,
        };
        nodes.push(SkeletonNode {
            name: node.element.name.to_string(),
            parent,
            rest: transform(&node.local_transform),
            ignore_parent_scale,
        });
    }
    Ok((nodes, sources))
}

/// How a mesh's corners follow the skeleton.
enum Binding<'a> {
    /// By the skin's weights; `cluster_joints` maps its clusters to joints (`None` for
    /// clusters without weights).
    Skin {
        skin: &'a ufbx::SkinDeformer,
        cluster_joints: Vec<Option<u8>>,
    },
    /// Every corner follows one joint fully.
    Rigid(u8),
}

/// How a material's corners are colored.
struct Look {
    /// Linear RGB; multiplies the texture if there is one.
    color: [f32; 3],
    /// Into the model's textures.
    texture: Option<usize>,
    /// The UV set the texture uses (empty: the mesh's first).
    uv_set: String,
}

/// The model's textures, gathered while the materials are read.
struct Textures<'a> {
    source: &'a Source<'a>,
    loaded: Vec<ModelTexture>,
    /// Textures already reported as unusable.
    reported: HashSet<String>,
}

impl Textures<'_> {
    fn look(&mut self, material: &ufbx::Material) -> Look {
        let maps = &material.fbx;
        let factor = if maps.diffuse_factor.has_value {
            maps.diffuse_factor.value_vec4.x
        } else {
            1.0
        };
        let texture = maps.diffuse_color.texture.as_deref().or(material
            .pbr
            .base_color
            .texture
            .as_deref());
        if let Some(texture) = texture
            && let Some(slot) = self.use_texture(material, texture)
        {
            return Look {
                color: [factor.clamp(0.0, 1.0) as f32; 3],
                texture: Some(slot),
                uv_set: texture.uv_set.to_string(),
            };
        }
        let name = &*material.element.name;
        let color = match self
            .source
            .options
            .material_colors
            .iter()
            .find(|(n, _)| *n == name)
        {
            Some(&(_, color)) => color,
            None => material_color(material, factor),
        };
        Look {
            color,
            texture: None,
            uv_set: String::new(),
        }
    }

    /// The slot of `texture` among the model's textures, loading it on first use.
    fn use_texture(&mut self, material: &ufbx::Material, texture: &ufbx::Texture) -> Option<usize> {
        let (key, data) = locate(
            texture,
            self.source.dir,
            self.source.options.texture_substitutes,
        );
        if let Some(slot) = self.loaded.iter().position(|loaded| loaded.key == key) {
            return Some(slot);
        }
        let Some(data) = data else {
            self.report(key, material, "texture file not found");
            return None;
        };
        if self.reported.contains(&key) {
            return None;
        }
        if self.loaded.len() >= MAX_TEXTURES {
            self.report(key, material, "too many textures in one model");
            return None;
        }
        let cache = self.source.options.cache;
        let cached = cache.and_then(|cache| cache.borrow().get(&key).cloned());
        let decoded = match cached {
            Some(image) => Ok(image),
            None => decode_texture(&data).inspect(|image| {
                if let Some(cache) = cache {
                    cache.borrow_mut().insert(key.clone(), image.clone());
                }
            }),
        };
        match decoded {
            Ok(image) => {
                tracing::debug!(texture = %key, width = image.width, height = image.height, "texture");
                self.loaded.push(ModelTexture { key, image });
                Some(self.loaded.len() - 1)
            }
            Err(problem) => {
                self.report(key, material, &problem);
                None
            }
        }
    }

    /// Warns once per texture; the material keeps its color.
    fn report(&mut self, key: String, material: &ufbx::Material, problem: &str) {
        if self.reported.insert(key.clone()) {
            tracing::warn!(
                texture = %key,
                material = %material.element.name,
                "{problem}; the material keeps its color"
            );
        }
    }
}

/// A texture's pixels: embedded in the file, or a file found next to the model.
enum TextureData<'a> {
    Embedded(&'a [u8]),
    File(PathBuf),
}

/// A key naming the texture (to recognize it again) and its data, if it can be found.
fn locate<'a>(
    texture: &'a ufbx::Texture,
    dir: &Path,
    substitutes: &[(&str, &str)],
) -> (String, Option<TextureData<'a>>) {
    let named = [&texture.relative_filename, &texture.filename]
        .into_iter()
        .find_map(|path| file_name(path))
        .unwrap_or(&texture.element.name);
    let name = substitutes
        .iter()
        .find(|(from, _)| *from == named)
        .map_or(named, |(_, to)| to)
        .to_string();
    let embedded = [&texture.content]
        .into_iter()
        .chain(texture.video.as_ref().map(|video| &video.content))
        .find(|content| !content.is_empty());
    if let Some(content) = embedded {
        return (
            format!("{name} (embedded)"),
            Some(TextureData::Embedded(content)),
        );
    }
    let mut candidates = vec![dir.join("textures").join(&name), dir.join(&name)];
    if let Some(parent) = dir.parent() {
        candidates.push(parent.join("Textures").join(&name));
    }
    let found = candidates.into_iter().find(|path| path.is_file());
    match found {
        Some(path) => (path.display().to_string(), Some(TextureData::File(path))),
        None => (name, None),
    }
}

/// The last component of a path written with either separator.
fn file_name(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
}

/// Decodes a texture (sRGB, straight alpha), scaled down to [`MAX_TEXTURE_SIDE`].
fn decode_texture(data: &TextureData<'_>) -> Result<RgbaImage, String> {
    let decoded = match data {
        TextureData::Embedded(bytes) => image::load_from_memory(bytes),
        TextureData::File(path) => std::fs::read(path)
            .map_err(image::ImageError::IoError)
            .and_then(|bytes| image::load_from_memory(&bytes)),
    }
    .map_err(|error| error.to_string())?;
    let longest = decoded.width().max(decoded.height());
    let decoded = if longest > MAX_TEXTURE_SIDE {
        let scale = MAX_TEXTURE_SIDE as f32 / longest as f32;
        let size = |side: u32| ((side as f32 * scale).round() as u32).max(1);
        // Area averaging: right for shrinking, and fast. (`DynamicImage`'s own method
        // is compiled optimized with the image crate; the generic `imageops` functions
        // would build unoptimized in this crate in debug builds.)
        decoded.thumbnail_exact(size(decoded.width()), size(decoded.height()))
    } else {
        decoded
    };
    let image = decoded.into_rgba8();
    Ok(RgbaImage {
        width: image.width(),
        height: image.height(),
        pixels: image.into_raw(),
    })
}

/// Diffuse color × factor, linear RGB.
fn material_color(material: &ufbx::Material, factor: f64) -> [f32; 3] {
    let maps = &material.fbx;
    if !maps.diffuse_color.has_value {
        return FALLBACK_COLOR;
    }
    let color = maps.diffuse_color.value_vec4;
    [color.x, color.y, color.z].map(|c| (c * factor).clamp(0.0, 1.0) as f32)
}

/// The indexed triangle list being built; identical corners share a vertex.
///
/// Only corners of the same source vertex (position index in the file) can be identical,
/// so each source vertex keeps a chain of the vertices made from it, and a corner is
/// compared with that short chain only. (Plain indexing; hashing every corner is slow in
/// unoptimized builds, and the tree has a million corners.)
#[derive(Default)]
struct Welder {
    vertices: Vec<SkinnedVertex>,
    /// Triangles per slot: 0 untextured, `n + 1` texture `n`.
    slots: Vec<Vec<u32>>,
    /// Per source vertex of the current mesh: the last vertex made from it.
    heads: Vec<u32>,
    /// Per vertex: the previous vertex made from the same source vertex.
    next: Vec<u32>,
}

impl Welder {
    const NONE: u32 = u32::MAX;

    /// Starts a mesh with `source_vertices` source vertices.
    fn begin_mesh(&mut self, source_vertices: usize) {
        self.heads.clear();
        self.heads.resize(source_vertices, Self::NONE);
    }

    /// Adds a corner of a triangle drawn in `slot`.
    fn push(&mut self, slot: usize, source: usize, vertex: SkinnedVertex) -> Result<(), String> {
        if self.slots.len() <= slot {
            self.slots.resize_with(slot + 1, Vec::new);
        }
        let head = self
            .heads
            .get(source)
            .copied()
            .ok_or("a corner's vertex index is out of range")?;
        let mut candidate = head;
        while candidate != Self::NONE {
            let index = candidate as usize;
            if bytemuck::bytes_of(&self.vertices[index]) == bytemuck::bytes_of(&vertex) {
                self.slots[slot].push(candidate);
                return Ok(());
            }
            candidate = self.next[index];
        }
        let index = u32::try_from(self.vertices.len())
            .ok()
            .filter(|&index| index != Self::NONE)
            .ok_or("too many vertices")?;
        self.vertices.push(vertex);
        self.next.push(head);
        self.heads[source] = index;
        self.slots[slot].push(index);
        Ok(())
    }

    /// The vertices, the indices grouped by slot, and one part per non-empty slot.
    fn finish(self) -> (Vec<SkinnedVertex>, Vec<u32>, Vec<MeshPart>) {
        let mut indices = Vec::new();
        let mut parts = Vec::new();
        for (slot, triangles) in self.slots.into_iter().enumerate() {
            if triangles.is_empty() {
                continue;
            }
            let start = indices.len() as u32;
            indices.extend(triangles);
            parts.push(MeshPart {
                indices: start..indices.len() as u32,
                texture: slot.checked_sub(1),
            });
        }
        (self.vertices, indices, parts)
    }
}

/// Appends the mesh's triangles (in face order) to `out`. Skinned corners without
/// weights follow the skin's first joint rigidly; unused joint slots point at it too.
fn append_mesh(
    mesh: &ufbx::Mesh,
    binding: &Binding<'_>,
    looks: &[Look],
    out: &mut Welder,
) -> Result<(), String> {
    let first_joint = match binding {
        Binding::Skin { cluster_joints, .. } => cluster_joints
            .iter()
            .flatten()
            .copied()
            .next()
            .ok_or_else(|| format!("the skin of {:?} has no clusters", &*mesh.element.name))?,
        Binding::Rigid(joint) => *joint,
    };
    // Each textured material's UV set; untextured ones need none.
    let uvs: Vec<Option<&ufbx::VertexVec2>> = looks
        .iter()
        .map(|look| {
            look.texture.is_some().then(|| {
                mesh.uv_sets
                    .iter()
                    .find(|set| *set.name == *look.uv_set)
                    .map_or(&mesh.vertex_uv, |set| &set.vertex_uv)
            })
        })
        .collect();
    let mut corners = Vec::new();
    let mut influences = Vec::new();
    let mut unweighted = 0usize;
    out.begin_mesh(mesh.num_vertices);
    for (face_index, face) in mesh.faces.iter().enumerate() {
        let count = ufbx::triangulate_face_vec(&mut corners, mesh, *face) as usize;
        let material = mesh.face_material.get(face_index).map(|&m| m as usize);
        let look = material.and_then(|m| looks.get(m));
        let color = look.map_or(FALLBACK_COLOR, |look| look.color);
        let uv_set = material
            .and_then(|m| uvs.get(m).copied().flatten())
            .filter(|set| set.exists);
        let texture_weight = if uv_set.is_some() { 1.0 } else { 0.0 };
        let slot = look
            .and_then(|look| look.texture)
            .filter(|_| uv_set.is_some())
            .map_or(0, |texture| texture + 1);
        for &corner in &corners[..count * 3] {
            let corner = corner as usize;
            let vertex = mesh.vertex_indices[corner] as usize;
            let position = &mesh.vertex_position[corner];
            let normal = if mesh.vertex_normal.exists {
                let n = &mesh.vertex_normal[corner];
                Vec3::new(n.x as f32, n.y as f32, n.z as f32).normalize_or(Vec3::Y)
            } else {
                Vec3::Y
            };
            // FBX puts the UV origin bottom-left; textures start top-left.
            let uv = uv_set.map_or([0.0; 2], |set| {
                let uv = &set[corner];
                [uv.x as f32, 1.0 - uv.y as f32]
            });

            let mut joints = [first_joint; 4];
            let mut weights = [1.0, 0.0, 0.0, 0.0];
            if let Binding::Skin {
                skin,
                cluster_joints,
            } = binding
            {
                influences.clear();
                if let Some(entry) = skin.vertices.get(vertex) {
                    let begin = entry.weight_begin as usize;
                    let end = begin + entry.num_weights as usize;
                    influences.extend(
                        skin.weights
                            .get(begin..end)
                            .unwrap_or_default()
                            .iter()
                            .map(|w| (w.cluster_index as usize, w.weight as f32)),
                    );
                }
                match strongest_influences(&mut influences) {
                    Some(kept) => {
                        weights = [0.0; 4];
                        let used = kept.into_iter().filter(|&(_, weight)| weight > 0.0);
                        for (slot, (cluster, weight)) in used.enumerate() {
                            joints[slot] = cluster_joints
                                .get(cluster)
                                .copied()
                                .flatten()
                                .ok_or_else(|| {
                                    format!("a weight refers to cluster {cluster}, which has none")
                                })?;
                            weights[slot] = weight;
                        }
                    }
                    None => unweighted += 1,
                }
            }
            out.push(
                slot,
                vertex,
                SkinnedVertex {
                    position: [position.x as f32, position.y as f32, position.z as f32],
                    normal: normal.to_array(),
                    color,
                    uv,
                    texture_weight,
                    joints,
                    weights,
                },
            )?;
        }
    }
    if unweighted > 0 {
        tracing::warn!(
            mesh = %mesh.element.name,
            corners = unweighted,
            "corners without skin weights follow the first joint"
        );
    }
    Ok(())
}

/// Samples the stack's local transforms of the skeleton's nodes; nodes that never leave
/// their rest transform get no track.
fn sample_stack(
    stack: &ufbx::AnimStack,
    nodes: &[SkeletonNode],
    sources: &[&ufbx::Node],
    file_name: &str,
) -> Clip {
    let name = clip_name(&stack.element.name, file_name);
    let duration = (stack.time_end - stack.time_begin).max(0.0) as f32;
    let frames = Clip::frame_count(duration, SAMPLE_RATE);
    let mut tracks = Vec::new();
    for (index, (node, source)) in nodes.iter().zip(sources).enumerate() {
        let samples: Vec<NodeTransform> = (0..frames)
            .map(|frame| {
                let offset = (frame as f32 / SAMPLE_RATE).min(duration);
                let time = stack.time_begin + f64::from(offset);
                transform(&ufbx::evaluate_transform(&stack.anim, source, time))
            })
            .collect();
        if samples
            .iter()
            .any(|sample| !sample.approx_eq(&node.rest, CONSTANT_EPSILON))
        {
            tracks.push(Track {
                node: index,
                samples,
            });
        }
    }
    Clip {
        name,
        duration,
        sample_rate: SAMPLE_RATE,
        tracks,
    }
}

/// The stack's name without Blender's `Armature|` prefix; Mixamo's stacks take the
/// file's name.
fn clip_name(stack_name: &str, file_name: &str) -> String {
    if stack_name == MIXAMO_STACK && !file_name.is_empty() {
        return file_name.to_string();
    }
    stack_name
        .rsplit_once('|')
        .map_or(stack_name, |(_, name)| name)
        .to_string()
}

fn transform(t: &ufbx::Transform) -> NodeTransform {
    NodeTransform {
        translation: vec3(&t.translation),
        rotation: Quat::from_xyzw(
            t.rotation.x as f32,
            t.rotation.y as f32,
            t.rotation.z as f32,
            t.rotation.w as f32,
        )
        .normalize(),
        scale: vec3(&t.scale),
    }
}

fn vec3(v: &ufbx::Vec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

/// ufbx matrices are affine 3x4, stored by column (`m<row><column>`).
fn matrix(m: &ufbx::Matrix) -> Mat4 {
    let column = |x: f64, y: f64, z: f64, w: f32| Vec4::new(x as f32, y as f32, z as f32, w);
    Mat4::from_cols(
        column(m.m00, m.m10, m.m20, 0.0),
        column(m.m01, m.m11, m.m21, 0.0),
        column(m.m02, m.m12, m.m22, 0.0),
        column(m.m03, m.m13, m.m23, 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(path: &Path) -> Result<SkinnedModel, ModelError> {
        load_with(path, LoadOptions::default())
    }

    #[test]
    fn matrices_keep_their_columns() {
        let m = ufbx::Matrix {
            m00: 1.0,
            m10: 2.0,
            m20: 3.0,
            m01: 4.0,
            m11: 5.0,
            m21: 6.0,
            m02: 7.0,
            m12: 8.0,
            m22: 9.0,
            m03: 10.0,
            m13: 11.0,
            m23: 12.0,
        };
        let converted = matrix(&m);
        assert_eq!(converted.x_axis, Vec4::new(1.0, 2.0, 3.0, 0.0));
        assert_eq!(converted.w_axis, Vec4::new(10.0, 11.0, 12.0, 1.0));
        assert_eq!(
            converted.transform_point3(Vec3::ZERO),
            Vec3::new(10.0, 11.0, 12.0)
        );
    }

    /// The skeleton's rest pose composed here (`parent * local`, parents first) moves
    /// every weighted cluster's vertices where ufbx's own evaluation of the file's bind
    /// state does, through a joint of the cluster's bone; clusters bound alike share it.
    #[test]
    fn rest_joints_match_the_files_bind_pose() {
        // The game's models serve as fixtures: one mesh with several materials, made in
        // Blender, and (where this checkout has it: it is not published) a Mixamo
        // character split into two meshes.
        for file in ["knight.fbx", "sword_and_shield_run.fbx"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/models/hero")
                .join(file);
            if !path.is_file() {
                eprintln!("skipped: {} is not in this checkout", path.display());
                continue;
            }
            let model = load(&path).unwrap();
            let scene = ufbx::load_memory(&std::fs::read(&path).unwrap(), load_options()).unwrap();
            let joints = super::super::Animator::new(&model)
                .joint_matrices(&model)
                .to_vec();
            let mut weighted = 0;
            for mesh in scene
                .meshes
                .iter()
                .filter(|mesh| !mesh.skin_deformers.is_empty())
            {
                // Geometry-space sample points (file units), compared in meters.
                let points: Vec<Vec3> = mesh.vertices.iter().step_by(97).map(vec3).collect();
                let clusters = mesh.skin_deformers[0].clusters.iter();
                for cluster in clusters.filter(|cluster| cluster.num_weights > 0) {
                    weighted += 1;
                    let bone = cluster.bone_node.as_ref().unwrap().element.name.to_string();
                    let expected = matrix(&cluster.geometry_to_world);
                    let found = model.joints().iter().zip(&joints).any(|(joint, actual)| {
                        model.nodes()[joint.node].name == bone
                            && points.iter().all(|&point| {
                                actual
                                    .transform_point3(point)
                                    .abs_diff_eq(expected.transform_point3(point), 1e-3)
                            })
                    });
                    assert!(found, "{file}: no joint binds {bone} like the file");
                }
            }
            assert!(
                (1..=weighted).contains(&model.joints().len()),
                "{file}: {} joints for {weighted} weighted clusters",
                model.joints().len()
            );
        }
    }

    /// Posed by its run, the Mixamo character's joints move vertices where ufbx's own
    /// evaluation of the animated scene does. Checked at sample times, so interpolation
    /// cannot differ; every bone there ignores its parent's scale.
    #[test]
    fn animated_joints_match_the_files_animation() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/models/hero/sword_and_shield_run.fbx");
        // The Mixamo character is not published; checkouts without it skip this.
        if !path.is_file() {
            eprintln!("skipped: {} is not in this checkout", path.display());
            return;
        }
        let model = load(&path).unwrap();
        let scene = ufbx::load_memory(&std::fs::read(&path).unwrap(), load_options()).unwrap();
        let stack = scene
            .anim_stacks
            .iter()
            .find(|stack| &*stack.element.name == MIXAMO_STACK)
            .unwrap();
        let clip = model.find_clip("sword_and_shield_run").unwrap();
        for frame in [0u8, 4, 11, 17] {
            let offset = f32::from(frame) / SAMPLE_RATE;
            let mut animator = super::super::Animator::new(&model);
            animator.play(clip);
            animator.advance(&model, offset);
            let joints = animator.joint_matrices(&model).to_vec();
            let time = stack.time_begin + f64::from(offset);
            let posed =
                ufbx::evaluate_scene(&scene, &stack.anim, time, Default::default()).unwrap();
            for mesh in scene.meshes.iter() {
                let points: Vec<Vec3> = mesh.vertices.iter().step_by(97).map(vec3).collect();
                let clusters = mesh.skin_deformers[0].clusters.iter();
                for cluster in clusters.filter(|cluster| cluster.num_weights > 0) {
                    let bone = cluster.bone_node.as_ref().unwrap();
                    let posed_bone = &posed.nodes[bone.element.typed_id as usize];
                    let expected =
                        matrix(&posed_bone.node_to_world) * matrix(&cluster.geometry_to_bone);
                    let name = bone.element.name.to_string();
                    let found = model.joints().iter().zip(&joints).any(|(joint, actual)| {
                        model.nodes()[joint.node].name == name
                            && points.iter().all(|&point| {
                                actual
                                    .transform_point3(point)
                                    .abs_diff_eq(expected.transform_point3(point), 1e-3)
                            })
                    });
                    assert!(found, "frame {frame}: {name} is posed unlike the file");
                }
            }
        }
    }

    #[test]
    fn clips_are_named_after_their_stack_or_the_mixamo_file() {
        assert_eq!(clip_name("Armature|Idle", "knight"), "Idle");
        assert_eq!(clip_name("Walking", "knight"), "Walking");
        assert_eq!(
            clip_name("mixamo.com", "sword_and_shield_run"),
            "sword_and_shield_run"
        );
        assert_eq!(clip_name("mixamo.com", ""), "mixamo.com");
    }

    #[test]
    fn missing_and_broken_files_are_errors() {
        let dir = std::env::temp_dir().join(format!("fbx_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = load(&dir.join("missing.fbx")).unwrap_err();
        assert!(matches!(missing, ModelError::Read { .. }), "{missing}");
        let broken_path = dir.join("broken.fbx");
        std::fs::write(&broken_path, b"Kaydara FBX Binary  \0 not really").unwrap();
        let broken = load(&broken_path).unwrap_err();
        assert!(matches!(broken, ModelError::Parse { .. }), "{broken}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
