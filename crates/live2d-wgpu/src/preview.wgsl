struct PreviewUniform {
    viewport: vec4<f32>,
    view_transform: vec4<f32>,
    tint_a: vec4<f32>,
    tint_b: vec4<f32>,
    grad_lo: vec4<f32>,
    grad_hi: vec4<f32>,
    ptcl_color: vec4<f32>,
    damage_fray_color: vec4<f32>,
    params0: vec4<f32>,
    params1: vec4<f32>,
    params2: vec4<f32>,
    params3: vec4<f32>,
    params4: vec4<f32>,
    params5: vec4<f32>,
    params6: vec4<f32>,
    params7: vec4<f32>,
    params8: vec4<f32>,
    params9: vec4<f32>,
    picker: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: PreviewUniform;

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let pos = positions[vertex_index];
    var out: VertexOut;
    out.pos = vec4<f32>(pos, 0.0, 1.0);
    out.uv = pos * 0.5 + vec2<f32>(0.5, 0.5);
    return out;
}

fn saturate(value: f32) -> f32 {
    return clamp(value, 0.0, 1.0);
}

fn saturate3(value: vec3<f32>) -> vec3<f32> {
    return clamp(value, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453123);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash22(i).x;
    let b = hash22(i + vec2<f32>(1.0, 0.0)).x;
    let c = hash22(i + vec2<f32>(0.0, 1.0)).x;
    let d = hash22(i + vec2<f32>(1.0, 1.0)).x;
    let m = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, m.x), mix(c, d, m.x), m.y);
}

fn flow_factor(pos: vec2<f32>, time: f32) -> f32 {
    let d = vec2<f32>(0.12, 0.08) * time;
    var f = 0.65 * noise(pos + d);
    f += 0.35 * noise(pos * 2.1 - d * 0.7 + vec2<f32>(17.3, 17.3));
    return smoothstep(0.3, 0.7, f);
}

fn recolor(base_rgb: vec3<f32>, tint: vec3<f32>, strength: f32, brightness: f32) -> vec3<f32> {
    let lum = dot(base_rgb, vec3<f32>(0.299, 0.587, 0.114));
    let recolored = tint * (lum * brightness);
    return mix(base_rgb, recolored, saturate(strength));
}

fn hue_rotate(c: vec3<f32>, angle_radians: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735026919);
    let cs = cos(angle_radians);
    return c * cs + cross(k, c) * sin(angle_radians) + k * dot(k, c) * (1.0 - cs);
}

fn channel_fx(c_in: vec3<f32>, time: f32) -> vec3<f32> {
    var c = c_in;
    let grad_amount = u.params1.y;
    if (grad_amount > 0.0) {
        let l = saturate(dot(c, vec3<f32>(0.299, 0.587, 0.114)));
        c = mix(c, mix(u.grad_lo.rgb, u.grad_hi.rgb, smoothstep(0.0, 1.0, l)), grad_amount);
    }

    let hue = u.params1.z + u.params1.w * time;
    if (abs(hue) > 0.0001) {
        c = hue_rotate(c, radians(hue));
    }

    let saturation = u.params2.w;
    if (abs(saturation - 1.0) > 0.0001) {
        let l = dot(c, vec3<f32>(0.299, 0.587, 0.114));
        c = mix(vec3<f32>(l), c, saturation);
    }

    let contrast = u.params3.x;
    if (abs(contrast - 1.0) > 0.0001) {
        c = (c - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5);
    }

    let posterize = u.params2.x;
    if (posterize >= 2.0) {
        let n = floor(posterize);
        c = floor(saturate3(c) * n) / max(n - 1.0, 1.0);
    }

    let pulse_amount = u.params2.y;
    if (pulse_amount > 0.0) {
        c *= 1.0 + pulse_amount * sin(time * u.params2.z * 6.28318530718);
    }
    return max(c, vec3<f32>(0.0));
}

fn particle_sd(q_in: vec2<f32>, shape: f32) -> f32 {
    var q = q_in;
    if (shape < 0.5) {
        return (abs(q.x) + abs(q.y) - 1.0) * 0.70710678;
    }
    if (shape < 1.5) {
        let y = q.y + 0.15 * (1.0 - abs(q.x));
        let heart = pow(q.x * q.x + y * y - 0.72, 3.0) - q.x * q.x * y * y * y;
        return heart;
    }
    if (shape < 2.5) {
        let a = atan2(q.y, q.x);
        let r = length(q);
        let star = 0.62 + 0.22 * cos(a * 5.0);
        return r - star;
    }
    return length(q) - 1.0;
}

