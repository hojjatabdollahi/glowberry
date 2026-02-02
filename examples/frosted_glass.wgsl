// [SHADER]
// name: Frosted Glass
// author: Supah
// source: https://www.shadertoy.com/view/7tyyDy
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.1 | max: 3.0 | step: 0.1 | label: Speed
// blur_amount: f32 = 0.13 | min: 0.05 | max: 0.3 | step: 0.01 | label: Blur Amount
// circle_size: f32 = 0.2 | min: 0.1 | max: 0.4 | step: 0.02 | label: Circle Size
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const blur_amount: f32 = 0.13;
const circle_size: f32 = 0.2;

fn circle(uv: vec2<f32>, r: f32, blur: bool) -> f32 {
    var a: f32;
    var b: f32;
    if blur {
        a = 0.01;
        b = blur_amount;
    } else {
        a = 0.0;
        b = 5.0 / iResolution.y;
    }
    return smoothstep(a, b, length(uv) - r);
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (fragCoord.xy - 0.5 * iResolution) / iResolution.y;
    
    // Animated offset for circle movement
    let t = vec2<f32>(
        sin(iTime * 2.0 * speed),
        cos(iTime * 3.0 * speed + cos(iTime * 0.5 * speed))
    ) * 0.1;
    
    // Colors
    let col0 = vec3<f32>(0.9);
    let col1 = vec3<f32>(0.1 + uv.y * 2.0, 0.4 + uv.x * -1.1, 0.8) * 0.828;
    let col2 = vec3<f32>(0.86);
    
    // Circle calculations
    let cir1 = circle(uv - t, circle_size, false);
    let cir2 = circle(uv + t, circle_size, false);
    let cir2_blur = circle(uv + t, circle_size - 0.05, true);
    
    // Color blending
    var col = mix(col1 + vec3<f32>(0.3, 0.1, 0.0), col2, cir2_blur);
    col = mix(col, col0, cir1);
    col = mix(col, col1, clamp(cir1 - cir2, 0.0, 1.0));
    
    return vec4<f32>(col, 1.0);
}
