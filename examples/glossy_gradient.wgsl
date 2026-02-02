// [SHADER]
// name: Glossy Gradients 
// author: @Peace (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/lX2GDR
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 0.15 | min: 0.05 | max: 0.5 | step: 0.05 | label: Speed
// iterations: i32 = 8 | min: 4 | max: 16 | step: 1 | label: Complexity
// saturation: f32 = 0.6 | min: 0.3 | max: 1.0 | step: 0.05 | label: Saturation
// [/PARAMS]

// Default parameter values
const speed: f32 = 0.15;
const iterations: i32 = 8;
const saturation: f32 = 0.6;

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let mr = min(iResolution.x, iResolution.y);
    let uv = (fragCoord.xy * 2.0 - iResolution) / mr;

    var d = -iTime * speed;
    var a = 0.0;
    for (var i = 0; i < iterations; i += 1) {
        let fi = f32(i);
        a += cos(fi - d - a * uv.x);
        d += sin(uv.y * fi + a);
    }
    d += iTime * speed;
    var col = vec3<f32>(cos(uv * vec2<f32>(d, a)) * saturation + 0.4, cos(a + d) * 0.5 + 0.5);
    col = cos(col * cos(vec3<f32>(d, a, 2.5)) * 0.5 + 0.5);
    return vec4<f32>(col, 1.0);
}
