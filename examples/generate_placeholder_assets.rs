//! Generates placeholder art: menu pieces into `assets/ui/` and the world maps into
//! `assets/maps/<name>/`.
//!
//! Everything here is procedural and original; the files are meant to be replaced by real
//! art with the same names, sizes and channel conventions:
//!
//! - `menu_panel_shade.webp`  512x8 RGBA, black with a horizontal alpha falloff.
//! - `slider_knob.webp`       64x64 RGBA: metal diamond frame around a teal gem.
//! - `slider_knob_glow.webp`  64x64 grayscale, linear: the gem and a soft halo.
//! - `ornament_divider.webp`  512x26 RGBA: gold line with a center diamond.
//! - `brush_underline.webp`   512x24 RGBA: orange brush stroke fading to the right.
//! - `selector_arrow.webp`    32x48 RGBA: bronze chevron pointing left (mirrored for the
//!   right arrow).
//! - `assets/maps/<name>/map.toml`: a map's terrain grid, laid out only when missing (it
//!   is meant to be edited by hand). `start` is a small continent (48x36); `plain` an
//!   empty plain (48x48) that "New game" opens.
//! - `assets/maps/<name>/terrain.webp`: the grid painted from above (40 texels per tile
//!   for `start`, 32 for `plain`), RGB; repainted from `map.toml` on every run.
//!
//! The button art (`button_base.webp`, `button_glow_mask.webp`) and the menu background
//! painting (`menu_background.png`) are real art and are not touched by this tool.
//!
//! Run with: `cargo run --release --example generate_placeholder_assets`

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use image::ExtendedColorType;
use image::codecs::webp::WebPEncoder;

const OUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ui");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = Path::new(OUT_DIR);
    std::fs::create_dir_all(out)?;

    let (w, h) = (512, 8);
    write_webp(
        &out.join("menu_panel_shade.webp"),
        &panel_shade(w, h),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;

    let size = 64;
    let (knob, knob_glow) = slider_knob(size);
    write_webp(
        &out.join("slider_knob.webp"),
        &knob,
        size,
        size,
        ExtendedColorType::Rgba8,
    )?;
    write_webp(
        &out.join("slider_knob_glow.webp"),
        &knob_glow,
        size,
        size,
        ExtendedColorType::L8,
    )?;

    let (w, h) = (512, 26);
    write_webp(
        &out.join("ornament_divider.webp"),
        &ornament_divider(w, h),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    let (w, h) = (512, 24);
    write_webp(
        &out.join("brush_underline.webp"),
        &brush_underline(w, h),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    let (w, h) = (32, 48);
    write_webp(
        &out.join("selector_arrow.webp"),
        &selector_arrow(w, h),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    world_maps()?;
    Ok(())
}

fn write_webp(
    path: &Path,
    data: &[u8],
    width: u32,
    height: u32,
    color: ExtendedColorType,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = BufWriter::new(File::create(path)?);
    WebPEncoder::new_lossless(file).encode(data, width, height, color)?;
    println!(
        "wrote {} ({} bytes)",
        path.display(),
        std::fs::metadata(path)?.len()
    );
    Ok(())
}

/// Black with alpha fading from dense on the left to clear on the right.
fn panel_shade(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for _row in 0..height {
        for column in 0..width {
            let u = (column as f32 + 0.5) / width as f32;
            let alpha = 0.93 * (1.0 - smoothstep(0.45, 1.0, u));
            out.extend_from_slice(&[0, 0, 0, to_u8(alpha * 255.0)]);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// World maps: each has a terrain grid (`map.toml`, hand-editable, written only when
// missing) and a painted placeholder of it (`terrain.webp`, always repainted from the
// grid).

const MAPS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/maps");

/// A map this tool lays out and paints.
struct MapSpec {
    /// Folder under `assets/maps`.
    name: &'static str,
    /// Texels per tile edge in `terrain.webp`; the game derives it from the image size
    /// divided by the grid size.
    tile_texels: usize,
    /// Lays out a new grid (only when `map.toml` is missing).
    layout: fn() -> Vec<String>,
    /// First comment line of a new `map.toml`.
    title: &'static str,
    /// Paint only the ground: the game stands 3D trees, crags and houses on it, so
    /// forests show their floor, the village its trodden earth and mountains bare rock.
    ground_only: bool,
}

const MAPS: [MapSpec; 2] = [
    MapSpec {
        name: "start",
        tile_texels: 40,
        layout: continent_tiles,
        title: "A small continent (48x36).",
        ground_only: false,
    },
    MapSpec {
        name: "plain",
        tile_texels: 32,
        layout: plain_tiles,
        title: "An empty plain (48x48): the map \"New game\" opens.",
        ground_only: true,
    },
];

/// Tile characters, as the game reads them.
const WATER: char = '~';
const PLAINS: char = '.';
const FOREST: char = 'f';
const HILLS: char = 'h';
const MOUNTAINS: char = '^';
const ROAD: char = '=';
const FIELDS: char = 'w';
const VILLAGE: char = '#';
const BRIDGE: char = 'b';
const TERRAINS: [char; 9] = [
    WATER, PLAINS, FOREST, HILLS, MOUNTAINS, ROAD, FIELDS, VILLAGE, BRIDGE,
];
/// Indices into `TERRAINS`.
const T_PLAINS: usize = 1;
const T_ROAD: usize = 5;
const T_FIELDS: usize = 6;
const T_VILLAGE: usize = 7;
const T_BRIDGE: usize = 8;

#[derive(serde::Deserialize)]
struct MapFile {
    version: u32,
    tiles: Vec<String>,
}

fn world_maps() -> Result<(), Box<dyn std::error::Error>> {
    for spec in &MAPS {
        world_map(spec)?;
    }
    Ok(())
}

fn world_map(spec: &MapSpec) -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(MAPS_DIR).join(spec.name);
    std::fs::create_dir_all(&dir)?;
    let map_path = dir.join("map.toml");
    if map_path.exists() {
        println!("kept {} (hand edits are preserved)", map_path.display());
    } else {
        std::fs::write(&map_path, map_toml(spec.title, &(spec.layout)()))?;
        println!("wrote {}", map_path.display());
    }
    let file: MapFile = toml::from_str(&std::fs::read_to_string(&map_path)?)?;
    if file.version != 1 {
        return Err(format!("unsupported map version {}", file.version).into());
    }
    let grid = parse_grid(&file.tiles)?;
    let texels = spec.tile_texels;
    let (width, height) = (grid[0].len() * texels, grid.len() * texels);
    write_webp(
        &dir.join("terrain.webp"),
        &paint_terrain(&grid, texels, spec.ground_only),
        width as u32,
        height as u32,
        ExtendedColorType::Rgb8,
    )?;
    Ok(())
}

fn map_toml(title: &str, rows: &[String]) -> String {
    let mut text = format!(
        "# {title} One character per tile, rows top to bottom:\n\
         #   ~ water   . plains   f forest   h hills   ^ mountains\n\
         #   = road   b bridge   w fields   # village\n\
         # After editing, repaint the placeholder art with\n\
         # `cargo run --release --example generate_placeholder_assets`.\n\
         version = 1\n\
         tiles = [\n"
    );
    for row in rows {
        text.push_str(&format!("    \"{row}\",\n"));
    }
    text.push_str("]\n");
    text
}

fn parse_grid(rows: &[String]) -> Result<Vec<Vec<usize>>, Box<dyn std::error::Error>> {
    let width = rows.first().map_or(0, |row| row.chars().count());
    if width == 0 {
        return Err("the map has no tiles".into());
    }
    rows.iter()
        .enumerate()
        .map(|(j, row)| {
            if row.chars().count() != width {
                return Err(format!("map row {j} is not {width} tiles wide").into());
            }
            row.chars()
                .map(|c| {
                    TERRAINS
                        .iter()
                        .position(|&t| t == c)
                        .ok_or_else(|| format!("unknown tile {c:?} in row {j}").into())
                })
                .collect()
        })
        .collect()
}

fn count_tiles(rows: &[Vec<char>]) -> [usize; 9] {
    let mut counts = [0usize; 9];
    for &tile in rows.iter().flatten() {
        counts[TERRAINS.iter().position(|&t| t == tile).unwrap_or(0)] += 1;
    }
    counts
}

// The starting continent ----------------------------------------------------------------

/// Tiles across and down of the continent.
const CONTINENT_TILES: [usize; 2] = [48, 36];
const CONTINENT_SEED: u32 = 2026;

/// A continent: noise heights falling off towards the edges (water all around), with
/// moisture deciding between forest and open plains.
fn continent_tiles() -> Vec<String> {
    let [w, h] = CONTINENT_TILES;
    let aspect = w as f32 / h as f32;
    let rows: Vec<Vec<char>> = (0..h)
        .map(|j| {
            (0..w)
                .map(|i| {
                    let u = (i as f32 + 0.5) / w as f32;
                    let v = (j as f32 + 0.5) / h as f32;
                    let (dx, dy) = ((u - 0.5) * 2.0, (v - 0.5) * 2.0);
                    let edge = (dx * dx + dy * dy).sqrt();
                    let height = fbm(u * 3.0 * aspect, v * 3.0, 5, CONTINENT_SEED)
                        - 0.5 * smoothstep(0.62, 1.1, edge)
                        + 0.15;
                    let moisture = fbm(u * 4.0 * aspect, v * 4.0, 4, CONTINENT_SEED + 11);
                    if height < 0.38 {
                        WATER
                    } else if height > 0.66 {
                        MOUNTAINS
                    } else if height > 0.58 {
                        HILLS
                    } else if moisture > 0.53 {
                        FOREST
                    } else {
                        PLAINS
                    }
                })
                .collect()
        })
        .collect();
    println!(
        "generated continent: {:?} tiles of {TERRAINS:?}",
        count_tiles(&rows)
    );
    rows.into_iter().map(String::from_iter).collect()
}

// The plain -----------------------------------------------------------------------------

/// Tiles across and down of the plain.
const PLAIN_TILES: usize = 48;

/// An empty plain, to fill with scenery later.
fn plain_tiles() -> Vec<String> {
    vec![PLAINS.to_string().repeat(PLAIN_TILES); PLAIN_TILES]
}

// Painting -------------------------------------------------------------------------------

/// Terrains that blend into each other on the ground; roads are painted over them.
const BLEND_KINDS: usize = 7;
const B_WATER: usize = 0;
const B_FIELDS: usize = 5;
const B_VILLAGE: usize = 6;

/// Runs `texel(column, row)` for every texel of a `width` x `height` image, rows spread
/// over the machine's cores.
fn par_texels<T: Send + Default + Clone>(
    width: usize,
    height: usize,
    texel: impl Fn(usize, usize) -> T + Sync,
) -> Vec<T> {
    let mut out = vec![T::default(); width * height];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows_per_chunk = height.div_ceil(threads * 4).max(1);
    {
        let chunks: Vec<(usize, &mut [T])> =
            out.chunks_mut(rows_per_chunk * width).enumerate().collect();
        let chunks = std::sync::Mutex::new(chunks);
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let next = chunks.lock().ok().and_then(|mut queue| queue.pop());
                        let Some((index, chunk)) = next else {
                            break;
                        };
                        for (k, value) in chunk.iter_mut().enumerate() {
                            *value = texel(k % width, index * rows_per_chunk + k / width);
                        }
                    }
                });
            }
        });
    }
    out
}

