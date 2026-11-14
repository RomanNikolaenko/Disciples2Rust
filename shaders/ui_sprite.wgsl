// UI sprite material: textured quads in physical pixel coordinates.
//
// Blend/color conventions:
// - `base_texture` holds premultiplied alpha and is sampled from an sRGB view, so values
//   arrive linear. The output is premultiplied: blend state is (One, OneMinusSrcAlpha).
// - `glow_mask` is linear single-channel data marking the crystals (its solid part; a
//   soft falloff around them is ignored). Hover light stays inside the crystals' opaque
//   pixels: nothing glows past them. Hit testing uses the base alpha on the CPU.
// - Instance parameters arrive per vertex, so one draw call covers many sprites that
//   share textures while each keeps its own hover/opacity values.
// - `clip` (min x, min y, max x, max y in physical pixels) zeroes fragments outside it
//   (scrolling lists). Multiplying instead of `discard` keeps texture sampling in
//   uniform control flow.
// - `gamma` adjusts the game image in linear light (1 = unchanged); the interface
//   always uses 1.

struct Globals {
    screen_size: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

@group(1) @binding(0) var base_texture: texture_2d<f32>;
@group(1) @binding(1) var glow_mask: texture_2d<f32>;
@group(1) @binding(2) var linear_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    // rgb: linear glow color, a: glow intensity.
    @location(2) glow: vec4<f32>,
    // x: hover_amount, y: opacity, z: press_amount, w: disabled_amount (all 0..1).
    @location(3) params: vec4<f32>,
    // Linear RGBA multiplier, straight alpha. White = unchanged.
    @location(4) tint: vec4<f32>,
    @location(5) clip: vec4<f32>,
    @location(6) gamma: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) glow: vec4<f32>,
    @location(2) params: vec4<f32>,
    @location(3) tint: vec4<f32>,
    @location(4) clip: vec4<f32>,
    @location(5) gamma: f32,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    let ndc = vec2<f32>(
        input.position.x / globals.screen_size.x * 2.0 - 1.0,
        1.0 - input.position.y / globals.screen_size.y * 2.0,
    );
    output.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    output.uv = input.uv;
    output.glow = input.glow;
    output.params = input.params;
    output.tint = input.tint;
    output.clip = input.clip;
    output.gamma = input.gamma;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(base_texture, linear_sampler, input.uv);
    let mask = textureSample(glow_mask, linear_sampler, input.uv).r;
    let hover = input.params.x;
    let opacity = input.params.y;
    let press = input.params.z;
    let disabled = input.params.w;
    // The crystal itself is the solid part of the mask (~0.85); the falloff around it is
    // lower and must not light up.
    let crystal = smoothstep(0.7, 0.84, mask);

    // Tint a premultiplied sample: color by tint.rgb * tint.a, coverage by tint.a. Hover
    // lifts a darkening tint off the crystals, so dark buttons light up like the others
    // (a selected tab never hovers, so its crystals stay dark).
    let tint_rgb = mix(input.tint.rgb, vec3<f32>(1.0), crystal * hover);
    let base = vec4<f32>(sample.rgb * tint_rgb * input.tint.a, sample.a * input.tint.a);

    var rgb = base.rgb;
    // Disabled: desaturate and dim.
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    rgb = mix(rgb, vec3<f32>(luma * 0.55), disabled);
    // Hover: the crystal becomes a lit gem of the glow color, shaded by its own facets so
    // they keep their detail. It is weighted by coverage, so it never lights transparent
    // pixels: the glow stays within the crystal.
    let lit = hover * crystal;
    let coverage = base.a;
    let shade = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) / max(coverage, 1.0e-5);
    let gem = input.glow.rgb * (0.25 + 2.2 * shade) * coverage;
    rgb = mix(rgb, gem, lit * input.glow.a);
    // Press: darken slightly.
    rgb = rgb * (1.0 - 0.3 * press);
    // Gamma on the straight (unpremultiplied) color; skipped at exactly 1.
    let alpha = max(base.a, 1.0e-5);
    let adjusted = pow(max(rgb / alpha, vec3<f32>(0.0)), vec3<f32>(1.0 / input.gamma)) * alpha;
    rgb = select(adjusted, rgb, input.gamma == 1.0);

    // Fragment positions are pixel centers in framebuffer coordinates.
    let p = input.clip_position.xy;
    let inside = f32(all(p >= input.clip.xy) && all(p < input.clip.zw));
    return vec4<f32>(rgb, base.a) * (opacity * inside);
}
