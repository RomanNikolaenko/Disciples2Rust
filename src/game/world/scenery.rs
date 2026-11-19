//! The map's scenery: props (trees, rocks, buildings…) standing on its tiles, the ground
//! they stand on, and the frame around the tile under the pointer.
//!
//! What stands where is data, in the map's `scenery.toml` (see [`SceneryConfig`]):
//! - **kinds**: a model each, with its size, contact shadow, and whether it is drawn as
//!   a baked sprite (trees: thousands of them would otherwise cost millions of triangles
//!   a frame), always drawn however far the camera zooms out (buildings), or turned to
//!   face a road beside it;
//! - **rules**: on which terrain a kind stands, how often, how many per tile, how big;
//!   a hash of each tile's position decides the rest, so a map always looks the same;
//! - **ground**: optional photographed surfaces (grass, soil, sand…) for the ground, each
//!   on its terrains in patches of a given share and size, lit by the sun;
//! - **heights**: optional relief, hills raised from a landscape height map (see
//!   [`Relief`]); the ground, the props and the figures stand on it.
//!
//! A map without the file, or with empty lists, shows just its painted ground.
//!
//! Scale: model meters become tiles at [`METERS_PER_TILE`], and every kind has a size of
//! its own on top (maps show buildings and crags larger than life).
//!
//! The ground is a flat grid over the map: its photographed surfaces where the map has
//! them, otherwise its painted terrain shown unlit (its light is painted in), either way
//! with soft contact shadows baked under the props, away from the sun. All of it is drawn
//! instanced by the 3D layer; copies off screen are skipped each frame, and as
//! the camera zooms out the ordinary props thin out (always the same ones, so nothing
//! flickers).

use std::path::Path;

use glam::{Mat4, Vec2, Vec3, Vec4Swizzles as _};
use serde::Deserialize;

use super::map::{Terrain, TerrainMap};
use super::relief::{Hill, Relief};
use crate::engine::assets::{self, RgbaImage};
use crate::engine::model::fbx::{self, LoadOptions, TextureCache};
use crate::engine::model::{
    Joint, MeshPart, ModelTexture, NodeTransform, SkeletonNode, SkinnedModel, SkinnedVertex,
};
use crate::engine::render::scene_renderer::{
    AtlasCell, GroundDetail, GroundLayer, GroundSquare, MAX_GROUND_LAYERS, ModelId, PropInstance,
    SceneFrame, SceneLight, SceneRenderer, SceneView,
};
use crate::engine::scene::iso_camera::IsoCamera;

/// Model meters per map tile.
const METERS_PER_TILE: f32 = 6.0;
/// World units over which the map fades to black at its edges.
const EDGE_FADE: f32 = 3.0;
/// Texels per tile of the ground surfaces' weight maps.
const WEIGHT_TEXELS: usize = 8;
/// How far (tiles) the borders between lands wander from straight lines, and the size
/// of their bends.
const LAND_WANDER: f32 = 4.0;
const LAND_WANDER_SCALE: f32 = 6.0;
/// Ambient light of the map: low, for a darker, more dramatic look.
const AMBIENT: f32 = 0.3;
/// Turns each sprite kind is baked at (copies take the nearest), the sprite atlas's
/// side in texels and cells per row, its texture key, and how far sprites stand out
/// towards the camera (world units) so the ground does not clip their base.
const SPRITE_TURNS: usize = 6;
const ATLAS_SIZE: u32 = 2048;
const ATLAS_COLUMNS: u32 = 10;
const SPRITE_ATLAS_KEY: &str = "scenery sprites";
const SPRITE_LIFT: f32 = 0.3;
/// Zoom (screen pixels per world unit) from which every prop is drawn, and the share
/// still drawn at [`THIN_ZOOM_FAR`] and below.
const FULL_ZOOM: f32 = 38.0;
const THIN_ZOOM_FAR: f32 = 22.0;
const THIN_SHARE: f32 = 0.4;
/// The hovered tile's frame: width (tiles), height above the ground, color (linear).
const OUTLINE_WIDTH: f32 = 0.05;
const OUTLINE_LIFT: f32 = 0.03;
const OUTLINE_COLOR: [f32; 3] = [0.85, 0.6, 0.25];
const SCENERY_VERSION: u32 = 1;

/// Towards the sun, world space: from the screen's upper left, about 45° up.
pub fn sun() -> Vec3 {
    Vec3::new(-0.7, 0.71, -0.1).normalize()
}

/// The map's `scenery.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneryFile {
    version: u32,
    #[serde(default)]
    ground: Option<GroundFile>,
    #[serde(default)]
    heights: Option<HeightsFile>,
    #[serde(default)]
    kinds: Vec<KindFile>,
    #[serde(default)]
    rules: Vec<RuleFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroundFile {
    /// Multiplies the lit surfaces' colors (photographed albedo is dark).
    #[serde(default = "one")]
    brightness: f32,
    /// Scales the slopes of the surfaces' normal maps.
    #[serde(default = "one")]
    relief: f32,
    /// Points (tiles) the map is shared out among: each place belongs to the nearest
    /// one's land, the borders wandering a little. Layers naming a land lie in it only.
    #[serde(default)]
    lands: Vec<[f32; 2]>,
    /// About how wide the blend between two neighbouring lands is, tiles.
    #[serde(default = "default_border")]
    border: f32,
    /// Painted in order, each over the ones before.
    #[serde(default)]
    layers: Vec<GroundLayerFile>,
}

/// The map's relief (see [`Relief`]).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeightsFile {
    /// The hills' landscape height map, unless they name their own: relative to the
    /// assets directory, a grayscale image, 16-bit for smooth slopes
    /// (`examples/import_ground_textures.rs height` makes one).
    #[serde(default)]
    map: Option<String>,
    /// 0..1: the share of the height map's range that stays plain.
    #[serde(default = "default_floor")]
    floor: f32,
    #[serde(default)]
    hills: Vec<HillFile>,
}

/// One copy of the height map.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HillFile {
    /// Map point (tiles) of its middle.
    at: [f32; 2],
    /// Tiles across.
    #[serde(default = "default_hill_size")]
    size: f32,
    /// Height of its highest point, tiles.
    #[serde(default = "default_peak")]
    peak: f32,
    /// Turn, degrees.
    #[serde(default)]
    turn: f32,
    /// Its own height map (as `[heights] map`).
    #[serde(default)]
    map: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroundLayerFile {
    /// Color and normal textures, relative to the assets directory (as
    /// `examples/import_ground_textures.rs` makes them).
    color: String,
    normal: String,
    /// Tiles one repeat of the textures covers.
    size: f32,
    /// Terrain names the surface lies on.
    terrain: Vec<String>,
    /// 0..1: the share of those terrains' area it covers (1: all of it).
    #[serde(default = "one")]
    cover: f32,
    /// About how wide its patches are, tiles.
    #[serde(default = "default_patch")]
    patch: f32,
    /// 0..1: how gradually its patches fade out at their edges.
    #[serde(default = "default_edge")]
    edge: f32,
    /// Layers naming the same field share one patch pattern (give them the same
    /// `patch`): the smaller share then lies inside the larger, as bare soil inside a
    /// stretch of trampled grass. Without one a layer's patches are its own.
    #[serde(default)]
    field: Option<String>,
    /// The land it lies in (1: the first of `lands`); without one, in every land.
    #[serde(default)]
    land: Option<usize>,
    /// How colorful (0: gray, 1: as photographed), applied before `tint`.
    #[serde(default = "one")]
    saturation: f32,
    /// Multiplies the color (linear), to match surfaces photographed apart.
    #[serde(default = "white")]
    tint: [f32; 3],
    /// Light given off by its lowest places, its cracks (linear color, may exceed 1):
    /// lava. Unlit by the sun.
    #[serde(default)]
    glow: [f32; 3],
    /// 0..1: up to which height the cracks glow (more: wider veins).
    #[serde(default = "default_glow_depth")]
    glow_depth: f32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KindFile {
    name: String,
    /// FBX, relative to the assets directory.
    model: String,
    #[serde(default = "one")]
    size: f32,
    /// Contact shadow radius at size 1, tiles.
    #[serde(default)]
    shadow: f32,
    #[serde(default)]
    sprite: bool,
    #[serde(default)]
    always_drawn: bool,
    #[serde(default)]
    faces_road: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    /// A terrain name: "plains", "forest", "hills", "mountains", …
    terrain: String,
    /// Kind names, one picked per prop.
    kinds: Vec<String>,
    /// 0..1: the share of the terrain's tiles that get props.
    chance: f32,
    #[serde(default = "one_count")]
    per_tile: usize,
    /// How far from the tile's middle a prop may stand (tiles).
    #[serde(default = "default_spread")]
    spread: f32,
    /// Scale range on top of the kind's size.
    #[serde(default = "one_range")]
    size: [f32; 2],
    /// Height range relative to the width (crags taller than wide).
    #[serde(default = "one_range")]
    stretch: [f32; 2],
    /// Brightness range.
    #[serde(default = "one_range")]
    tint: [f32; 2],
}

fn default_patch() -> f32 {
    4.0
}
fn default_edge() -> f32 {
    0.3
}
fn default_border() -> f32 {
    1.5
}
fn default_glow_depth() -> f32 {
    0.35
}
fn default_floor() -> f32 {
    0.15
}
fn default_hill_size() -> f32 {
    16.0
}
fn default_peak() -> f32 {
    3.0
}
fn white() -> [f32; 3] {
    [1.0; 3]
}
fn one() -> f32 {
    1.0
}
fn one_count() -> usize {
    1
}
fn default_spread() -> f32 {
    0.25
}
fn one_range() -> [f32; 2] {
    [1.0, 1.0]
}

/// A kind of prop: its model and how it stands and is drawn.
#[derive(Debug, Clone, PartialEq)]
struct PropKind {
    model: String,
    size: f32,
    shadow: f32,
    sprite: bool,
    always_drawn: bool,
    faces_road: bool,
}

/// A placement rule (see [`RuleFile`]), with names resolved.
#[derive(Debug, Clone, PartialEq)]
struct Rule {
    terrain: Terrain,
    kinds: Vec<usize>,
    chance: f32,
    per_tile: usize,
    spread: f32,
    size: [f32; 2],
    stretch: [f32; 2],
    tint: [f32; 2],
}

/// One ground surface (see [`GroundLayerFile`]), with names resolved.
#[derive(Debug, Clone, PartialEq)]
struct GroundLayerConfig {
    color: String,
    normal: String,
    size: f32,
    terrains: Vec<Terrain>,
    cover: f32,
    patch: f32,
    edge: f32,
    /// Seed of its patch pattern: shared by the layers of one field.
    pattern: u32,
    /// Index into [`GroundConfig::lands`].
    land: Option<usize>,
    saturation: f32,
    tint: [f32; 3],
    glow: [f32; 3],
    glow_depth: f32,
}

/// The ground's surfaces (see [`GroundFile`]); at least one.
#[derive(Debug, Clone, PartialEq)]
struct GroundConfig {
    brightness: f32,
    relief: f32,
    lands: Vec<Vec2>,
    border: f32,
    layers: Vec<GroundLayerConfig>,
}

/// The map's relief, resolved (see [`HeightsFile`]); at least one hill.
#[derive(Debug, Clone, PartialEq)]
struct HeightsConfig {
    /// Every height map the hills use, once each; [`Hill::map`] indexes it.
    maps: Vec<String>,
    floor: f32,
    hills: Vec<Hill>,
}

/// A map's scenery description; empty when the map has none.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneryConfig {
    kinds: Vec<PropKind>,
    rules: Vec<Rule>,
    ground: Option<GroundConfig>,
    heights: Option<HeightsConfig>,
}

