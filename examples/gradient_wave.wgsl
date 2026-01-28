// Animated gradient wave live wallpaper
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Normalize coordinates to 0..1
    let uv = pos.xy / iResolution;
    
    // Create animated wave pattern
    let wave1 = sin(uv.x * 6.0 + iTime * 0.5) * 0.5 + 0.5;
    let wave2 = sin(uv.y * 4.0 - iTime * 0.3) * 0.5 + 0.5;
    let wave3 = sin((uv.x + uv.y) * 3.0 + iTime * 0.7) * 0.5 + 0.5;
    
    // Blend waves into color channels
    let r = wave1 * 0.3 + 0.1;
    let g = wave2 * 0.2 + 0.05;
    let b = wave3 * 0.4 + 0.3;
    
    return vec4<f32>(r, g, b, 1.0);
}
