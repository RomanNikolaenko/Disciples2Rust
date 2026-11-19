//! Skinned 3D models on the CPU: an indexed triangle mesh bound to a skeleton, split
//! into parts that each sample one texture (or none), animation clips, and an animator
//! that turns a playing clip into joint matrices for the GPU. Rigid (unskinned) meshes
//! are bound to their node as one joint with full weight, so every model is drawn the
//! same way; a model that never moves can be baked into its rest pose
//! ([`SkinnedModel::rest_pose_vertices`]) and drawn as static scenery.
//!
//! Spaces:
//! - **mesh space**: vertex positions as stored in the file (the mesh's geometry space);
//! - **model space**: the file's scene after the loader's conversion: right-handed,
//!   +Y up, the model's front towards +Z (see [`fbx`]). Units are whatever the file
//!   uses; [`SkinnedModel::bounds`] tells the size, so the game scales the model itself.
//!
//! Skinning: node transforms compose as `parent * local` (normal inheritance), or as
//! `parent * T * inverse(parent's local scale) * R * S` for nodes that ignore their
//! parent's scale (Maya's segment scale compensation, on every Mixamo bone). Each
//! joint has an inverse bind matrix (mesh space → the joint node's space at bind time),
//! so for a pose `joint matrix = node_to_model(pose) * inverse_bind`, and a vertex is the
//! weighted sum of its (up to [`JOINTS_PER_VERTEX`]) joint matrices applied to it.

pub mod fbx;

use std::ops::Range;
use std::path::PathBuf;

use glam::{Mat4, Quat, Vec3};

use crate::engine::assets::RgbaImage;

/// Most joints a model may have: the size of the skinning shader's joint array. Keeps
/// one draw's uniforms (8 KiB) well inside the 16 KiB every adapter allows.
pub const MAX_JOINTS: usize = 128;
/// Joints influencing one vertex: the strongest are kept and renormalized.
pub const JOINTS_PER_VERTEX: usize = 4;

/// One vertex of the indexed triangle list.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinnedVertex {
    /// Mesh space.
    pub position: [f32; 3],
    /// Mesh space, unit length.
    pub normal: [f32; 3],
    /// Linear RGB of the vertex's material (a multiplier where it is textured).
    pub color: [f32; 3],
    /// Into the model's base color texture: origin top-left, v down.
    pub uv: [f32; 2],
    /// 0: `color` alone; 1: `color` × its part's texture (whose alpha below a quarter
    /// cuts the surface away: leaves).
    pub texture_weight: f32,
    /// Indices into [`SkinnedModel::joints`].
    pub joints: [u8; JOINTS_PER_VERTEX],
    /// Sum to 1.
    pub weights: [f32; JOINTS_PER_VERTEX],
}

/// A node's transform relative to its parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeTransform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl NodeTransform {
    #[cfg(test)]
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Interpolates towards `other`; rotations take the shorter arc.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            scale: self.scale.lerp(other.scale, t),
        }
    }

    fn approx_eq(&self, other: &Self, epsilon: f32) -> bool {
        self.translation.abs_diff_eq(other.translation, epsilon)
            && self.scale.abs_diff_eq(other.scale, epsilon)
            // q and -q are the same rotation.
            && self.rotation.dot(other.rotation).abs() >= 1.0 - epsilon
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonNode {
    pub name: String,
    /// Always an earlier node: parents come before their children.
    pub parent: Option<usize>,
    /// The transform when no clip moves the node.
    pub rest: NodeTransform,
    /// The parent's own local scale does not reach this node (its parent's parents'
    /// scales still do).
    pub ignore_parent_scale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Joint {
    /// Index into [`SkinnedModel::nodes`].
    pub node: usize,
    /// Mesh space → the node's space at bind time.
    pub inverse_bind: Mat4,
}

/// The local transforms of the nodes a clip moves, sampled at a fixed rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub node: usize,
    /// One per clip frame.
    pub samples: Vec<NodeTransform>,
}

/// An animation: sample `i` is at `i / sample_rate` seconds, except the last one, which
/// is at `duration` exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Seconds.
    pub duration: f32,
    pub sample_rate: f32,
    pub tracks: Vec<Track>,
}

