//! Terrain of a world map, loaded from `assets/maps/<name>/map.toml`.
//!
//! The file lists one row of characters per line of tiles, top to bottom:
//! `~` water, `.` plains, `f` forest, `h` hills, `^` mountains, `=` road, `b` bridge,
//! `w` fields, `#` village. World coordinates are in tiles: tile `(x, y)` covers
//! `x..x + 1` across and `y..y + 1` down.

use std::path::{Path, PathBuf};

use serde::Deserialize;

const MAP_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terrain {
    Water,
    Plains,
    Forest,
    Hills,
    Mountains,
    Road,
    /// A road over water.
    Bridge,
    /// Farmland.
    Fields,
    /// Houses and the ground between them.
    Village,
}

impl Terrain {
    pub const ALL: [Self; 9] = [
        Self::Water,
        Self::Plains,
        Self::Forest,
        Self::Hills,
        Self::Mountains,
        Self::Road,
        Self::Bridge,
        Self::Fields,
        Self::Village,
    ];

    fn from_char(c: char) -> Option<Self> {
        Some(match c {
            '~' => Self::Water,
            '.' => Self::Plains,
            'f' => Self::Forest,
            'h' => Self::Hills,
            '^' => Self::Mountains,
            '=' => Self::Road,
            'b' => Self::Bridge,
            'w' => Self::Fields,
            '#' => Self::Village,
            _ => return None,
        })
    }

    /// English name, also the translation key.
    pub fn name(self) -> &'static str {
        match self {
            Self::Water => "Water",
            Self::Plains => "Plains",
            Self::Forest => "Forest",
            Self::Hills => "Hills",
            Self::Mountains => "Mountains",
            Self::Road => "Road",
            Self::Bridge => "Bridge",
            Self::Fields => "Fields",
            Self::Village => "Village",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("cannot read map {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("map {path} is not valid TOML: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("map {path}: {problem}")]
    Invalid { path: PathBuf, problem: String },
}

#[derive(Deserialize)]
struct MapFile {
    version: u32,
    tiles: Vec<String>,
}

/// A rectangular grid of terrain.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainMap {
    width: usize,
    height: usize,
    /// Row-major, `width * height` tiles.
    tiles: Vec<Terrain>,
}

impl TerrainMap {
    pub fn load(path: &Path) -> Result<Self, MapError> {
        let text = std::fs::read_to_string(path).map_err(|source| MapError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&text).map_err(|error| match error {
            ParseError::Toml(source) => MapError::Parse {
                path: path.to_owned(),
                source,
            },
            ParseError::Invalid(problem) => MapError::Invalid {
                path: path.to_owned(),
                problem,
            },
        })
    }

