// [SHADER]
// name: Neon Waves
// author: al-ro (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/WdK3Dz
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

fn Line(uv_in: vec2<f32>, spd: f32, height: f32, col: vec3<f32>) -> vec4<f32> {
    var uv = uv_in;
    uv.y += smoothstep(1.0, 0.0, abs(uv.x)) * sin(iTime * spd + uv.x * height) * 0.2;
    let line = smoothstep(glow * smoothstep(0.2, 0.9, abs(uv.x)), 0.0, abs(uv.y) - 0.004) * col;
    return vec4<f32>(line, 1.0) * smoothstep(1.0, 0.3, abs(uv.x));
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (fragCoord.xy - 0.5 * iResolution) / iResolution.y;
    var o = vec4<f32>(0.0);
    
    let waves = num_waves - 1;
    for (var i = 0; i <= waves; i += 1) {
        let t = f32(i) / f32(waves);
        o += Line(uv, speed + t * speed, wave_height + t, vec3<f32>(0.2 + t * 0.7, 0.2 + t * 0.4, 0.3));
    }
    
    return o;
}
