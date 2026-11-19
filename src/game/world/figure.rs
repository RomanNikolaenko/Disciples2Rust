//! Figures on the world map: animated 3D models standing on tiles (the hero and
//! creatures), each playing its animation in a loop. A [`FigureKind`] says what a figure
//! looks like; its model is loaded once ([`FigureModel`]) and shared by every
//! [`Figure`] of that kind.
//!
//! Frames, in order: model space (+Y up, front +Z, see `engine::model`) → normalized
//! (feet at the origin, rest height 1) → scaled to the kind's height in tiles, turned to
//! face the viewer (plus the kind's own turn) and placed on its tile in the world (see
//! `engine::scene::iso_camera`) → the camera's clip space. The figures are lit in the
//! camera's view frame, like the scenery.
//!
//! Art, with the LICENSE.txt beside each: the hero is `models/hero/knight.fbx`
//! ("LowPoly Animated Knight" by Quaternius, CC0). Two models are used when present but
//! not published with the game, their licenses not allowing it or unknown:
//! `models/hero/sword_and_shield_run.fbx` (a Mixamo character, a loader test fixture) and
//! `models/dragon/dragon.fbx` ("BGE Dragon 2.0" by 3DHaupt); tests that need them skip
//! without them.

use std::path::Path;
use std::rc::Rc;

use glam::{Mat4, Vec2, Vec3};

use super::relief::Relief;
use crate::engine::model::fbx::{self, LoadOptions};
use crate::engine::model::{Animator, ModelError, SkinnedModel};
use crate::engine::render::scene_renderer::{ModelId, SceneFrame, SceneRenderer};
use crate::engine::scene::iso_camera::IsoCamera;

/// What a kind of figure looks like on the map.
#[derive(Debug)]
pub struct FigureKind {
    /// The model, relative to the assets directory.
    pub model: &'static str,
    /// Played in a loop; `None` keeps the rest pose.
    pub clip: Option<&'static str>,
    /// Rest-pose height, in tiles. Figures are not to scale with the scenery: the map
    /// shows a man nearly as tall as a tree.
    pub height_tiles: f32,
    /// Turn about +Y away from facing the viewer; negative turns the figure to the
    /// screen's left.
    pub facing: f32,
}

/// The player's hero: a knight, standing idle (its file also holds walking, running,
/// sword and death clips).
pub const HERO: FigureKind = FigureKind {
    model: "models/hero/knight.fbx",
    clip: Some("Idle"),
    height_tiles: 0.85,
    facing: -0.4,
};

/// A dragon resting on the map ("Idel_New" is the file's own spelling). The model's
/// head points along +X, so a turn of about -127° brings it towards the viewer and the
/// screen's left.
pub const DRAGON: FigureKind = FigureKind {
    model: "models/dragon/dragon.fbx",
    clip: Some("Idel_New"),
    height_tiles: 1.0,
    facing: -2.2,
};

/// Where the feet stand inside the tile (tile units from its north-west corner).
const FEET_IN_TILE: Vec2 = Vec2::new(0.5, 0.5);

/// A kind's model, loaded and uploaded once and shared by its figures.
pub struct FigureModel {
    kind: &'static FigureKind,
    model: SkinnedModel,
    gpu: ModelId,
    /// Normalized model space: feet at the origin, rest height 1.
    normalize: Mat4,
    /// How far the rest pose reaches sideways from the feet, in heights.
    half_width: f32,
}

impl FigureModel {
    pub fn load(
        kind: &'static FigureKind,
        assets_dir: &Path,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &mut SceneRenderer,
    ) -> Result<Rc<Self>, ModelError> {
        let path = assets_dir.join(kind.model);
        let model = fbx::load_with(&path, LoadOptions::default())?;
        let normalize = normalization(&model).ok_or_else(|| ModelError::Invalid {
            path: path.clone(),
            problem: "the model has no height".into(),
        })?;
        let size = model.bounds().size();
        let half_width = size.x.max(size.z) / size.y / 2.0;
        if let Some(clip) = kind.clip
            && model.find_clip(clip).is_none()
        {
            tracing::warn!(path = %path.display(), clip, "clip not found; showing the rest pose");
        }
        let gpu = scene.create_model(device, queue, &model);
        Ok(Rc::new(Self {
            kind,
            model,
            gpu,
            normalize,
            half_width,
        }))
    }

    /// Frees the GPU copy; figures of this model draw nothing afterwards.
    pub fn release(&self, scene: &mut SceneRenderer) {
        scene.release_model(self.gpu);
    }
}

pub struct Figure {
    model: Rc<FigureModel>,
    animator: Animator,
    /// The tile the figure stands on.
    tile: (usize, usize),
}