/// Paints the grid seen from above, `texels` per tile: soft, irregular borders between
/// terrains, grass patches, tree crowns, rocky ridges with snow, shallows along the
/// coast, patchwork fields with furrows and hedges, village houses with lit and shaded
/// roofs and cast shadows, dirt roads with ruts, plank bridges, and light from the upper
/// left over the relief. RGB.
fn paint_terrain(grid: &[Vec<usize>], texels: usize, ground_only: bool) -> Vec<u8> {
    let (tiles_w, tiles_h) = (grid[0].len(), grid.len());
    let (width, height) = (tiles_w * texels, tiles_h * texels);
    let texel = 1.0 / texels as f32;
    // What each tile blends as: roads as the ground under them, bridges as water.
    let blend: Vec<Vec<usize>> = (0..tiles_h)
        .map(|j| {
            (0..tiles_w)
                .map(|i| match grid[j][i] {
                    T_BRIDGE => B_WATER,
                    T_ROAD => T_PLAINS,
                    T_FIELDS => B_FIELDS,
                    T_VILLAGE => B_VILLAGE,
                    kind => kind,
                })
                .collect()
        })
        .collect();

    // Terrain weights at a point (tile units): bilinear between tile centers on
    // noise-warped coordinates, sharpened so borders are crisp but irregular.
    let weights_at = |x: f32, y: f32| -> [f32; BLEND_KINDS] {
        let warp_x = 0.7 * (fbm(x * 0.9, y * 0.9, 3, 501) - 0.5)
            + 0.35 * (fbm(x * 3.1, y * 3.1, 2, 505) - 0.5);
        let warp_y = 0.7 * (fbm(x * 0.9 + 7.3, y * 0.9, 3, 502) - 0.5)
            + 0.35 * (fbm(x * 3.1 + 2.9, y * 3.1, 2, 506) - 0.5);
        let (qx, qy) = (x + warp_x - 0.5, y + warp_y - 0.5);
        let (i0, j0) = (qx.floor(), qy.floor());
        let (fx, fy) = (qx - i0, qy - j0);
        let mut w = [0.0f32; BLEND_KINDS];
        for (di, dj, k) in [
            (0, 0, (1.0 - fx) * (1.0 - fy)),
            (1, 0, fx * (1.0 - fy)),
            (0, 1, (1.0 - fx) * fy),
            (1, 1, fx * fy),
        ] {
            let i = (i0 as i64 + di).clamp(0, tiles_w as i64 - 1) as usize;
            let j = (j0 as i64 + dj).clamp(0, tiles_h as i64 - 1) as usize;
            w[blend[j][i]] += k;
        }
        let mut sum = 0.0;
        for value in &mut w {
            *value = value.powi(3);
            sum += *value;
        }
        w.map(|value| value / sum)
    };

    // Pass 1: elevation (for the light) and how much water, per texel.
    let pass1: Vec<(f32, f32)> = par_texels(width, height, |column, row| {
        let (x, y) = ((column as f32 + 0.5) * texel, (row as f32 + 0.5) * texel);
        let w = weights_at(x, y);
        let rough = fbm(x * 2.2, y * 2.2, 4, 503);
        let peaks = smoothstep(0.3, 0.8, fbm(x * 1.6, y * 1.6, 4, 504));
        let e = w[1] * (0.22 + 0.05 * rough)
            + w[2] * (0.28 + 0.06 * rough)
            + w[3] * (0.42 + 0.12 * rough)
            + w[4] * (0.6 + 0.35 * peaks)
            + w[B_FIELDS] * (0.22 + 0.03 * rough)
            + w[B_VILLAGE] * 0.23;
        (e, w[B_WATER])
    });

    // Pass 2: color.
    let pixels: Vec<[u8; 3]> = par_texels(width, height, |column, row| {
        let index = row * width + column;
        let w = weights_at((column as f32 + 0.5) * texel, (row as f32 + 0.5) * texel);
        let (x, y) = ((column as f32 + 0.5) * texel, (row as f32 + 0.5) * texel);
        let e = pass1[index].0;
        let left = pass1[row * width + column.saturating_sub(1)].0;
        let up = pass1[row.saturating_sub(1) * width + column].0;
        // Light from the upper left: brighter where the ground rises to the right/down.
        let slope = ((e - left) + (e - up)) * texels as f32;
        let light = (1.0 + 0.6 * slope).clamp(0.7, 1.25);

        let patches = fbm(x * 0.8, y * 0.8, 4, 510);
        let fine = fbm(x * 9.0, y * 9.0, 2, 511);
        let plains = if ground_only {
            // Olive grass worn through to brown earth in places.
            let worn = smoothstep(0.45, 0.75, fbm(x * 1.7, y * 1.7, 4, 516));
            scale3(
                mix3(
                    mix3([82.0, 88.0, 44.0], [108.0, 104.0, 56.0], patches),
                    [112.0, 92.0, 62.0],
                    0.55 * worn,
                ),
                0.88 + 0.24 * fine,
            )
        } else {
            scale3(
                mix3([92.0, 112.0, 56.0], [136.0, 138.0, 74.0], patches),
                0.9 + 0.2 * fine,
            )
        };
        // Tree crowns: bright tops, dark gaps.
        let crowns = fbm(x * 5.5, y * 5.5, 3, 512);
        let crown_light = fbm(x * 5.5 - 0.06, y * 5.5 - 0.06, 3, 512);
        let mut forest = mix3(
            [24.0, 42.0, 24.0],
            [58.0, 88.0, 44.0],
            smoothstep(0.38, 0.7, crowns),
        );
        forest = scale3(
            forest,
            0.85 + 0.35 * (crown_light - crowns + 0.5).clamp(0.0, 1.0),
        );
        let hills = if ground_only {
            scale3(
                mix3([96.0, 86.0, 56.0], [124.0, 108.0, 74.0], patches),
                0.9 + 0.2 * fine,
            )
        } else {
            scale3(
                mix3([104.0, 108.0, 62.0], [138.0, 124.0, 82.0], patches),
                0.9 + 0.2 * fine,
            )
        };
        let rock = mix3([96.0, 92.0, 86.0], [140.0, 134.0, 124.0], fine);
        let snow = smoothstep(0.88, 0.95, e + 0.02 * fine);
        let mountains = if ground_only {
            // Scree between the crags.
            scale3(
                mix3([78.0, 74.0, 66.0], [112.0, 106.0, 96.0], fine),
                0.9 + 0.2 * patches,
            )
        } else {
            mix3(rock, [228.0, 230.0, 234.0], snow)
        };
        if ground_only {
            // The forest floor: dark earth and moss under the trees.
            forest = scale3(
                mix3(
                    [56.0, 50.0, 32.0],
                    [50.0, 68.0, 36.0],
                    smoothstep(0.35, 0.7, crowns),
                ),
                0.9 + 0.2 * fine,
            );
        }
        // Water: shallow near land, deeper further out, faint waves.
        let land = 1.0 - w[B_WATER];
        let near_land = smoothstep(
            0.0,
            0.5,
            land_nearby(&pass1, width, height, column, row, texels),
        );
        let waves = fbm(x * 3.0, y * 12.0, 2, 513);
        let (deep, shallow) = if ground_only {
            // Clear turquoise.
            ([22.0, 92.0, 110.0], [64.0, 160.0, 160.0])
        } else {
            ([28.0, 52.0, 82.0], [58.0, 104.0, 120.0])
        };
        let water = scale3(mix3(deep, shallow, near_land), 0.94 + 0.12 * waves);
        let fields = field_color(x, y, fine);
        // Village ground: trampled earth between patches of grass.
        let village = mix3(
            plains,
            scale3([132.0, 116.0, 86.0], 0.9 + 0.2 * fine),
            0.45 + 0.35 * patches,
        );

        let mut color = [0.0f32; 3];
        for (k, terrain) in [water, plains, forest, hills, mountains, fields, village]
            .iter()
            .enumerate()
        {
            for c in 0..3 {
                color[c] += w[k] * terrain[c];
            }
        }
        // Sandy (or, beside 3D scenery, stony) shore where land meets water.
        let shore = (1.0 - (2.0 * land - 1.0).abs()) * smoothstep(0.1, 0.9, land + w[1]);
        let beach = if ground_only {
            [128.0, 118.0, 92.0]
        } else {
            [176.0, 164.0, 116.0]
        };
        color = mix3(color, beach, 0.55 * shore);

        // Houses (over village ground) and their shadows.
        if !ground_only && w[B_VILLAGE] > 0.05 {
            match house_at(grid, x, y) {
                Some(roof) => color = mix3(color, roof, w[B_VILLAGE].sqrt()),
                None => {
                    if house_at(grid, x - 0.07, y - 0.07).is_some() {
                        color = scale3(color, 1.0 - 0.38 * w[B_VILLAGE]);
                    }
                }
            }
        }
        // Roads and bridges.
        if let Some(distance) = road_distance(grid, x, y) {
            let on_water = w[B_WATER] > 0.5;
            let half = if on_water {
                0.22
            } else {
                0.19 + 0.04 * (fbm(x * 2.0, y * 2.0, 2, 514) - 0.5)
            };
            let edge = 1.5 * texel;
            let cover = smoothstep(half + edge, half - edge, distance);
            if cover > 0.0 {
                let surface = if on_water {
                    // Planks across the bridge, dark rails along its sides.
                    let planks = 0.85 + 0.25 * fbm(x * 30.0, y * 30.0, 1, 515);
                    let rail = smoothstep(half - 0.06, half - 0.03, distance);
                    mix3(
                        scale3([118.0, 86.0, 54.0], planks),
                        [58.0, 40.0, 26.0],
                        rail,
                    )
                } else {
                    // Dirt with two ruts.
                    let rut = 1.0 - 0.18 * smoothstep(0.03, 0.0, (distance - half * 0.5).abs());
                    scale3([136.0, 108.0, 72.0], (0.92 + 0.16 * fine) * rut)
                };
                color = mix3(color, surface, cover);
            }
        }

        if land > 0.05 {
            color = scale3(color, 1.0 + (light - 1.0) * land);
        }
        [to_u8(color[0]), to_u8(color[1]), to_u8(color[2])]
    });
    pixels.into_iter().flatten().collect()
}