impl Clip {
    /// Frames needed to cover `duration` at `sample_rate` (both ends included).
    pub fn frame_count(duration: f32, sample_rate: f32) -> usize {
        (duration * sample_rate).ceil().max(0.0) as usize + 1
    }

    /// Overwrites the nodes this clip moves with their transforms at `time` (seconds,
    /// clamped to the clip); other nodes keep what `pose` had.
    pub fn sample_into(&self, time: f32, pose: &mut [NodeTransform]) {
        let frames = Self::frame_count(self.duration, self.sample_rate);
        let (index, t) = if frames < 2 || !time.is_finite() {
            (0, 0.0)
        } else {
            let time = time.clamp(0.0, self.duration);
            let index = ((time * self.sample_rate) as usize).min(frames - 2);
            let start = index as f32 / self.sample_rate;
            let end = ((index + 1) as f32 / self.sample_rate).min(self.duration);
            let span = end - start;
            let t = if span > 0.0 {
                ((time - start) / span).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (index, t)
        };
        for track in &self.tracks {
            let (Some(a), Some(target)) = (track.samples.get(index), pose.get_mut(track.node))
            else {
                continue;
            };
            *target = match track.samples.get(index + 1) {
                Some(b) if t > 0.0 => a.lerp(b, t),
                _ => *a,
            };
        }
    }
}

/// Axis-aligned bounds in model space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("cannot read model {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("model {path} cannot be parsed: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("model {path}: {problem}")]
    Invalid { path: PathBuf, problem: String },
}

/// A texture a model samples: where it came from (renderers share one GPU copy per key)
/// and its pixels (sRGB, straight alpha).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelTexture {
    pub key: String,
    pub image: RgbaImage,
}

/// A run of triangles drawn with one texture.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshPart {
    /// Into [`SkinnedModel::indices`]; whole triangles.
    pub indices: Range<u32>,
    /// Into [`SkinnedModel::textures`]; `None` draws with plain white.
    pub texture: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkinnedModel {
    vertices: Vec<SkinnedVertex>,
    /// Triangles: three indices into `vertices` each.
    indices: Vec<u32>,
    textures: Vec<ModelTexture>,
    /// Cover `indices` in order, without gaps.
    parts: Vec<MeshPart>,
    nodes: Vec<SkeletonNode>,
    joints: Vec<Joint>,
    clips: Vec<Clip>,
    /// Rest pose, model space.
    bounds: Bounds,
}

impl SkinnedModel {
    /// Checks the parts against each other (indices, ordering, sizes) and measures the
    /// rest pose. The error describes the first problem found.
    pub fn new(
        vertices: Vec<SkinnedVertex>,
        indices: Vec<u32>,
        nodes: Vec<SkeletonNode>,
        joints: Vec<Joint>,
        clips: Vec<Clip>,
    ) -> Result<Self, String> {
        if indices.is_empty() || !indices.len().is_multiple_of(3) {
            return Err(format!("{} indices do not form triangles", indices.len()));
        }
        if indices
            .iter()
            .any(|&index| index as usize >= vertices.len())
        {
            return Err("an index refers to a missing vertex".into());
        }
        for (index, node) in nodes.iter().enumerate() {
            if node.parent.is_some_and(|parent| parent >= index) {
                return Err(format!("node {:?} comes before its parent", node.name));
            }
        }
        if joints.is_empty() || joints.len() > MAX_JOINTS {
            return Err(format!(
                "{} joints (1 to {MAX_JOINTS} are supported)",
                joints.len()
            ));
        }
        if let Some(joint) = joints.iter().find(|joint| joint.node >= nodes.len()) {
            return Err(format!("a joint refers to missing node {}", joint.node));
        }
        if vertices.iter().any(|vertex| {
            vertex
                .joints
                .iter()
                .any(|&j| usize::from(j) >= joints.len())
        }) {
            return Err("a vertex refers to a missing joint".into());
        }
        for clip in &clips {
            if !(clip.duration.is_finite() && clip.duration >= 0.0 && clip.sample_rate > 0.0) {
                return Err(format!("clip {:?} has no valid timing", clip.name));
            }
            let frames = Clip::frame_count(clip.duration, clip.sample_rate);
            if let Some(track) = clip
                .tracks
                .iter()
                .find(|track| track.node >= nodes.len() || track.samples.len() != frames)
            {
                return Err(format!(
                    "clip {:?} has a broken track for node {}",
                    clip.name, track.node
                ));
            }
        }

        let parts = vec![MeshPart {
            indices: 0..indices.len() as u32,
            texture: None,
        }];
        let mut model = Self {
            vertices,
            indices,
            textures: Vec::new(),
            parts,
            nodes,
            joints,
            clips,
            bounds: Bounds {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
            },
        };
        let mut animator = Animator::new(&model);
        let joints = animator.joint_matrices(&model).to_vec();
        let mut positions = model.skinned_positions(&joints);
        let first = positions.next().unwrap_or(Vec3::ZERO);
        let (min, max) = positions.fold((first, first), |(min, max), p| (min.min(p), max.max(p)));
        if !(min.is_finite() && max.is_finite()) {
            return Err("the rest pose is not finite".into());
        }
        model.bounds = Bounds { min, max };
        Ok(model)
    }

