// SPDX-License-Identifier: MPL-2.0

//! Shared WGSL shader definitions for the GlowBerry rendering pipeline.
//!
//! These constants define the shader contract between the engine, the settings
//! app preview renderer, and the shader analysis module. They must be kept in
//! sync — which is why they live in one place.

/// WGSL preamble prepended to user shaders (uniforms only).
pub const WGSL_PREAMBLE: &str = r#"
// GlowBerry live wallpaper uniforms
@group(0) @binding(0) var<uniform> iResolution: vec2f;
@group(0) @binding(1) var<uniform> iTime: f32;
"#;

/// WGSL preamble with texture support.
pub const WGSL_PREAMBLE_WITH_TEXTURE: &str = r#"
// GlowBerry live wallpaper uniforms
@group(0) @binding(0) var<uniform> iResolution: vec2f;
@group(0) @binding(1) var<uniform> iTime: f32;
@group(0) @binding(2) var iTexture: texture_2d<f32>;
@group(0) @binding(3) var iTextureSampler: sampler;
"#;

/// Full-screen vertex shader used by both the daemon and the preview renderer.
pub const VERTEX_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Full-screen triangle strip
    var positions = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0),
    );

    var out: VertexOutput;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    return out;
}
"#;

/// Align a row's byte count to wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`.
pub fn aligned_bytes_per_row(width: u32, bytes_per_pixel: u32) -> u32 {
    let unpadded = width.saturating_mul(bytes_per_pixel);
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    unpadded.div_ceil(alignment) * alignment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preamble_constants_are_consistent() {
        // The texture preamble must be a superset of the base preamble
        assert!(
            WGSL_PREAMBLE_WITH_TEXTURE.contains("iResolution"),
            "texture preamble missing iResolution"
        );
        assert!(
            WGSL_PREAMBLE_WITH_TEXTURE.contains("iTime"),
            "texture preamble missing iTime"
        );
        assert!(
            WGSL_PREAMBLE_WITH_TEXTURE.contains("iTexture"),
            "texture preamble missing iTexture"
        );
    }

    #[test]
    fn aligns_bytes_per_row_to_wgpu_requirement() {
        let bytes_per_pixel = 4u32;
        let aligned = aligned_bytes_per_row(1, bytes_per_pixel);
        assert_eq!(aligned % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
        assert!(aligned >= bytes_per_pixel);
    }
}
