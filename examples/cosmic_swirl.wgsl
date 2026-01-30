// [SHADER]
// name: Cosmic Swirl
// author: Xor (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/Mss3WN
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 0.5 | min: 0.1 | max: 2.0 | step: 0.1 | label: Speed
// zoom: i32 = 40 | min: 10 | max: 80 | step: 5 | label: Zoom
// brightness: f32 = 0.975 | min: 0.5 | max: 1.5 | step: 0.025 | label: Brightness
// vignette: f32 = 5.0 | min: 0.0 | max: 10.0 | step: 0.5 | label: Vignette
// [/PARAMS]

// Default parameter values
const speed: f32 = 0.5;
const zoom: i32 = 40;
const brightness: f32 = 0.975;
const vignette: f32 = 5.0;

fn cosRange(amt: f32, range: f32, minimum: f32) -> f32 {
    return (((1.0 + cos(radians(amt))) * 0.5) * range) + minimum;
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let time = iTime * speed;

    let uv = fragCoord.xy / iResolution;

    // Fill the screen instead of inscribing into a square
    var p = (2.0 * fragCoord.xy - iResolution) / iResolution; // [-1..1] both axes
    // Preserve aspect so it doesn't look squished
    p.x *= iResolution.x / iResolution.y;

    let ct     = cosRange(time * 5.0,  3.0,  1.1);
    let xBoost = cosRange(time * 0.2,  5.0,  5.0);
    let yBoost = cosRange(time * 0.1, 10.0,  5.0);
    let fScale = cosRange(time * 15.5, 1.25, 0.5);

    for (var i: i32 = 1; i < zoom; i++) {
        let fi = f32(i);
        var newp = p;

        newp.x += (0.25 / fi) * sin(fi * p.y + time * cos(ct) * 0.5 / 20.0 + 0.005 * fi) * fScale + xBoost;
        newp.y += (0.25 / fi) * sin(fi * p.x + time * ct        * 0.3 / 40.0 + 0.03 * f32(i + 15)) * fScale + yBoost;

        p = newp;
    }

    var col = vec3<f32>(
        0.5 * sin(3.0 * p.x) + 0.5,
        0.5 * sin(3.0 * p.y) + 0.5,
        0.5 * sin(p.x + p.y) + 0.5
    );
    col *= brightness;

    let vig =
        (1.0 - vignette * (uv.y - 0.5) * (uv.y - 0.5)) *
        (1.0 - vignette * (uv.x - 0.5) * (uv.x - 0.5));

    // Apply vignette to darken edges toward black
    col *= max(vig, 0.0);

    // Clamp to ensure no negative values
    col = max(col, vec3<f32>(0.0));

    return vec4<f32>(col, 1.0);
}