impl SceneryConfig {
    /// Reads `path`; a missing file is an empty scenery, a broken one is reported and
    /// empty too (the map still works).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|problem| {
                tracing::error!(path = %path.display(), "scenery ignored: {problem}");
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::error!(path = %path.display(), "scenery unreadable: {error}");
                Self::default()
            }
        }
    }

    fn parse(text: &str) -> Result<Self, String> {
        let file: SceneryFile = toml::from_str(text).map_err(|error| error.to_string())?;
        if file.version != SCENERY_VERSION {
            return Err(format!("unsupported version {}", file.version));
        }
        let kinds: Vec<PropKind> = file
            .kinds
            .iter()
            .map(|kind| PropKind {
                model: kind.model.clone(),
                size: kind.size.max(0.0),
                shadow: kind.shadow.max(0.0),
                sprite: kind.sprite,
                always_drawn: kind.always_drawn,
                faces_road: kind.faces_road,
            })
            .collect();
        let rules = file
            .rules
            .iter()
            .map(|rule| {
                let terrain = terrain_named(&rule.terrain)?;
                let kind_indices = rule
                    .kinds
                    .iter()
                    .map(|name| {
                        file.kinds
                            .iter()
                            .position(|kind| kind.name == *name)
                            .ok_or_else(|| format!("unknown kind {name:?}"))
                    })
                    .collect::<Result<Vec<usize>, String>>()?;
                if kind_indices.is_empty() {
                    return Err(format!("a rule for {:?} names no kinds", rule.terrain));
                }
                Ok(Rule {
                    terrain,
                    kinds: kind_indices,
                    chance: rule.chance.clamp(0.0, 1.0),
                    per_tile: rule.per_tile.clamp(1, 4),
                    spread: rule.spread.clamp(0.0, 0.5),
                    size: rule.size,
                    stretch: rule.stretch,
                    tint: rule.tint,
                })
            })
            .collect::<Result<Vec<Rule>, String>>()?;
        let ground = match file.ground {
            Some(ground) if !ground.layers.is_empty() => {
                if ground.layers.len() > MAX_GROUND_LAYERS {
                    return Err(format!(
                        "{} ground layers, at most {MAX_GROUND_LAYERS}",
                        ground.layers.len()
                    ));
                }
                let land_count = ground.lands.len();
                let layers = ground
                    .layers
                    .into_iter()
                    .enumerate()
                    .map(|(index, layer)| {
                        let land = match layer.land {
                            Some(land) if (1..=land_count).contains(&land) => Some(land - 1),
                            Some(land) => {
                                return Err(format!(
                                    "ground layer {} names land {land} of {land_count}",
                                    index + 1
                                ));
                            }
                            None => None,
                        };
                        Ok(GroundLayerConfig {
                            pattern: match &layer.field {
                                Some(field) => pattern_seed(field),
                                None => 300 + index as u32 * 7,
                            },
                            land,
                            saturation: layer.saturation.max(0.0),
                            tint: layer.tint.map(|c| c.max(0.0)),
                            glow: layer.glow.map(|c| c.max(0.0)),
                            glow_depth: layer.glow_depth.clamp(0.01, 1.0),
                            terrains: layer
                                .terrain
                                .iter()
                                .map(|name| terrain_named(name))
                                .collect::<Result<Vec<Terrain>, String>>()?,
                            color: layer.color,
                            normal: layer.normal,
                            size: layer.size.max(0.01),
                            cover: layer.cover.clamp(0.0, 1.0),
                            patch: layer.patch.max(0.25),
                            edge: layer.edge.clamp(0.0, 1.0),
                        })
                    })
                    .collect::<Result<Vec<GroundLayerConfig>, String>>()?;
                Some(GroundConfig {
                    brightness: ground.brightness.max(0.0),
                    relief: ground.relief.max(0.0),
                    lands: ground.lands.iter().map(|&at| Vec2::from(at)).collect(),
                    border: ground.border.max(0.05),
                    layers,
                })
            }
            _ => None,
        };
        let heights = match file.heights {
            Some(heights) if !heights.hills.is_empty() => {
                let mut maps: Vec<String> = Vec::new();
                let mut hills = Vec::new();
                for (index, hill) in heights.hills.iter().enumerate() {
                    let path = hill.map.as_ref().or(heights.map.as_ref()).ok_or_else(|| {
                        format!("hill {} has no height map (nor has [heights])", index + 1)
                    })?;
                    let map = match maps.iter().position(|known| known == path) {
                        Some(map) => map,
                        None => {
                            maps.push(path.clone());
                            maps.len() - 1
                        }
                    };
                    hills.push(Hill {
                        at: Vec2::from(hill.at),
                        size: hill.size.max(0.5),
                        peak: hill.peak.max(0.0),
                        turn: hill.turn.to_radians(),
                        map,
                    });
                }
                Some(HeightsConfig {
                    maps,
                    floor: heights.floor.clamp(0.0, 0.99),
                    hills,
                })
            }
            _ => None,
        };
        Ok(Self {
            kinds,
            rules,
            ground,
            heights,
        })
    }
}

/// The relief `config` describes over `map`; flat without one, or when its height map
/// cannot be read (reported).
fn build_relief(config: Option<&HeightsConfig>, map: &TerrainMap, assets_dir: &Path) -> Relief {
    let (width, height) = (map.width(), map.height());
    let Some(config) = config else {
        return Relief::flat(width, height);
    };
    let maps: Result<Vec<_>, _> = config
        .maps
        .iter()
        .map(|map| assets::load_height(&assets_dir.join(map)))
        .collect();
    match maps {
        Ok(maps) => {
            let relief = Relief::new(width, height, &maps, config.floor, &config.hills);
            if relief.is_flat() {
                tracing::warn!("the relief's hills are all below its floor");
            } else {
                tracing::info!(
                    hills = config.hills.len(),
                    highest = relief.highest(),
                    "relief raised"
                );
            }
            relief
        }
        Err(error) => {
            tracing::error!("the map stays flat: {error}");
            Relief::flat(width, height)
        }
    }
}

/// A patch pattern's seed for a field's name (FNV-1a).
fn pattern_seed(field: &str) -> u32 {
    field.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// The terrain called `name` (any case).
fn terrain_named(name: &str) -> Result<Terrain, String> {
    Terrain::ALL
        .into_iter()
        .find(|terrain| terrain.name().eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("unknown terrain {name:?}"))
}

/// One placed prop.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    kind: usize,
    /// Map point of its base.
    position: Vec2,
    /// Turn about +Y.
    turn: f32,
    /// Scale on top of the kind's size: across, and up.
    scale: Vec2,
    /// Color multiplier.
    tint: [f32; 3],
}

