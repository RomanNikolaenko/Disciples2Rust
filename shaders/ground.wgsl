// The map's ground: photographed surfaces (grass, soil, sand…) tiled in world space and
// lit by the sun through their normal maps.
//
// Same vertices, instances and frame as the scenery (shaders/props.wgsl). Group 2 holds
// up to eight surface layers in texture arrays: color (sRGB albedo, occlusion baked in)
// and normal (OpenGL convention, green up the image; alpha: a height for blending), each
// layer with its own repeat size. Two weight maps over the map (an array, four layers per
// RGBA texel) say how much of each layer lies where; where they sum to zero (water) the
// ground's painted texture (group 1) shows, as before. A shade map over the same area as
// the painted texture darkens the ground under props (baked contact shadows). A layer
// may glow in its lowest places (lava in the cracks between stones), unlit by the sun.
//
// Tiling: each layer is sampled on a triangle grid (hex tiling, after Heitz & Neyret
// 2018): the three corners of the triangle around a point each show the texture shifted
// and turned at random, blended by closeness, so no repeat ever lines up. Each normal is
// turned back with its sample. Where layers meet, the higher surface wins (weight plus
// height), so stones poke through grass instead of fading into it.
//
// Cost: each pixel draws its two heaviest layers only, the second only where it lies at
// all (most of a meadow is one layer). Each of the two fades out as its weight nears the
// third heaviest, the same way for both, so no seam shows where any two trade places
// (except where all three weigh alike). A layer's reads
// are never split by branches: reads in separate branches wait for each other, which
// on a small integrated GPU made the ground three times slower.
//
// Relief: the ground mesh rises and falls (src/game/world/relief.rs); its vertex
// normals tilt each pixel's surface normal (whiteout blend), so slopes are lit by the
// sun. Textures stay projected from above. The hovered tile's frame is drawn here too,
// onto the surface itself, so it follows the slopes.
//
// Spaces: u runs along world +x and v along world +z. A normal map's (x, y) point along
// +u and up the image (-v); in (u, v, up) terms that is (x, -y, z), and in world terms
// (u, up, v) → (x, y, z).

const MAX_LAYERS: i32 = 8;
// Triangle grid corners per texture repeat, and how sharply their samples blend.
const GRID_SCALE: f32 = 2.0;
const GRID_SHARPNESS: f32 = 3.0;
// Where layers meet: how much height counts against weight, and how soft the edge is.
const HEIGHT_SHARE: f32 = 0.5;
const BLEND_SOFTNESS: f32 = 0.1;
// A drawn layer fades in over this much weight above the third heaviest.
const THIRD_FADE: f32 = 0.15;
// No local arrays below: indexed by a variable, some drivers keep them in slow memory.

struct Frame {
    clip_from_world: mat4x4<f32>,
    light_from_world: mat4x4<f32>,
    // xyz: unit vector towards the light, in the lighting frame; w: ambient level.
    light: vec4<f32>,
    surface: vec4<f32>,
    // x, y: the map's size along world x and z; z: fade width.
    map: vec4<f32>,
    // A square frame on the ground: x, y its corner (world x, z), z its side, w its line
    // width; square_color: rgb (linear, unlit), a 1 when shown.
    square: vec4<f32>,
    square_color: vec4<f32>,
}

struct Detail {
    // World units per repeat: layer i in sizes[i / 4][i % 4].
    sizes: array<vec4<f32>, 2>,
    // x: layer count; y: brightness; z: relief (scales the normals' slopes).
    params: vec4<f32>,
    // Layer i's glow in glows[i]: rgb its color (linear), a the height below which it
    // glows, the more the lower.
    glows: array<vec4<f32>, 8>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var paint: texture_2d<f32>;
@group(1) @binding(1) var paint_sampler: sampler;
@group(2) @binding(0) var weights: texture_2d_array<f32>;
@group(2) @binding(1) var map_sampler: sampler;
@group(2) @binding(2) var colors: texture_2d_array<f32>;
@group(2) @binding(3) var normals: texture_2d_array<f32>;
@group(2) @binding(4) var surface_sampler: sampler;
@group(2) @binding(5) var shade: texture_2d<f32>;
@group(2) @binding(6) var<uniform> detail: Detail;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) texture_weight: f32,
    @location(5) model_0: vec4<f32>,
    @location(6) model_1: vec4<f32>,
    @location(7) model_2: vec4<f32>,
    @location(8) model_3: vec4<f32>,
    @location(9) tint: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) tint: vec3<f32>,
    @location(3) normal: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let world_from_model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    let world = world_from_model * vec4<f32>(input.position, 1.0);
    var output: VertexOutput;
    output.clip = frame.clip_from_world * world;
    output.uv = input.uv;
    output.world = world.xyz;
    output.tint = input.color * input.tint.rgb;
    output.normal = (world_from_model * vec4<f32>(input.normal, 0.0)).xyz;
    return output;
}

