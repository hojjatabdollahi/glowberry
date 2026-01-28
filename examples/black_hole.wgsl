// Black Hole with Accretion Disk
// Converted to WGSL for cosmic-bg
// cosmic-bg provides: iResolution (vec2f), iTime (f32)

@fragment
fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
    // Flip Y coordinate to match GLSL convention (Y=0 at bottom)
    let F = vec2<f32>(fragCoord.x, iResolution.y - fragCoord.y);
    var i = 0.2;
    let rotationSpeed = 0.03;
    let gasSpeed = 0.2;
    let r = iResolution;
    
    // Center coordinates: map to [-aspect, aspect] x [-1, 1] centered at screen middle
    // (2*F - r) / r.y gives us centered coordinates scaled by height
    let p = (2.0 * F - r) / r.y / 0.7;
    
    let d = vec2<f32>(-1.0, 1.0);
    let b = p - i * d;
    
    // In GLSL: vec2 c = p * mat2(1.0, 1.0, d / (0.1 + i / dot(b, b)));
    // GLSL mat2(a,b,c,d) constructs: column0=(a,b), column1=(c,d)
    // So mat2(1.0, 1.0, d/(..)) = mat2(1.0, 1.0, d.x/(..), d.y/(..))
    //   column0 = (1.0, 1.0)
    //   column1 = (d.x/denom, d.y/denom) = (-1/denom, 1/denom)
    // vec2 * mat2 in GLSL: result = vec2(dot(p, col0), dot(p, col1))
    let denom = 0.1 + i / dot(b, b);
    let c = vec2<f32>(
        dot(p, vec2<f32>(1.0, 1.0)),
        dot(p, d / denom)
    );

    let a = dot(c, c);
    let angle = 0.5 * log(a) + iTime * rotationSpeed;
    let cosA = cos(angle);
    let sinA = sin(angle);
    
    // GLSL: c * mat2(cosA, sinA, -sinA, cosA)
    // column0 = (cosA, sinA), column1 = (-sinA, cosA)
    // result = vec2(dot(c, col0), dot(c, col1))
    let rotated = vec2<f32>(
        dot(c, vec2<f32>(cosA, sinA)),
        dot(c, vec2<f32>(-sinA, cosA))
    );
    var v = rotated / i;

    var w = vec4<f32>(0.0);
    
    // GLSL: for (; i++ < 9.0; ) means: check i<9, then increment
    loop {
        if (i >= 9.0) {
            break;
        }
        i += 1.0;
        let s = sin(v);
        w += vec4<f32>(s.x, s.y, s.y, s.x) + 1.0;
        v += 0.7 * sin(v.yx * i + iTime * gasSpeed) / i + 0.5;
    }

    let i2 = length(sin(v / 0.3) * 0.4 + c * (3.0 + d));

    // Final color
    let exp_term = exp(c.x * vec4<f32>(0.6, -0.4, -1.0, 0.0));
    let denom2 = 2.0 + i2 * i2 / 4.0 - i2;
    let denom3 = 0.5 + 1.0 / a;
    let denom4 = 0.03 + abs(length(p) - 0.7);
    
    var color = 1.0 - exp(-exp_term / w / denom2 / denom3 / denom4);
    
    // Clamp and ensure solid black background
    color = clamp(color, vec4<f32>(0.0), vec4<f32>(1.0));
    color.w = 1.0;
    
    return color;
}
