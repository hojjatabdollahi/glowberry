// [SHADER]
// name: Turbulence
// author: 4echme (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/tXV3Dd
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.1 | max: 3.0 | step: 0.1 | label: Speed
// iterations: i32 = 50 | min: 10 | max: 80 | step: 5 | label: Detail
// saturation: f32 = 1.0 | min: 0.5 | max: 1.5 | step: 0.1 | label: Saturation
// brightness: f32 = 1.0 | min: 0.5 | max: 2.0 | step: 0.1 | label: Brightness
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const iterations: i32 = 50;
const saturation: f32 = 1.0;
const brightness: f32 = 1.0;

const TAU: f32 = 6.283185307;

// Compute rainbow color based on warped UV
fn rainbow(uv: vec2<f32>) -> vec3<f32> {
    let d = length(uv);
    let angle = atan2(uv.y, uv.x);
    let hue = angle / TAU + 0.5;
    let rgb = clamp(
        abs(((hue * 6.0 + vec3<f32>(0.0, 4.0, 2.0)) % 6.0) - 3.0) - 1.0,
        vec3<f32>(0.0),
        vec3<f32>(1.0)
    );
    return rgb * d;
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    // Normalize UV to 0..1
    var uv = fragCoord.xy / iResolution;
    
    let time = 193.52 + iTime * speed;
    let m = vec2<f32>(time, time);
    
    // Domain warping loop
    for (var i = 0; i < iterations; i += 1) {
        let fi = f32(i + 1);
        let divisor = f32(i * 6 + 10);
        uv.y += sin(uv.x * TAU * fi + m.x) / divisor;
        uv.x += sin(uv.y * TAU * fi + m.y) / divisor;
    }
    
    // Convert warped UV to centered coordinates for rainbow calculation
    let centered = uv * 2.0 - 1.0;
    let aspect = iResolution.x / iResolution.y;
    let corrected = vec2<f32>(centered.x, centered.y / max(aspect, 1.0 / aspect));
    
    // Get rainbow color
    var col = rainbow(corrected);
    
    // Apply saturation and brightness
    let gray = dot(col, vec3<f32>(0.299, 0.587, 0.114));
    col = mix(vec3<f32>(gray), col, saturation);
    col *= brightness;
    
    return vec4<f32>(col, 1.0);
}
