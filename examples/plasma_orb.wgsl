// [SHADER]
// name: Plasma Orb
// author: @ns8 (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/tXGyRG
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.1 | max: 3.0 | step: 0.1 | label: Speed
// zoom: f32 = 1.3 | min: 0.5 | max: 2.5 | step: 0.1 | label: Zoom
// intensity: f32 = 2.5 | min: 1.0 | max: 5.0 | step: 0.25 | label: Intensity
// iterations: i32 = 8 | min: 4 | max: 16 | step: 1 | label: Detail
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const zoom: f32 = 1.3;
const intensity: f32 = 2.5;
const iterations: i32 = 8;

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    var o = vec4<f32>(0.0);
    
    let r = iResolution;
    let p = (fragCoord.xy * 2.0 - r) / r.y * zoom;
    
    let l = abs(0.7 - dot(p, p));
    var v = p * (1.0 - l) / 0.2;
    
    for (var i = 1; i <= iterations; i += 1) {
        let fi = f32(i);
        o += (sin(v.xyyx) + 1.0) * abs(v.x - v.y) * 0.2;
        v += cos(v.yx * fi + vec2<f32>(0.0, fi) + iTime * speed) / fi + 0.7;
    }
    
    // Color calculation with black background
    var col = tanh(exp(p.y * vec4<f32>(1.0, -1.0, -2.0, 0.0)) * exp(-4.0 * l) / o);
    
    // Boost colors and fade to black at edges
    col = col * col * intensity;
    col = col * smoothstep(1.2, 0.3, l);
    
    return vec4<f32>(col.rgb, 1.0);
}
