// [SHADER]
// name: Origami
// author: XorDev (adapted for GlowBerry)
// source: https://x.com/XorDev/status/1727206969038213426
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.2 | max: 3.0 | step: 0.1 | label: Speed
// layers: i32 = 6 | min: 3 | max: 10 | step: 1 | label: Layers
// saturation: f32 = 0.2 | min: 0.0 | max: 0.5 | step: 0.05 | label: Saturation
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const layers: i32 = 6;
const saturation: f32 = 0.2;

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let r = iResolution;
    
    // Initialize output to white
    var O = vec4<f32>(1.0);
    var h = vec4<f32>(0.0);
    
    // Layers
    let layer_step = 0.5 / f32(layers);
    var i = 0.6;
    for (var layer = 0; layer < layers; layer++) {
        // Smoothly rotate a quarter at a time
        var a = (iTime * speed + i) * 4.0;
        a = a - sin(a);
        a = a - sin(a);
        
        // Rotation matrix
        let cv = cos(a / 4.0 + vec4<f32>(0.0, 11.0, 33.0, 0.0));
        let R = mat2x2<f32>(cv.x, cv.z, cv.y, cv.w);
        
        // Scale and center
        var u = (fragCoord.xy * 2.0 - r) / r.y;
        
        // Compute round square SDF
        let uR = u * R;
        let clamped = clamp(uR, vec2<f32>(-i), vec2<f32>(i));
        u = u - R * clamped;
        
        let l = max(length(u), 0.1);
        let A = min((l - 0.1) * r.y * 0.2, 1.0);
        
        h = sin(i / 0.1 + a / 3.0 + vec4<f32>(1.0, 3.0, 5.0, 0.0)) * saturation + 0.7;
        
        let shading = mix(vec4<f32>(1.0), h + 0.5 * A * u.y / l, 0.1 / l);
        O = mix(h, O, A) * shading;
        
        i -= layer_step;
    }
    
    return vec4<f32>(O.rgb, 1.0);
}