impl Placement {
    /// Its base on the ground of `relief`.
    fn base(&self, relief: &Relief) -> Vec3 {
        let (x, z) = (self.position.x, self.position.y);
        Vec3::new(x, relief.height(x, z), z)
    }

    fn world_from_model(&self, kinds: &[PropKind], relief: &Relief) -> Mat4 {
        let size = kinds[self.kind].size / METERS_PER_TILE;
        Mat4::from_translation(self.base(relief))
            * Mat4::from_rotation_y(self.turn)
            * Mat4::from_scale(Vec3::new(self.scale.x, self.scale.y, self.scale.x) * size)
    }

    /// The instance tint: its color, and how unlit (0 lit, 1 as baked).
    fn tint4(&self, unlit: f32) -> [f32; 4] {
        [self.tint[0], self.tint[1], self.tint[2], unlit]
    }

    /// Contact shadow radius, tiles.
    fn shadow(&self, kinds: &[PropKind]) -> f32 {
        kinds[self.kind].shadow * kinds[self.kind].size * self.scale.x
    }
}

/// A copy ready to draw: where it stands (for culling) and its instance.
struct Prop {
    position: Vec3,
    instance: PropInstance,
    /// 0 for props always drawn; otherwise drawn while the zoom's share is above it.
    rank: f32,
}

/// Copies of one drawable: a 3D model, or one baked view of one (a sprite).
struct Batch {
    model: ModelId,
    props: Vec<Prop>,
}

/// How one kind's sprites were framed in the atlas (model units, in the view frame):
/// the cell's half side, and the height of its center above the model's base.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SpriteFrame {
    half: f32,
    center_y: f32,
}

pub struct Scenery {
    /// Everything drawn instanced, each with its copies.
    batches: Vec<Batch>,
    /// Every model uploaded, to free on leaving.
    uploaded: Vec<ModelId>,
    ground: Option<ModelId>,
    /// Whether the ground has its surfaces (then it draws the hovered tile's frame
    /// itself, following the relief).
    detailed: bool,
    outline: Option<ModelId>,
    map_size: Vec2,
    relief: Relief,
}

impl Scenery {
    /// Places `config`'s props on `map` (keeping `reserved` tiles clear for figures),
    /// loads and uploads their models, bakes the sprite kinds, and lays the ground under
    /// them: `config`'s surfaces, over `ground` (the painted terrain covering the whole
    /// map) where they leave it bare.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        map: &TerrainMap,
        config: &SceneryConfig,
        reserved: &[(usize, usize)],
        mut ground: RgbaImage,
        assets_dir: &Path,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &mut SceneRenderer,
    ) -> Self {
        let started = std::time::Instant::now();
        let kinds = &config.kinds;
        let mut placements = place(map, reserved, &config.rules, kinds);
        placements.sort_by_key(|placement| placement.kind);
        let map_size = Vec2::new(map.width() as f32, map.height() as f32);
        let relief = build_relief(config.heights.as_ref(), map, assets_dir);
        bake_shadows(&mut ground, map_size, &placements, kinds);
        let mut detailed = false;
        if let Some(ground_config) = &config.ground {
            // The surfaces' shade: the same contact shadows, alone.
            let mut shade = RgbaImage::solid(ground.width, ground.height, [255; 4]);
            bake_shadows(&mut shade, map_size, &placements, kinds);
            detailed =
                set_ground_surfaces(ground_config, map, &shade, assets_dir, device, queue, scene);
        }

        // Models that share textures decode each once.
        let cache = TextureCache::default();
        let options = LoadOptions {
            cache: Some(&cache),
            ..LoadOptions::default()
        };
        let loaded: Vec<Option<(SkinnedModel, ModelId)>> = kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| {
                if !placements.iter().any(|placement| placement.kind == index) {
                    return None;
                }
                match fbx::load_with(&assets_dir.join(&kind.model), options) {
                    Ok(model) => {
                        let id = scene.create_static_model(device, queue, &model);
                        Some((model, id))
                    }
                    Err(error) => {
                        tracing::error!("scenery model missing: {error}");
                        None
                    }
                }
            })
            .collect();
        let mut uploaded: Vec<ModelId> = loaded.iter().flatten().map(|(_, id)| *id).collect();

        // Sprites: bake each sprite kind seen from the camera at a few turns.
        let rotation = IsoCamera::rotation();
        let sprite_kinds: Vec<(usize, SpriteFrame)> = loaded
            .iter()
            .enumerate()
            .filter(|(kind, _)| kinds[*kind].sprite)
            .filter_map(|(kind, model)| {
                let (model, _) = model.as_ref()?;
                Some((kind, sprite_frame(model, rotation)))
            })
            .collect();
        let cells: Vec<AtlasCell> = sprite_kinds
            .iter()
            .flat_map(|&(kind, frame)| {
                let model = loaded[kind].as_ref().map(|(_, id)| *id);
                (0..SPRITE_TURNS).filter_map(move |turn| {
                    Some(AtlasCell {
                        model: model?,
                        cell_from_model: cell_from_model(frame, rotation, turn),
                    })
                })
            })
            .collect();
        let mut sprite_models: Vec<Option<ModelId>> = Vec::new();
        if !cells.is_empty() {
            let light = SceneLight {
                direction: rotation.transform_vector3(sun()),
                ambient: AMBIENT,
                ..SceneLight::default()
            };
            match scene.bake_atlas(device, queue, ATLAS_SIZE, ATLAS_COLUMNS, &light, &cells) {
                Ok(atlas) => {
                    // One quad per cell; the first carries the atlas, the others share
                    // it by key.
                    let mut atlas = Some(atlas);
                    for (index, &(_, frame)) in sprite_kinds
                        .iter()
                        .flat_map(|entry| std::iter::repeat_n(entry, SPRITE_TURNS))
                        .enumerate()
                    {
                        let image = atlas
                            .take()
                            .unwrap_or_else(|| RgbaImage::solid(1, 1, [0; 4]));
                        let id = sprite_model(frame, index as u32, image)
                            .ok()
                            .map(|model| scene.create_static_model(device, queue, &model));
                        uploaded.extend(id);
                        sprite_models.push(id);
                    }
                }
                Err(problem) => {
                    tracing::error!(
                        "sprite kinds stay 3D, their sprites cannot be baked: {problem}"
                    );
                }
            }
        }
        // Once baked, the sprite kinds' own models are no longer drawn.
        if !sprite_models.is_empty() {
            for &(kind, _) in &sprite_kinds {
                if let Some((_, id)) = &loaded[kind] {
                    scene.release_model(*id);
                    uploaded.retain(|model| model != id);
                }
            }
        }

        // Batches: 3D models, and one per sprite cell.
        let back_to_world = rotation.transpose();
        let towards_eye = IsoCamera::eye_direction() * SPRITE_LIFT;
        let mut batches: Vec<Batch> = Vec::new();
        for (kind, model) in loaded.iter().enumerate() {
            let Some((_, id)) = model else {
                continue;
            };
            let of_kind = placements.iter().filter(|placement| placement.kind == kind);
            let sprite = sprite_kinds
                .iter()
                .position(|&(sprite_kind, _)| sprite_kind == kind)
                .filter(|_| !sprite_models.is_empty());
            match sprite {
                None => batches.push(Batch {
                    model: *id,
                    props: of_kind
                        .map(|placement| Prop {
                            position: placement.base(&relief),
                            instance: PropInstance::new(
                                placement.world_from_model(kinds, &relief),
                                placement.tint4(0.0),
                            ),
                            rank: thinning_rank(placement, kinds),
                        })
                        .collect(),
                }),
                Some(slot) => {
                    let mut per_turn: Vec<Vec<Prop>> =
                        (0..SPRITE_TURNS).map(|_| Vec::new()).collect();
                    for placement in of_kind {
                        let size = kinds[kind].size / METERS_PER_TILE * placement.scale.x;
                        let base = placement.base(&relief) + towards_eye;
                        let world_from_sprite = Mat4::from_translation(base)
                            * back_to_world
                            * Mat4::from_scale(Vec3::splat(size));
                        per_turn[nearest_turn(placement.turn)].push(Prop {
                            position: placement.base(&relief),
                            instance: PropInstance::new(world_from_sprite, placement.tint4(1.0)),
                            rank: thinning_rank(placement, kinds),
                        });
                    }
                    for (turn, props) in per_turn.into_iter().enumerate() {
                        if let Some(Some(model)) = sprite_models.get(slot * SPRITE_TURNS + turn) {
                            batches.push(Batch {
                                model: *model,
                                props,
                            });
                        }
                    }
                }
            }
        }

        let ground = match ground_model(&relief, map_size, ground) {
            Ok(model) => Some(scene.create_static_model(device, queue, &model)),
            Err(problem) => {
                tracing::error!("the ground cannot be built: {problem}");
                None
            }
        };
        let outline = match outline_model() {
            Ok(model) => Some(scene.create_static_model(device, queue, &model)),
            Err(problem) => {
                tracing::error!("the tile frame cannot be built: {problem}");
                None
            }
        };
        tracing::info!(
            props = placements.len(),
            sprite_cells = cells.len(),
            ms = started.elapsed().as_millis(),
            "scenery placed"
        );
        Self {
            batches,
            uploaded,
            ground,
            detailed,
            outline,
            map_size,
            relief,
        }
    }

    /// The ground's heights.
    pub fn relief(&self) -> &Relief {
        &self.relief
    }

    /// Sets the frame's view and light for `camera` and queues the ground, the frame
    /// around `hovered` and the props on screen.
    pub fn draw(
        &self,
        camera: &IsoCamera,
        hovered: Option<(usize, usize)>,
        frame: &mut SceneFrame,
    ) {
        let clip_from_world = camera.clip_from_world();
        let light_from_world = camera.view_rotation();
        // The ground with surfaces draws the hovered tile's frame onto itself.
        let square = hovered
            .filter(|_| self.detailed)
            .map(|(x, y)| GroundSquare {
                corner: Vec2::new(x as f32, y as f32),
                side: 1.0,
                width: OUTLINE_WIDTH,
                color: OUTLINE_COLOR,
            });
        frame.view = SceneView {
            clip_from_world,
            light_from_world,
            map_size: self.map_size,
            edge_fade: EDGE_FADE,
            ground_square: square,
        };
        frame.light.direction = light_from_world.transform_vector3(sun());
        frame.light.ambient = AMBIENT;

        let unlit = [1.0, 1.0, 1.0, 1.0];
        if let Some(ground) = self.ground {
            frame.push_ground(ground, PropInstance::new(Mat4::IDENTITY, unlit));
        }
        if let (Some(outline), Some((x, y)), false) = (self.outline, hovered, self.detailed) {
            // A flat frame, at the height of the tile's middle.
            let ground = self.relief.height(x as f32 + 0.5, y as f32 + 0.5);
            let at = Mat4::from_translation(Vec3::new(x as f32, ground + OUTLINE_LIFT, y as f32));
            frame.push_props(outline, [PropInstance::new(at, unlit)]);
        }

        // On screen, with a margin of two tiles' worth of pixels for the props' height.
        let margin = 2.0 * camera.zoom();
        let viewport = camera.viewport();
        let visible = |position: Vec3| {
            let clip = clip_from_world * position.extend(1.0);
            let ndc = clip.xy() / clip.w;
            let x = (ndc.x + 1.0) / 2.0 * viewport.x;
            let y = (1.0 - ndc.y) / 2.0 * viewport.y;
            x > -margin && x < viewport.x + margin && y > -margin && y < viewport.y + margin * 2.0
        };
        let share = thinning_share(camera.zoom());
        for batch in &self.batches {
            frame.push_props(
                batch.model,
                batch
                    .props
                    .iter()
                    .filter(|prop| prop.rank < share && visible(prop.position))
                    .map(|prop| prop.instance),
            );
        }
    }

    /// Frees the GPU copies of the models.
    pub fn release(&self, scene: &mut SceneRenderer) {
        scene.clear_ground_detail();
        for model in self
            .uploaded
            .iter()
            .chain(&self.ground)
            .chain(&self.outline)
        {
            scene.release_model(*model);
        }
    }
}

