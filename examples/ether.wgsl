// [SHADER]
// name: Ether
// author: @nimitz (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/MsjSW3
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.2 | max: 3.0 | step: 0.1 | label: Speed
// iterations: i32 = 6 | min: 3 | max: 12 | step: 1 | label: Detail
// glow: f32 = 0.7 | min: 0.3 | max: 1.5 | step: 0.1 | label: Glow
// distance: f32 = 2.5 | min: 1.5 | max: 5.0 | step: 0.25 | label: Distance
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const iterations: i32 = 6;
const glow: f32 = 0.7;
const distance: f32 = 2.5;

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
}

fn map(p_in: vec3<f32>) -> f32 {
    var p = p_in;
    let t = iTime * speed;
    
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
    var d = distance;
    
    for (var i = 0; i < iterations; i++) {
        let p = vec3<f32>(0.0, 0.0, 5.0) + normalize(vec3<f32>(uv, -1.0)) * d;
        let rz = map(p);
        let f = clamp((rz - map(p + 0.1)) * 0.5, -0.1, 1.0);
        // Purple/violet and cyan/teal colors
        let l = vec3<f32>(0.2, 0.05, 0.3) + vec3<f32>(2.0, 4.0, 5.0) * f;
        cl = cl * l + smoothstep(distance, 0.0, rz) * glow * l;
        d += min(rz, 1.0);
    }
    
    return vec4<f32>(cl, 1.0);
}
