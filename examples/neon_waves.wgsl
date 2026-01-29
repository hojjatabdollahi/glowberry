// Neon Waves - animated glowing wave lines
// Efficient shader with 6 iterations
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

fn Line(uv_in: vec2<f32>, speed: f32, height: f32, col: vec3<f32>) -> vec4<f32> {
    var uv = uv_in;
    uv.y += smoothstep(1.0, 0.0, abs(uv.x)) * sin(iTime * speed + uv.x * height) * 0.2;
    let line = smoothstep(0.06 * smoothstep(0.2, 0.9, abs(uv.x)), 0.0, abs(uv.y) - 0.004) * col;
    return vec4<f32>(line, 1.0) * smoothstep(1.0, 0.3, abs(uv.x));
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (fragCoord.xy - 0.5 * iResolution) / iResolution.y;
    var o = vec4<f32>(0.0);
    
    for (var i = 0.0; i <= 5.0; i += 1.0) {
        let t = i / 5.0;
        o += Line(uv, 0.3 + t * 0.3, 4.0 + t, vec3<f32>(0.2 + t * 0.7, 0.2 + t * 0.4, 0.3));
    }
    
    return o;
}
