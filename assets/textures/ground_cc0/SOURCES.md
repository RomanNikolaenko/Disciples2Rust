# Ground textures (CC0)

Converted by `examples/import_ground_textures.rs` from Poly Haven texture sets
(2K EXR: diffuse, OpenGL normal, ambient occlusion, displacement), downloaded on
2026-11-19 from <https://polyhaven.com>. `assets/maps/plain/scenery.toml` says which
surface covers which land; the others are kept for other maps.

License: **CC0** (public domain; see <https://polyhaven.com/license>). They may be
committed, changed and shipped freely; credit is appreciated, not required.

| Files | Poly Haven asset | Scanned area |
| --- | --- | --- |
| `aerial_grass_rock_*` | <https://polyhaven.com/a/aerial_grass_rock> | 15 × 15 m |
| `grass_ground_*` | <https://polyhaven.com/a/grass_ground> | 2.5 × 2.5 m |
| `forrest_ground_01_*` | <https://polyhaven.com/a/forrest_ground_01> | 2 × 2 m |
| `grass_path_2_*` | <https://polyhaven.com/a/grass_path_2> | 1 × 1 m |
| `dry_ground_rocks_*` | <https://polyhaven.com/a/dry_ground_rocks> | 4 × 4 m |
| `dirt_floor_*` | <https://polyhaven.com/a/dirt_floor> | 2.1 × 2.1 m |
| `coast_sand_01_*` | <https://polyhaven.com/a/coast_sand_01> | 15 × 15 m |
| `rocks_ground_02_*` | <https://polyhaven.com/a/rocks_ground_02> | 2 × 2 m |
| `forest_floor_*` | <https://polyhaven.com/a/forest_floor> | 2.1 × 2.1 m |
| `snow_02_*` | <https://polyhaven.com/a/snow_02> | 2 × 2 m |
| `leafy_grass_*` | <https://polyhaven.com/a/leafy_grass> | 2 × 2 m |

Each surface is `<name>_color.webp` (sRGB albedo, occlusion multiplied in) and
`<name>_normal.webp` (OpenGL normal; alpha: displacement stretched over 0..1, used as a
height where surfaces blend), 1024 × 1024, lossless.
