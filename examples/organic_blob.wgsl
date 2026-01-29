// Organic Blob - animated 3D blob with soft lighting
// Very efficient ray marcher with only 6 iterations
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
}

fn map(p_in: vec3<f32>) -> f32 {
    var p = p_in;
    let t = iTime;
    
    // Rotate xz and xy
    let rxz = rot(t * 0.4);
    let rxy = rot(t * 0.3);
    let pxz = rxz * p.xz;
    p.x = pxz.x;
    p.z = pxz.y;
    let pxy = rxy * p.xy;
    p.x = pxy.x;
    p.y = pxy.y;
    
    let q = p * 2.0 + t;
    return length(p + vec3<f32>(sin(t * 0.7))) * log(length(p) + 1.0) 
         + sin(q.x + sin(q.z + sin(q.y))) * 0.5 - 1.0;
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = fragCoord.xy / iResolution.y - vec2<f32>(0.9, 0.5);
    var cl = vec3<f32>(0.0);
    var d = 2.5;
    
    for (var i = 0; i <= 5; i++) {
        let p = vec3<f32>(0.0, 0.0, 5.0) + normalize(vec3<f32>(uv, -1.0)) * d;
        let rz = map(p);
        let f = clamp((rz - map(p + 0.1)) * 0.5, -0.1, 1.0);
        let l = vec3<f32>(0.1, 0.3, 0.4) + vec3<f32>(5.0, 2.5, 3.0) * f;
        cl = cl * l + smoothstep(2.5, 0.0, rz) * 0.7 * l;
        d += min(rz, 1.0);
    }
    
    return vec4<f32>(cl, 1.0);
}
