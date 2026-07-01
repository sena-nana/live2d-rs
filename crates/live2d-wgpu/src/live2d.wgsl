struct Live2dUniform {
    viewport: vec4<f32>,
    view_transform: vec4<f32>,
    canvas: vec4<f32>,
    effect: vec4<f32>,
    mask: vec4<f32>,
    blend: vec4<u32>,
    sampling: vec4<u32>,
};

@group(0) @binding(0) var<uniform> u: Live2dUniform;
@group(1) @binding(0) var model_texture: texture_2d<f32>;
@group(1) @binding(1) var model_sampler: sampler;
@group(1) @binding(2) var model_linear_sampler: sampler;
@group(2) @binding(0) var mask_texture: texture_2d<f32>;
@group(2) @binding(1) var mask_sampler: sampler;
@group(3) @binding(0) var blend_texture: texture_2d<f32>;
@group(3) @binding(1) var blend_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) screen_uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    let canvas_size = max(u.canvas.xy, vec2<f32>(0.0001));
    var uv = (input.position + u.canvas.zw) / canvas_size;
    uv.y = 1.0 - uv.y;
    uv = (uv - vec2<f32>(0.5, 0.5)) * max(u.view_transform.z, 0.0001) + vec2<f32>(0.5, 0.5) + u.view_transform.xy;

    let viewport_aspect = max(u.viewport.x, 1.0) / max(u.viewport.y, 1.0);
    let geometry_aspect = max(u.viewport.z, 0.0001);
    let clip = vec2<f32>(
        (uv.x - 0.5) * geometry_aspect / max(viewport_aspect, 0.0001) * 2.0,
        (0.5 - uv.y) * 2.0,
    );

    var out: VertexOut;
    out.pos = vec4<f32>(clip, 0.0, 1.0);
    out.uv = vec2<f32>(input.uv.x, 1.0 - input.uv.y);
    out.screen_uv = uv;
    return out;
}

fn cubic_weight(x: f32) -> f32 {
    let ax = abs(x);
    if (ax <= 1.0) {
        return ((1.5 * ax - 2.5) * ax) * ax + 1.0;
    }
    if (ax < 2.0) {
        return (((-0.5 * ax + 2.5) * ax - 4.0) * ax) + 2.0;
    }
    return 0.0;
}

