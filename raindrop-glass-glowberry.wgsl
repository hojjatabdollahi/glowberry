// [SHADER]
// name: Raindrop Glass
// author: Martijn Steinrucken / BigWings; WGSLab port adapted for GlowBerry
// source: https://www.shadertoy.com/view/ltffzl
// port: https://github.com/taiyuuki/wgslab/tree/main/examples/raindrop-glass
// license: CC BY-NC-SA 3.0
// notes: Single-file GlowBerry variant with a procedural background
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.2 | max: 3.0 | step: 0.1 | label: Rain Speed
// rain_amount: f32 = 0.5 | min: 0.0 | max: 1.0 | step: 0.05 | label: Rain Amount
// distortion: f32 = 1.2 | min: 0.0 | max: 2.5 | step: 0.1 | label: Refraction
// blur_amount: f32 = 0.9 | min: 0.0 | max: 2.0 | step: 0.1 | label: Background Blur
// [/PARAMS]

// GlowBerry prepends iResolution and iTime. Do not declare bindings or a
// vertex shader here; GlowBerry provides both.

const speed: f32 = 1.0;
const rain_amount: f32 = 0.5;
const distortion: f32 = 1.2;
const blur_amount: f32 = 0.9;

fn s(a: f32, b: f32, t: f32) -> f32 {
    return smoothstep(a, b, t);
}

fn n13(p: f32) -> vec3f {
    var p3 = fract(vec3f(p) * vec3f(0.1031, 0.11369, 0.13787));
    p3 = p3 + vec3f(dot(p3, p3.yzx + vec3f(19.19)));
    return fract(vec3f(
        (p3.x + p3.y) * p3.z,
        (p3.x + p3.z) * p3.y,
        (p3.y + p3.z) * p3.x,
    ));
}

fn n(t: f32) -> f32 {
    return fract(sin(t * 12345.564) * 7658.76);
}

fn saw(b: f32, t: f32) -> f32 {
    return s(0.0, b, t) * s(1.0, b, t);
}

fn drop_layer(uv_in: vec2f, t: f32) -> vec2f {
    let uv_base = uv_in;
    var uv = uv_in;

    uv.y = uv.y + t * 0.75;
    let a = vec2f(6.0, 1.0);
    let grid = a * 2.0;
    var id = floor(uv * grid);

    let column_shift = n(id.x);
    uv.y = uv.y + column_shift;

    id = floor(uv * grid);
    let noise = n13(id.x * 35.2 + id.y * 2376.1);
    let st = fract(uv * grid) - vec2f(0.5, 0.0);

    var x = noise.x - 0.5;
    var y = uv_base.y * 20.0;
    let wiggle = sin(y + sin(y));
    x = x + wiggle * (0.5 - abs(x)) * (noise.z - 0.5);
    x = x * 0.7;

    let ti = fract(t + noise.z);
    y = (saw(0.85, ti) - 0.5) * 0.9 + 0.5;
    let p = vec2f(x, y);

    let d = length((st - p) * a.yx);
    let main_drop = s(0.4, 0.0, d);

    let r = sqrt(s(1.0, y, st.y));
    let cd = abs(st.x - x);
    var trail = s(0.23 * r, 0.15 * r * r, cd);
    let trail_front = s(-0.02, 0.02, st.y - y);
    trail = trail * trail_front * r * r;

    y = uv_base.y;
    let trail_mask = s(0.2 * r, 0.0, cd);
    var droplets = max(0.0, sin(y * (1.0 - y) * 120.0) - st.y)
        * trail_mask * trail_front * noise.z;
    y = fract(y * 10.0) + (st.y - 0.5);
    let droplet_distance = length(st - vec2f(x, y));
    droplets = s(0.3, 0.0, droplet_distance);

    let mask = main_drop + droplets * r * trail_front;
    return vec2f(mask, trail);
}

fn static_drops(uv_in: vec2f, t: f32) -> f32 {
    var uv = uv_in * 40.0;
    let id = floor(uv);
    uv = fract(uv) - 0.5;

    let noise = n13(id.x * 107.45 + id.y * 3543.654);
    let p = (noise.xy - vec2f(0.5)) * 0.7;
    let d = length(uv - p);

    let fade = saw(0.025, fract(t + noise.z));
    return s(0.3, 0.0, d) * fract(noise.z * 10.0) * fade;
}

fn drops(uv: vec2f, t: f32, l0: f32, l1: f32, l2: f32) -> vec2f {
    let s0 = static_drops(uv, t) * l0;
    let m1 = drop_layer(uv, t) * l1;
    let m2 = drop_layer(uv * 1.85, t) * l2;

    var coverage = s0 + m1.x + m2.x;
    coverage = s(0.3, 1.0, coverage);

    return vec2f(coverage, max(m1.y * l0, m2.y * l1));
}

