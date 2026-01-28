// Star Nest by Pablo Roman Andrioli
// License: MIT
// Converted to WGSL for cosmic-bg
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

const iterations: i32 = 17;
const formuparam: f32 = 0.53;

const volsteps: i32 = 20;
const stepsize: f32 = 0.1;

const zoom: f32 = 1.800;
const tile: f32 = 1.850;
const speed: f32 = 0.005;

const brightness: f32 = 0.0015;
const darkmatter: f32 = 0.300;
const distfading: f32 = 0.730;
const saturation: f32 = 0.850;

fn rot2d(angle: f32) -> mat2x2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return mat2x2<f32>(c, s, -s, c);
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    // Since we don't have mouse input, use a fixed value
    // (or slowly rotate based on time)
    let mousePx = vec2<f32>(0.5, 0.5) * iResolution;

    // get coords and direction
    var uv = fragCoord.xy / iResolution - 0.5;
    uv.y *= iResolution.y / iResolution.x;
    var dir = vec3<f32>(uv * zoom, 1.0);
    let time = iTime * speed + 0.25;

    // rotation angles
    let a1 = 0.5 + (mousePx.x / iResolution.x) * 2.0;
    let a2 = 0.8 + (mousePx.y / iResolution.y) * 2.0;
    let rot1 = rot2d(a1);
    let rot2 = rot2d(a2);

    // Apply rotations to direction
    let dir_xz = rot1 * dir.xz;
    dir = vec3<f32>(dir_xz.x, dir.y, dir_xz.y);
    let dir_xy = rot2 * dir.xy;
    dir = vec3<f32>(dir_xy.x, dir_xy.y, dir.z);

    // Starting position
    var origin = vec3<f32>(1.0, 0.5, 0.5);
    origin += vec3<f32>(time * 2.0, time, -2.0);
    let origin_xz = rot1 * origin.xz;
    origin = vec3<f32>(origin_xz.x, origin.y, origin_xz.y);
    let origin_xy = rot2 * origin.xy;
    origin = vec3<f32>(origin_xy.x, origin_xy.y, origin.z);

    // volumetric rendering
    var s = 0.1;
    var fade = 1.0;
    var v = vec3<f32>(0.0);

    for (var r: i32 = 0; r < volsteps; r++) {
        var p = origin + s * dir * 0.5;

        // tiling fold
        let tile2 = tile * 2.0;
        p = abs(vec3<f32>(tile) - (p - floor(p / tile2) * tile2));

        var pa = 0.0;
        var a = 0.0;

        for (var i: i32 = 0; i < iterations; i++) {
            p = abs(p) / dot(p, p) - formuparam;   // the magic formula
            a += abs(length(p) - pa);              // absolute sum of average change
            pa = length(p);
        }

        let dm = max(0.0, darkmatter - a * a * 0.001); // dark matter
        a *= a * a;                                      // add contrast

        if (r > 6) { 
            fade *= (1.0 - dm);                   // don't render near dark matter
        }

        v += fade;
        v += vec3<f32>(s, s * s, s * s * s * s) * a * brightness * fade; // distance-based coloring
        fade *= distfading;                               // distance fading
        s += stepsize;
    }

    v = mix(vec3<f32>(length(v)), v, saturation);              // color adjust
    return vec4<f32>(v * 0.01, 1.0);
}