fn particle_layer(view_px: vec2<f32>, time: f32) -> f32 {
    if (u.params5.z < 0.5) {
        return 0.0;
    }
    let cell = clamp(200.0 / max(u.params6.x, 0.0001), 6.0, 600.0);
    let cell_id = floor(view_px / cell);
    let phase = time * u.params6.z + hash22(cell_id + vec2<f32>(17.13, 17.13)).y;
    let cycle = floor(phase);
    let life = fract(phase);
    let seed = cell_id + cycle * 31.7 + vec2<f32>(3.1, 3.1);
    let r1 = hash22(seed);
    if (r1.x > 0.8) {
        return 0.0;
    }
    let r2 = hash22(seed + vec2<f32>(17.13, 17.13));
    let r3 = hash22(seed - vec2<f32>(7.31, 7.31));
    let size_f = clamp(u.params6.y, 0.02, 0.5);
    let radius_px = min(size_f * (0.7 + 0.6 * r2.x), 0.48) * cell;
    let margin = 0.5 - radius_px / cell;
    let center_px = (cell_id + vec2<f32>(0.5) + (r3 - vec2<f32>(0.5)) * 2.0 * margin) * cell;
    let ang = r1.y * 6.28318530718;
    let cs = cos(ang);
    let sn = sin(ang);
    let d = (view_px - center_px) / max(radius_px, 0.5);
    let q = vec2<f32>(cs * d.x - sn * d.y, sn * d.x + cs * d.y);
    let sd = particle_sd(q, u.params5.w);
    let aa = 1.2 / max(radius_px, 1.0);
    let cov = 1.0 - smoothstep(-aa, aa, sd);
    let tw = smoothstep(0.0, 0.5, life) - smoothstep(0.5, 1.0, life);
    return cov * tw;
}

fn dissolve_coverage(norm_pos: vec2<f32>) -> f32 {
    let progress = saturate(u.params4.w);
    if (progress <= 0.0001) {
        return 1.0;
    }
    if (progress >= 0.999) {
        return 0.0;
    }
    let cell = clamp(u.params5.x, 0.02, 0.5);
    let cell_uv = fract(norm_pos / cell);
    let d = max(abs(cell_uv.x - 0.5), abs(cell_uv.y - 0.5));
    let radius = 0.5 * (1.0 - progress);
    return 1.0 - smoothstep(radius, radius + 0.015, d);
}

fn dissolve_glow(norm_pos: vec2<f32>) -> f32 {
    let progress = saturate(u.params4.w);
    if (progress <= 0.0001 || progress >= 0.999) {
        return 0.0;
    }
    let cell = clamp(u.params5.x, 0.02, 0.5);
    let cell_uv = fract(norm_pos / cell);
    let d = max(abs(cell_uv.x - 0.5), abs(cell_uv.y - 0.5));
    let radius = 0.5 * (1.0 - progress);
    let edge = 1.0 - smoothstep(0.0, 0.035, abs(d - radius));
    return edge * dissolve_coverage(norm_pos) * progress;
}

