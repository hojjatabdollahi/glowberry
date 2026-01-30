// Cosmic swirl live wallpaper
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

fn cosRange(amt: f32, range: f32, minimum: f32) -> f32 {
    return (((1.0 + cos(radians(amt))) * 0.5) * range) + minimum;
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let zoom: i32 = 40;
    let brightness: f32 = 0.975;

    let time = iTime * 0.5;

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

    let vigAmt = 5.0;
    let vignette =
        (1.0 - vigAmt * (uv.y - 0.5) * (uv.y - 0.5)) *
        (1.0 - vigAmt * (uv.x - 0.5) * (uv.x - 0.5));

    // Apply vignette to darken edges toward black
    col *= max(vignette, 0.0);

    // Clamp to ensure no negative values
    col = max(col, vec3<f32>(0.0));

    return vec4<f32>(col, 1.0);
}