/// Where every prop goes: each rule on each tile of its terrain (tiles in `reserved`
/// stay clear), decided by a hash of the tile and the rule.
fn place(
    map: &TerrainMap,
    reserved: &[(usize, usize)],
    rules: &[Rule],
    kinds: &[PropKind],
) -> Vec<Placement> {
    let mut out = Vec::new();
    for y in 0..map.height() {
        for x in 0..map.width() {
            if reserved.contains(&(x, y)) {
                continue;
            }
            let Some(terrain) = map.get(x, y) else {
                continue;
            };
            for (index, rule) in rules.iter().enumerate() {
                if rule.terrain != terrain {
                    continue;
                }
                let salt = |n: u32| 1000 * index as u32 + n;
                let h = |n: u32| hash(x, y, salt(n));
                if h(0) >= rule.chance {
                    continue;
                }
                let middle = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                for copy in 0..rule.per_tile as u32 {
                    let n = 10 + copy * 10;
                    // Copies of one tile spread around its middle, then jitter.
                    let around = if rule.per_tile > 1 {
                        let angle =
                            std::f32::consts::TAU * (copy as f32 / rule.per_tile as f32 + h(1));
                        Vec2::new(angle.cos(), angle.sin()) * rule.spread * 0.6
                    } else {
                        Vec2::ZERO
                    };
                    let jitter = Vec2::new(h(n) - 0.5, h(n + 1) - 0.5) * 2.0 * rule.spread * 0.5;
                    let kind = rule.kinds
                        [((h(n + 2) * rule.kinds.len() as f32) as usize).min(rule.kinds.len() - 1)];
                    let turn = match kinds.get(kind) {
                        Some(found) if found.faces_road => facing_road(map, x, y),
                        _ => None,
                    }
                    .unwrap_or_else(|| h(n + 3) * std::f32::consts::TAU);
                    let lerp = |range: [f32; 2], t: f32| range[0] + (range[1] - range[0]) * t;
                    let across = lerp(rule.size, h(n + 4));
                    let up = across * lerp(rule.stretch, h(n + 5));
                    let bright = lerp(rule.tint, h(n + 6));
                    out.push(Placement {
                        kind,
                        position: middle + around + jitter,
                        turn,
                        scale: Vec2::new(across, up),
                        tint: [bright; 3],
                    });
                }
            }
        }
    }
    out
}

/// A number in 0..1 from a tile and a salt.
fn hash(x: usize, y: usize, salt: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d)
        ^ (y as u32).wrapping_mul(0x1656_67b1)
        ^ salt.wrapping_mul(0x9e37_79b9);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

/// The share of the thinnable props drawn at `zoom`: all of them from [`FULL_ZOOM`] in,
/// [`THIN_SHARE`] from [`THIN_ZOOM_FAR`] out.
fn thinning_share(zoom: f32) -> f32 {
    let t = ((zoom - THIN_ZOOM_FAR) / (FULL_ZOOM - THIN_ZOOM_FAR)).clamp(0.0, 1.0);
    // Just above 1 when fully zoomed in, so every rank (below 1) passes.
    THIN_SHARE + (1.0001 - THIN_SHARE) * t
}

/// When a prop thins out (see [`Prop::rank`]): kinds always drawn never, the others by
/// a hash of where they stand.
fn thinning_rank(placement: &Placement, kinds: &[PropKind]) -> f32 {
    if kinds[placement.kind].always_drawn {
        return 0.0;
    }
    let roll = hash(
        (placement.position.x * 16.0) as usize,
        (placement.position.y * 16.0) as usize,
        200,
    );
    roll * 0.999
}

/// The turn that faces a building's front (+Z) towards a road or bridge beside it.
fn facing_road(map: &TerrainMap, x: usize, y: usize) -> Option<f32> {
    let sides = [(0i64, 1i64), (1, 0), (0, -1), (-1, 0)];
    sides.into_iter().find_map(|(dx, dy)| {
        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
        if nx < 0 || ny < 0 {
            return None;
        }
        let terrain = map.get(nx as usize, ny as usize)?;
        matches!(terrain, Terrain::Road | Terrain::Bridge).then(|| (dx as f32).atan2(dy as f32))
    })
}

/// The sprite turn (0..SPRITE_TURNS) nearest a prop's turn.
fn nearest_turn(turn: f32) -> usize {
    let step = std::f32::consts::TAU / SPRITE_TURNS as f32;
    ((turn / step).round() as i64).rem_euclid(SPRITE_TURNS as i64) as usize
}

fn turn_angle(turn: usize) -> f32 {
    turn as f32 * std::f32::consts::TAU / SPRITE_TURNS as f32
}

/// The cell framing that fits `model` at every sprite turn, seen through `rotation`.
fn sprite_frame(model: &SkinnedModel, rotation: Mat4) -> SpriteFrame {
    let bounds = model.bounds();
    let corners = (0..8).map(|i| {
        Vec3::new(
            if i & 1 == 0 {
                bounds.min.x
            } else {
                bounds.max.x
            },
            if i & 2 == 0 {
                bounds.min.y
            } else {
                bounds.max.y
            },
            if i & 4 == 0 {
                bounds.min.z
            } else {
                bounds.max.z
            },
        )
    });
    let (mut half_width, mut low, mut high) = (0.0f32, f32::MAX, f32::MIN);
    for turn in 0..SPRITE_TURNS {
        let turned = rotation * Mat4::from_rotation_y(turn_angle(turn));
        for corner in corners.clone() {
            let p = turned.transform_point3(corner);
            half_width = half_width.max(p.x.abs());
            low = low.min(p.y);
            high = high.max(p.y);
        }
    }
    // A little room so mipmaps do not bleed between cells.
    let half = (half_width.max((high - low) / 2.0) * 1.04).max(1e-3);
    SpriteFrame {
        half,
        center_y: (low + high) / 2.0,
    }
}

/// Model space → a cell's view frame (see `AtlasCell`): turned, seen through
/// `rotation`, centered and scaled so the frame's square fills -1..1.
fn cell_from_model(frame: SpriteFrame, rotation: Mat4, turn: usize) -> Mat4 {
    Mat4::from_scale(Vec3::splat(1.0 / frame.half))
        * Mat4::from_translation(Vec3::new(0.0, -frame.center_y, 0.0))
        * rotation
        * Mat4::from_rotation_y(turn_angle(turn))
}

