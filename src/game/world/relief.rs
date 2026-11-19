//! The map's relief: how high the ground stands at each point.
//!
//! The heights come from copies ("hills") of landscape height maps laid over the
//! otherwise flat map (`[heights]` in the map's `scenery.toml`): each copy, of its own
//! height map, is centered on
//! a map point, turned, scaled to a width in tiles, and its range stretched to a peak
//! height in tiles. Heights below the `floor` (a share of the height map's range) stay
//! plain, each copy sinks to nothing towards its rim and the whole relief towards the
//! map's edges, so the hills rise out of a plain; where copies overlap the higher wins.
//!
//! The relief is a grid of [`SAMPLES_PER_TILE`] heights per tile, read back with
//! bilinear filtering. The ground mesh takes its vertices from this grid, so whatever
//! stands on [`Relief::height`] stands exactly on the drawn ground.
//!
//! Units: world units are tiles; map point (x, y) is world (x, height, y).

use glam::{Vec2, Vec3};

use crate::engine::assets::HeightImage;

/// Height samples per tile, along x and along z.
pub const SAMPLES_PER_TILE: usize = 4;
/// Tiles from the map's edge over which the relief sinks to nothing.
const EDGE_CALM: f32 = 3.0;
/// Share of a copy's radius from which it starts sinking towards its rim.
const RIM: f32 = 0.7;

/// One copy of the height map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hill {
    /// Map point of its middle.
    pub at: Vec2,
    /// Tiles across.
    pub size: f32,
    /// Height of its highest point, tiles.
    pub peak: f32,
    /// Turn about +Y, radians.
    pub turn: f32,
    /// Index of its height map.
    pub map: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relief {
    /// Samples along x and z: the map's tiles times [`SAMPLES_PER_TILE`], plus one.
    columns: usize,
    rows: usize,
    /// Row by row (z), tiles.
    heights: Vec<f32>,
    highest: f32,
}

impl Relief {
    /// No relief: a `width` x `height` tile map at height 0.
    pub fn flat(width: usize, height: usize) -> Self {
        let (columns, rows) = (width * SAMPLES_PER_TILE + 1, height * SAMPLES_PER_TILE + 1);
        Self {
            columns,
            rows,
            heights: vec![0.0; columns * rows],
            highest: 0.0,
        }
    }

    /// `hills`, copies of `maps`, over a `width` x `height` tile map (see the module); a
    /// hill naming no map there stays flat.
    pub fn new(
        width: usize,
        height: usize,
        maps: &[HeightImage],
        floor: f32,
        hills: &[Hill],
    ) -> Self {
        let mut relief = Self::flat(width, height);
        let size = Vec2::new(width as f32, height as f32);
        let floor = floor.clamp(0.0, 0.99);
        for row in 0..relief.rows {
            for column in 0..relief.columns {
                let point = Vec2::new(column as f32, row as f32) / SAMPLES_PER_TILE as f32;
                let calm = smoothstep(0.0, EDGE_CALM, point.min(size - point).min_element());
                let height = hills
                    .iter()
                    .filter_map(|hill| Some(hill_height(hill, maps.get(hill.map)?, floor, point)))
                    .fold(0.0, f32::max);
                relief.heights[row * relief.columns + column] = height * calm;
            }
        }
        relief.highest = relief.heights.iter().copied().fold(0.0, f32::max);
        relief
    }

    pub fn is_flat(&self) -> bool {
        self.highest <= 0.0
    }

    /// The highest point, tiles.
    pub fn highest(&self) -> f32 {
        self.highest
    }

    /// Samples along x and along z.
    pub fn grid(&self) -> (usize, usize) {
        (self.columns, self.rows)
    }

    /// The height at grid sample (`column`, `row`), clamped to the grid.
    pub fn sample(&self, column: usize, row: usize) -> f32 {
        let (column, row) = (column.min(self.columns - 1), row.min(self.rows - 1));
        self.heights[row * self.columns + column]
    }

    /// The ground's height at world point (`x`, `z`); off the map, the nearest edge's.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let s = SAMPLES_PER_TILE as f32;
        let gx = (x * s).clamp(0.0, (self.columns - 1) as f32);
        let gz = (z * s).clamp(0.0, (self.rows - 1) as f32);
        let (column, row) = (gx.floor() as usize, gz.floor() as usize);
        let (fx, fz) = (gx - column as f32, gz - row as f32);
        let a = self.sample(column, row);
        let b = self.sample(column + 1, row);
        let c = self.sample(column, row + 1);
        let d = self.sample(column + 1, row + 1);
        let top = a + (b - a) * fx;
        let bottom = c + (d - c) * fx;
        top + (bottom - top) * fz
    }

    /// The ground's unit normal at world point (`x`, `z`).
    pub fn normal(&self, x: f32, z: f32) -> Vec3 {
        let step = 0.5 / SAMPLES_PER_TILE as f32;
        let dx = (self.height(x + step, z) - self.height(x - step, z)) / (2.0 * step);
        let dz = (self.height(x, z + step) - self.height(x, z - step)) / (2.0 * step);
        Vec3::new(-dx, 1.0, -dz).normalize()
    }

    /// Where the ray from `origin` along `forward` (pointing down) first meets the
    /// ground, the side facing the ray's start; `None` if the ray never comes down.
    pub fn hit(&self, origin: Vec3, forward: Vec3) -> Option<Vec3> {
        if forward.y >= -1e-6 {
            return None;
        }
        let above = |t: f32| {
            let point = origin + forward * t;
            point.y - self.height(point.x, point.z)
        };
        // From where the ray comes down to the highest point, walk along it in steps of
        // about a quarter sample until it is below the ground (by height 0 it must be),
        // then halve the last step until close enough.
        let start = (self.highest + 0.01 - origin.y) / forward.y;
        let end = -origin.y / forward.y;
        let step = 0.25 / SAMPLES_PER_TILE as f32 / forward.length();
        let mut before = start;
        let mut after = start;
        while after < end {
            after = (after + step).min(end);
            if above(after) <= 0.0 {
                break;
            }
            before = after;
        }
        for _ in 0..12 {
            let middle = (before + after) / 2.0;
            if above(middle) > 0.0 {
                before = middle;
            } else {
                after = middle;
            }
        }
        Some(origin + forward * after)
    }
}

