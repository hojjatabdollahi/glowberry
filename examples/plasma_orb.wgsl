// Plasma Orb - smooth animated plasma effect
// Efficient shader with only 8 iterations
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    var o = vec4<f32>(0.0);
    
    let r = iResolution;
    let p = (fragCoord.xy * 2.0 - r) / r.y * 1.3;  // Zoom out 30%
    
    let l = abs(0.7 - dot(p, p));
    var v = p * (1.0 - l) / 0.2;
    
    for (var i = 1.0; i <= 8.0; i += 1.0) {
        o += (sin(v.xyyx) + 1.0) * abs(v.x - v.y) * 0.2;
        v += cos(v.yx * i + vec2<f32>(0.0, i) + iTime) / i + 0.7;
    }
    
    // Color calculation with black background
    var col = tanh(exp(p.y * vec4<f32>(1.0, -1.0, -2.0, 0.0)) * exp(-4.0 * l) / o);
    
    // Boost colors and fade to black at edges
    col = col * col * 2.5;  // Increase contrast and saturation
    col = col * smoothstep(1.2, 0.3, l);  // Fade to black away from orb
    
    return vec4<f32>(col.rgb, 1.0);
}