/// The quad showing atlas cell `cell`: in the view frame's plane (x right, y up, model
/// units, the base at the origin), textured as a baked sprite.
fn sprite_model(frame: SpriteFrame, cell: u32, atlas: RgbaImage) -> Result<SkinnedModel, String> {
    let side = (ATLAS_SIZE / ATLAS_COLUMNS) as f32 / ATLAS_SIZE as f32;
    let (column, row) = ((cell % ATLAS_COLUMNS) as f32, (cell / ATLAS_COLUMNS) as f32);
    let (u0, v0) = (column * side, row * side);
    let (u1, v1) = (u0 + side, v0 + side);
    let (left, right) = (-frame.half, frame.half);
    let (bottom, top) = (frame.center_y - frame.half, frame.center_y + frame.half);
    let corner = |x: f32, y: f32, u: f32, v: f32| SkinnedVertex {
        position: [x, y, 0.0],
        normal: [0.0, 0.0, 1.0],
        color: [1.0; 3],
        uv: [u, v],
        // 2: a baked sprite (premultiplied texture).
        texture_weight: 2.0,
        joints: [0; 4],
        weights: [1.0, 0.0, 0.0, 0.0],
    };
    let vertices = vec![
        corner(left, top, u0, v0),
        corner(right, top, u1, v0),
        corner(left, bottom, u0, v1),
        corner(right, bottom, u1, v1),
    ];
    let texture = ModelTexture {
        key: SPRITE_ATLAS_KEY.into(),
        image: atlas,
    };
    flat_model(vertices, vec![0, 2, 1, 1, 2, 3], Some(texture))
}

/// Loads the ground's surfaces and hands them to the 3D layer with where each lies; on
/// failure the ground stays painted. True when the surfaces are in place.
fn set_ground_surfaces(
    config: &GroundConfig,
    map: &TerrainMap,
    shade: &RgbaImage,
    assets_dir: &Path,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &mut SceneRenderer,
) -> bool {
    let images = match load_ground_images(config, assets_dir) {
        Ok(images) => images,
        Err(problem) => {
            tracing::error!("the ground stays painted: {problem}");
            return false;
        }
    };
    let layers: Vec<GroundLayer<'_>> = images
        .iter()
        .zip(&config.layers)
        .map(|((color, normal), layer)| GroundLayer {
            color,
            normal,
            size: layer.size,
            glow: layer.glow,
            glow_depth: layer.glow_depth,
        })
        .collect();
    let [low, high] = ground_weights(map, config);
    let detail = GroundDetail {
        layers: &layers,
        weights: [&low, &high],
        shade,
        brightness: config.brightness,
        relief: config.relief,
    };
    match scene.set_ground_detail(device, queue, &detail) {
        Ok(()) => true,
        Err(problem) => {
            tracing::error!("the ground stays painted: {problem}");
            false
        }
    }
}

