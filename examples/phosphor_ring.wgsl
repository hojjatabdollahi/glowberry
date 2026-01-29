// Phosphor Ring by @XorDev
// Glowing phosphor ring with turbulence
// https://x.com/XorDev/status/1945504914253205515
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let t = iTime;
    var z = 0.0;
    var d = 0.0;
    var s = 0.0;
    var o = vec4<f32>(0.0);
    
    // Raymarch 25 steps (reduced from 80)
    for (var i = 0.0; i < 25.0; i += 1.0) {
        // Sample point (from ray direction)
        var p = z * normalize(vec3<f32>(fragCoord.xy * 2.0, 0.0) - vec3<f32>(iResolution, iResolution.y));
        
        // Rotation axis
        var a = normalize(cos(vec3<f32>(1.0, 2.0, 0.0) + t - d * 8.0));
        
        // Move camera back 5 units
        p.z += 5.0;
        
        // Rotated coordinates
        a = a * dot(a, p) - cross(a, p);
        
        // Turbulence loop (reduced from 8 to 4 iterations)
        d = 1.0;
        for (var j = 2.0; j < 6.0; j += 1.0) {
            a += sin(a * j + t).yzx / j;
        }
        
        // Distance to ring
        s = a.y;
        d = 0.1 * abs(length(p) - 3.0) + 0.04 * abs(s);
        z += d;
        
        // Coloring and brightness
        o += (cos(s + vec4<f32>(0.0, 1.0, 2.0, 0.0)) + 1.0) / d * z;
    }
    
    // Tanh tonemap (adjusted for fewer iterations)
    let col = tanh(o / 4e3);
    return vec4<f32>(col.rgb, 1.0);
}
