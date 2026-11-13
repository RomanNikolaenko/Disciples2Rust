// UI sprite material: textured quads in physical pixel coordinates.
//
// Blend/color conventions:
// - `base_texture` holds premultiplied alpha and is sampled from an sRGB view, so values
//   arrive linear. The output is premultiplied: blend state is (One, OneMinusSrcAlpha).
// - `glow_mask` is linear single-channel data: 1 = this texel glows / brightens on hover.
//   It only shapes the effect; hit testing uses the base alpha on the CPU.
// - Instance parameters arrive per vertex, so one draw call covers many sprites that
//   share textures while each keeps its own hover/opacity values.

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
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) glow: vec4<f32>,
    @location(2) params: vec4<f32>,
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
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(base_texture, linear_sampler, input.uv);
    let mask = textureSample(glow_mask, linear_sampler, input.uv).r;
    let hover = input.params.x;
    let opacity = input.params.y;
    let press = input.params.z;
    let disabled = input.params.w;

    var rgb = base.rgb;
    // Disabled: desaturate and dim.
    let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    rgb = mix(rgb, vec3<f32>(luma * 0.55), disabled);
    // Hover: masked areas brighten and pick up the glow color (localized glow, not bloom).
    let glow_amount = hover * mask;
    rgb = rgb * (1.0 + 0.35 * glow_amount) + input.glow.rgb * (input.glow.a * glow_amount * base.a);
    // Press: darken slightly.
    rgb = rgb * (1.0 - 0.3 * press);

    return vec4<f32>(rgb, base.a) * opacity;
}