/// Every surface's color and normal map, decoded side by side and brought to the first
/// color map's size (the GPU keeps them in arrays of one size).
fn load_ground_images(
    config: &GroundConfig,
    assets_dir: &Path,
) -> Result<Vec<(RgbaImage, RgbaImage)>, String> {
    let started = std::time::Instant::now();
    let loaded = std::thread::scope(|scope| {
        let workers: Vec<_> = config
            .layers
            .iter()
            .map(|layer| {
                scope.spawn(move || -> Result<(RgbaImage, RgbaImage), String> {
                    let load = |path: &str| {
                        assets::load_rgba(&assets_dir.join(path)).map_err(|e| e.to_string())
                    };
                    let mut color = load(&layer.color)?;
                    adjust_color(&mut color, layer.saturation, layer.tint);
                    Ok((color, load(&layer.normal)?))
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|_| Err("a ground texture failed to decode".into()))
            })
            .collect::<Result<Vec<_>, String>>()
    })?;
    let Some((first, _)) = loaded.first() else {
        return Err("no ground layers".into());
    };
    let (width, height) = (first.width, first.height);
    let fitted = loaded
        .into_iter()
        .map(|(color, normal)| (fit(color, width, height), fit(normal, width, height)))
        .collect();
    tracing::info!(
        layers = config.layers.len(),
        ms = started.elapsed().as_millis(),
        "ground surfaces decoded"
    );
    Ok(fitted)
}

/// Changes an sRGB image's colors in linear light: towards gray by `saturation` (1
/// keeps them, 0 is gray of the same brightness), then times `tint`. Alpha unchanged.
fn adjust_color(image: &mut RgbaImage, saturation: f32, tint: [f32; 3]) {
    if saturation == 1.0 && tint == [1.0; 3] {
        return;
    }
    /// Steps of the linear → sRGB table: fine enough to round-trip every stored value.
    const STEPS: usize = 4096;
    let decode: [f32; 256] = std::array::from_fn(|value| {
        let c = value as f32 / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    let encode: Vec<u8> = (0..=STEPS)
        .map(|step| {
            let c = step as f32 / STEPS as f32;
            let e = if c <= 0.003_130_8 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            (e * 255.0).round() as u8
        })
        .collect();
    for pixel in image.pixels.as_chunks_mut::<4>().0 {
        let rgb = [0, 1, 2].map(|c| decode[usize::from(pixel[c])]);
        let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        for c in 0..3 {
            let value = (luma + (rgb[c] - luma) * saturation) * tint[c];
            pixel[c] = encode[(value.clamp(0.0, 1.0) * STEPS as f32).round() as usize];
        }
    }
}

/// `image` scaled to `width` x `height` (unchanged if it already is).
fn fit(image: RgbaImage, width: u32, height: u32) -> RgbaImage {
    if (image.width, image.height) == (width, height) {
        return image;
    }
    tracing::warn!(
        "a ground texture is {}x{}, scaled to {width}x{height}",
        image.width,
        image.height
    );
    let Some(buffer) = image::RgbaImage::from_raw(image.width, image.height, image.pixels) else {
        return RgbaImage::solid(width, height, [128, 128, 255, 255]);
    };
    let scaled = image::DynamicImage::ImageRgba8(buffer)
        .resize_exact(width, height, image::imageops::FilterType::Triangle)
        .to_rgba8();
    RgbaImage {
        width,
        height,
        pixels: scaled.into_raw(),
    }
}

/// Where each ground surface lies (see [`GroundDetail::weights`]). The map is shared
/// out among the ground's lands, if it names any; in each land its layers (those of the
/// land and those of none) are painted in turn, each covering its share of its terrains'
/// area in noise-shaped patches over the layers before it, and neighbouring lands blend
/// across their borders. No layer lies on terrain none names (the painted ground shows
/// there). Softened across tile borders.
fn ground_weights(map: &TerrainMap, ground: &GroundConfig) -> [RgbaImage; 2] {
    let layers = &ground.layers[..ground.layers.len().min(MAX_GROUND_LAYERS)];
    let (width, height) = (map.width() * WEIGHT_TEXELS, map.height() * WEIGHT_TEXELS);
    let texels = width * height;
    // Every texel's middle, in tiles, and its terrain.
    let tiles: Vec<Vec2> = (0..texels)
        .map(|texel| {
            Vec2::new((texel % width) as f32 + 0.5, (texel / width) as f32 + 0.5)
                / WEIGHT_TEXELS as f32
        })
        .collect();
    let terrain: Vec<Option<Terrain>> = (0..texels)
        .map(|texel| map.get(texel % width / WEIGHT_TEXELS, texel / width / WEIGHT_TEXELS))
        .collect();
    let shares = land_shares(&ground.lands, ground.border, &tiles);
    let amounts: Vec<Vec<f32>> = layers
        .iter()
        .map(|layer| {
            let on: Vec<bool> = terrain
                .iter()
                .map(|t| t.is_some_and(|t| layer.terrains.contains(&t)))
                .collect();
            let land = layer.land.and_then(|land| shares.get(land));
            layer_amounts(layer, &on, &tiles, land.map(Vec::as_slice))
        })
        .collect();
    let mut weights = vec![[0.0f32; MAX_GROUND_LAYERS]; texels];
    for (land, share) in shares.iter().enumerate() {
        for (texel, weight) in weights.iter_mut().enumerate() {
            if share[texel] <= 0.0 {
                continue;
            }
            let mut painted = [0.0f32; MAX_GROUND_LAYERS];
            for (index, layer) in layers.iter().enumerate() {
                let amount = amounts[index][texel];
                if amount <= 0.0 || layer.land.is_some_and(|own| own != land) {
                    continue;
                }
                for w in &mut painted {
                    *w *= 1.0 - amount;
                }
                painted[index] += amount;
            }
            for (w, p) in weight.iter_mut().zip(painted) {
                *w += share[texel] * p;
            }
        }
    }
    for _ in 0..2 {
        weights = box_blur(&weights, width, height, 2);
    }
    [0, 4].map(|first| RgbaImage {
        width: width as u32,
        height: height as u32,
        pixels: weights
            .iter()
            .flat_map(|texel| {
                [0, 1, 2, 3].map(|c| (texel[first + c] * 255.0).round().clamp(0.0, 255.0) as u8)
            })
            .collect(),
    })
}

/// How much a layer lies on each texel before lands are shared out: nothing off its
/// terrains (`on`), all of it where it covers all, else its patches, where its noise is
/// highest; the threshold leaves `cover` of its area (of its land's core, `land` above
/// one half, when it has one) above it.
fn layer_amounts(
    layer: &GroundLayerConfig,
    on: &[bool],
    tiles: &[Vec2],
    land: Option<&[f32]>,
) -> Vec<f32> {
    if layer.cover <= 0.0 {
        return vec![0.0; on.len()];
    }
    if layer.cover >= 1.0 {
        return on.iter().map(|&on| if on { 1.0 } else { 0.0 }).collect();
    }
    let noise: Vec<f32> = tiles
        .iter()
        .map(|&tile| patch_noise(tile / layer.patch, layer.pattern))
        .collect();
    let mut area: Vec<f32> = (0..on.len())
        .filter(|&texel| on[texel] && land.is_none_or(|share| share[texel] >= 0.5))
        .map(|texel| noise[texel])
        .collect();
    if area.is_empty() {
        return vec![0.0; on.len()];
    }
    area.sort_by(f32::total_cmp);
    let rank = ((1.0 - layer.cover) * (area.len() - 1) as f32).round() as usize;
    let threshold = area[rank.min(area.len() - 1)];
    // The noise spans about 0.25 around the threshold; `edge` 1 fades over it.
    let soft = (layer.edge * 0.25).max(1e-3);
    noise
        .iter()
        .zip(on)
        .map(|(&n, &on)| {
            if on {
                smoothstep(threshold - soft, threshold + soft, n)
            } else {
                0.0
            }
        })
        .collect()
}

/// How much each texel (its middle in `tiles`) belongs to each land, summing to 1: all
/// to the nearest land's point, blending over about `border` tiles where two lands meet.
/// The borders wander by noise so that they look grown, not drawn. Without lands, one
/// land is everywhere.
fn land_shares(lands: &[Vec2], border: f32, tiles: &[Vec2]) -> Vec<Vec<f32>> {
    if lands.is_empty() {
        return vec![vec![1.0; tiles.len()]];
    }
    let mut shares = vec![vec![0.0f32; tiles.len()]; lands.len()];
    let mut distances = vec![0.0f32; lands.len()];
    for (texel, &tile) in tiles.iter().enumerate() {
        let drift = Vec2::new(
            patch_noise(tile / LAND_WANDER_SCALE, 500),
            patch_noise(tile / LAND_WANDER_SCALE, 600),
        ) - Vec2::splat(0.5);
        let at = tile + drift * 2.0 * LAND_WANDER;
        for (distance, land) in distances.iter_mut().zip(lands) {
            *distance = land.distance(at);
        }
        let nearest = distances.iter().copied().fold(f32::INFINITY, f32::min);
        let mut sum = 0.0;
        for (land, &distance) in distances.iter().enumerate() {
            let share = (1.0 - (distance - nearest) / border).max(0.0);
            shares[land][texel] = share;
            sum += share;
        }
        for share in &mut shares {
            share[texel] /= sum;
        }
    }
    shares
}

/// Smooth noise in 0..1 with features about one unit wide and ragged edges.
fn patch_noise(p: Vec2, seed: u32) -> f32 {
    value_noise(p, seed) * 0.6
        + value_noise(p * 2.03 + Vec2::new(5.2, 1.3), seed + 1) * 0.28
        + value_noise(p * 4.1 + Vec2::new(9.7, 3.1), seed + 2) * 0.12
}

/// Smooth noise in 0..1 between random values on the unit grid (`p` not negative).
fn value_noise(p: Vec2, seed: u32) -> f32 {
    let cell = p.floor();
    let f = p - cell;
    let s = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let (x, y) = (cell.x.max(0.0) as usize, cell.y.max(0.0) as usize);
    let corner = |dx: usize, dy: usize| hash(x + dx, y + dy, seed);
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * s.x;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * s.x;
    top + (bottom - top) * s.y
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A box blur of `radius` texels, across then down.
fn box_blur<const N: usize>(
    source: &[[f32; N]],
    width: usize,
    height: usize,
    radius: usize,
) -> Vec<[f32; N]> {
    let pass = |source: &[[f32; N]], step: (usize, usize)| -> Vec<[f32; N]> {
        let mut out = vec![[0.0; N]; source.len()];
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0.0f32; N];
                let mut count = 0.0;
                for offset in 0..=2 * radius {
                    let (sx, sy) = (
                        (x + offset * step.0).checked_sub(radius * step.0),
                        (y + offset * step.1).checked_sub(radius * step.1),
                    );
                    if let (Some(sx), Some(sy)) = (sx, sy)
                        && sx < width
                        && sy < height
                    {
                        let texel = source[sy * width + sx];
                        for c in 0..N {
                            sum[c] += texel[c];
                        }
                        count += 1.0;
                    }
                }
                out[y * width + x] = sum.map(|c| c / count);
            }
        }
        out
    };
    let across = pass(source, (1, 0));
    pass(&across, (0, 1))
}

/// Darkens the ground image under each prop with a soft disc, shifted away from the sun.
fn bake_shadows(
    ground: &mut RgbaImage,
    map_size: Vec2,
    placements: &[Placement],
    kinds: &[PropKind],
) {
    if map_size.x <= 0.0 || ground.width == 0 || ground.height == 0 {
        return;
    }
    let texels = ground.width as f32 / map_size.x;
    let away = -Vec2::new(sun().x, sun().z).normalize_or_zero();
    let (width, height) = (ground.width as i64, ground.height as i64);
    for placement in placements {
        let radius = placement.shadow(kinds);
        if radius <= 0.0 {
            continue;
        }
        let center = (placement.position + away * radius * 0.45) * texels;
        let reach = radius * texels;
        let (x0, x1) = (
            (center.x - reach).floor() as i64,
            (center.x + reach).ceil() as i64,
        );
        let (y0, y1) = (
            (center.y - reach).floor() as i64,
            (center.y + reach).ceil() as i64,
        );
        for py in y0.max(0)..y1.min(height) {
            for px in x0.max(0)..x1.min(width) {
                let d = Vec2::new(px as f32 + 0.5, py as f32 + 0.5).distance(center) / reach;
                if d >= 1.0 {
                    continue;
                }
                let falloff = (1.0 - d * d) * (1.0 - d * d);
                let keep = 1.0 - 0.42 * falloff;
                let index = ((py * width + px) * 4) as usize;
                for channel in &mut ground.pixels[index..index + 3] {
                    *channel = (*channel as f32 * keep) as u8;
                }
            }
        }
    }
}

/// A flat model with one root joint, from vertices in world units.
fn flat_model(
    vertices: Vec<SkinnedVertex>,
    indices: Vec<u32>,
    texture: Option<ModelTexture>,
) -> Result<SkinnedModel, String> {
    let nodes = vec![SkeletonNode {
        name: "root".into(),
        parent: None,
        rest: NodeTransform {
            translation: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: Vec3::ONE,
        },
        ignore_parent_scale: false,
    }];
    let joints = vec![Joint {
        node: 0,
        inverse_bind: Mat4::IDENTITY,
    }];
    let count = indices.len() as u32;
    let model = SkinnedModel::new(vertices, indices, nodes, joints, Vec::new())?;
    match texture {
        Some(texture) => model.with_textures(
            vec![texture],
            vec![MeshPart {
                indices: 0..count,
                texture: Some(0),
            }],
        ),
        None => Ok(model),
    }
}

fn flat_vertex(
    position: Vec3,
    color: [f32; 3],
    uv: [f32; 2],
    texture_weight: f32,
) -> SkinnedVertex {
    SkinnedVertex {
        position: position.to_array(),
        normal: [0.0, 1.0, 0.0],
        color,
        uv,
        texture_weight,
        joints: [0; 4],
        weights: [1.0, 0.0, 0.0, 0.0],
    }
}

/// The ground: a grid over the whole map, one vertex per sample of `relief` at its
/// height (with its normal), textured with `image`.
fn ground_model(relief: &Relief, map_size: Vec2, image: RgbaImage) -> Result<SkinnedModel, String> {
    let (columns, rows) = relief.grid();
    let mut vertices = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            let uv = [
                column as f32 / (columns - 1).max(1) as f32,
                row as f32 / (rows - 1).max(1) as f32,
            ];
            let (x, z) = (uv[0] * map_size.x, uv[1] * map_size.y);
            let position = Vec3::new(x, relief.sample(column, row), z);
            let mut vertex = flat_vertex(position, [1.0; 3], uv, 1.0);
            vertex.normal = relief.normal(x, z).to_array();
            vertices.push(vertex);
        }
    }
    let (cells_x, cells_z) = (columns - 1, rows - 1);
    let mut indices = Vec::with_capacity(cells_x * cells_z * 6);
    let at = |i: usize, j: usize| (j * columns + i) as u32;
    for j in 0..cells_z {
        for i in 0..cells_x {
            indices.extend([
                at(i, j),
                at(i, j + 1),
                at(i + 1, j),
                at(i + 1, j),
                at(i, j + 1),
                at(i + 1, j + 1),
            ]);
        }
    }
    let texture = ModelTexture {
        key: "map ground".into(),
        image,
    };
    flat_model(vertices, indices, Some(texture))
}

