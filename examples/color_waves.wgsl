// Color Waves - animated colorful wave pattern
// Efficient shader with only 8 iterations
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let mr = min(iResolution.x, iResolution.y);
    let uv = (fragCoord.xy * 2.0 - iResolution) / mr;

    var d = -iTime * 0.15;
    var a = 0.0;
    for (var i = 0.0; i < 8.0; i += 1.0) {
        a += cos(i - d - a * uv.x);
        d += sin(uv.y * i + a);
    }
    d += iTime * 0.15;
    var col = vec3<f32>(cos(uv * vec2<f32>(d, a)) * 0.6 + 0.4, cos(a + d) * 0.5 + 0.5);
    col = cos(col * cos(vec3<f32>(d, a, 2.5)) * 0.5 + 0.5);
    return vec4<f32>(col, 1.0);
}