fn damage_mask(uv: vec2<f32>) -> vec3<f32> {
    if (u.params9.w < 0.5 || u.params6.w <= 0.0) {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    let count = i32(clamp(floor(u.params7.x), 1.0, 24.0));
    let anchor = clamp(vec2<f32>(0.5 + u.params7.w, 0.62 + u.params8.x), vec2<f32>(0.0), vec2<f32>(1.0));
    let angle = radians(u.params8.w);
    let ca = cos(angle);
    let sa = sin(angle);
    var sd = 999.0;
    for (var k = 0; k < 24; k = k + 1) {
        if (k >= count) {
            break;
        }
        let hk = hash22(vec2<f32>(f32(k) * 1.7 + 1.0, u.params9.z * 3.1 + 2.0));
        let a = mix(0.08, 0.92, hk.x) * 6.28318530718;
        let rad = select(sqrt(hk.y) * u.params7.y, 0.0, k == 0);
        let center = anchor + vec2<f32>(cos(a) * 1.25, -abs(sin(a)) * 0.75) * rad;
        let hs = hash22(vec2<f32>(f32(k) * 5.3 + 3.0, u.params9.z * 2.0 + 9.0));
        let boost = select(1.0, 1.35, k == 0);
        let hy = mix(0.04, 0.22, clamp(u.params7.z * (0.6 + 0.8 * hs.x) * boost, 0.0, 1.0));
        let hx = hy * u.params8.z;
        let d = uv - center;
        let dd = abs(vec2<f32>(d.x * ca + d.y * sa, -d.x * sa + d.y * ca));
        let f = dd.x / max(hx, 0.001) + dd.y / max(hy, 0.001) - 1.0;
        sd = min(sd, f * min(hx, hy));
    }
    let ragged = (noise(uv * 85.0 + vec2<f32>(u.params9.z * 3.7)) - 0.5) * u.params9.x * 0.045;
    sd += ragged;
    let amount = saturate(u.params6.w);
    let edge_width = mix(0.001, 0.02, saturate(u.params8.y));
    let keep = mix(1.0, smoothstep(-edge_width, edge_width, sd), amount);
    let fray_width = u.params9.y * 0.035;
    let fray = (1.0 - smoothstep(0.0, fray_width, sd)) * step(0.0, sd) * amount;
    let shadow = (1.0 - smoothstep(fray_width, fray_width * 2.0, sd)) * step(fray_width, sd) * amount;
    return vec3<f32>(keep, fray, shadow);
}

fn rotate2(p: vec2<f32>, angle_degrees: f32) -> vec2<f32> {
    let a = radians(angle_degrees);
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

fn base_art(uv: vec2<f32>) -> vec4<f32> {
    let centered = uv - vec2<f32>(0.5, 0.52);
    let body = smoothstep(0.72, 0.69, length(centered / vec2<f32>(0.48, 0.62)));
    let head = smoothstep(0.37, 0.34, length((uv - vec2<f32>(0.5, 0.28)) / vec2<f32>(0.95, 1.0)));
    let alpha = max(body, head);
    let stripe = 0.5 + 0.5 * sin((uv.x * 9.0 + uv.y * 5.0) * 3.14159);
    var color = vec3<f32>(0.68, 0.60, 0.74) * (0.82 + 0.18 * uv.y) + vec3<f32>(0.18, 0.10, 0.24) * stripe * 0.18;
    let blush = smoothstep(0.18, 0.0, distance(uv, vec2<f32>(0.34, 0.35))) + smoothstep(0.18, 0.0, distance(uv, vec2<f32>(0.66, 0.35)));
    color += vec3<f32>(0.28, 0.08, 0.10) * blush * 0.18;
    return vec4<f32>(color, alpha);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let time = u.viewport.x;
    let size = max(u.viewport.yz, vec2<f32>(1.0));
    let aspect = size.x / size.y;
    let bg = vec3<f32>(0.0, 0.0, 0.0);

    var uv = in.uv;
    uv.x = (uv.x - 0.5) * aspect + 0.5;
    uv = (uv - vec2<f32>(0.5, 0.5) - u.view_transform.xy) / max(u.view_transform.z, 0.0001) + vec2<f32>(0.5, 0.5);
    let scale = max(u.params3.z, 0.0001);
    uv = (uv - vec2<f32>(0.5)) / scale + vec2<f32>(0.5);

    let sphere = saturate(u.params3.w);
    if (sphere > 0.0) {
        let p = (uv - vec2<f32>(0.5)) * 2.0;
        let r = length(p);
        if (r < 1.0) {
            let bulge = sqrt(max(1.0 - r * r, 0.0));
            let rotated = rotate2(p, u.params4.x + u.params4.y * time);
            uv = mix(uv, rotated * (0.42 + 0.10 * bulge) + vec2<f32>(0.5), sphere);
        }
    }

    var art = base_art(uv);
    if (art.a <= 0.001) {
        return vec4<f32>(bg, 1.0);
    }

    let view_px = in.uv * size;
    let tint_mix = flow_factor(view_px * max(u.params1.x, 0.001) * 0.01, time * u.params0.w);
    let target_tint = mix(u.tint_a.rgb, u.tint_b.rgb, tint_mix * step(0.5, u.params0.z));
    let effective_strength = select(u.params0.x, 1.0, u.params0.z > 0.5 && u.params0.x <= 0.0);
    var color = recolor(art.rgb, target_tint, effective_strength, u.params0.y);
    color = channel_fx(color, time);
    color += u.ptcl_color.rgb * particle_layer(view_px, time);

    let dissolve_norm = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let dissolve_alpha = dissolve_coverage(dissolve_norm);
    let glow = dissolve_glow(dissolve_norm) * u.params5.y;
    color += max(color, u.tint_a.rgb) * glow;

    let normal = normalize(vec3<f32>((uv - vec2<f32>(0.5)) * 2.0, 0.85));
    let light = normalize(vec3<f32>(-0.35, 0.45, 0.82));
    let lit = 0.58 + 0.42 * saturate(dot(normal, light) * 0.5 + 0.5);
    color *= mix(1.0, lit, sphere * saturate(u.params4.z));

    let damage = damage_mask(uv);
    color = mix(color, u.damage_fray_color.rgb, damage.y);
    color = mix(color, vec3<f32>(1.0, 0.08, 0.06), saturate(u.picker.x) * 0.72);
    color *= 1.0 - damage.z * 0.55;
    art.a *= damage.x * dissolve_alpha * saturate(u.params3.y);

    let out_rgb = mix(bg, saturate3(color), saturate(art.a));
    return vec4<f32>(out_rgb, 1.0);
}
