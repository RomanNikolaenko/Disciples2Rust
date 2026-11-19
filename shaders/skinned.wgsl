// Skinned, lit models: the game's 3D layer.
//
// Spaces (see src/engine/model): vertices arrive in mesh space; the blended joint
// matrices take them to model space; `clip_from_model` takes them to clip space and
// `light_from_model` turns normals into the lighting frame, in which `light.xyz` points
// towards the light and +Z towards the viewer.
//
// Color: material colors and the output are linear; a part's texture is sRGB (the
// sampler decodes it) and multiplies a vertex's color by its texture weight; texture
// alpha below a quarter discards the fragment (leaves). The target
// is sRGB, cleared to transparent black, and every fragment is opaque, so the target
// ends up holding premultiplied alpha (including resolved MSAA edges) for the UI to
// composite.

const MAX_JOINTS: u32 = 128u;

struct Draw {
    clip_from_model: mat4x4<f32>,
    light_from_model: mat4x4<f32>,
    // xyz: unit vector towards the light; w: ambient level (0..1).
    light: vec4<f32>,
    // rgb: linear color multiplier; w: rim light strength.
    surface: vec4<f32>,
    joints: array<mat4x4<f32>, MAX_JOINTS>,
}

@group(0) @binding(0) var<uniform> draw: Draw;
// The drawn part's texture (1x1 white for parts without one).
@group(1) @binding(0) var base_color: texture_2d<f32>;
@group(1) @binding(1) var base_sampler: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) texture_weight: f32,
    @location(5) joints: vec4<u32>,
    @location(6) weights: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) texture_weight: f32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    // Weights sum to 1, so the blend stays affine.
    let skin = draw.joints[min(input.joints.x, MAX_JOINTS - 1u)] * input.weights.x
        + draw.joints[min(input.joints.y, MAX_JOINTS - 1u)] * input.weights.y
        + draw.joints[min(input.joints.z, MAX_JOINTS - 1u)] * input.weights.z
        + draw.joints[min(input.joints.w, MAX_JOINTS - 1u)] * input.weights.w;
    let model_position = skin * vec4<f32>(input.position, 1.0);
    // Joint matrices carry no shear in practice; normals are renormalized per fragment.
    let model_normal = (skin * vec4<f32>(input.normal, 0.0)).xyz;

    var output: VertexOutput;
    output.clip = draw.clip_from_model * model_position;
    output.normal = (draw.light_from_model * vec4<f32>(model_normal, 0.0)).xyz;
    output.color = input.color * draw.surface.rgb;
    output.uv = input.uv;
    output.texture_weight = input.texture_weight;
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
    // Half-Lambert keeps the side away from the light readable on a small figure.
    let wrap = dot(n, draw.light.xyz) * 0.5 + 0.5;
    let ambient = draw.light.w;
    let lit = ambient + (1.0 - ambient) * wrap * wrap;
    // A thin rim where the surface turns away from the viewer separates the figure
    // from the ground behind it.
    let rim = pow(clamp(1.0 - n.z, 0.0, 1.0), 3.0) * draw.surface.w;
    let albedo = input.color * mix(vec3<f32>(1.0), texel.rgb, input.texture_weight);
    return vec4<f32>(albedo * lit + vec3<f32>(rim), 1.0);
}
