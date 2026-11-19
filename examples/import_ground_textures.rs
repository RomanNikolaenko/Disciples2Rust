//! Turns scanned ground surfaces (EXR texture sets from Megascans or Poly Haven) into
//! the game's ground textures: a color map (sRGB, the ambient-occlusion map multiplied
//! in) and a normal map (OpenGL convention: green points up the image; alpha: a height
//! the ground shader uses where surfaces blend), both `side` x `side` lossless WebP.
//!
//! The source sets stay where they are; only the converted files are written. Each set
//! is a folder of EXRs, told apart by name (any case): the color (`*_BaseColor.exr`,
//! `*_diff_*.exr`, `*_col_*.exr`), the OpenGL normal (`*_Normal.exr`, `*_nor_gl_*.exr`)
//! and, optionally, the occlusion (`*_AO.exr`, `*_ao_*.exr`) and displacement
//! (`*_Displacement.exr`, `*_disp_*.exr`, `*_height_*.exr`). The height is the displacement, or else the occlusion (high on what
//! sticks out, low in the gaps), stretched over 0..1 so that every surface's heights
//! compare. The output is named after the folder (`wild_grass_pjwgw0_4k_exr` →
//! `wild_grass_pjwgw0`).
//!
//! Usage:
//!   cargo run --release --example import_ground_textures -- convert <sets folder> <output folder> [side]
//!   cargo run --release --example import_ground_textures -- probe <set folder>
//!   cargo run --release --example import_ground_textures -- height <height.exr> <out.png> [side]
//!
//! `height` turns a landscape height map (such as ambientCG's Terrain sets) into a
//! 16-bit grayscale PNG, its lowest point 0 and highest 65535, for the map's relief.
//!
//! `probe` prints what the conversion relies on: the maps' levels and, from a
//! displacement map, which way the normal map's green points.
//!
//! What the probe found: Megascans color EXRs hold linear albedo, but Poly Haven's
//! (`*_diff_*.exr`, `*_col_*.exr`) hold sRGB-encoded values, the same as its JPGs (aerial_grass_rock:
//! mean 0.447, 0.381, 0.143 in both). The normal maps are the usual 0.5·n + 0.5 in the
//! OpenGL convention (Poly Haven's `nor_gl`; its `nor_dx` is the other one, not read).
//! Poly Haven's displacement is one luminance channel (`Y`). Shrinking uses
//! [`shrink`]'s plain block average: the image crate's `thumbnail` adds half a step to
//! float images (rounding meant for integers).

use std::path::{Path, PathBuf};

use image::{ImageBuffer, Rgb32FImage};

/// How strongly the ambient-occlusion map darkens the color (0..1).
const AO_STRENGTH: f32 = 0.6;
const DEFAULT_SIDE: u32 = 1024;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("probe") if args.len() == 2 => probe(Path::new(&args[1])),
        Some("height") if args.len() >= 3 => {
            let side = args.get(3).map_or(Ok(DEFAULT_SIDE), |s| s.parse())?;
            height_map(Path::new(&args[1]), Path::new(&args[2]), side)
        }
        Some("convert") if args.len() >= 3 => {
            let side = args.get(3).map_or(Ok(DEFAULT_SIDE), |s| s.parse())?;
            convert_all(Path::new(&args[1]), Path::new(&args[2]), side)
        }
        _ => {
            Err("usage: import_ground_textures (probe <set> | convert <sets> <out> [side]                  | height <exr> <png> [side])"
                .into())
        }
    }
}

/// The maps of a texture set, by the name parts that mark them (Megascans, Poly Haven).
#[derive(Debug, Clone, Copy)]
enum Map {
    Color,
    Normal,
    Occlusion,
    Displacement,
}

impl Map {
    fn marks(self) -> &'static [&'static str] {
        match self {
            Map::Color => &["_basecolor.", "_diff_", "_col_"],
            Map::Normal => &["_normal.", "_nor_gl_"],
            Map::Occlusion => &["_ao.", "_ao_"],
            Map::Displacement => &["_displacement.", "_disp_", "_height_"],
        }
    }
}

/// Whether a color map's values are already sRGB-encoded (Poly Haven's) rather than
/// linear (Megascans').
fn srgb_encoded(color: &Path) -> bool {
    color
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            name.contains("_diff_") || name.contains("_col_")
        })
}

/// The EXR in `set` holding `map`.
fn find(set: &Path, map: Map) -> Option<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(set)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths.into_iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(str::to_ascii_lowercase)
            .is_some_and(|name| {
                name.ends_with(".exr") && map.marks().iter().any(|mark| name.contains(mark))
            })
    })
}

fn require(set: &Path, map: Map) -> Result<PathBuf, String> {
    find(set, map).ok_or_else(|| format!("{}: no {map:?} map", set.display()))
}

