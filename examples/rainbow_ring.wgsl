// Rainbow Ring - animated rainbow beam in polar coordinates
// Very efficient shader with no loops
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

const TAU: f32 = 6.2831853070;

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = (2.0 * fragCoord.xy - iResolution) / iResolution.y;
    let a = atan2(p.x, p.y);
    let r = length(p) * 0.75;
    var uv = vec2<f32>(a / TAU, r);

    // Get the color
    var xCol = (uv.x - (iTime / 3.0)) * 3.0;
    xCol = xCol % 3.0;
    if (xCol < 0.0) {
        xCol += 3.0;
    }
    var horColour = vec3<f32>(0.25, 0.25, 0.25);

    if (xCol < 1.0) {
        horColour.r += 1.0 - xCol;
        horColour.g += xCol;
    } else if (xCol < 2.0) {
        xCol -= 1.0;
        horColour.g += 1.0 - xCol;
        horColour.b += xCol;
    } else {
        xCol -= 2.0;
        horColour.b += 1.0 - xCol;
        horColour.r += xCol;
    }

    // Draw color beam
    uv = (2.0 * uv) - 1.0;
    let beamWidth = abs(1.0 / (60.0 * uv.y));  // Sharper ring (increased from 30)
    let horBeam = vec3<f32>(pow(beamWidth, 1.5));  // Harder edge falloff
    return vec4<f32>(horBeam * horColour, 1.0);
}