/// A square frame around one tile (0..1 on x and z), in its color.
fn outline_model() -> Result<SkinnedModel, String> {
    let w = OUTLINE_WIDTH;
    let bars = [
        (0.0, 0.0, 1.0, w),
        (0.0, 1.0 - w, 1.0, 1.0),
        (0.0, 0.0, w, 1.0),
        (1.0 - w, 0.0, 1.0, 1.0),
    ];
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (x0, z0, x1, z1) in bars {
        let base = vertices.len() as u32;
        for (x, z) in [(x0, z0), (x1, z0), (x0, z1), (x1, z1)] {
            vertices.push(flat_vertex(
                Vec3::new(x, 0.0, z),
                OUTLINE_COLOR,
                [0.0; 2],
                0.0,
            ));
        }
        indices.extend([base, base + 2, base + 1, base + 1, base + 2, base + 3]);
    }
    flat_model(vertices, indices, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
version = 1

[ground]
brightness = 1.5

[[ground.layers]]
color = "textures/ground/grass_color.webp"
normal = "textures/ground/grass_normal.webp"
size = 1.5
terrain = ["plains", "village"]

[[ground.layers]]
color = "textures/ground/soil_color.webp"
normal = "textures/ground/soil_normal.webp"
size = 0.8
terrain = ["Plains"]
cover = 0.3
patch = 2.0

[[kinds]]
name = "oak"
model = "models/oak.fbx"
shadow = 0.4
sprite = true

[[kinds]]
name = "house"
model = "models/house.fbx"
size = 1.3
always_drawn = true
faces_road = true

[[rules]]
terrain = "Plains"
kinds = ["oak"]
chance = 0.5
per_tile = 2
size = [0.8, 1.2]

[[rules]]
terrain = "village"
kinds = ["house"]
chance = 1.0
"#;

    fn sample_map() -> TerrainMap {
        TerrainMap::from_rows(&["....~", "..#=.", "....."])
    }

    #[test]
    fn scenery_reads_its_kinds_rules_and_ground() {
        let config = SceneryConfig::parse(SAMPLE).unwrap();
        assert_eq!(config.kinds.len(), 2);
        assert!(config.kinds[0].sprite && !config.kinds[0].always_drawn);
        assert!(config.kinds[1].always_drawn && config.kinds[1].faces_road);
        assert_eq!(config.rules[0].terrain, Terrain::Plains);
        assert_eq!(config.rules[1].terrain, Terrain::Village);
        assert_eq!(config.rules[1].kinds, [1]);
        let ground = config.ground.as_ref().unwrap();
        assert_eq!((ground.brightness, ground.relief), (1.5, 1.0));
        assert_eq!(ground.layers.len(), 2);
        assert_eq!(
            ground.layers[0].terrains,
            [Terrain::Plains, Terrain::Village]
        );
        assert_eq!(
            (ground.layers[0].cover, ground.layers[0].patch),
            (1.0, default_patch())
        );
        assert_eq!((ground.layers[1].cover, ground.layers[1].patch), (0.3, 2.0));
        // A ground without layers is none.
        let bare = SceneryConfig::parse("version = 1\n[ground]\nbrightness = 2.0").unwrap();
        assert_eq!(bare.ground, None);
        // Nothing at all is a valid, empty scenery.
        assert_eq!(
            SceneryConfig::parse("version = 1").unwrap(),
            SceneryConfig::default()
        );
        for broken in [
            "version = 2",
            "version = 1\n[[rules]]\nterrain = \"lava\"\nkinds = []\nchance = 1.0",
            "version = 1\n[[rules]]\nterrain = \"plains\"\nkinds = [\"ghost\"]\nchance = 1.0",
            "version = 1\nunknown = 3",
            "version = 1\n[[ground.layers]]\ncolor = \"c\"\nnormal = \"n\"\nsize = 1.0\nterrain = [\"lava\"]",
        ] {
            assert!(SceneryConfig::parse(broken).is_err(), "{broken}");
        }
        let missing = SceneryConfig::load(Path::new("does/not/exist/scenery.toml"));
        assert_eq!(missing, SceneryConfig::default());
    }

    #[test]
    fn rules_place_props_on_their_terrain_and_stay_put() {
        let config = SceneryConfig::parse(SAMPLE).unwrap();
        let map = sample_map();
        let placements = place(&map, &[(0, 0)], &config.rules, &config.kinds);
        assert_eq!(
            placements,
            place(&map, &[(0, 0)], &config.rules, &config.kinds)
        );
        for placement in &placements {
            let tile = (placement.position.x as usize, placement.position.y as usize);
            let expected = if placement.kind == 0 {
                Terrain::Plains
            } else {
                Terrain::Village
            };
            assert_eq!(map.get(tile.0, tile.1), Some(expected), "{placement:?}");
            assert!(tile != (0, 0), "the reserved tile stays clear");
            assert!((0.8..=1.2).contains(&placement.scale.x) || placement.kind == 1);
        }
        // The house always stands, facing the road to its east (+x).
        let house = placements.iter().find(|p| p.kind == 1).unwrap();
        let front = Mat4::from_rotation_y(house.turn).transform_vector3(Vec3::Z);
        assert!(front.abs_diff_eq(Vec3::X, 1e-5), "{front}");
        // Oaks: two per chosen plains tile, about half of the tiles.
        let oaks = placements.iter().filter(|p| p.kind == 0).count();
        assert!(oaks % 2 == 0 && oaks > 4 && oaks < 24, "{oaks}");
    }

    #[test]
    fn zooming_out_thins_the_ordinary_but_keeps_what_is_always_drawn() {
        assert!(thinning_share(FULL_ZOOM) > 1.0 && thinning_share(80.0) > 1.0);
        assert!((thinning_share(THIN_ZOOM_FAR) - THIN_SHARE).abs() < 1e-6);
        assert!((thinning_share(5.0) - THIN_SHARE).abs() < 1e-6);
        let config = SceneryConfig::parse(SAMPLE).unwrap();
        let placements = place(&sample_map(), &[], &config.rules, &config.kinds);
        for placement in &placements {
            let rank = thinning_rank(placement, &config.kinds);
            assert!((0.0..1.0).contains(&rank));
            if placement.kind == 1 {
                assert_eq!(rank, 0.0);
            }
        }
    }

    /// A unit box standing on the origin, as a model.
    fn box_model() -> SkinnedModel {
        let corners: Vec<SkinnedVertex> = (0..8)
            .map(|i| {
                let p = Vec3::new(
                    if i & 1 == 0 { -0.5 } else { 0.5 },
                    if i & 2 == 0 { 0.0 } else { 2.0 },
                    if i & 4 == 0 { -0.5 } else { 0.5 },
                );
                flat_vertex(p, [1.0; 3], [0.0; 2], 0.0)
            })
            .collect();
        flat_model(corners, vec![0, 1, 2, 4, 5, 6, 3, 7, 2], None).unwrap()
    }

    #[test]
    fn sprites_take_the_nearest_turn_and_fill_their_cells() {
        assert_eq!(nearest_turn(0.0), 0);
        assert_eq!(nearest_turn(std::f32::consts::TAU - 0.01), 0);
        assert_eq!(nearest_turn(turn_angle(2) + 0.1), 2);
        assert_eq!(nearest_turn(-turn_angle(1)), SPRITE_TURNS - 1);
        let model = box_model();
        let rotation = IsoCamera::rotation();
        let frame = sprite_frame(&model, rotation);
        let bounds = model.bounds();
        for turn in 0..SPRITE_TURNS {
            let cell = cell_from_model(frame, rotation, turn);
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 {
                        bounds.min.x
                    } else {
                        bounds.max.x
                    },
                    if i & 2 == 0 {
                        bounds.min.y
                    } else {
                        bounds.max.y
                    },
                    if i & 4 == 0 {
                        bounds.min.z
                    } else {
                        bounds.max.z
                    },
                );
                let p = cell.transform_point3(corner);
                assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0, "{p}");
                assert!(p.z.abs() <= 2.0, "{p}");
            }
        }
        // The quad covers its own cell of the atlas, second row first.
        let quad = sprite_model(frame, 11, RgbaImage::solid(1, 1, [0; 4])).unwrap();
        let side = (ATLAS_SIZE / ATLAS_COLUMNS) as f32 / ATLAS_SIZE as f32;
        for vertex in quad.vertices() {
            for c in vertex.uv {
                assert!((side - 1e-5..=2.0 * side + 1e-5).contains(&c), "{c}");
            }
            assert_eq!(vertex.texture_weight, 2.0);
        }
    }

    #[test]
    fn shadows_darken_under_props_only() {
        let config = SceneryConfig::parse(SAMPLE).unwrap();
        let mut ground = RgbaImage::solid(40, 40, [200, 200, 200, 255]);
        let oak = Placement {
            kind: 0,
            position: Vec2::new(5.0, 5.0),
            turn: 0.0,
            scale: Vec2::ONE,
            tint: [1.0; 3],
        };
        bake_shadows(&mut ground, Vec2::new(10.0, 10.0), &[oak], &config.kinds);
        let pixel = |x: usize, y: usize| ground.pixels[(y * 40 + x) * 4];
        // Four texels per tile: the oak stands at texel (20, 20); the shadow leans away
        // from the sun (+x), and far corners keep their color.
        assert!(pixel(21, 20) < 180, "{}", pixel(21, 20));
        assert!(
            pixel(21, 20) < pixel(19, 20),
            "the shadow leans away from the sun"
        );
        assert_eq!(pixel(0, 0), 200);
        assert_eq!(pixel(39, 39), 200);
        assert!(ground.pixels.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn surfaces_cover_their_share_of_their_terrain_and_spare_the_water() {
        let config = SceneryConfig::parse(SAMPLE).unwrap();
        let map = TerrainMap::from_rows(&[
            "................~~~~",
            "................~~~~",
            "................~~~~",
            "................~~~~",
            "................~~~~",
            "................~~~~",
            "................~~~~",
            "................~~~~",
            "................~~~~",
            "................~~~~",
        ]);
        let [low, high] = ground_weights(&map, config.ground.as_ref().unwrap());
        assert_eq!(low.width as usize, map.width() * WEIGHT_TEXELS);
        assert_eq!((high.width, high.height), (low.width, low.height));
        assert!(
            high.pixels.iter().all(|&c| c == 0),
            "two layers fit the first map"
        );
        let texel = |x: usize, y: usize| {
            let at = (y * low.width as usize + x) * 4;
            [low.pixels[at], low.pixels[at + 1]].map(|c| f32::from(c) / 255.0)
        };
        // Away from the water, the layers share every texel; the soil covers about 30%.
        let mut soil = 0.0;
        let mut count = 0.0;
        for y in 0..10 * WEIGHT_TEXELS {
            for x in 0..14 * WEIGHT_TEXELS {
                let [grass, dirt] = texel(x, y);
                assert!((grass + dirt - 1.0).abs() < 0.02, "{grass} + {dirt}");
                soil += dirt;
                count += 1.0;
            }
        }
        assert!((soil / count - 0.3).abs() < 0.08, "{}", soil / count);
        // No layer names water: it keeps its painted ground.
        assert_eq!(texel(19 * WEIGHT_TEXELS, 5 * WEIGHT_TEXELS), [0.0, 0.0]);
        // The same map always gets the same ground.
        assert_eq!(
            ground_weights(&map, config.ground.as_ref().unwrap())[0],
            low
        );
    }

    #[test]
    fn layers_of_one_field_nest_and_tints_change_only_color() {
        let layer = |cover, field: &str| {
            format!(
                "[[ground.layers]]
color = \"c\"
normal = \"n\"
size = 1.0
                 terrain = [\"plains\"]
cover = {cover}
patch = 3.0
edge = 0.0
{field}
"
            )
        };
        let text = format!(
            "version = 1
{}{}{}",
            layer(1.0, ""),
            layer(0.4, "field = \"dry\""),
            layer(0.15, "field = \"dry\""),
        );
        let config = SceneryConfig::parse(&text).unwrap();
        let layers = &config.ground.as_ref().unwrap().layers;
        assert_eq!(layers[1].pattern, layers[2].pattern);
        assert_ne!(layers[0].pattern, layers[1].pattern);
        let rows = vec!["........................"; 24];
        let map = TerrainMap::from_rows(&rows);
        let [low, _] = ground_weights(&map, config.ground.as_ref().unwrap());
        // Wherever the small share lies, the large one lay first: the small one sits
        // inside it, and grass (layer 0) is gone there.
        let inside = low
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|texel| texel[2] > 200)
            .count();
        assert!(inside > 0);
        assert!(
            low.pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|texel| texel[2] > 200)
                .all(|texel| texel[0] < 30),
        );

        let mut image = RgbaImage::solid(2, 1, [188, 188, 188, 77]);
        adjust_color(&mut image, 1.0, [0.5, 1.0, 2.0]);
        // sRGB 188 is about half of its linear value at 0.5…
        assert!(
            (136..=138).contains(&image.pixels[0]),
            "{}",
            image.pixels[0]
        );
        assert_eq!(&image.pixels[1..4], &[188, 255, 77]);
        // Without saturation, color turns to a gray of the same brightness.
        let mut image = RgbaImage::solid(1, 1, [200, 40, 40, 255]);
        adjust_color(&mut image, 0.0, [1.0; 3]);
        let [r, g, b, _] = image.pixels[..] else {
            panic!("one pixel")
        };
        assert!(r == g && g == b && (90..=110).contains(&r), "{r} {g} {b}");
    }

    #[test]
    fn lands_share_out_the_map_with_wandering_blended_borders() {
        let text = "version = 1
[ground]
lands = [[6.0, 12.0], [18.0, 12.0]]
border = 1.5
[[ground.layers]]
color = \"c\"
normal = \"n\"
size = 1.0
terrain = [\"plains\"]
land = 1
[[ground.layers]]
color = \"c\"
normal = \"n\"
size = 1.0
terrain = [\"plains\"]
land = 2
glow = [1.0, 0.2, 0.0]
";
        let config = SceneryConfig::parse(text).unwrap();
        let ground = config.ground.as_ref().unwrap();
        assert_eq!(ground.lands, [Vec2::new(6.0, 12.0), Vec2::new(18.0, 12.0)]);
        assert_eq!(
            (ground.layers[0].land, ground.layers[1].land),
            (Some(0), Some(1))
        );
        assert_eq!(ground.layers[1].glow, [1.0, 0.2, 0.0]);
        let map = TerrainMap::from_rows(&vec!["........................"; 24]);
        let [low, _] = ground_weights(&map, ground);
        let side = low.width as usize;
        let at = |x: usize, y: usize| {
            let i = (y * side + x) * 4;
            [low.pixels[i], low.pixels[i + 1]].map(|c| f32::from(c) / 255.0)
        };
        // Deep in each land only its own layer lies; everywhere the two add up.
        assert!(at(2 * WEIGHT_TEXELS, 12 * WEIGHT_TEXELS)[0] > 0.98);
        assert!(at(22 * WEIGHT_TEXELS, 12 * WEIGHT_TEXELS)[1] > 0.98);
        let mut crossings = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let [a, b] = at(x, y);
                assert!((a + b - 1.0).abs() < 0.02, "{a} + {b} at {x}, {y}");
            }
            crossings.push((0..side).find(|&x| at(x, y)[1] > 0.5).unwrap_or(side));
        }
        // The border is not a straight line down the middle.
        let (left, right) = (
            crossings.iter().min().unwrap(),
            crossings.iter().max().unwrap(),
        );
        assert!(right - left >= WEIGHT_TEXELS, "{left}..{right}");
        // A blend, not a cut: some texels hold a fair share of both.
        assert!((0..side).any(|x| {
            let [a, b] = at(x, 5 * WEIGHT_TEXELS);
            a > 0.2 && b > 0.2
        }));
        for broken in ["land = 3", "land = 0"] {
            let text = text.replacen("land = 2", broken, 1);
            assert!(SceneryConfig::parse(&text).is_err(), "{broken}");
        }
    }

    #[test]
    fn hills_share_their_height_maps_and_need_one() {
        let text = "version = 1
[heights]
map = \"a.png\"
[[heights.hills]]
at = [4, 5]
[[heights.hills]]
at = [9, 9]
map = \"b.png\"
peak = 2
turn = 90
[[heights.hills]]
at = [1, 1]
map = \"a.png\"
";
        let heights = SceneryConfig::parse(text).unwrap().heights.unwrap();
        assert_eq!(heights.maps, ["a.png", "b.png"]);
        let maps: Vec<usize> = heights.hills.iter().map(|hill| hill.map).collect();
        assert_eq!(maps, [0, 1, 0]);
        let second = heights.hills[1];
        assert_eq!((second.at, second.peak), (Vec2::new(9.0, 9.0), 2.0));
        assert!((second.turn - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(heights.hills[0].size, default_hill_size());
        let no_map = "version = 1
[heights]
[[heights.hills]]
at = [4, 5]";
        assert!(SceneryConfig::parse(no_map).is_err());
        assert_eq!(
            SceneryConfig::parse(
                "version = 1
[heights]"
            )
            .unwrap()
            .heights,
            None
        );
    }

    #[test]
    fn the_ground_and_the_tile_frame_are_flat_models() {
        let ground = ground_model(
            &Relief::flat(48, 48),
            Vec2::new(48.0, 48.0),
            RgbaImage::solid(4, 4, [9; 4]),
        )
        .unwrap();
        let bounds = ground.bounds();
        assert!(bounds.min.abs_diff_eq(Vec3::ZERO, 1e-5));
        assert!(bounds.max.abs_diff_eq(Vec3::new(48.0, 0.0, 48.0), 1e-4));
        assert_eq!(ground.textures().len(), 1);
        let outline = outline_model().unwrap();
        assert!(
            outline
                .bounds()
                .max
                .abs_diff_eq(Vec3::new(1.0, 0.0, 1.0), 1e-5)
        );
        assert!(outline.vertices().iter().all(|v| v.texture_weight == 0.0));
    }
}
