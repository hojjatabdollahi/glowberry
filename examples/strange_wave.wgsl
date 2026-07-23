// [SHADER]
// name: Strange Wave
// author: bitless (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/NdK3R3
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.2 | max: 3.0 | step: 0.1 | label: Speed
// layers: i32 = 15 | min: 5 | max: 25 | step: 1 | label: Layers
// wave_scale: f32 = 2.0 | min: 1.0 | max: 4.0 | step: 0.25 | label: Wave Scale
// color_speed: f32 = 0.3 | min: 0.1 | max: 1.0 | step: 0.1 | label: Color Speed
// scroll_speed: f32 = 0.2 | min: 0.0 | max: 0.5 | step: 0.05 | label: Scroll Speed
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const layers: i32 = 15;
const wave_scale: f32 = 2.0;
const color_speed: f32 = 0.3;
const scroll_speed: f32 = 0.2;

// Random hash
fn r(n: f32) -> f32 {
    return fract(sin(n) * 43758.5453);
}

// 1D noise
fn noise(v: f32) -> f32 {
    return mix(r(floor(v)), r(floor(v) + 1.0), smoothstep(0.0, 1.0, fract(v)));
}

// Hue to RGB
fn hue(v: f32) -> vec4<f32> {
    return 0.6 + 0.6 * cos(6.3 * v + vec4<f32>(0.0, 23.0, 21.0, 0.0));
}

// Wave layer. `n_top`/`n_bottom` are noise(u.x) and noise(u.x + 5.0),
// which are identical for every layer (layer scaling only affects y),
// so the caller evaluates them once instead of per layer.
fn wave(u: vec2<f32>, s: f32, C: vec4<f32>, t: f32, n_top: f32, n_bottom: f32) -> vec4<f32> {
    let top = n_top * noise(u.x - s - t) + 0.4;
    let bottom = n_bottom * noise(u.x - 9.0 - s - t) - 0.8;
    
    let wave_pos = (u.y - bottom) / (top - bottom) - 0.5;
    let color_blend = pow(abs(wave_pos) * 2.0, 9.0) + 0.05;
    let edge_mask = smoothstep(0.0, 5.0 / iResolution.y, (top - u.y) * (u.y - bottom));
    
    return mix(C, hue(s * 0.2 + t * color_speed + u.x * 0.1), color_blend) * edge_mask;
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let res = iResolution;
    let t = iTime * speed;
    
    // Flip Y for correct orientation
    let flipped = vec2<f32>(fragCoord.x, res.y - fragCoord.y);
    var u = (flipped * 2.0 - res) / res.y;
    
    var C = vec4<f32>(0.0);
    
    u.x -= t * scroll_speed;
    
    // Wave layers
    let ux = u.x * wave_scale;
    let n_top = noise(ux);
    let n_bottom = noise(ux + 5.0);
    let layer_step = 1.5 / f32(layers);
    var s = 1.5;
    for (var i = 0; i < layers; i++) {
        C = wave(u * vec2<f32>(wave_scale, 1.0 + s), -s, C, t, n_top, n_bottom);
        s -= layer_step;
    }
    
    return vec4<f32>(C.rgb, 1.0);
}