/// One copy's height at map point `point` (see the module).
fn hill_height(hill: &Hill, map: &HeightImage, floor: f32, point: Vec2) -> f32 {
    let offset = point - hill.at;
    let radius = hill.size / 2.0;
    let distance = offset.length();
    if radius <= 0.0 || distance >= radius {
        return 0.0;
    }
    // Into the copy's own frame (turned back), then 0..1 across it.
    let (sin, cos) = hill.turn.sin_cos();
    let local = Vec2::new(
        cos * offset.x + sin * offset.y,
        -sin * offset.x + cos * offset.y,
    );
    let uv = local / hill.size + Vec2::splat(0.5);
    let lifted = ((map.sample(uv.x, uv.y) - floor) / (1.0 - floor)).max(0.0);
    let rim = 1.0 - smoothstep(RIM, 1.0, distance / radius);
    lifted * hill.peak * rim
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cone: 1 in the middle, 0 at the rim of the circle inscribed in the image.
    fn cone(side: u32) -> HeightImage {
        let values = (0..side * side)
            .map(|i| {
                let (x, y) = ((i % side) as f32 + 0.5, (i / side) as f32 + 0.5);
                let d = Vec2::new(x, y).distance(Vec2::splat(side as f32 / 2.0));
                (1.0 - d / (side as f32 / 2.0)).max(0.0)
            })
            .collect();
        HeightImage {
            width: side,
            height: side,
            values,
        }
    }

    fn hill(at: Vec2) -> Hill {
        Hill {
            at,
            size: 10.0,
            peak: 4.0,
            turn: 0.6,
            map: 0,
        }
    }

    #[test]
    fn hills_rise_from_a_plain_that_stays_flat_at_the_edges() {
        let relief = Relief::new(24, 24, &[cone(64)], 0.0, &[hill(Vec2::new(12.0, 12.0))]);
        assert!(
            (relief.height(12.0, 12.0) - 4.0).abs() < 0.2,
            "{}",
            relief.height(12.0, 12.0)
        );
        assert!(relief.highest() > 3.8 && relief.highest() <= 4.0);
        assert_eq!(relief.height(2.0, 2.0), 0.0, "outside the copy");
        assert_eq!(relief.height(17.5, 12.0), 0.0, "past its rim");
        // Up the slope towards the middle.
        assert!(relief.height(14.0, 12.0) > relief.height(15.0, 12.0));
        // The middle's normal is up; on the slope it leans away from the peak.
        assert!(relief.normal(12.0, 12.0).y > 0.95);
        assert!(relief.normal(14.0, 12.0).x > 0.3);
        // A hill by the map's edge sinks there.
        let edge = Relief::new(24, 24, &[cone(64)], 0.0, &[hill(Vec2::new(1.0, 12.0))]);
        assert_eq!(edge.height(0.0, 12.0), 0.0);
        // Below the floor stays plain; the top keeps its peak.
        let floored = Relief::new(24, 24, &[cone(64)], 0.5, &[hill(Vec2::new(12.0, 12.0))]);
        assert_eq!(floored.height(15.0, 12.0), 0.0);
        assert!((floored.height(12.0, 12.0) - 4.0).abs() < 0.3);
        assert!(Relief::flat(4, 4).is_flat() && !relief.is_flat());
        // Each hill rises from its own map; one naming a missing map stays flat.
        let two = [
            cone(64),
            HeightImage {
                width: 1,
                height: 1,
                values: vec![0.0],
            },
        ];
        let mut low = hill(Vec2::new(12.0, 12.0));
        low.map = 1;
        assert!(Relief::new(24, 24, &two, 0.0, &[low]).is_flat());
        low.map = 7;
        assert!(Relief::new(24, 24, &two, 0.0, &[low]).is_flat());
    }

    #[test]
    fn rays_hit_the_slope_facing_them_first() {
        let relief = Relief::new(24, 24, &[cone(64)], 0.0, &[hill(Vec2::new(12.0, 12.0))]);
        // Looking down at 45° along +x: the near (west) slope is met first.
        let forward = Vec3::new(1.0, -1.0, 0.0).normalize();
        let origin = Vec3::new(0.0, 12.0, 12.0);
        let hit = relief.hit(origin, forward).unwrap();
        assert!((hit.y - relief.height(hit.x, hit.z)).abs() < 1e-3, "{hit}");
        assert!(hit.x < 12.0 && hit.y > 0.5, "{hit}");
        // Over the plain, the ray meets height 0 where the flat ground would be.
        let plain = relief.hit(Vec3::new(-30.0, 20.0, 2.0), forward).unwrap();
        assert!(
            plain.y.abs() < 1e-3 && (plain.x + 10.0).abs() < 1e-2,
            "{plain}"
        );
        assert_eq!(relief.hit(origin, Vec3::X), None);
        // A flat relief is the plane.
        let flat = Relief::flat(24, 24).hit(origin, forward).unwrap();
        assert!(flat.abs_diff_eq(Vec3::new(12.0, 0.0, 12.0), 1e-3), "{flat}");
    }
}
