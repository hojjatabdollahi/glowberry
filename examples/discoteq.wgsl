// [SHADER]
// name: Discoteq
// author: supah (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/DtXfDr
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 0.3 | min: 0.1 | max: 1.0 | step: 0.05 | label: Speed
// wave_height: f32 = 4.0 | min: 1.0 | max: 10.0 | step: 0.5 | label: Wave Height
// num_waves: i32 = 6 | min: 2 | max: 12 | step: 1 | label: Number of Waves
// glow: f32 = 0.06 | min: 0.02 | max: 0.15 | step: 0.01 | label: Glow Size
// [/PARAMS]

// Default parameter values
const speed: f32 = 0.3;
const wave_height: f32 = 4.0;
const num_waves: i32 = 6;
const glow: f32 = 0.06;

// `envelope`, `glow_w` and `fade` depend only on abs(uv.x), which is the same
// for every wave — the caller computes them once instead of per wave.
fn Line(
    uv_in: vec2<f32>,
    spd: f32,
    height: f32,
    col: vec3<f32>,
    envelope: f32,
    glow_w: f32,
    fade: f32,
) -> vec4<f32> {
    var uv = uv_in;
    uv.y += envelope * sin(iTime * spd + uv.x * height) * 0.2;
    let line = smoothstep(glow_w, 0.0, abs(uv.y) - 0.004) * col;
    return vec4<f32>(line, 1.0) * fade;
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (fragCoord.xy - 0.5 * iResolution) / iResolution.y;
    var o = vec4<f32>(0.0);

    let ax = abs(uv.x);
    let envelope = smoothstep(1.0, 0.0, ax);
    let glow_w = glow * smoothstep(0.2, 0.9, ax);
    let fade = smoothstep(1.0, 0.3, ax);

    let waves = num_waves - 1;
    for (var i = 0; i <= waves; i += 1) {
        let t = f32(i) / f32(waves);
        o += Line(uv, speed + t * speed, wave_height + t, vec3<f32>(0.2 + t * 0.7, 0.2 + t * 0.4, 0.3), envelope, glow_w, fade);
    }

    return o;
}