/// The first channel of `image`, stretched so its 2nd..98th percentiles span 0..1.
fn stretched(image: &Rgb32FImage) -> Vec<f32> {
    let values: Vec<f32> = image.pixels().map(|pixel| pixel[0]).collect();
    let mut sorted = values.clone();
    sorted.sort_by(f32::total_cmp);
    let at = |share: f32| sorted[((sorted.len() - 1) as f32 * share) as usize];
    let (low, high) = (at(0.02), at(0.98));
    let span = (high - low).max(1e-6);
    values
        .iter()
        .map(|value| ((value - low) / span).clamp(0.0, 1.0))
        .collect()
}

fn load(path: &Path) -> Result<Rgb32FImage, Box<dyn std::error::Error>> {
    let started = std::time::Instant::now();
    let image = read_exr(path)?;
    println!(
        "  read {} ({}x{}, {:.1}s)",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
        image.width(),
        image.height(),
        started.elapsed().as_secs_f32()
    );
    Ok(image)
}

/// An EXR's first layer as RGB: channels whose names end in R, G and B (any prefix, such
/// as "BaseColor.R"), or else a luminance channel (Y) or a single channel as gray.
fn read_exr(path: &Path) -> Result<Rgb32FImage, Box<dyn std::error::Error>> {
    use exr::prelude::*;
    let image = read()
        .no_deep_data()
        .largest_resolution_level()
        .all_channels()
        .first_valid_layer()
        .all_attributes()
        .from_file(path)?;
    let layer = &image.layer_data;
    let size = layer.size;
    let channels = &layer.channel_data.list;
    let names: Vec<String> = channels.iter().map(|c| c.name.to_string()).collect();
    let find = |suffix: &str| {
        names
            .iter()
            .position(|name| name == suffix || name.ends_with(&format!(".{suffix}")))
    };
    let picks: [usize; 3] = match (find("R"), find("G"), find("B"), find("Y")) {
        (Some(r), Some(g), Some(b), _) => [r, g, b],
        (_, _, _, Some(y)) => [y; 3],
        _ if channels.len() == 1 => [0, 0, 0],
        _ => return Err(format!("{}: no RGB channels in {names:?}", path.display()).into()),
    };
    let samples: Vec<Vec<f32>> = picks
        .iter()
        .map(|&index| channels[index].sample_data.values_as_f32().collect())
        .collect();
    let (width, height) = (size.width() as u32, size.height() as u32);
    Ok(ImageBuffer::from_fn(width, height, |x, y| {
        let i = (y * width + x) as usize;
        image::Rgb([samples[0][i], samples[1][i], samples[2][i]])
    }))
}

/// Averaged down to `side` x `side` over whole blocks (the side must divide the
/// image's).
fn shrink(image: Rgb32FImage, side: u32) -> Rgb32FImage {
    let (width, height) = image.dimensions();
    if width == side && height == side {
        return image;
    }
    let (bx, by) = ((width / side).max(1), (height / side).max(1));
    let count = (bx * by) as f32;
    ImageBuffer::from_fn(side, side, |x, y| {
        let mut sum = [0.0f32; 3];
        for dy in 0..by {
            for dx in 0..bx {
                let pixel =
                    image.get_pixel((x * bx + dx).min(width - 1), (y * by + dy).min(height - 1));
                for c in 0..3 {
                    sum[c] += pixel[c];
                }
            }
        }
        image::Rgb(sum.map(|c| c / count))
    })
}

fn mean(image: &Rgb32FImage) -> [f32; 3] {
    let mut sum = [0.0f64; 3];
    for pixel in image.pixels() {
        for c in 0..3 {
            sum[c] += f64::from(pixel[c]);
        }
    }
    let n = f64::from(image.width() * image.height()).max(1.0);
    sum.map(|s| (s / n) as f32)
}

fn probe(set: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let color = shrink(load(&require(set, Map::Color)?)?, 512);
    println!(
        "color mean {:?} (linear albedo is ~0.05-0.25; sRGB-encoded ~0.3-0.5)",
        mean(&color)
    );
    let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
    for pixel in color.pixels() {
        for c in 0..3 {
            low[c] = low[c].min(pixel[c]);
            high[c] = high[c].max(pixel[c]);
        }
    }
    println!("color range {low:?} .. {high:?}");
    let normal = shrink(load(&require(set, Map::Normal)?)?, 512);
    println!(
        "normal mean {:?} (encoded 0..1: ~[0.5, 0.5, 1])",
        mean(&normal)
    );
    let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
    for pixel in normal.pixels() {
        for c in 0..3 {
            low[c] = low[c].min(pixel[c]);
            high[c] = high[c].max(pixel[c]);
        }
    }
    println!("normal range {low:?} .. {high:?}");
    for (x, y) in [(10, 10), (200, 300), (400, 50)] {
        println!("normal at {x},{y}: {:?}", normal.get_pixel(x, y).0);
    }
    let Some(height) = find(set, Map::Displacement) else {
        println!("no displacement map: normal convention not checked");
        return Ok(());
    };
    let height = shrink(load(&height)?, 512);
    // Height rising down the image: an OpenGL normal leans its green up (+), DirectX down.
    let (w, h) = (height.width(), height.height());
    let mut correlation = 0.0f64;
    for y in 1..h - 1 {
        for x in 0..w {
            let rise = height.get_pixel(x, y + 1)[0] - height.get_pixel(x, y - 1)[0];
            correlation += f64::from(rise * (normal.get_pixel(x, y)[1] - 0.5));
        }
    }
    let convention = if correlation > 0.0 {
        "OpenGL (green up)"
    } else {
        "DirectX (green down)"
    };
    println!("green vs. height rising down the image: {correlation:.3} → {convention}");
    Ok(())
}