impl Figure {
    /// A figure on `tile`, playing its kind's clip from the start.
    pub fn new(model: Rc<FigureModel>, tile: (usize, usize)) -> Self {
        let mut animator = Animator::new(&model.model);
        if let Some(clip) = model.kind.clip.and_then(|name| model.model.find_clip(name)) {
            animator.play(clip);
        }
        Self {
            model,
            animator,
            tile,
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.animator.advance(&self.model.model, dt);
    }

    /// Queues the figure for the 3D layer, standing on `relief`, unless it is off
    /// screen. True when queued.
    pub fn draw(&mut self, camera: &IsoCamera, relief: &Relief, frame: &mut SceneFrame) -> bool {
        let map = self.feet();
        let feet = Vec3::new(map.x, relief.height(map.x, map.y), map.y);
        let screen = camera.point_to_screen(feet);
        let viewport = camera.viewport();
        let height = self.model.kind.height_tiles;
        // Generous screen bounds: a height up, a rest-pose half width (plus a margin for
        // the animation) sideways, in pixels at this zoom.
        let reach = camera.zoom() * height * (1.0 + self.model.half_width) * 1.5;
        if screen.x + reach < 0.0
            || screen.x - reach > viewport.x
            || screen.y + reach < 0.0
            || screen.y - reach > viewport.y
        {
            return false;
        }
        let turn = Mat4::from_rotation_y(IsoCamera::facing_viewer() + self.model.kind.facing);
        let world_from_model = world_from_model(feet, height, turn) * self.model.normalize;
        let clip_from_model = camera.clip_from_world() * world_from_model;
        let light_from_model = camera.view_rotation() * turn;
        let joints = self.animator.joint_matrices(&self.model.model);
        frame.push(self.model.gpu, clip_from_model, light_from_model, joints);
        true
    }

    /// Map (tile) coordinates of the feet.
    pub fn feet(&self) -> Vec2 {
        Vec2::new(self.tile.0 as f32, self.tile.1 as f32) + FEET_IN_TILE
    }
}

/// Normalized space → world: `height` tiles tall, turned by `turn`, feet on world point
/// `feet`.
fn world_from_model(feet: Vec3, height: f32, turn: Mat4) -> Mat4 {
    Mat4::from_translation(feet) * turn * Mat4::from_scale(Vec3::splat(height))
}

/// Model space → normalized space: the rest pose's feet (lowest point, horizontal center)
/// at the origin and its height scaled to 1. `None` for a flat model.
pub fn normalization(model: &SkinnedModel) -> Option<Mat4> {
    let bounds = model.bounds();
    let height = bounds.size().y;
    if !(height > 1e-6 && height.is_finite()) {
        return None;
    }
    let center = (bounds.min + bounds.max) / 2.0;
    let feet = Vec3::new(center.x, bounds.min.y, center.z);
    Some(Mat4::from_scale(Vec3::splat(1.0 / height)) * Mat4::from_translation(-feet))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(kind: &FigureKind) -> SkinnedModel {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(kind.model);
        fbx::load_with(&path, LoadOptions::default()).unwrap()
    }

    /// The model of `kind` if this checkout has it (some are not published).
    fn load_if_present(kind: &FigureKind) -> Option<SkinnedModel> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(kind.model);
        if !path.is_file() {
            eprintln!("skipped: {} is not in this checkout", kind.model);
            return None;
        }
        Some(fbx::load_with(&path, LoadOptions::default()).unwrap())
    }

    fn distinct_colors(model: &SkinnedModel) -> Vec<[u32; 3]> {
        let mut colors: Vec<[u32; 3]> = model
            .vertices()
            .iter()
            .map(|v| v.color.map(|c| (c * 1000.0).round() as u32))
            .collect();
        colors.sort_unstable();
        colors.dedup();
        colors
    }

    #[test]
    fn the_hero_loads_with_its_skeleton_and_clip() {
        let model = load(&HERO);
        // One mesh weighted to the knight's 28 deforming bones.
        assert_eq!(model.joints().len(), 28);
        assert_eq!(model.indices().len(), 956 * 3);
        // Flat-shaded and low: few corners share a normal, so welding merges few.
        assert!(model.vertices().len() <= model.indices().len());
        // Twelve clips (idle, walk, run, sword, death…); the idle lasts 3⅓ s.
        assert_eq!(model.clips().len(), 12);
        let clip = &model.clips()[model.find_clip(HERO.clip.unwrap()).unwrap()];
        assert!((clip.duration - 3.333).abs() < 0.01, "{}", clip.duration);
        assert!(!clip.tracks.is_empty());
        // One color per material (armor, skin, boots; linear RGB); no texture.
        assert_eq!(distinct_colors(&model).len(), 3);
        assert!(model.textures().is_empty());
        let weights_ok = model.vertices().iter().all(|v| {
            let total: f32 = v.weights.iter().sum();
            (total - 1.0).abs() < 1e-4
        });
        assert!(weights_ok);
    }

