# Terrain height maps (CC0)

| File | Source | Size |
| --- | --- | --- |
| `terrain001_height.png` | ambientCG Terrain001 (<https://ambientcg.com/view?id=Terrain001>), 2K height map EXR, downloaded 2026-11-19 | ca. 5 × 5 km, 2 km high (procedural, Gaea) |
| `terrain002_height.png` | ambientCG Terrain002 (<https://ambientcg.com/view?id=Terrain002>), 2K height map EXR, downloaded 2026-11-19 | procedural landscape |
| `terrain003_height.png` | ambientCG Terrain003 (<https://ambientcg.com/view?id=Terrain003>), 2K height map EXR, downloaded 2026-11-19 | procedural landscape |
| `terrain004_height.png` | ambientCG Terrain004 (<https://ambientcg.com/view?id=Terrain004>), 2K height map EXR, downloaded 2026-11-19 | procedural landscape |
| `terrain005_height.png` | ambientCG Terrain005 (<https://ambientcg.com/view?id=Terrain005>), 2K height map EXR, downloaded 2026-11-19 | procedural landscape |

License: **CC0** (<https://docs.ambientcg.com/license/>): free to use, change and
redistribute.

Each converted by `cargo run --release --example import_ground_textures -- height
<TerrainNNN_2K.exr> assets/textures/terrain/terrainNNN_height.png 1024`: 1024 × 1024,
16-bit grayscale, its lowest point black and highest white (Terrain004's own range,
about half of the EXR's, is stretched like the others). The map's relief uses it
through `[heights]` in a map's `scenery.toml`.