    fn parse(text: &str) -> Result<Self, ParseError> {
        let file: MapFile = toml::from_str(text).map_err(ParseError::Toml)?;
        if file.version != MAP_VERSION {
            return Err(ParseError::Invalid(format!(
                "unsupported version {} (expected {MAP_VERSION})",
                file.version
            )));
        }
        let width = file.tiles.first().map_or(0, |row| row.chars().count());
        if width == 0 {
            return Err(ParseError::Invalid("the map has no tiles".into()));
        }
        let mut tiles = Vec::with_capacity(width * file.tiles.len());
        for (y, row) in file.tiles.iter().enumerate() {
            if row.chars().count() != width {
                return Err(ParseError::Invalid(format!(
                    "row {y} is not {width} tiles wide"
                )));
            }
            for (x, c) in row.chars().enumerate() {
                let terrain = Terrain::from_char(c).ok_or_else(|| {
                    ParseError::Invalid(format!("unknown tile {c:?} at column {x}, row {y}"))
                })?;
                tiles.push(terrain);
            }
        }
        Ok(Self {
            width,
            height: file.tiles.len(),
            tiles,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The tile at `(x, y)`, or `None` outside the map.
    pub fn get(&self, x: usize, y: usize) -> Option<Terrain> {
        (x < self.width && y < self.height).then(|| self.tiles[y * self.width + x])
    }

    /// The tile of kind `terrain` closest to `from` (straight-line distance between tile
    /// centers; ties go to the first in reading order), if the map has one.
    pub fn nearest(&self, from: (usize, usize), terrain: Terrain) -> Option<(usize, usize)> {
        self.by_distance(from, terrain).into_iter().next()
    }

    /// Every tile of `terrain`, nearest to `from` first (straight-line distance; ties in
    /// reading order).
    pub fn by_distance(&self, from: (usize, usize), terrain: Terrain) -> Vec<(usize, usize)> {
        let distance = |(x, y): (usize, usize)| {
            let dx = x.abs_diff(from.0);
            let dy = y.abs_diff(from.1);
            dx * dx + dy * dy
        };
        let mut tiles: Vec<(usize, usize)> = (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .filter(|&(x, y)| self.tiles[y * self.width + x] == terrain)
            .collect();
        // Stable: equal distances keep reading order.
        tiles.sort_by_key(|&tile| distance(tile));
        tiles
    }

    /// A map from rows of tile characters (as in `map.toml`), for tests.
    #[cfg(test)]
    pub fn from_rows(rows: &[&str]) -> Self {
        let tiles = rows
            .iter()
            .map(|row| format!("\"{row}\""))
            .collect::<Vec<_>>()
            .join(", ");
        match Self::parse(&format!("version = 1\ntiles = [{tiles}]")) {
            Ok(map) => map,
            Err(_) => panic!("broken test map {rows:?}"),
        }
    }

    /// The tile containing world point (`x`, `y`) (tile units), if it is on the map.
    pub fn tile_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        if x.is_nan() || y.is_nan() || x < 0.0 || y < 0.0 {
            return None;
        }
        let (column, row) = (x.floor() as usize, y.floor() as usize);
        (column < self.width && row < self.height).then_some((column, row))
    }
}

enum ParseError {
    Toml(toml::de::Error),
    Invalid(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<TerrainMap, String> {
        TerrainMap::parse(text).map_err(|error| match error {
            ParseError::Toml(error) => error.to_string(),
            ParseError::Invalid(problem) => problem,
        })
    }

    #[test]
    fn reads_a_grid_of_terrain() {
        let map = parse("version = 1\ntiles = [\"~.f=\", \"h^b#\", \"ww..\"]").unwrap();
        assert_eq!((map.width(), map.height()), (4, 3));
        assert_eq!(map.get(3, 0), Some(Terrain::Road));
        assert_eq!(map.get(2, 1), Some(Terrain::Bridge));
        assert_eq!(map.get(3, 1), Some(Terrain::Village));
        assert_eq!(map.get(0, 2), Some(Terrain::Fields));
        assert_eq!(map.get(0, 0), Some(Terrain::Water));
        assert_eq!(map.get(2, 0), Some(Terrain::Forest));
        assert_eq!(map.get(1, 1), Some(Terrain::Mountains));
        assert_eq!(map.get(4, 0), None);
        assert_eq!(map.get(0, 3), None);
    }

    #[test]
    fn rejects_broken_maps() {
        assert!(
            parse("version = 2\ntiles = [\"..\"]")
                .unwrap_err()
                .contains("version")
        );
        assert!(
            parse("version = 1\ntiles = []")
                .unwrap_err()
                .contains("no tiles")
        );
        assert!(
            parse("version = 1\ntiles = [\"...\", \"..\"]")
                .unwrap_err()
                .contains("row 1")
        );
        let unknown = parse("version = 1\ntiles = [\".x\"]").unwrap_err();
        assert!(
            unknown.contains("'x'") && unknown.contains("column 1"),
            "{unknown}"
        );
        assert!(parse("tiles = ").is_err());
    }

    #[test]
    fn world_points_map_to_tiles() {
        let map = parse("version = 1\ntiles = [\"...\", \"...\"]").unwrap();
        assert_eq!(map.tile_at(0.0, 0.0), Some((0, 0)));
        assert_eq!(map.tile_at(2.99, 1.5), Some((2, 1)));
        assert_eq!(map.tile_at(3.0, 0.5), None);
        assert_eq!(map.tile_at(-0.01, 0.5), None);
        assert_eq!(map.tile_at(f32::NAN, 0.5), None);
    }

    #[test]
    fn finds_the_nearest_tile_of_a_kind() {
        let map = parse(
            "version = 1
tiles = [\"~~~.\", \"~f~~\", \".~~~\"]",
        )
        .unwrap();
        assert_eq!(map.nearest((1, 1), Terrain::Forest), Some((1, 1)));
        // Of the two plains, (0, 2) is nearer to the forest than (3, 0).
        assert_eq!(map.nearest((1, 1), Terrain::Plains), Some((0, 2)));
        assert_eq!(map.nearest((3, 1), Terrain::Plains), Some((3, 0)));
        assert_eq!(map.nearest((0, 0), Terrain::Mountains), None);
        assert_eq!(map.by_distance((3, 1), Terrain::Plains), [(3, 0), (0, 2)]);
        assert!(map.by_distance((0, 0), Terrain::Hills).is_empty());
    }

    #[test]
    fn the_continent_map_loads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/maps/start/map.toml");
        let map = TerrainMap::load(&path).unwrap();
        assert!(map.width() >= 16 && map.height() >= 16);
        let land = (0..map.height())
            .flat_map(|y| (0..map.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| map.get(x, y) != Some(Terrain::Water))
            .count();
        assert!(
            land * 4 > map.width() * map.height(),
            "mostly land, some sea"
        );
    }

    /// The map "New game" opens: an empty plain, 48x48.
    #[test]
    fn the_plain_map_loads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/maps/plain/map.toml");
        let map = TerrainMap::load(&path).unwrap();
        assert_eq!((map.width(), map.height()), (48, 48));
        let plains = (0..48)
            .flat_map(|y| (0..48).map(move |x| (x, y)))
            .all(|(x, y)| map.get(x, y) == Some(Terrain::Plains));
        assert!(plains);
    }

    #[test]
    fn terrain_names_are_translated() {
        for terrain in Terrain::ALL {
            assert!(
                crate::game::locale::is_translated(terrain.name()),
                "{terrain:?}"
            );
        }
    }
}