/// How much land is around a texel (0 open sea, 1 land), from a few samples about half a
/// tile away; shades the shallows.
fn land_nearby(
    pass1: &[(f32, f32)],
    width: usize,
    height: usize,
    column: usize,
    row: usize,
    texels: usize,
) -> f32 {
    let reach = texels as isize;
    let mut land = 0.0;
    let mut samples = 0.0;
    for (dx, dy) in [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (1, 1),
        (-1, 1),
        (1, -1),
    ] {
        let x = (column as isize + dx * reach / 2).clamp(0, width as isize - 1) as usize;
        let y = (row as isize + dy * reach / 2).clamp(0, height as isize - 1) as usize;
        land += 1.0 - pass1[y * width + x].1;
        samples += 1.0;
    }
    land / samples
}

/// Patchwork fields at a point (tile units): plots of about 2.4 x 1.6 tiles, each wheat,
/// green crops, ploughed earth or fallow, with furrows along the plot and hedges between
/// plots.
fn field_color(x: f32, y: f32, fine: f32) -> [f32; 3] {
    let column = (x / 2.4).floor();
    let shift = hash(column as i32, 0, 520) * 1.6;
    let row = ((y + shift) / 1.6).floor();
    let (ci, cj) = (column as i32, row as i32);
    let crop = hash(ci, cj, 521);
    let base = if crop < 0.35 {
        [192.0, 166.0, 88.0]
    } else if crop < 0.62 {
        [110.0, 136.0, 58.0]
    } else if crop < 0.85 {
        [118.0, 92.0, 60.0]
    } else {
        [150.0, 146.0, 94.0]
    };
    let along_x = hash(ci, cj, 522) < 0.5;
    let furrow = if along_x { y } else { x } * 9.0;
    let stripes = 0.9 + 0.1 * (furrow * std::f32::consts::TAU).sin();
    // Distance to the plot's border, in tiles.
    let (fx, fy) = (x / 2.4 - column, (y + shift) / 1.6 - row);
    let border = (fx.min(1.0 - fx) * 2.4).min(fy.min(1.0 - fy) * 1.6);
    let hedge = smoothstep(0.07, 0.03, border + 0.03 * (fine - 0.5));
    mix3(
        scale3(base, stripes * (0.94 + 0.12 * fine)),
        [50.0, 70.0, 36.0],
        hedge,
    )
}

