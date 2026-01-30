// [SHADER]
// name: Black Hole
// author: srtuss (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/MsXXWl
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// rotation_speed: f32 = 0.03 | min: 0.0 | max: 0.2 | step: 0.01 | label: Rotation Speed
// gas_speed: f32 = 0.2 | min: 0.0 | max: 1.0 | step: 0.05 | label: Gas Speed
// zoom: f32 = 0.7 | min: 0.3 | max: 1.5 | step: 0.1 | label: Zoom
// [/PARAMS]

// Default parameter values
const rotation_speed: f32 = 0.03;
const gas_speed: f32 = 0.2;
const zoom: f32 = 0.7;

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    // Flip Y coordinate to match GLSL convention (Y=0 at bottom)
    let F = vec2<f32>(fragCoord.x, iResolution.y - fragCoord.y);
    var i = 0.2;
    let r = iResolution;
    
    // Center coordinates: map to [-aspect, aspect] x [-1, 1] centered at screen middle
    let p = (2.0 * F - r) / r.y / zoom;
    
    let d = vec2<f32>(-1.0, 1.0);
    let b = p - i * d;
    
    let denom = 0.1 + i / dot(b, b);
    let c = vec2<f32>(
        dot(p, vec2<f32>(1.0, 1.0)),
        dot(p, d / denom)
    );

    let a = dot(c, c);
    let angle = 0.5 * log(a) + iTime * rotation_speed;
    let cosA = cos(angle);
    let sinA = sin(angle);
    
    let rotated = vec2<f32>(
        dot(c, vec2<f32>(cosA, sinA)),
        dot(c, vec2<f32>(-sinA, cosA))
    );
    var v = rotated / i;

    var w = vec4<f32>(0.0);
    
    loop {
        if (i >= 9.0) {
            break;
        }
        i += 1.0;
        let s = sin(v);
        w += vec4<f32>(s.x, s.y, s.y, s.x) + 1.0;
        v += 0.7 * sin(v.yx * i + iTime * gas_speed) / i + 0.5;
    }

    let i2 = length(sin(v / 0.3) * 0.4 + c * (3.0 + d));

    // Final color
    let exp_term = exp(c.x * vec4<f32>(0.6, -0.4, -1.0, 0.0));
    let denom2 = 2.0 + i2 * i2 / 4.0 - i2;
    let denom3 = 0.5 + 1.0 / a;
    let denom4 = 0.03 + abs(length(p) - zoom);
    
    var color = 1.0 - exp(-exp_term / w / denom2 / denom3 / denom4);
    
    // Clamp and ensure solid black background
    color = clamp(color, vec4<f32>(0.0), vec4<f32>(1.0));
    color.w = 1.0;
    
    return color;
}
