// [SHADER]
// name: Color Swirly
// author: morphix (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/tlVfz1
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.1 | max: 3.0 | step: 0.1 | label: Speed
// iterations: i32 = 8 | min: 4 | max: 16 | step: 1 | label: Detail
// x_scale: f32 = 1.9 | min: 1.0 | max: 4.0 | step: 0.1 | label: X Scale
// y_scale: f32 = 1.6 | min: 1.0 | max: 4.0 | step: 0.1 | label: Y Scale
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const iterations: i32 = 8;
const x_scale: f32 = 1.9;
const y_scale: f32 = 1.6;

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    var uv = (fragCoord.xy - 0.5 * iResolution) / iResolution.y;
    
    let time = iTime * speed;
    
    for (var i = 1; i < iterations + 1; i += 1) {
        let fi = f32(i);
        let new_x = uv.x + 0.5 / fi * sin(x_scale * fi * uv.y + time / 2.0 - cos(time / 66.0 + uv.x)) + 21.0;
        let new_y = uv.y + 0.4 / fi * cos(y_scale * fi * uv.x + time / 3.0 + sin(time / 55.0 + uv.y)) + 31.0;
        uv = vec2<f32>(new_x, new_y);
    }
    
    let col = vec3<f32>(
        sin(3.0 * uv.x - uv.y),
        sin(3.0 * uv.y),
        sin(3.0 * uv.x)
    );
    
    return vec4<f32>(col, 1.0);
}
