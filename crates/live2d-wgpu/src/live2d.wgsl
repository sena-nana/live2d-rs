struct Live2dUniform {
    viewport: vec4<f32>,
    view_transform: vec4<f32>,
    canvas: vec4<f32>,
    effect: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Live2dUniform;
@group(1) @binding(0) var model_texture: texture_2d<f32>;
@group(1) @binding(1) var model_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    let canvas_size = max(u.canvas.xy, vec2<f32>(0.0001));
    var uv = (input.position + u.canvas.zw) / canvas_size;
    uv.y = 1.0 - uv.y;
    uv = (uv - vec2<f32>(0.5, 0.5)) * max(u.view_transform.z, 0.0001) + vec2<f32>(0.5, 0.5) + u.view_transform.xy;

    let aspect = max(u.viewport.x, 1.0) / max(u.viewport.y, 1.0);
    let clip = vec2<f32>((uv.x - 0.5) / max(aspect, 0.0001) * 2.0, (0.5 - uv.y) * 2.0);

    var out: VertexOut;
    out.pos = vec4<f32>(clip, 0.0, 1.0);
    out.uv = input.uv;
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let color = textureSample(model_texture, model_sampler, input.uv);
    return vec4<f32>(color.rgb * u.effect.rgb, color.a * u.effect.a);
}
