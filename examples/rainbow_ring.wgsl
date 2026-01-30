// [SHADER]
// name: Rainbow Ring
// author: al-ro (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/wl2Bzw
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 0.33 | min: 0.1 | max: 1.0 | step: 0.05 | label: Speed
// ring_size: f32 = 0.75 | min: 0.3 | max: 1.5 | step: 0.05 | label: Ring Size
// sharpness: f32 = 60.0 | min: 20.0 | max: 120.0 | step: 5.0 | label: Sharpness
// brightness: f32 = 1.5 | min: 0.5 | max: 3.0 | step: 0.1 | label: Brightness
// [/PARAMS]

// Default parameter values
const speed: f32 = 0.33;
const ring_size: f32 = 0.75;
const sharpness: f32 = 60.0;
const brightness: f32 = 1.5;

const TAU: f32 = 6.2831853070;

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = (2.0 * fragCoord.xy - iResolution) / iResolution.y;
    let a = atan2(p.x, p.y);
    let r = length(p) * ring_size;
    var uv = vec2<f32>(a / TAU, r);

    // Get the color
    var xCol = (uv.x - (iTime * speed)) * 3.0;
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
    let beamWidth = abs(1.0 / (sharpness * uv.y));
    let horBeam = vec3<f32>(pow(beamWidth, brightness));
    return vec4<f32>(horBeam * horColour, 1.0);
}