/// A house: center, half extents (tiles) and roof color; its ridge runs along the longer
/// side.
struct House {
    center: [f32; 2],
    half: [f32; 2],
    roof: [f32; 3],
}

/// The houses of village tile (i, j): one or two, placed from the tile's hash.
fn houses(i: usize, j: usize) -> impl Iterator<Item = House> {
    let (hi, hj) = (i as i32, j as i32);
    let roofs = [
        [176.0, 146.0, 84.0], // thatch
        [150.0, 74.0, 54.0],  // red tiles
        [96.0, 98.0, 108.0],  // slate
        [124.0, 92.0, 64.0],  // wooden shingles
    ];
    (0..2).filter_map(move |k| {
        let h = |salt: u32| hash(hi, hj, 600 + k * 10 + salt);
        if h(0) > [0.9, 0.45][k as usize] {
            return None;
        }
        let long = [0.28, 0.18][k as usize] + 0.1 * h(1);
        let short = [0.17, 0.13][k as usize] + 0.05 * h(2);
        let half = if h(3) < 0.5 {
            [long, short]
        } else {
            [short, long]
        };
        // Anywhere in its tile, but not over its edge (a road may run beside it).
        let place = |axis: usize, roll: f32| {
            let room = (0.48 - half[axis]).max(0.0);
            [i, j][axis] as f32 + 0.5 + room * (roll * 2.0 - 1.0)
        };
        let center = [place(0, h(4)), place(1, h(5))];
        let roof = roofs[(h(6) * roofs.len() as f32) as usize % roofs.len()];
        Some(House { center, half, roof })
    })
}

