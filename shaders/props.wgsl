// Static scenery, drawn instanced: trees, rocks, buildings and the ground of the map.
//
// Spaces (see src/engine/render/scene_renderer.rs): vertices arrive baked into their
// model's rest pose; each instance's matrix places them in the world (x and z across
// the map, one unit per tile, +Y up); `clip_from_world` projects them and
// `light_from_world` turns normals into the lighting frame, in which `light.xyz` points
// towards the light and +Z towards the viewer (the same frame the figures are lit in).
//
// Color: linear; textures are sRGB (the sampler decodes them) and multiply a vertex's
// color by its texture weight; texture alpha below a quarter discards the fragment
// (leaves). A texture weight of 2 marks a baked sprite: its texture is premultiplied
// (an atlas the scenery was rendered into) and is turned back to straight color. Every
// fragment is opaque, so the sRGB target ends up premultiplied. Near the map's edges
// everything fades to black.

struct Frame {
    clip_from_world: mat4x4<f32>,
    light_from_world: mat4x4<f32>,
    // xyz: unit vector towards the light; w: ambient level (0..1).
    light: vec4<f32>,
    // rgb: linear color multiplier; w: rim light strength.
    surface: vec4<f32>,
    // x, y: the map's size along world x and z (0: no fade); z: fade width.
    map: vec4<f32>,
    // The ground square (see shaders/ground.wgsl); unused here.
    square: vec4<f32>,
    square_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var base_color: texture_2d<f32>;
@group(1) @binding(1) var base_sampler: sampler;

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
    // rgb: color multiplier; a: 1 shows the colors unlit (painted ground).
    @location(9) tint: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) texture_weight: f32,
    @location(4) world: vec3<f32>,
    @location(5) unlit: f32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let world_from_model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    let world = world_from_model * vec4<f32>(input.position, 1.0);
    // Instances scale uniformly or stretch a little (rocks); renormalizing per fragment
    // is close enough for either.
    let world_normal = (world_from_model * vec4<f32>(input.normal, 0.0)).xyz;

    var output: VertexOutput;
    output.clip = frame.clip_from_world * world;
    output.normal = (frame.light_from_world * vec4<f32>(world_normal, 0.0)).xyz;
    output.color = input.color * input.tint.rgb * frame.surface.rgb;
    output.uv = input.uv;
    output.texture_weight = input.texture_weight;
    output.world = world.xyz;
    output.unlit = input.tint.a;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(base_color, base_sampler, input.uv);
    if (input.texture_weight > 0.5 && texel.a < 0.25) {
        discard;
    }
    let length_n = length(input.normal);
    let n = select(vec3<f32>(0.0, 0.0, 1.0), input.normal / length_n, length_n > 1e-6);
    // Half-Lambert keeps the shaded sides readable from far away.
    let wrap = dot(n, frame.light.xyz) * 0.5 + 0.5;
    let ambient = frame.light.w;
    let lit = mix(ambient + (1.0 - ambient) * wrap * wrap, 1.0, input.unlit);
    let rim = pow(clamp(1.0 - n.z, 0.0, 1.0), 3.0) * frame.surface.w * (1.0 - input.unlit);
    let sprite = input.texture_weight > 1.5;
    let straight = select(texel.rgb, texel.rgb / max(texel.a, 1e-3), sprite);
    let albedo = input.color * mix(vec3<f32>(1.0), straight, min(input.texture_weight, 1.0));

    var fade = 1.0;
    if (frame.map.x > 0.0 && frame.map.z > 0.0) {
        let inside = min(
            min(input.world.x, input.world.z),
            min(frame.map.x - input.world.x, frame.map.y - input.world.z),
        );
        fade = smoothstep(0.0, frame.map.z, inside);
    }
    return vec4<f32>((albedo * lit + vec3<f32>(rim)) * fade, 1.0);
}
