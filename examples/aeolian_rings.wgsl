// [SHADER]
// name: Aeolian Rings
// author: @Pelsmith3377 (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/w3tyWn
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
    // Both sine arguments are affine in the loop index, so each step advances
    // the previous sample by a constant-angle rotation. This replaces the two
    // sin() calls per iteration (100 for the default detail) with four
    // evaluated once plus a few multiply-adds per step.
    let a1 = petals * pi * (t + time * 0.1); // petal angle at i=0
    let d1 = petals * pi / n; //                per-step increment
    var s1 = sin(a1);
    var c1 = cos(a1);
    let sd1 = sin(d1);
    let cd1 = cos(d1);
    var s2 = sin(-time); //                     shimmer angle at i=0
    var c2 = cos(-time);
    let sd2 = sin(0.1);
    let cd2 = cos(0.1);
    for (var i = 0; i < iterations; i += 1) {
        acc += 0.002 / abs(0.25 * s1 + s2 * 0.1 - radius);
        let ns1 = s1 * cd1 + c1 * sd1;
        c1 = c1 * cd1 - s1 * sd1;
        s1 = ns1;
        let ns2 = s2 * cd2 + c2 * sd2;
        c2 = c2 * cd2 - s2 * sd2;
        s2 = ns2;
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