/// The roof color at a point (tile units), lit on the side facing the light (upper
/// left), shaded on the other, with dark eaves and a bright ridge; `None` off any roof.
fn house_at(grid: &[Vec<usize>], x: f32, y: f32) -> Option<[f32; 3]> {
    let (ti, tj) = (x.floor() as i64, y.floor() as i64);
    for j in tj - 1..=tj + 1 {
        for i in ti - 1..=ti + 1 {
            if j < 0 || i < 0 || j as usize >= grid.len() || i as usize >= grid[0].len() {
                continue;
            }
            if grid[j as usize][i as usize] != T_VILLAGE {
                continue;
            }
            for house in houses(i as usize, j as usize) {
                let (dx, dy) = (x - house.center[0], y - house.center[1]);
                if dx.abs() > house.half[0] || dy.abs() > house.half[1] {
                    continue;
                }
                // Across the ridge: the coordinate perpendicular to the longer side.
                let (across, half_across, along, half_along) = if house.half[0] >= house.half[1] {
                    (dy, house.half[1], dx, house.half[0])
                } else {
                    (dx, house.half[0], dy, house.half[1])
                };
                let lit = if across < 0.0 { 1.12 } else { 0.76 };
                let toward_ridge = 1.0 - across.abs() / half_across;
                let mut shade = lit * (0.92 + 0.12 * toward_ridge);
                if across.abs() < 0.015 {
                    shade = 1.2;
                }
                if half_across - across.abs() < 0.025 || half_along - along.abs() < 0.025 {
                    shade *= 0.62;
                }
                return Some(scale3(house.roof, shade));
            }
        }
    }
    None
}

