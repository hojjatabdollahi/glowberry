// [SHADER]
// name: Gradient Flow
// author: @hahnzhu (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/wdyczG
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 0.5 | min: 0.1 | max: 2.0 | step: 0.1 | label: Speed
// wave_scale: f32 = 6.0 | min: 2.0 | max: 12.0 | step: 0.5 | label: Wave Scale
// red_intensity: f32 = 0.3 | min: 0.0 | max: 1.0 | step: 0.05 | label: Red
// green_intensity: f32 = 0.2 | min: 0.0 | max: 1.0 | step: 0.05 | label: Green
// blue_intensity: f32 = 0.4 | min: 0.0 | max: 1.0 | step: 0.05 | label: Blue
// [/PARAMS]

// Default parameter values
const speed: f32 = 0.5;
const wave_scale: f32 = 6.0;
const red_intensity: f32 = 0.3;
const green_intensity: f32 = 0.2;
const blue_intensity: f32 = 0.4;

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Normalize coordinates to 0..1
    let uv = pos.xy / iResolution;
    
    // Create animated wave pattern
    let wave1 = sin(uv.x * wave_scale + iTime * speed) * 0.5 + 0.5;
    let wave2 = sin(uv.y * wave_scale * 0.67 - iTime * speed * 0.6) * 0.5 + 0.5;
    let wave3 = sin((uv.x + uv.y) * wave_scale * 0.5 + iTime * speed * 1.4) * 0.5 + 0.5;
    
    // Blend waves into color channels
    let r = wave1 * red_intensity + 0.1;
    let g = wave2 * green_intensity + 0.05;
    let b = wave3 * blue_intensity + 0.3;
    
    return vec4<f32>(r, g, b, 1.0);
}