/// A well-mixed 32-bit number from a grid point and a seed (integer hash: the same on
/// every GPU).
fn hash_bits(point: vec2<i32>, seed: u32) -> u32 {
    var h = bitcast<u32>(point.x) * 1597334673u ^ bitcast<u32>(point.y) * 3812015801u
        ^ seed * 2798796415u;
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    return h ^ (h >> 16u);
}

/// Two numbers in 0..1 from a grid point and a seed.
fn hash(point: vec2<i32>, seed: u32) -> vec2<f32> {
    let h = hash_bits(point, seed);
    return vec2<f32>(f32(h & 0xffffu), f32(h >> 16u)) / 65535.0;
}

/// Smooth noise in 0..1, one feature per unit.
fn value_noise(p: vec2<f32>, seed: u32) -> f32 {
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash(cell, seed).x;
    let b = hash(cell + vec2<i32>(1, 0), seed).x;
    let c = hash(cell + vec2<i32>(0, 1), seed).x;
    let d = hash(cell + vec2<i32>(1, 1), seed).x;
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

struct Surface {
    color: vec3<f32>,
    // (u, v, up); not normalized.
    normal: vec3<f32>,
    height: f32,
}

/// One tap of a layer for one grid corner: the texture turned and shifted at random
/// (the corner decides), `share` of the blend.
fn tap(
    layer: i32,
    corner: vec2<i32>,
    uv: vec2<f32>,
    gx: vec2<f32>,
    gy: vec2<f32>,
    share: f32,
) -> Surface {
    // One hash: 10 bits of turn, 11 bits each of shift.
    let bits = hash_bits(corner, u32(layer) * 2u + 1u);
    let angle = f32(bits & 0x3ffu) * (6.2831853 / 1024.0);
    let shift = vec2<f32>(f32((bits >> 10u) & 0x7ffu), f32(bits >> 21u)) / 2048.0;
    let c = cos(angle);
    let s = sin(angle);
    // Turns uv by `angle`; its transpose turns back.
    let turn = mat2x2<f32>(c, s, -s, c);
    let at = turn * uv + shift;
    let ddx = turn * gx;
    let ddy = turn * gy;
    let color = textureSampleGrad(colors, surface_sampler, at, layer, ddx, ddy).rgb;
    let texel = textureSampleGrad(normals, surface_sampler, at, layer, ddx, ddy);
    // The map's (x, up the image) in the turned (u, v) frame, turned back.
    let slope = transpose(turn) * vec2<f32>(texel.x * 2.0 - 1.0, 1.0 - texel.y * 2.0);
    return Surface(
        color * share,
        vec3<f32>(slope * detail.params.z, texel.z * 2.0 - 1.0) * share,
        texel.a * share,
    );
}

fn add(a: Surface, b: Surface) -> Surface {
    return Surface(a.color + b.color, a.normal + b.normal, a.height + b.height);
}

/// One layer at world point `world` (x, z), hex-tiled; `dx`, `dy`: the point's change
/// to the next pixel across and down (for mip selection inside non-uniform branches).
fn sample_layer(layer: i32, world: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> Surface {
    let size = max(detail.sizes[layer / 4][layer % 4], 1e-3);
    let uv = world / size;
    let gx = dx / size;
    let gy = dy / size;

    // The triangle of the skewed grid around uv: its corners and their closeness.
    let skewed = mat2x2<f32>(1.0, 0.0, -0.57735027, 1.15470054) * (uv * GRID_SCALE);
    let base = vec2<i32>(floor(skewed));
    let f = fract(skewed);
    let rest = 1.0 - f.x - f.y;
    let upper = rest <= 0.0;
    let c0 = select(base, base + vec2<i32>(1, 1), upper);
    let c1 = select(base + vec2<i32>(0, 1), base + vec2<i32>(1, 0), upper);
    let c2 = select(base + vec2<i32>(1, 0), base + vec2<i32>(0, 1), upper);
    let closeness = select(
        vec3<f32>(rest, f.y, f.x),
        vec3<f32>(-rest, 1.0 - f.y, 1.0 - f.x),
        upper,
    );
    // A plain average of three samples looks washed out: favor the nearest corner.
    var blend = pow(closeness, vec3<f32>(GRID_SHARPNESS));
    blend = blend / (blend.x + blend.y + blend.z);
    let a = tap(layer, c0, uv, gx, gy, blend.x);
    let b = tap(layer, c1, uv, gx, gy, blend.y);
    let c = tap(layer, c2, uv, gx, gy, blend.z);
    return add(add(a, b), c);
}

struct Pick {
    first: i32,
    second: i32,
    // The three heaviest weights, heaviest first.
    weights: vec3<f32>,
}

/// The two heaviest of the first `count` layers in `low` (layers 0..4) and `high`
/// (4..8).
fn heaviest_two(low: vec4<f32>, high: vec4<f32>, count: i32) -> Pick {
    var first = vec2<f32>(0.0, 0.0);
    var second = vec2<f32>(0.0, 0.0);
    var third = 0.0;
    for (var layer = 0; layer < MAX_LAYERS; layer++) {
        let w = select(high[layer & 3], low[layer & 3], layer < 4) * f32(layer < count);
        let candidate = vec2<f32>(f32(layer), w);
        if (w > first.y) {
            third = second.y;
            second = first;
            first = candidate;
        } else if (w > second.y) {
            third = second.y;
            second = candidate;
        } else if (w > third) {
            third = w;
        }
    }
    return Pick(i32(first.x), i32(second.x), vec3<f32>(first.y, second.y, third));
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let painted = textureSample(paint, paint_sampler, input.uv).rgb * input.tint;
    let map_size = max(frame.map.xy, vec2<f32>(1.0));
    let map_uv = input.world.xz / map_size;
    let low = textureSample(weights, map_sampler, map_uv, 0);
    let high = textureSample(weights, map_sampler, map_uv, 1);
    let shadow = textureSample(shade, map_sampler, input.uv).r;
    let dx = dpdx(input.world.xz);
    let dy = dpdy(input.world.xz);
    let pixel = max(length(dx), length(dy));

    let count = min(i32(detail.params.x), MAX_LAYERS);
    let coverage = clamp(dot(low, vec4<f32>(1.0)) + dot(high, vec4<f32>(1.0)), 0.0, 1.0);
    let pick = heaviest_two(low, high, count);
    let fade_a = max(smoothstep(0.0, THIRD_FADE, pick.weights.x - pick.weights.z), 1e-4);
    let fade_b = smoothstep(0.0, THIRD_FADE, pick.weights.y - pick.weights.z);
    let a = sample_layer(pick.first, input.world.xz, dx, dy);
    var b = a;
    if (fade_b > 0.0) {
        b = sample_layer(pick.second, input.world.xz, dx, dy);
    }
    // Weight plus height decides, softly: b's share is fade_b·e^score_b over the sum of
    // both such terms (a layer that is not here takes no part).
    let score_a = (pick.weights.x + HEIGHT_SHARE * a.height) / BLEND_SOFTNESS;
    let score_b = (pick.weights.y + HEIGHT_SHARE * b.height) / BLEND_SOFTNESS;
    let share_b = fade_b / (fade_b + fade_a * exp(score_a - score_b));
    let color = mix(a.color, b.color, share_b);
    let normal = mix(a.normal, b.normal, share_b);
    let glow_a = detail.glows[pick.first];
    let glow_b = detail.glows[pick.second];
    let lava_a = glow_a.rgb * (1.0 - smoothstep(0.0, glow_a.a, a.height));
    let lava_b = glow_b.rgb * (1.0 - smoothstep(0.0, glow_b.a, b.height));
    let glow = mix(lava_a, lava_b, share_b);

    var shaded = painted;
    if (coverage > 0.0) {
        let n_map = normalize(normal + vec3<f32>(0.0, 0.0, 1e-4));
        let detail_normal = vec3<f32>(n_map.x, n_map.z, n_map.y);
        // The relief's slope under the texture's own bumps (whiteout blend, +Y up).
        let slope = normalize(input.normal + vec3<f32>(0.0, 1e-4, 0.0));
        let world_normal = normalize(vec3<f32>(
            detail_normal.x + slope.x,
            detail_normal.y * slope.y,
            detail_normal.z + slope.z,
        ));
        let n = normalize((frame.light_from_world * vec4<f32>(world_normal, 0.0)).xyz);
        let ambient = frame.light.w;
        let lit = ambient + (1.0 - ambient) * max(dot(n, frame.light.xyz), 0.0);
        // Slow, broad changes of brightness, so a wide meadow does not look printed.
        let broad = value_noise(input.world.xz / 7.0, 7u) * 0.65
            + value_noise(input.world.xz / 2.3, 13u) * 0.35;
        let variation = mix(0.82, 1.15, broad);
        let ground = color * lit * variation * shadow * detail.params.y + glow;
        shaded = mix(painted, ground, coverage);
    }

    // The frame: lines along the square's inside edges, a pixel of smoothing.
    let local = input.world.xz - frame.square.xy;
    let side = frame.square.z;
    let within = all(local >= vec2<f32>(0.0)) && all(local <= vec2<f32>(side));
    let edge = min(min(local.x, local.y), min(side - local.x, side - local.y));
    let line = (1.0 - smoothstep(frame.square.w, frame.square.w + pixel, edge))
        * f32(within) * frame.square_color.a;
    shaded = mix(shaded, frame.square_color.rgb, line);

    var fade = 1.0;
    if (frame.map.x > 0.0 && frame.map.z > 0.0) {
        let inside = min(
            min(input.world.x, input.world.z),
            min(frame.map.x - input.world.x, frame.map.y - input.world.z),
        );
        fade = smoothstep(0.0, frame.map.z, inside);
    }
    return vec4<f32>(shaded * fade, 1.0);
}