    /// Gives the model its textures and the parts that use them. The parts must cover
    /// the indices in order, in whole triangles, and name existing textures.
    pub fn with_textures(
        mut self,
        textures: Vec<ModelTexture>,
        parts: Vec<MeshPart>,
    ) -> Result<Self, String> {
        let mut next = 0;
        for part in &parts {
            let range = &part.indices;
            if range.start != next
                || range.end < range.start
                || !(range.end - range.start).is_multiple_of(3)
            {
                return Err(format!("mesh part {range:?} does not follow on at {next}"));
            }
            if part
                .texture
                .is_some_and(|texture| texture >= textures.len())
            {
                return Err(format!("mesh part {range:?} names a missing texture"));
            }
            next = range.end;
        }
        if next as usize != self.indices.len() {
            return Err(format!(
                "mesh parts cover {next} of {} indices",
                self.indices.len()
            ));
        }
        self.textures = textures;
        self.parts = parts;
        Ok(self)
    }

    pub fn vertices(&self) -> &[SkinnedVertex] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn textures(&self) -> &[ModelTexture] {
        &self.textures
    }

    pub fn parts(&self) -> &[MeshPart] {
        &self.parts
    }

    /// The vertices posed at rest in model space, each following joint 0 fully: for
    /// drawing a model that never moves without its skeleton.
    pub fn rest_pose_vertices(&self) -> Vec<SkinnedVertex> {
        let joints = Animator::new(self).joint_matrices(self).to_vec();
        self.vertices
            .iter()
            .map(|vertex| {
                let skin: Mat4 = vertex
                    .joints
                    .iter()
                    .zip(vertex.weights)
                    .filter(|(_, weight)| *weight > 0.0)
                    .map(|(&joint, weight)| joints[usize::from(joint)] * weight)
                    .fold(Mat4::ZERO, |sum, matrix| sum + matrix);
                let position = skin.transform_point3(Vec3::from(vertex.position));
                let normal = skin
                    .transform_vector3(Vec3::from(vertex.normal))
                    .normalize_or(Vec3::Y);
                SkinnedVertex {
                    position: position.to_array(),
                    normal: normal.to_array(),
                    joints: [0; JOINTS_PER_VERTEX],
                    weights: [1.0, 0.0, 0.0, 0.0],
                    ..*vertex
                }
            })
            .collect()
    }

    #[cfg(test)]
    pub fn nodes(&self) -> &[SkeletonNode] {
        &self.nodes
    }

    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }

    /// The rest pose's extent in model space.
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }

    /// The first clip called `name`.
    pub fn find_clip(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|clip| clip.name == name)
    }

    /// Model-space vertex positions for the given joint matrices (CPU skinning, used to
    /// measure poses).
    pub fn skinned_positions<'a>(&'a self, joints: &'a [Mat4]) -> impl Iterator<Item = Vec3> + 'a {
        self.vertices.iter().map(move |vertex| {
            let position = Vec3::from(vertex.position);
            vertex
                .joints
                .iter()
                .zip(vertex.weights)
                .filter(|(_, weight)| *weight > 0.0)
                .map(|(&joint, weight)| {
                    joints[usize::from(joint)].transform_point3(position) * weight
                })
                .sum()
        })
    }
}

