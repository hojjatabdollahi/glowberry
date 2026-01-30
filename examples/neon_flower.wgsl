// Neon Flower - flowing neon spiral with rainbow colors
// Moderate cost shader with 50 iterations
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

fn hsv2rgb(c: vec3<f32>) -> vec3<f32> {
    let K = vec4<f32>(1.0, 2.0/3.0, 1.0/3.0, 3.0);
    let p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), c.y);
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let time = iTime;
    let resolution = iResolution;
    let EXPOSURE = 1.3;
    
    var pos = (fragCoord.xy - resolution * 0.5) / resolution.y;
    pos.x += sin(time + pos.y * 5.0) * 0.1;
    pos.y += cos(time * 0.5 + pos.x * 5.0) * 0.1;
    pos += 0.06 * vec2<f32>(sin(time * 0.25), cos(time * 0.21));
    pos += 0.02 * vec2<f32>(sin(time * 0.9 + pos.y * 2.0), cos(time * 0.8 + pos.x * 2.0));
    
    let pi = 3.14159;
    let n = 50.0;
    let radius = length(pos) * 5.0 - 1.6;
    let t = atan2(pos.y, pos.x) / pi;
    
    var acc = 0.0;
    for (var i = 0.0; i < n; i += 1.0) {
        acc += 0.002 / abs(
            0.25 * sin(3.0 * pi * (t + time * 0.1 + i / n)) +
            sin(i * 0.1 - time) * 0.1 -
            radius
        );
    }
    
    // Neon color mapping
    let hue = fract(t * 0.5 + time * 0.05);
    let sat = 1.0;
    // Structure-preserving compression (tames spikes)
    let val = acc / (1.0 + acc);
    
    // Build neon color
    var neon = hsv2rgb(vec3<f32>(hue, sat, val));
    // Apply exposure
    neon *= EXPOSURE;
    
    return vec4<f32>(neon, 1.0);
}
