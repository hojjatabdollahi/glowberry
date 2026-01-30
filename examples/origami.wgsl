// Origami by @XorDev
// Layered paper effect with soft bounce lighting
// https://x.com/XorDev/status/1727206969038213426
// cosmic-bg provides: iResolution (vec2f), iTime (f32)
// 
// Ported from the "Original [329]" version for clarity

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let r = iResolution;
    
    // Initialize output to white
    var O = vec4<f32>(1.0);
    var h = vec4<f32>(0.0);
    
    // 6 layers (i from 0.6 down to 0.1, step 0.1)
    var i = 0.6;
    while (i > 0.1) {
        // Smoothly rotate a quarter at a time
        var a = (iTime + i) * 4.0;
        a = a - sin(a);
        a = a - sin(a);
        
        // Rotation matrix: mat2(cos(a/4.+vec4(0,11,33,0)))
        let cv = cos(a / 4.0 + vec4<f32>(0.0, 11.0, 33.0, 0.0));
        let R = mat2x2<f32>(cv.x, cv.z, cv.y, cv.w);
        
        // Scale and center
        var u = (fragCoord.xy * 2.0 - r) / r.y;
        
        // Compute round square SDF
        // u -= R * clamp(u*R, -i, i)
        let uR = u * R;
        let clamped = clamp(uR, vec2<f32>(-i), vec2<f32>(i));
        u = u - R * clamped;
        
        // l = max(length(u), 0.1)
        let l = max(length(u), 0.1);
        
        // A = min((l-0.1)*r.y*0.2, 1.0)
        let A = min((l - 0.1) * r.y * 0.2, 1.0);
        
        // h = sin(i/0.1 + a/3 + vec4(1,3,5,0)) * 0.2 + 0.7
        h = sin(i / 0.1 + a / 3.0 + vec4<f32>(1.0, 3.0, 5.0, 0.0)) * 0.2 + 0.7;
        
        // O = mix(h, O, A) * mix(h/h, h + 0.5*A*u.y/l, 0.1/l)
        let shading = mix(vec4<f32>(1.0), h + 0.5 * A * u.y / l, 0.1 / l);
        O = mix(h, O, A) * shading;
        
        i -= 0.1;
    }
    
    return vec4<f32>(O.rgb, 1.0);
}