/// Plays one clip of a model in a loop and computes the resulting joint matrices.
/// Owns its scratch buffers, so a frame allocates nothing.
#[derive(Debug, Clone)]
pub struct Animator {
    clip: Option<usize>,
    /// Seconds into the clip.
    time: f32,
    pose: Vec<NodeTransform>,
    node_matrices: Vec<Mat4>,
    joint_matrices: Vec<Mat4>,
}

impl Animator {
    /// Shows the rest pose until a clip is played.
    pub fn new(model: &SkinnedModel) -> Self {
        Self {
            clip: None,
            time: 0.0,
            pose: model.nodes.iter().map(|node| node.rest).collect(),
            node_matrices: vec![Mat4::IDENTITY; model.nodes.len()],
            joint_matrices: vec![Mat4::IDENTITY; model.joints.len()],
        }
    }

    /// Loops `clip` from its start; a clip already playing keeps its time.
    pub fn play(&mut self, clip: usize) {
        if self.clip != Some(clip) {
            self.clip = Some(clip);
            self.time = 0.0;
        }
    }

    #[cfg(test)]
    pub fn time(&self) -> f32 {
        self.time
    }

    /// Moves the playing clip on by `dt` seconds, wrapping at its end.
    pub fn advance(&mut self, model: &SkinnedModel, dt: f32) {
        let Some(clip) = self.clip.and_then(|clip| model.clips.get(clip)) else {
            return;
        };
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.time = if clip.duration > 0.0 {
            (self.time + dt).rem_euclid(clip.duration)
        } else {
            0.0
        };
    }

    /// Joint matrices (model space, see the module docs) of the current pose.
    pub fn joint_matrices(&mut self, model: &SkinnedModel) -> &[Mat4] {
        for (pose, node) in self.pose.iter_mut().zip(&model.nodes) {
            *pose = node.rest;
        }
        if let Some(clip) = self.clip.and_then(|clip| model.clips.get(clip)) {
            clip.sample_into(self.time, &mut self.pose);
        }
        for (index, node) in model.nodes.iter().enumerate() {
            let pose = &self.pose[index];
            self.node_matrices[index] = match node.parent {
                Some(parent) if node.ignore_parent_scale => {
                    // Maya's joint order (ufbx agrees): the translation lives in the
                    // parent's scaled space, the rotation and scale do not.
                    let undo_scale = Mat4::from_scale(inverse_scale(self.pose[parent].scale));
                    self.node_matrices[parent]
                        * Mat4::from_translation(pose.translation)
                        * undo_scale
                        * Mat4::from_scale_rotation_translation(
                            pose.scale,
                            pose.rotation,
                            Vec3::ZERO,
                        )
                }
                Some(parent) => self.node_matrices[parent] * pose.matrix(),
                None => pose.matrix(),
            };
        }
        for (matrix, joint) in self.joint_matrices.iter_mut().zip(&model.joints) {
            *matrix = self.node_matrices[joint.node] * joint.inverse_bind;
        }
        &self.joint_matrices
    }
}

/// Per-axis 1/scale; a zero axis stays collapsed (0) instead of becoming infinite.
fn inverse_scale(scale: Vec3) -> Vec3 {
    let invert = |s: f32| if s != 0.0 { 1.0 / s } else { 0.0 };
    Vec3::new(invert(scale.x), invert(scale.y), invert(scale.z))
}

