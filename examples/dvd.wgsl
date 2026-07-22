// [SHADER]
// name: DVD
// author: tdhooper (adapted for GlowBerry)
// source: https://www.shadertoy.com/view/wsjXWt
// license: CC BY-NC-SA 3.0
//
// [PARAMS]
// speed: f32 = 1.0 | min: 0.0 | max: 3.0 | step: 0.1 | label: Speed
// logo_scale: f32 = 0.1 | min: 0.03 | max: 0.3 | step: 0.01 | label: Logo Scale
// [/PARAMS]

// Default parameter values
const speed: f32 = 1.0;
const logo_scale: f32 = 0.1;

const PI: f32 = 3.14159265359;

// GLSL-style mod (WGSL's % is a truncated remainder, which differs for
// negative operands).
fn mod1(x: f32, y: f32) -> f32 { return x - y * floor(x / y); }
fn modv2(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> { return x - y * floor(x / y); }

fn vmin(v: vec2<f32>) -> f32 { return min(v.x, v.y); }
fn vmax(v: vec2<f32>) -> f32 { return max(v.x, v.y); }

fn ellip(p: vec2<f32>, s: vec2<f32>) -> f32 {
    let m = vmin(s);
    return (length(p / s) * m) - m;
}

fn halfEllip(p: vec2<f32>, s: vec2<f32>) -> f32 {
    var q = p;
    q.x = max(0.0, q.x);
    let m = vmin(s);
    return (length(q / s) * m) - m;
}

// --- DVD logo distance field ------------------------------------------------

fn dvd_d(p: vec2<f32>) -> f32 {
    var d = halfEllip(p, vec2<f32>(0.8, 0.5));
    d = max(d, -p.x - 0.5);
    var d2 = halfEllip(p, vec2<f32>(0.45, 0.3));
    d2 = max(d2, min(-p.y + 0.2, -p.x - 0.15));
    d = max(d, -d2);
    return d;
}

fn dvd_v(p: vec2<f32>) -> f32 {
    let pp = p;
    var q = p;
    q.y += 0.7;
    q.x = abs(q.x);
    let a = normalize(vec2<f32>(1.0, -0.55));
    var d = dot(q, a);
    var d2 = d + 0.3;
    q = pp;
    d = min(d, -q.y + 0.3);
    d2 = min(d2, -q.y + 0.5);
    d = max(d, -d2);
    d = max(d, abs(q.x + 0.3) - 1.1);
    return d;
}

fn dvd_c(p: vec2<f32>) -> f32 {
    var q = p;
    q.y += 0.95;
    var d = ellip(q, vec2<f32>(1.8, 0.25));
    let d2 = ellip(q, vec2<f32>(0.45, 0.09));
    d = max(d, -d2);
    return d;
}

fn dvd(p: vec2<f32>) -> f32 {
    var q = p;
    q.y -= 0.345;
    q.x -= 0.035;
    q = q * mat2x2<f32>(1.0, -0.2, 0.0, 1.0);
    var d = dvd_v(q);
    d = min(d, dvd_c(q));
    q.x += 1.3;
    d = min(d, dvd_d(q));
    q.x -= 2.4;
    d = min(d, dvd_d(q));
    return d;
}

// --- Helpers ----------------------------------------------------------------

fn range_(vmin_: f32, vmax_: f32, value: f32) -> f32 {
    return (value - vmin_) / (vmax_ - vmin_);
}

fn rangec(a: f32, b: f32, t: f32) -> f32 {
    return clamp(range_(a, b, t), 0.0, 1.0);
}

// https://www.shadertoy.com/view/ll2GD3
fn pal(t: f32, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return a + b * cos(6.28318 * (c * t + d));
}

fn spectrum(n: f32) -> vec3<f32> {
    return pal(n, vec3<f32>(0.5), vec3<f32>(0.5), vec3<f32>(1.0), vec3<f32>(0.0, 0.33, 0.67));
}

fn drawHit(col: ptr<function, vec4<f32>>, p: vec2<f32>, hitPos: vec2<f32>, hitDist: f32) {
    let d = length(p - hitPos);

    let wavefront = d - hitDist * 1.5;
    let freq = 2.0;

    let spec = (1.0 - spectrum(-wavefront * freq + hitDist * freq));
    let ripple = sin((wavefront * freq) * PI * 2.0 - PI / 2.0);

    var blend = smoothstep(3.0, 0.0, hitDist);
    blend *= smoothstep(0.2, -0.5, wavefront);
    blend *= rangec(-4.0, 0.0, wavefront);

    let newRgb = (*col).rgb * mix(vec3<f32>(1.0), spec, pow(blend, 4.0));
    let height = (ripple * blend);
    let newA = (*col).a - height * 1.9 / freq;
    *col = vec4<f32>(newRgb, newA);
}

fn reflectPlane(p: vec2<f32>, planeNormal: vec2<f32>, offset: f32) -> vec2<f32> {
    let t = dot(p, planeNormal) + offset;
    return p - (2.0 * t) * planeNormal;
}

fn drawReflectedHit(col: ptr<function, vec4<f32>>, p: vec2<f32>, hitPos: vec2<f32>, hitDist: f32, screenSize: vec2<f32>) {
    (*col).a += length(p) * 0.0001; // fix normal when flat
    drawHit(col, p, reflectPlane(hitPos, vec2<f32>(0.0, 1.0), 1.0), hitDist);
    drawHit(col, p, reflectPlane(hitPos, vec2<f32>(0.0, -1.0), 1.0), hitDist);
    drawHit(col, p, reflectPlane(hitPos, vec2<f32>(1.0, 0.0), screenSize.x / screenSize.y), hitDist);
    drawHit(col, p, reflectPlane(hitPos, vec2<f32>(-1.0, 0.0), screenSize.x / screenSize.y), hitDist);
}

// Flip every second cell to create reflection
fn flip(pos: ptr<function, vec2<f32>>) {
    let f = modv2(floor(*pos), vec2<f32>(2.0));
    *pos = abs(f - modv2(*pos, vec2<f32>(1.0)));
}

fn stepSign(a: f32) -> f32 {
    return step(0.0, a) * 2.0 - 1.0;
}

fn compassDir(p: vec2<f32>) -> vec2<f32> {
    let a = vec2<f32>(stepSign(p.x), 0.0);
    let b = vec2<f32>(0.0, stepSign(p.y));
    let s = stepSign(p.x - p.y) * stepSign(-p.x - p.y);
    return mix(a, b, s * 0.5 + 0.5);
}

fn calcHitPos(move_: vec2<f32>, dir: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    var hitPos = modv2(move_, vec2<f32>(1.0));
    let xCross = hitPos - hitPos.x / (size / size.x) * (dir / dir.x);
    let yCross = hitPos - hitPos.y / (size / size.y) * (dir / dir.y);
    hitPos = max(xCross, yCross);
    hitPos += floor(move_);
    return hitPos;
}

// Procedural dither noise (replaces the iChannel0 texture sample used to
// break up colour banding in the original Shadertoy version).
fn hash4(p: vec2<f32>) -> vec4<f32> {
    let q = vec4<f32>(
        dot(p, vec2<f32>(127.1, 311.7)),
        dot(p, vec2<f32>(269.5, 183.3)),
        dot(p, vec2<f32>(113.5, 271.9)),
        dot(p, vec2<f32>(246.1, 124.6))
    );
    return fract(sin(q) * 43758.5453);
}

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    // Flip Y coordinate to match GLSL/Shadertoy convention (Y=0 at bottom)
    let F = vec2<f32>(fragCoord.x, iResolution.y - fragCoord.y);
    let p = (-iResolution + 2.0 * F) / iResolution.y;

    let screenSize = vec2<f32>(iResolution.x / iResolution.y, 1.0) * 2.0;

    let t = iTime;
    let dir = normalize(vec2<f32>(9.0, 16.0) * screenSize);
    var move_ = dir * t * speed / 1.5;
    let logoScale = logo_scale;
    let logoSize = vec2<f32>(2.0, 0.85) * logoScale;

    let size = screenSize - logoSize * 2.0;

    // Remap so (0,0) is bottom left, and (1,1) is top right
    move_ = move_ / size + 0.5;

    // Calculate the point we last crossed a cell boundary
    var lastHitPos = calcHitPos(move_, dir, size);
    var col = vec4<f32>(1.0, 1.0, 1.0, 0.0);
    var colFx = vec4<f32>(1.0, 1.0, 1.0, 0.0);
    var colFy = vec4<f32>(1.0, 1.0, 1.0, 0.0);
    let e = vec2<f32>(0.8, 0.0) / iResolution.y;

    for (var i = 0; i < 5; i++) {
        var hitPos = lastHitPos;

        if (i > 0) {
            // Nudge it before the boundary to find the previous hit point
            hitPos = calcHitPos(hitPos - 0.00001 / size, dir, size);
        }

        lastHitPos = hitPos;

        // How far are we from the hit point
        let hitDist = distance(hitPos, move_);

        // Flip every second cell to create reflection
        flip(&hitPos);

        // Remap back to screen space
        hitPos = (hitPos - 0.5) * size;

        // Push the hits to the edges of the screen
        hitPos += logoSize * compassDir(hitPos / size);

        drawReflectedHit(&col, p, hitPos, hitDist, screenSize);
        drawReflectedHit(&colFx, p + e, hitPos, hitDist, screenSize);
        drawReflectedHit(&colFy, p + e.yx, hitPos, hitDist, screenSize);
    }

    // Flip every second cell to create reflection
    flip(&move_);

    // Remap back to screen space
    move_ = (move_ - 0.5) * size;

    // Calc normals
    let bf = 0.1; // Bump factor
    let fx = (col.a - colFx.a) * 99.0; // Nearby horizontal samples.
    let fy = (col.a - colFy.a) * 0.0; // Nearby vertical samples.
    let ff = length(vec2<f32>(fx, fy));
    let ee = rangec(0.0, 10.0 / iResolution.y, ff);
    let nor = normalize(vec3<f32>(vec2<f32>(fx, fy) * ee, ff));

    // invert colours
    col = vec4<f32>(clamp(1.0 - col.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) / 2.0, col.a);

    // lighting
    // iq https://www.shadertoy.com/view/Xds3zN
    let lig = normalize(vec3<f32>(1.0, 2.0, 2.0));
    let rd = normalize(vec3<f32>(p, -10.0));
    let hal = normalize(lig - rd);

    let dif = clamp(dot(lig, nor), 0.0, 1.0);
    let spe = pow(clamp(dot(nor, hal), 0.0, 1.0), 16.0) *
        dif *
        (0.04 + 0.06 * pow(clamp(1.0 + dot(hal, rd), 0.0, 1.0), 5.0));

    var lin = vec3<f32>(0.0);
    lin += 5.0 * dif;
    lin += 0.2;
    col = vec4<f32>(col.rgb * lin + 5.0 * spe, col.a);

    // dvd logo
    var d = dvd((p - move_) / logoScale);
    d = d / fwidth(d);
    d = 1.0 - clamp(d, 0.0, 1.0);
    col = vec4<f32>(mix(col.rgb, vec3<f32>(1.0), d), col.a);

    // banding be gone
    col += (hash4(fragCoord.xy) * 2.0 - 1.0) * 0.005;

    // gamma
    col = vec4<f32>(pow(col.rgb, vec3<f32>(1.0 / 1.5)), col.a);

    // Opaque output for the wallpaper (the original relies on Shadertoy
    // ignoring alpha).
    col.a = 1.0;
    return col;
}