fn model_texture_load_clamped(coord: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    return textureLoad(model_texture, clamp(coord, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

fn sample_model_texture_cubic(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(model_texture));
    let texture_size = vec2<f32>(f32(size.x), f32(size.y));
    let texel = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * texture_size - vec2<f32>(0.5);
    let base = vec2<i32>(i32(floor(texel.x)), i32(floor(texel.y)));
    let f = texel - vec2<f32>(f32(base.x), f32(base.y));
    var color = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var y = -1; y <= 2; y = y + 1) {
        let wy = cubic_weight(f.y - f32(y));
        for (var x = -1; x <= 2; x = x + 1) {
            let weight = cubic_weight(f.x - f32(x)) * wy;
            color += model_texture_load_clamped(base + vec2<i32>(x, y), size) * weight;
            weight_sum += weight;
        }
    }
    return clamp(color / max(weight_sum, 0.0001), vec4<f32>(0.0), vec4<f32>(1.0));
}

fn sample_model_texture(uv: vec2<f32>) -> vec4<f32> {
    if (u.sampling.x == 2u) {
        return sample_model_texture_cubic(uv);
    }
    return textureSample(model_texture, model_sampler, uv);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    let color = sample_model_texture(input.uv);
    let alpha = live2d_alpha(color.a, input);
    return vec4<f32>(color.rgb * u.effect.rgb * alpha, alpha);
}

@fragment
fn fs_blend(input: VertexOut) -> @location(0) vec4<f32> {
    let color = sample_model_texture(input.uv);
    let source = vec4<f32>(color.rgb * u.effect.rgb, live2d_alpha(color.a, input));
    let destination = premultiplied_to_straight(textureSample(blend_texture, blend_sampler, input.screen_uv));
    let blended_color = color_blend(u.blend.x, source.rgb, destination.rgb);
    return alpha_blend(u.blend.y, blended_color, source, destination);
}

@fragment
fn fs_mask(input: VertexOut) -> @location(0) vec4<f32> {
    let color = textureSample(model_texture, model_linear_sampler, input.uv);
    return vec4<f32>(1.0, 1.0, 1.0, color.a * u.effect.a);
}

fn live2d_alpha(texture_alpha: f32, input: VertexOut) -> f32 {
    var alpha = texture_alpha * u.effect.a;
    if (abs(u.mask.w) > 0.000001) {
        let slot_origin = u.mask.xz;
        let slot_scale = vec2<f32>(u.mask.y, abs(u.mask.w));
        let mask_size = vec2<f32>(textureDimensions(mask_texture));
        let half_texel = 0.5 / max(mask_size, vec2<f32>(1.0, 1.0));
        let slot_min = slot_origin + half_texel;
        let slot_max = max(slot_min, slot_origin + slot_scale - half_texel);
        let mask_uv = vec2<f32>(
            slot_origin.x + input.screen_uv.x * slot_scale.x,
            slot_origin.y + input.screen_uv.y * slot_scale.y,
        );
        let mask_alpha = textureSample(mask_texture, mask_sampler, clamp(mask_uv, slot_min, slot_max)).a;
        let coverage = select(mask_alpha, 1.0 - mask_alpha, u.mask.w < 0.0);
        alpha *= coverage;
    }
    return alpha;
}

fn premultiplied_to_straight(color: vec4<f32>) -> vec4<f32> {
    if (color.a < 0.00001) {
        return vec4<f32>(0.0, 0.0, 0.0, color.a);
    }
    return vec4<f32>(color.rgb / color.a, color.a);
}

fn color_blend(mode: u32, source: vec3<f32>, destination: vec3<f32>) -> vec3<f32> {
    if (mode == 3u) {
        return min(source + destination, vec3<f32>(1.0));
    }
    if (mode == 4u) {
        return source + destination;
    }
    if (mode == 5u) {
        return min(source, destination);
    }
    if (mode == 6u) {
        return source * destination;
    }
    if (mode == 7u) {
        return vec3<f32>(
            color_burn(source.r, destination.r),
            color_burn(source.g, destination.g),
            color_burn(source.b, destination.b),
        );
    }
    if (mode == 8u) {
        return max(vec3<f32>(0.0), source + destination - vec3<f32>(1.0));
    }
    if (mode == 9u) {
        return max(source, destination);
    }
    if (mode == 10u) {
        return source + destination - source * destination;
    }
    if (mode == 11u) {
        return vec3<f32>(
            color_dodge(source.r, destination.r),
            color_dodge(source.g, destination.g),
            color_dodge(source.b, destination.b),
        );
    }
    if (mode == 12u) {
        return vec3<f32>(
            overlay(source.r, destination.r),
            overlay(source.g, destination.g),
            overlay(source.b, destination.b),
        );
    }
    if (mode == 13u) {
        return vec3<f32>(
            soft_light(source.r, destination.r),
            soft_light(source.g, destination.g),
            soft_light(source.b, destination.b),
        );
    }
    if (mode == 14u) {
        return vec3<f32>(
            hard_light(source.r, destination.r),
            hard_light(source.g, destination.g),
            hard_light(source.b, destination.b),
        );
    }
    if (mode == 15u) {
        return vec3<f32>(
            linear_light(source.r, destination.r),
            linear_light(source.g, destination.g),
            linear_light(source.b, destination.b),
        );
    }
    if (mode == 16u) {
        return set_luma(set_saturation(source, saturation(destination)), luma(destination));
    }
    if (mode == 17u) {
        return set_luma(source, luma(destination));
    }
    return source;
}

fn color_burn(source: f32, destination: f32) -> f32 {
    if (abs(destination - 1.0) < 0.000001) {
        return 1.0;
    }
    if (abs(source) < 0.000001) {
        return 0.0;
    }
    return 1.0 - min(1.0, (1.0 - destination) / source);
}

fn color_dodge(source: f32, destination: f32) -> f32 {
    if (destination <= 0.0) {
        return 0.0;
    }
    if (source == 1.0) {
        return 1.0;
    }
    return min(1.0, destination / (1.0 - source));
}

fn overlay(source: f32, destination: f32) -> f32 {
    let mul = 2.0 * source * destination;
    let scr = 1.0 - 2.0 * (1.0 - source) * (1.0 - destination);
    return select(scr, mul, destination < 0.5);
}

fn soft_light(source: f32, destination: f32) -> f32 {
    let val1 = destination - (1.0 - 2.0 * source) * destination * (1.0 - destination);
    let val2 = destination + (2.0 * source - 1.0) * destination * ((16.0 * destination - 12.0) * destination + 3.0);
    let val3 = destination + (2.0 * source - 1.0) * (sqrt(destination) - destination);
    if (source <= 0.5) {
        return val1;
    }
    if (destination <= 0.25) {
        return val2;
    }
    return val3;
}

fn hard_light(source: f32, destination: f32) -> f32 {
    let mul = 2.0 * source * destination;
    let scr = 1.0 - 2.0 * (1.0 - source) * (1.0 - destination);
    return select(scr, mul, source < 0.5);
}

fn linear_light(source: f32, destination: f32) -> f32 {
    let burn = max(0.0, 2.0 * source + destination - 1.0);
    let dodge = min(1.0, 2.0 * (source - 0.5) + destination);
    return select(dodge, burn, source < 0.5);
}

fn max_channel(color: vec3<f32>) -> f32 {
    return max(color.r, max(color.g, color.b));
}

fn min_channel(color: vec3<f32>) -> f32 {
    return min(color.r, min(color.g, color.b));
}

fn saturation(color: vec3<f32>) -> f32 {
    return max_channel(color) - min_channel(color);
}

fn luma(color: vec3<f32>) -> f32 {
    return 0.30 * color.r + 0.59 * color.g + 0.11 * color.b;
}

fn clip_color(color: vec3<f32>) -> vec3<f32> {
    let lum = luma(color);
    let minv = min_channel(color);
    let maxv = max_channel(color);
    var out_color = color;
    if (minv < 0.0) {
        out_color = vec3<f32>(lum) + (out_color - vec3<f32>(lum)) * (lum / (lum - minv));
    }
    if (maxv > 1.0) {
        out_color = vec3<f32>(lum) + (out_color - vec3<f32>(lum)) * ((1.0 - lum) / (maxv - lum));
    }
    return out_color;
}

fn set_luma(color: vec3<f32>, target_luma: f32) -> vec3<f32> {
    return clip_color(color + vec3<f32>(target_luma - luma(color)));
}

fn set_saturation(color: vec3<f32>, target_saturation: f32) -> vec3<f32> {
    let maxv = max_channel(color);
    let minv = min_channel(color);
    let medv = color.r + color.g + color.b - maxv - minv;
    var out_max = 0.0;
    var out_med = 0.0;
    if (minv < maxv) {
        out_max = target_saturation;
        out_med = (medv - minv) * target_saturation / (maxv - minv);
    }
    let out_min = 0.0;
    if (color.r == maxv) {
        if (color.b < color.g) {
            return vec3<f32>(out_max, out_med, out_min);
        }
        return vec3<f32>(out_max, out_min, out_med);
    }
    if (color.g == maxv) {
        if (color.r < color.b) {
            return vec3<f32>(out_min, out_max, out_med);
        }
        return vec3<f32>(out_med, out_max, out_min);
    }
    if (color.g < color.r) {
        return vec3<f32>(out_med, out_min, out_max);
    }
    return vec3<f32>(out_min, out_med, out_max);
}

fn alpha_blend(mode: u32, color: vec3<f32>, source: vec4<f32>, destination: vec4<f32>) -> vec4<f32> {
    var weights: vec3<f32>;
    if (mode == 1u) {
        weights = vec3<f32>(
            source.a * destination.a,
            0.0,
            destination.a * (1.0 - source.a),
        );
    } else if (mode == 2u) {
        weights = vec3<f32>(
            0.0,
            0.0,
            destination.a * (1.0 - source.a),
        );
    } else if (mode == 3u) {
        weights = vec3<f32>(
            min(source.a, destination.a),
            max(source.a - destination.a, 0.0),
            max(destination.a - source.a, 0.0),
        );
    } else if (mode == 4u) {
        weights = vec3<f32>(
            max(source.a + destination.a - 1.0, 0.0),
            min(source.a, 1.0 - destination.a),
            min(destination.a, 1.0 - source.a),
        );
    } else {
        weights = vec3<f32>(
            source.a * destination.a,
            source.a * (1.0 - destination.a),
            destination.a * (1.0 - source.a),
        );
    }
    return vec4<f32>(
        color * weights.x + source.rgb * weights.y + destination.rgb * weights.z,
        weights.x + weights.y + weights.z,
    );
}
