// Four Flames - colorful flame effect
// Based on https://x.com/cmzw_/status/1912538189010739688
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

// Procedural noise to replace texture lookup
fn hash(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i + vec2<f32>(0.0, 0.0)), hash(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y
    );
}

fn flame(u_in: vec2<f32>, s: f32, c1: vec3<f32>, c2: vec3<f32>) -> vec3<f32> {
    var u = u_in;
    let y = smoothstep(-0.4, 0.4, u.y);
    
    // Noise-based distortion (replacing texture lookup)
    let noise_uv = u * 0.02 + vec2<f32>(s - iTime * 0.03, s - iTime * 0.1);
    let n = noise(noise_uv * 50.0);
    u += n * y * vec2<f32>(0.7, 0.2);
    
    // Flame shape
    var f = smoothstep(0.2, 0.0, length(u) - 0.4);
    f *= smoothstep(0.0, 1.0, length(u + vec2<f32>(0.0, 0.35)));
    
    return f * mix(c1, c2, y);
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    // Flip Y coordinate (cosmic-bg has Y=0 at top, Shadertoy has Y=0 at bottom)
    let flipped = vec2<f32>(fragCoord.x, iResolution.y - fragCoord.y);
    let u = (flipped - 0.5 * iResolution) / iResolution.y * vec2<f32>(10.0, 1.3);
    
    // Four flames with different colors
    let f1 = flame(u + vec2<f32>( 3.0, 0.0), 0.1, vec3<f32>(0.9, 0.4, 0.6), vec3<f32>(0.9, 0.7, 0.3));  // Pink→Yellow
    let f2 = flame(u + vec2<f32>( 1.0, 0.0), 0.2, vec3<f32>(0.2, 0.6, 0.7), vec3<f32>(0.6, 0.8, 0.9));  // Cyan→Light blue
    let f3 = flame(u + vec2<f32>(-1.0, 0.0), 0.3, vec3<f32>(0.9, 0.4, 0.3), vec3<f32>(1.0, 0.8, 0.5));  // Orange→Yellow
    let f4 = flame(u + vec2<f32>(-3.0, 0.0), 0.4, vec3<f32>(0.2, 0.3, 0.8), vec3<f32>(0.9, 0.6, 0.9));  // Blue→Purple
    
    let C = f1 + f2 + f3 + f4;
    return vec4<f32>(C + C, 1.0);  // Double brightness
}
