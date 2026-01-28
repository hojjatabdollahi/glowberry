// Pulsing brightness effect for background images
// cosmic-bg provides: iResolution (vec2f), iTime (f32), iTexture, iTextureSampler
// Requires a background_image to be set in the config

@fragment
fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / iResolution;
    
    // Sample the background texture
    let bg = textureSample(iTexture, iTextureSampler, uv);
    
    // Create a gentle pulsing brightness effect
    let pulse = 0.85 + 0.15 * sin(iTime * 1.5);
    
    // Add subtle color shift based on position
    let shift = 0.02 * sin(iTime * 0.5 + uv.x * 3.14159);
    
    var color = bg.rgb * pulse;
    color.r += shift;
    color.b -= shift;
    
    return vec4<f32>(color, bg.a);
}