/// Distance (tiles) from a point to the road network (roads and bridges) near it: a
/// segment joins the centers of every two neighboring road tiles (diagonal neighbors only
/// when no road tile links them orthogonally), and roads on the map's edge run off it.
fn road_distance(grid: &[Vec<usize>], x: f32, y: f32) -> Option<f32> {
    let (w, h) = (grid[0].len() as i64, grid.len() as i64);
    let is_road = |i: i64, j: i64| {
        i >= 0
            && j >= 0
            && i < w
            && j < h
            && matches!(grid[j as usize][i as usize], T_ROAD | T_BRIDGE)
    };
    let (ti, tj) = (x.floor() as i64, y.floor() as i64);
    let mut best: Option<f32> = None;
    let mut consider = |a: [f32; 2], b: [f32; 2]| {
        let d = if a == b {
            ((x - a[0]).powi(2) + (y - a[1]).powi(2)).sqrt()
        } else {
            segment_distance([x, y], a, b)
        };
        best = Some(best.map_or(d, |current: f32| current.min(d)));
    };
    for j in tj - 1..=tj + 1 {
        for i in ti - 1..=ti + 1 {
            if !is_road(i, j) {
                continue;
            }
            let center = [i as f32 + 0.5, j as f32 + 0.5];
            let mut linked = false;
            for (di, dj) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                let (ni, nj) = (i + di, j + dj);
                let diagonal = di != 0 && dj != 0;
                let joined = if diagonal {
                    is_road(ni, nj) && !is_road(i + di, j) && !is_road(i, j + dj)
                } else {
                    is_road(ni, nj)
                };
                // Off the map's edge: the road continues out of view.
                let leaves = !diagonal && (ni < 0 || nj < 0 || ni >= w || nj >= h);
                if joined || leaves {
                    linked = true;
                    consider(center, [ni as f32 + 0.5, nj as f32 + 0.5]);
                }
            }
            if !linked {
                consider(center, center);
            }
        }
    }
    best
}

// ---------------------------------------------------------------------------------------
// Small UI ornaments. Shapes use signed distances in pixels (negative inside) and an
// antialiased coverage of one pixel.

const GOLD: [f32; 3] = [215.0, 193.0, 163.0];

fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

/// Distance to a diamond outline of "radius" `r` (center to corner) and width `width`.
fn diamond_outline(dx: f32, dy: f32, r: f32, width: f32) -> f32 {
    ((dx.abs() + dy.abs() - r) / std::f32::consts::SQRT_2).abs() - width / 2.0
}

/// Distance to a filled diamond of "radius" `r`.
fn diamond_fill(dx: f32, dy: f32, r: f32) -> f32 {
    (dx.abs() + dy.abs() - r) / std::f32::consts::SQRT_2
}

fn push_rgba(out: &mut Vec<u8>, color: [f32; 3], alpha: f32) {
    out.extend_from_slice(&[
        to_u8(color[0]),
        to_u8(color[1]),
        to_u8(color[2]),
        to_u8(alpha * 255.0),
    ]);
}

fn slider_knob(size: u32) -> (Vec<u8>, Vec<u8>) {
    let center = size as f32 / 2.0;
    let outer = center - 2.0;
    let gem = outer * 0.52;
    let mut base = Vec::with_capacity((size * size * 4) as usize);
    let mut glow = Vec::with_capacity((size * size) as usize);
    for row in 0..size {
        for column in 0..size {
            let dx = column as f32 + 0.5 - center;
            let dy = row as f32 + 0.5 - center;
            let alpha = coverage(diamond_fill(dx, dy, outer));
            // Metal frame lit from the top-left, with a bright bevel along the edge.
            let light = (0.5 - (dx + dy) / (4.0 * outer)).clamp(0.0, 1.0);
            let mut metal = mix3([52.0, 48.0, 40.0], [150.0, 140.0, 118.0], light);
            let bevel = coverage(diamond_outline(dx, dy, outer - 2.5, 1.2));
            metal = mix3(metal, [190.0, 180.0, 160.0], 0.6 * bevel * light);
            let rim = coverage(diamond_outline(dx, dy, gem + 1.5, 1.6));
            metal = mix3(metal, [25.0, 22.0, 18.0], rim);
            // Faceted teal gem: the upper-left facets catch the light.
            let gem_cover = coverage(diamond_fill(dx, dy, gem));
            let facet = if dx + dy < 0.0 { 1.0 } else { 0.62 } * if dx < dy { 1.0 } else { 0.85 };
            let gem_color = mix3([20.0, 110.0, 112.0], [150.0, 255.0, 250.0], facet);
            push_rgba(&mut base, mix3(metal, gem_color, gem_cover), alpha);

            let halo = (-diamond_fill(dx, dy, gem).max(0.0) / 5.0).exp() * 0.55;
            glow.push(to_u8(gem_cover.max(halo) * 255.0));
        }
    }
    (base, glow)
}