fn convert_all(sets: &Path, out: &Path, side: u32) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;
    let mut folders: Vec<PathBuf> = std::fs::read_dir(sets)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    for set in folders {
        let folder = set.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        // "wild_grass_pjwgw0_4k_exr" → "wild_grass_pjwgw0".
        let name = folder
            .trim_end_matches("_exr")
            .trim_end_matches("_8k")
            .trim_end_matches("_4k")
            .trim_end_matches("_2k")
            .to_string();
        println!("{name}:");
        convert(&set, out, &name, side)?;
    }
    Ok(())
}

fn convert(
    set: &Path,
    out: &Path,
    name: &str,
    side: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let color_path = require(set, Map::Color)?;
    // Linear values either way: occlusion multiplies, and shrinking averages, in light.
    let decode = srgb_encoded(&color_path);
    let mut color = load(&color_path)?;
    if decode {
        for pixel in color.pixels_mut() {
            pixel.0 = pixel.0.map(from_srgb);
        }
    }
    let color = shrink(color, side);
    let normal_path = require(set, Map::Normal)?;
    let ao = match find(set, Map::Occlusion) {
        Some(path) => Some(shrink(load(&path)?, side)),
        None => None,
    };
    let height = match find(set, Map::Displacement) {
        Some(path) => Some(stretched(&shrink(load(&path)?, side))),
        None => ao.as_ref().map(stretched),
    };
    // Occlusion multiplies in linear light, then the result is encoded to sRGB.
    let color_out: ImageBuffer<image::Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_fn(side, side, |x, y| {
            let occlusion = ao.as_ref().map_or(1.0, |ao| ao.get_pixel(x, y)[0]);
            let shade = 1.0 - AO_STRENGTH * (1.0 - occlusion.clamp(0.0, 1.0));
            let pixel = color.get_pixel(x, y);
            image::Rgb([0, 1, 2].map(|c| to_srgb8(pixel[c] * shade)))
        });
    let normal = shrink(load(&normal_path)?, side);
    // Averaging shortens normals: renormalize, then encode as 0.5·n + 0.5. The sources
    // are OpenGL (see `probe`), as the game expects. Alpha carries the height (opaque
    // without one).
    let normal_out: ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_fn(side, side, |x, y| {
            let p = normal.get_pixel(x, y);
            let mut n = [p[0] * 2.0 - 1.0, p[1] * 2.0 - 1.0, p[2] * 2.0 - 1.0];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
            n = n.map(|c| c / length);
            let [r, g, b] = n.map(|c| ((c * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8);
            let h = height
                .as_ref()
                .map_or(1.0, |height| height[(y * side + x) as usize]);
            image::Rgba([r, g, b, (h * 255.0).round() as u8])
        });
    write_webp(
        &out.join(format!("{name}_color.webp")),
        color_out.as_raw(),
        side,
        image::ExtendedColorType::Rgb8,
    )?;
    write_webp(
        &out.join(format!("{name}_normal.webp")),
        normal_out.as_raw(),
        side,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(())
}

/// A height map EXR → `side` x `side` 16-bit grayscale PNG spanning its full range.
fn height_map(source: &Path, out: &Path, side: u32) -> Result<(), Box<dyn std::error::Error>> {
    let heights = shrink(load(source)?, side);
    let (low, high) = heights
        .pixels()
        .fold((f32::MAX, f32::MIN), |(low, high), p| {
            (low.min(p[0]), high.max(p[0]))
        });
    let span = (high - low).max(1e-6);
    println!("  heights {low} .. {high}");
    let png: ImageBuffer<image::Luma<u16>, Vec<u16>> = ImageBuffer::from_fn(side, side, |x, y| {
        let h = (heights.get_pixel(x, y)[0] - low) / span;
        image::Luma([(h.clamp(0.0, 1.0) * 65535.0).round() as u16])
    });
    if let Some(folder) = out.parent() {
        std::fs::create_dir_all(folder)?;
    }
    png.save(out)?;
    println!(
        "  wrote {} ({} KB)",
        out.display(),
        std::fs::metadata(out)?.len() / 1024
    );
    Ok(())
}

fn write_webp(
    path: &Path,
    pixels: &[u8],
    side: u32,
    layout: image::ExtendedColorType,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    image::codecs::webp::WebPEncoder::new_lossless(file).encode(pixels, side, side, layout)?;
    println!(
        "  wrote {} ({} KB)",
        path.display(),
        std::fs::metadata(path)?.len() / 1024
    );
    Ok(())
}

fn from_srgb(encoded: f32) -> f32 {
    let c = encoded.clamp(0.0, 1.0);
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb8(linear: f32) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let encoded = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}