    #[test]
    fn the_hero_stands_upright_facing_the_viewer() {
        let model = load(&HERO);
        let bounds = model.bounds();
        let size = bounds.size();
        // A standing figure, centered left-right, feet near the ground.
        assert!(size.y > size.z * 2.0, "{size}");
        assert!(bounds.min.y.abs() < size.y * 0.05, "{bounds:?}");
        assert!(
            (bounds.min.x + bounds.max.x).abs() < size.x * 0.05,
            "{bounds:?}"
        );
        // Facing +Z: the knee's pole target (where the knee bends towards) is in front
        // of the knee.
        let position = |name: &str| {
            let node = model.nodes().iter().position(|node| node.name == name);
            world_translation(&model, node.unwrap_or_else(|| panic!("{name}")))
        };
        let target = position("PoleTarget.L");
        let knee = position("LowerLeg.L");
        assert!(target.z > knee.z + 0.05, "{target} vs {knee}");
    }

    fn world_translation(model: &SkinnedModel, node: usize) -> Vec3 {
        let mut matrix = Mat4::IDENTITY;
        let mut current = Some(node);
        while let Some(index) = current {
            matrix = model.nodes()[index].rest.matrix() * matrix;
            current = model.nodes()[index].parent;
        }
        matrix.transform_point3(Vec3::ZERO)
    }

    #[test]
    fn the_heros_clip_moves_the_figure_but_keeps_it_in_place() {
        let model = load(&HERO);
        let mut animator = Animator::new(&model);
        animator.play(model.find_clip(HERO.clip.unwrap()).unwrap());
        let rest = model.bounds();
        let rest_positions: Vec<Vec3> = model
            .skinned_positions(Animator::new(&model).joint_matrices(&model))
            .collect();
        let mut moved = false;
        for _ in 0..20 {
            animator.advance(&model, 0.17);
            let joints = animator.joint_matrices(&model).to_vec();
            assert!(joints.iter().all(|m| m.is_finite()));
            let positions: Vec<Vec3> = model.skinned_positions(&joints).collect();
            let center = positions.iter().copied().sum::<Vec3>() / positions.len() as f32;
            let rest_center = (rest.min + rest.max) / 2.0;
            // The clip stays on the spot: within a fifth of the height of the rest pose.
            assert!(
                (center - rest_center).length() < rest.size().y * 0.2,
                "{center} vs {rest_center}"
            );
            let lowest = positions.iter().map(|p| p.y).fold(f32::MAX, f32::min);
            assert!(
                (lowest - rest.min.y).abs() < rest.size().y * 0.1,
                "feet stay on the ground"
            );
            moved |= positions
                .iter()
                .zip(&rest_positions)
                .any(|(a, b)| (*a - *b).length() > rest.size().y * 0.01);
        }
        assert!(moved, "the clip animates");
    }

    #[test]
    fn the_dragon_loads_textured_with_its_clips() {
        let Some(model) = load_if_present(&DRAGON) else {
            return;
        };
        assert!(model.joints().len() > 64 && model.joints().len() <= 128);
        for clip in ["Fly_New", "Idel_New", "Run_New", "Walk_New"] {
            let index = model.find_clip(clip).unwrap_or_else(|| panic!("{clip}"));
            assert!(!model.clips()[index].tracks.is_empty(), "{clip}");
        }
        assert_eq!(
            model.textures().len(),
            1,
            "the color texture next to the model"
        );
        let texture = &model.textures()[0].image;
        // 4096x4096 in the file, scaled down.
        assert_eq!((texture.width, texture.height), (1024, 1024));
        let mut textured = model.vertices().iter().filter(|v| v.texture_weight > 0.0);
        assert!(textured.clone().count() > model.vertices().len() / 2);
        assert!(textured.all(|v| v.uv.iter().all(|c| (-0.01..=1.01).contains(c))));
    }

    #[test]
    fn normalized_figures_stand_on_the_origin_and_are_one_unit_tall() {
        let model = load(&HERO);
        let normalize = normalization(&model).unwrap();
        let bounds = model.bounds();
        let low = normalize.transform_point3(Vec3::new(0.0, bounds.min.y, 0.0));
        let high = normalize.transform_point3(Vec3::new(0.0, bounds.max.y, 0.0));
        assert!(low.y.abs() < 1e-5 && (high.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_figure_stands_on_its_tile_at_its_height() {
        let feet = Vec3::new(23.5, 1.25, 14.5);
        let place = world_from_model(feet, 0.85, Mat4::from_rotation_y(0.7));
        assert!(place.transform_point3(Vec3::ZERO).abs_diff_eq(feet, 1e-5));
        let head = place.transform_point3(Vec3::Y);
        assert!(head.abs_diff_eq(Vec3::new(23.5, 2.1, 14.5), 1e-5));
    }
}