fn ornament_divider(width: u32, height: u32) -> Vec<u8> {
    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for column in 0..width {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let (dx, dy) = (x - cx, y - cy);
            let outline = diamond_outline(dx, dy, 10.0, 1.6);
            let core = diamond_fill(dx, dy, 4.0);
            let rings = [-36.0, 36.0]
                .map(|offset| (((dx - offset).powi(2) + dy.powi(2)).sqrt() - 3.5).abs() - 0.7);
            let gap = dx.abs() > 16.0 && (dx.abs() - 36.0).abs() > 5.0;
            let line = if gap { dy.abs() - 0.75 } else { f32::MAX };
            let d = outline.min(core).min(rings[0]).min(rings[1]).min(line);
            // Fade the line towards both ends.
            let edge = 1.0 - (dx.abs() / cx);
            let fade = smoothstep(0.0, 0.25, edge);
            push_rgba(&mut out, GOLD, coverage(d) * fade);
        }
    }
    out
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1])).clamp(0.0, 1.0);
    let closest = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - closest[0]).powi(2) + (p[1] - closest[1]).powi(2)).sqrt()
}

/// Bronze chevron pointing left, with a dark rim and a small barb at the tip.
fn selector_arrow(width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as f32, height as f32);
    let tip = [6.0, h / 2.0];
    let ends = [[w - 7.0, 6.0], [w - 7.0, h - 6.0]];
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for column in 0..width {
            let p = [column as f32 + 0.5, row as f32 + 0.5];
            let arms = segment_distance(p, tip, ends[0]).min(segment_distance(p, tip, ends[1]));
            let barb = segment_distance(p, [tip[0] - 2.0, tip[1]], [tip[0] + 6.0, tip[1]]);
            let d = arms.min(barb + 0.5) - 3.2;
            let rim = coverage(d);
            let core = coverage(d + 1.4);
            let light = 1.0 - p[1] / h;
            let bronze = mix3([92.0, 60.0, 30.0], [168.0, 118.0, 62.0], light);
            let color = mix3([30.0, 20.0, 10.0], bronze, core);
            push_rgba(&mut out, color, rim);
        }
    }
    out
}

fn brush_underline(width: u32, height: u32) -> Vec<u8> {
    let cy = height as f32 / 2.0;
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for column in 0..width {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let u = x / width as f32;
            // Stroke tapers from left to right; ragged edges and bristle streaks.
            let half = 1.0 + 5.0 * (1.0 - u).powf(0.7);
            let wobble = (fbm(x / 40.0, 3.0, 3, 41) - 0.5) * 3.0;
            let edge_noise = (fbm(x / 6.0, y / 3.0, 2, 43) - 0.5) * 2.5;
            let d = (y - cy - wobble).abs() - half - edge_noise;
            let streaks = 0.7 + 0.3 * fbm(x / 9.0, y * 1.7, 2, 47);
            let fade = 1.0 - smoothstep(0.55, 1.0, u);
            let start = smoothstep(0.0, 0.02, u);
            let alpha = coverage(d) * streaks * fade * start;
            let tone = 0.85 + 0.3 * fbm(x / 20.0, y / 4.0, 2, 53);
            push_rgba(&mut out, scale3([196.0, 106.0, 44.0], tone), alpha);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Helpers

fn to_u8(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn scale3(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d)
        ^ (y as u32).wrapping_mul(0x1656_67b1)
        ^ seed.wrapping_mul(0x9e37_79b9);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 / 16_777_215.0
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let a = hash(ix, iy, seed);
    let b = hash(ix + 1, iy, seed);
    let c = hash(ix, iy + 1, seed);
    let d = hash(ix + 1, iy + 1, seed);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

/// Fractal value noise in roughly `[0, 1]`.
fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amplitude, mut frequency, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for octave in 0..octaves {
        sum += amplitude * value_noise(x * frequency, y * frequency, seed + octave);
        norm += amplitude;
        amplitude *= 0.5;
        frequency *= 2.03;
    }
    sum / norm
}