/// Keeps the [`JOINTS_PER_VERTEX`] strongest `(joint, weight)` influences and scales
/// them to sum to 1. `None` when no influence has a positive weight.
pub fn strongest_influences(
    influences: &mut [(usize, f32)],
) -> Option<[(usize, f32); JOINTS_PER_VERTEX]> {
    influences.sort_by(|a, b| b.1.total_cmp(&a.1));
    let kept = &influences[..influences.len().min(JOINTS_PER_VERTEX)];
    let total: f32 = kept
        .iter()
        .map(|&(_, weight)| weight)
        .filter(|weight| *weight > 0.0)
        .sum();
    if !(total > 0.0 && total.is_finite()) {
        return None;
    }
    let mut result = [(0, 0.0); JOINTS_PER_VERTEX];
    for (slot, &(joint, weight)) in result.iter_mut().zip(kept) {
        *slot = (joint, weight.max(0.0) / total);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex(position: [f32; 3], joint: u8) -> SkinnedVertex {
        SkinnedVertex {
            position,
            normal: [0.0, 1.0, 0.0],
            color: [1.0; 3],
            uv: [0.0; 2],
            texture_weight: 0.0,
            joints: [joint, 0, 0, 0],
            weights: [1.0, 0.0, 0.0, 0.0],
        }
    }

    fn at(translation: Vec3) -> NodeTransform {
        NodeTransform {
            translation,
            ..NodeTransform::IDENTITY
        }
    }

    /// A two-node chain: a root at the origin and a child one unit up; one triangle per
    /// joint, bound where the nodes rest. The clip turns the child a quarter turn about Z
    /// over one second.
    fn chain() -> SkinnedModel {
        let nodes = vec![
            SkeletonNode {
                name: "root".into(),
                parent: None,
                rest: NodeTransform::IDENTITY,
                ignore_parent_scale: false,
            },
            SkeletonNode {
                name: "child".into(),
                parent: Some(0),
                rest: at(Vec3::Y),
                ignore_parent_scale: false,
            },
        ];
        let joints = vec![
            Joint {
                node: 0,
                inverse_bind: Mat4::IDENTITY,
            },
            Joint {
                node: 1,
                inverse_bind: Mat4::from_translation(-Vec3::Y),
            },
        ];
        let vertices = vec![
            vertex([0.0, 0.0, 0.0], 0),
            vertex([0.1, 0.0, 0.0], 0),
            vertex([0.0, 0.1, 0.0], 0),
            vertex([0.0, 1.0, 0.0], 1),
            vertex([0.0, 2.0, 0.0], 1),
            vertex([0.1, 1.0, 0.0], 1),
        ];
        let turn = |angle: f32| NodeTransform {
            rotation: Quat::from_rotation_z(angle),
            ..at(Vec3::Y)
        };
        let frames = Clip::frame_count(1.0, 2.0);
        let samples = (0..frames)
            .map(|i| turn(std::f32::consts::FRAC_PI_2 * i as f32 / (frames - 1) as f32))
            .collect();
        let clips = vec![Clip {
            name: "Bend".into(),
            duration: 1.0,
            sample_rate: 2.0,
            tracks: vec![Track { node: 1, samples }],
        }];
        SkinnedModel::new(vertices, (0..6).collect(), nodes, joints, clips).unwrap()
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        a.abs_diff_eq(b, 1e-4)
    }

    #[test]
    fn the_rest_pose_reproduces_the_bind_pose() {
        let model = chain();
        let mut animator = Animator::new(&model);
        let joints = animator.joint_matrices(&model).to_vec();
        for (skinned, vertex) in model.skinned_positions(&joints).zip(model.vertices()) {
            assert!(close(skinned, Vec3::from(vertex.position)), "{skinned}");
        }
        assert_eq!(model.bounds().min, Vec3::ZERO);
        assert_eq!(model.bounds().max, Vec3::new(0.1, 2.0, 0.0));
    }

    #[test]
    fn a_clip_moves_the_child_and_what_hangs_on_it() {
        let model = chain();
        let mut animator = Animator::new(&model);
        animator.play(model.find_clip("Bend").unwrap());
        animator.advance(&model, 0.999_99);
        let joints = animator.joint_matrices(&model).to_vec();
        let tip = model.skinned_positions(&joints).nth(4).unwrap();
        // A quarter turn about Z at (0, 1): the tip one unit up now points along -X.
        assert!(close(tip, Vec3::new(-1.0, 1.0, 0.0)), "{tip}");
        let base = model.skinned_positions(&joints).next().unwrap();
        assert!(close(base, Vec3::ZERO), "the root's vertices stay");
    }

    /// A root turned a quarter about Z and scaled 2x, with a child one unit up: the
    /// child's position follows the scaled root either way, its own axes keep the root's
    /// scale only under normal inheritance.
    #[test]
    fn segment_scale_compensation_stops_the_parents_scale() {
        let posed = |ignore_parent_scale: bool| {
            let nodes = vec![
                SkeletonNode {
                    name: "root".into(),
                    parent: None,
                    rest: NodeTransform {
                        rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                        scale: Vec3::splat(2.0),
                        ..NodeTransform::IDENTITY
                    },
                    ignore_parent_scale: false,
                },
                SkeletonNode {
                    name: "child".into(),
                    parent: Some(0),
                    rest: at(Vec3::Y),
                    ignore_parent_scale,
                },
            ];
            let joints = vec![Joint {
                node: 1,
                inverse_bind: Mat4::IDENTITY,
            }];
            let vertices = vec![
                vertex([0.0, 0.0, 0.0], 0),
                vertex([0.1, 0.0, 0.0], 0),
                vertex([0.0, 0.1, 0.0], 0),
            ];
            let model =
                SkinnedModel::new(vertices, vec![0, 1, 2], nodes, joints, Vec::new()).unwrap();
            Animator::new(&model).joint_matrices(&model)[0]
        };
        let compensated = posed(true);
        assert!(close(
            compensated.transform_point3(Vec3::ZERO),
            Vec3::new(-2.0, 0.0, 0.0)
        ));
        assert!(close(compensated.transform_vector3(Vec3::X), Vec3::Y));
        let normal = posed(false);
        assert!(close(
            normal.transform_point3(Vec3::ZERO),
            Vec3::new(-2.0, 0.0, 0.0)
        ));
        assert!(close(normal.transform_vector3(Vec3::X), Vec3::Y * 2.0));
    }

    #[test]
    fn sampling_interpolates_between_frames() {
        let model = chain();
        let clip = &model.clips()[0];
        let mut pose = vec![NodeTransform::IDENTITY; 2];
        // Frames at 0, 0.5 and 1 s; 0.25 s is halfway to the second (a 22.5° turn).
        clip.sample_into(0.25, &mut pose);
        let expected = Quat::from_rotation_z(std::f32::consts::FRAC_PI_8);
        assert!(pose[1].rotation.dot(expected).abs() > 0.999_99);
        assert_eq!(
            pose[0],
            NodeTransform::IDENTITY,
            "untracked nodes untouched"
        );
        // Out-of-range and broken times clamp instead of failing.
        clip.sample_into(5.0, &mut pose);
        let end = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        assert!(pose[1].rotation.dot(end).abs() > 0.999_99);
        clip.sample_into(f32::NAN, &mut pose);
        assert!(pose[1].rotation.dot(Quat::IDENTITY).abs() > 0.999_99);
    }

    #[test]
    fn the_last_frame_may_be_short() {
        // 0.7 s at 2 Hz: frames at 0, 0.5 and 0.7 s.
        assert_eq!(Clip::frame_count(0.7, 2.0), 3);
        assert_eq!(Clip::frame_count(1.0, 2.0), 3);
        assert_eq!(Clip::frame_count(0.0, 30.0), 1);
        let clip = Clip {
            name: "Short".into(),
            duration: 0.7,
            sample_rate: 2.0,
            tracks: vec![Track {
                node: 0,
                samples: vec![at(Vec3::ZERO), at(Vec3::X), at(Vec3::X * 3.0)],
            }],
        };
        let mut pose = vec![NodeTransform::IDENTITY];
        clip.sample_into(0.6, &mut pose);
        assert!(close(pose[0].translation, Vec3::X * 2.0));
    }

    #[test]
    fn playback_loops_and_restarts_only_for_a_new_clip() {
        let model = chain();
        let mut animator = Animator::new(&model);
        animator.advance(&model, 0.5);
        assert_eq!(animator.time(), 0.0, "nothing plays yet");
        animator.play(0);
        animator.advance(&model, 0.75);
        animator.advance(&model, 0.5);
        assert!((animator.time() - 0.25).abs() < 1e-5, "{}", animator.time());
        animator.play(0);
        assert!(
            (animator.time() - 0.25).abs() < 1e-5,
            "same clip keeps going"
        );
        animator.advance(&model, f32::NAN);
        animator.advance(&model, -1.0);
        assert!((animator.time() - 0.25).abs() < 1e-5);
    }

    #[test]
    fn influences_keep_the_strongest_four() {
        let mut influences = [(3, 0.1), (7, 0.4), (1, 0.05), (2, 0.2), (9, 0.25)];
        let kept = strongest_influences(&mut influences).unwrap();
        assert_eq!(kept.map(|(joint, _)| joint), [7, 9, 2, 3]);
        let total: f32 = kept.iter().map(|(_, weight)| weight).sum();
        assert!((total - 1.0).abs() < 1e-6);
        assert!((kept[0].1 - 0.4 / 0.95).abs() < 1e-6);
        assert_eq!(
            strongest_influences(&mut [(2, 1.0)]),
            Some([(2, 1.0), (0, 0.0), (0, 0.0), (0, 0.0)])
        );
        assert_eq!(strongest_influences(&mut []), None);
        assert_eq!(strongest_influences(&mut [(1, 0.0)]), None);
    }

    #[test]
    fn textured_parts_must_cover_the_triangles_in_order() {
        let texture = || ModelTexture {
            key: "t".into(),
            image: RgbaImage::solid(1, 1, [255; 4]),
        };
        let part = |range: Range<u32>, texture| MeshPart {
            indices: range,
            texture,
        };
        let ok =
            chain().with_textures(vec![texture()], vec![part(0..3, Some(0)), part(3..6, None)]);
        let model = ok.unwrap();
        assert_eq!(model.parts().len(), 2);
        assert_eq!(model.textures().len(), 1);
        for broken in [
            vec![part(0..3, Some(0))],
            vec![part(0..3, Some(1)), part(3..6, None)],
            vec![part(0..2, None), part(2..6, None)],
            vec![part(3..6, None), part(0..3, None)],
        ] {
            assert!(chain().with_textures(vec![texture()], broken).is_err());
        }
    }

    #[test]
    fn the_rest_pose_bakes_into_static_vertices() {
        let model = chain();
        let baked = model.rest_pose_vertices();
        assert_eq!(baked.len(), model.vertices().len());
        // At rest the joints are where the mesh was bound, so nothing moves.
        for (baked, vertex) in baked.iter().zip(model.vertices()) {
            assert!(close(
                Vec3::from(baked.position),
                Vec3::from(vertex.position)
            ));
            assert_eq!(baked.joints, [0; 4]);
            assert_eq!(baked.weights, [1.0, 0.0, 0.0, 0.0]);
        }
    }

    #[test]
    fn broken_parts_are_rejected() {
        let model = chain();
        let rebuild = |model: SkinnedModel| {
            SkinnedModel::new(
                model.vertices,
                model.indices,
                model.nodes,
                model.joints,
                model.clips,
            )
        };
        assert!(rebuild(model.clone()).is_ok());

        let mut broken = model.clone();
        broken.indices.truncate(4);
        assert!(rebuild(broken).is_err(), "not whole triangles");
        let mut broken = model.clone();
        broken.indices[5] = 6;
        assert!(rebuild(broken).is_err(), "missing vertex");
        let mut broken = model.clone();
        broken.nodes[0].parent = Some(1);
        assert!(rebuild(broken).is_err(), "child before parent");
        let mut broken = model.clone();
        broken.joints[1].node = 9;
        assert!(rebuild(broken).is_err(), "missing node");
        let mut broken = model.clone();
        broken.vertices[0].joints[0] = 5;
        assert!(rebuild(broken).is_err(), "missing joint");
        let mut broken = model.clone();
        broken.clips[0].tracks[0].samples.pop();
        assert!(rebuild(broken).is_err(), "short track");
        let mut broken = model.clone();
        broken.joints.resize(MAX_JOINTS + 1, broken.joints[0]);
        assert!(rebuild(broken).is_err(), "too many joints");
    }
}
