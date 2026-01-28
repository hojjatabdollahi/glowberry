// Classic plasma effect live wallpaper
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / iResolution;
    let t = iTime * 0.5;
    
    // Multiple overlapping sine waves create plasma effect
    var v = 0.0;
    v += sin((uv.x * 10.0) + t);
    v += sin((uv.y * 10.0) + t);
    v += sin((uv.x * 10.0 + uv.y * 10.0) + t);
    
    let cx = uv.x + 0.5 * sin(t / 5.0);
    let cy = uv.y + 0.5 * cos(t / 3.0);
    v += sin(sqrt(100.0 * (cx * cx + cy * cy) + 1.0) + t);
    
    v = v / 2.0;
    
    // Color palette
    let r = sin(v * 3.14159) * 0.5 + 0.5;
    let g = sin(v * 3.14159 + 2.094) * 0.5 + 0.5;
    let b = sin(v * 3.14159 + 4.188) * 0.5 + 0.5;
    
    return vec4<f32>(r * 0.8, g * 0.6, b, 1.0);
}