fn light_glow(
    uv: vec2f,
    center: vec2f,
    radius: f32,
    color: vec3f,
) -> vec3f {
    let delta = uv - center;
    let radius_squared = radius * radius;
    let glow = radius_squared / (dot(delta, delta) + radius_squared);
    return color * glow * glow;
}

// A deliberately soft, colorful background gives the refraction something to
// bend while keeping this shader installable without a separate image asset.
fn procedural_background(uv_in: vec2f) -> vec3f {
    let uv = clamp(uv_in, vec2f(0.0), vec2f(1.0));
    let aspect = iResolution.x / max(iResolution.y, 1.0);
    let p = vec2f((uv.x - 0.5) * aspect + 0.5, uv.y);

    var color = mix(
        vec3f(0.018, 0.025, 0.075),
        vec3f(0.075, 0.035, 0.105),
        smoothstep(0.0, 1.0, uv.y),
    );

    color = color + light_glow(p, vec2f(0.16, 0.72), 0.105, vec3f(0.95, 0.18, 0.10));
    color = color + light_glow(p, vec2f(0.36, 0.62), 0.080, vec3f(0.10, 0.35, 1.00));
    color = color + light_glow(p, vec2f(0.56, 0.76), 0.120, vec3f(0.75, 0.12, 0.90));
    color = color + light_glow(p, vec2f(0.77, 0.58), 0.070, vec3f(0.05, 0.85, 0.75));
    color = color + light_glow(p, vec2f(0.92, 0.70), 0.095, vec3f(1.00, 0.44, 0.08));

    let horizon = smoothstep(0.62, 1.0, uv.y);
    color = mix(color, color * vec3f(0.34, 0.30, 0.43), horizon * 0.65);

    return color;
}

fn frosted_glass(texture_uv: vec2f, normal: vec2f, blur_pixels: f32) -> vec3f {
    let base = clamp(texture_uv + normal, vec2f(0.0), vec2f(1.0));
    let px = blur_pixels / max(iResolution, vec2f(1.0));

    var color = procedural_background(base) * 0.20;
    color = color + procedural_background(base + vec2f(px.x, 0.0)) * 0.13;
    color = color + procedural_background(base - vec2f(px.x, 0.0)) * 0.13;
    color = color + procedural_background(base + vec2f(0.0, px.y)) * 0.13;
    color = color + procedural_background(base - vec2f(0.0, px.y)) * 0.13;
    color = color + procedural_background(base + px) * 0.07;
    color = color + procedural_background(base - px) * 0.07;
    color = color + procedural_background(base + vec2f(px.x, -px.y)) * 0.07;
    color = color + procedural_background(base + vec2f(-px.x, px.y)) * 0.07;
    return color;
}

@fragment
fn main(@builtin(position) frag_coord_top_left: vec4f) -> @location(0) vec4f {
    let texture_uv = frag_coord_top_left.xy / iResolution;

    // The source effect expects Shadertoy-style coordinates with Y increasing
    // upward; WebGPU fragment coordinates increase downward.
    let frag_coord = vec2f(
        frag_coord_top_left.x,
        iResolution.y - frag_coord_top_left.y,
    );
    let rain_uv = (frag_coord - iResolution * 0.5) / iResolution.y;

    let amount = clamp(rain_amount, 0.0, 1.0);
    let t = iTime * 0.2 * speed;
    let static_layer = s(-0.5, 1.0, amount) * 2.0;
    let layer1 = s(0.25, 0.75, amount);
    let layer2 = s(0.0, 0.5, amount);

    let coverage = drops(rain_uv, t, static_layer, layer1, layer2);

    let e = vec2f(0.001, 0.0);
    let coverage_x = drops(rain_uv + e, t, static_layer, layer1, layer2).x;
    let coverage_y = drops(rain_uv + e.yx, t, static_layer, layer1, layer2).x;
    let normal = vec2f(coverage_x - coverage.x, coverage_y - coverage.x);

    let min_blur = 2.5;
    let max_blur = mix(4.0, 7.0, amount);
    let focus = mix(max_blur - coverage.y, min_blur, s(0.08, 0.22, coverage.x));

    let texture_normal = vec2f(normal.x, -normal.y) * distortion;
    var color = frosted_glass(texture_uv, texture_normal, focus * blur_amount);

    let fog = smoothstep(min_blur, max_blur, focus);
    color = mix(color, vec3f(0.72, 0.74, 0.78), fog * 0.18);
    color = color + vec3f(0.04, 0.05, 0.06) * coverage.y;
    color = color + vec3f(0.10, 0.11, 0.12) * coverage.x * 0.12;

    let vignette_uv = texture_uv - vec2f(0.5);
    let vignette = 1.0 - dot(vignette_uv, vignette_uv) * 0.9;
    color = color * vignette;

    return vec4f(color, 1.0);
}
