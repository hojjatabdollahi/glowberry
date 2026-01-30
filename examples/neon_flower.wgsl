// [SHADER]
// name: Neon Flower
// author: al-ro (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/7ltBzl
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.2 | max: 3.0 | step: 0.1 | label: Speed
// exposure: f32 = 1.3 | min: 0.5 | max: 3.0 | step: 0.1 | label: Exposure
// petals: f32 = 3.0 | min: 2.0 | max: 8.0 | step: 1.0 | label: Petal Count
// iterations: i32 = 50 | min: 20 | max: 100 | step: 5 | label: Detail
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const exposure: f32 = 1.3;
const petals: f32 = 3.0;
const iterations: i32 = 50;

fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(1.0, 2.0/3.0, 1.0/3.0, 3.0);
    let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let time = iTime * speed;
    let resolution = iResolution;
    
    var pos = (fragCoord.xy - resolution * 0.5) / resolution.y;
    pos.x += sin(time + pos.y * 5.0) * 0.1;
    pos.y += cos(time * 0.5 + pos.x * 5.0) * 0.1;
    pos += 0.06 * vec2<f32>(sin(time * 0.25), cos(time * 0.21));
    pos += 0.02 * vec2<f32>(sin(time * 0.9 + pos.y * 2.0), cos(time * 0.8 + pos.x * 2.0));
    
    let pi = 3.14159;
    let n = f32(iterations);
    let radius = length(pos) * 5.0 - 1.6;
    let t = atan2(pos.y, pos.x) / pi;
    
    var acc = 0.0;
    for (var i = 0; i < iterations; i += 1) {
        let fi = f32(i);
        acc += 0.002 / abs(
            0.25 * sin(petals * pi * (t + time * 0.1 + fi / n)) +
            sin(fi * 0.1 - time) * 0.1 -
            radius
        );
    }
    
    // Neon color mapping
    let hue = fract(t * 0.5 + time * 0.05);
    let sat = 1.0;
    let val = acc / (1.0 + acc);
    
    // Build neon color
    var neon = hsv2rgb(vec3<f32>(hue, sat, val));
    neon *= exposure;
    
    return vec4<f32>(neon, 1.0);
}
