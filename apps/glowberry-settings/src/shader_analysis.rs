// SPDX-License-Identifier: MPL-2.0

//! Shader complexity analysis using naga AST
//!
//! This module provides accurate shader resource estimation by parsing WGSL
//! into an AST and analyzing the actual structure rather than using string matching.

use naga::{Expression, Function, MathFunction, Module, Statement};

/// Shader complexity level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Complexity {
    Low,
    Medium,
    High,
}

/// Detailed metrics from shader analysis
#[derive(Debug, Clone, Default)]
pub struct ShaderMetrics {
    /// Number of loops found
    pub loop_count: u32,
    /// Maximum loop nesting depth
    pub max_loop_depth: u32,
    /// Count of expensive transcendental operations (sin, cos, exp, pow, etc.)
    pub transcendental_ops: u32,
    /// Count of moderate-cost math operations (sqrt, length, normalize, etc.)
    pub moderate_math_ops: u32,
    /// Count of cheap math operations (dot, mix, clamp, etc.)
    pub cheap_math_ops: u32,
    /// Number of texture sample operations
    pub texture_samples: u32,
    /// Number of conditional branches
    pub branches: u32,
    /// Estimated iteration multiplier from parameters (if provided)
    pub iteration_multiplier: f32,
}

impl ShaderMetrics {
    /// Calculate a weighted complexity score
    pub fn score(&self) -> f32 {
        let base_score = self.loop_count as f32 * 10.0
            + self.max_loop_depth as f32 * 15.0
            + self.transcendental_ops as f32 * 1.5
            + self.moderate_math_ops as f32 * 0.7
            + self.cheap_math_ops as f32 * 0.2
            + self.texture_samples as f32 * 3.0
            + self.branches as f32 * 0.5;

        base_score * self.iteration_multiplier.max(1.0)
    }

    /// Convert score to complexity level
    pub fn complexity(&self) -> Complexity {
        let score = self.score();
        if score < 15.0 {
            Complexity::Low
        } else if score < 40.0 {
            Complexity::Medium
        } else {
            Complexity::High
        }
    }
}

/// Analyze a WGSL shader and return detailed metrics
///
/// # Arguments
/// * `wgsl_source` - The WGSL shader source code
/// * `iteration_multiplier` - Optional multiplier for iteration-based parameters
///
/// # Returns
/// * `Ok(ShaderMetrics)` - Analysis results
/// * `Err(String)` - Parse error description
pub fn analyze_shader(
    wgsl_source: &str,
    iteration_multiplier: Option<f32>,
) -> Result<ShaderMetrics, String> {
    let module =
        naga::front::wgsl::parse_str(wgsl_source).map_err(|e| e.emit_to_string(wgsl_source))?;

    let mut metrics = ShaderMetrics {
        iteration_multiplier: iteration_multiplier.unwrap_or(1.0),
        ..Default::default()
    };

    analyze_module(&module, &mut metrics);

    Ok(metrics)
}

/// Analyze a parsed naga module
fn analyze_module(module: &Module, metrics: &mut ShaderMetrics) {
    // Analyze all functions
    for (_, function) in module.functions.iter() {
        analyze_function(function, metrics);
    }

    // Analyze entry points
    for entry_point in &module.entry_points {
        analyze_function(&entry_point.function, metrics);
    }
}

/// Analyze a single function
fn analyze_function(function: &Function, metrics: &mut ShaderMetrics) {
    // Analyze expressions for math operations and texture samples
    for (_, expr) in function.expressions.iter() {
        analyze_expression(expr, metrics);
    }

    // Analyze statements for control flow
    analyze_block(&function.body, metrics, 0);
}

/// Analyze an expression for math operations
fn analyze_expression(expr: &Expression, metrics: &mut ShaderMetrics) {
    match expr {
        Expression::Math { fun, .. } => {
            categorize_math_function(*fun, metrics);
        }
        Expression::ImageSample { .. } | Expression::ImageLoad { .. } => {
            metrics.texture_samples += 1;
        }
        _ => {}
    }
}

/// Categorize a math function by cost
fn categorize_math_function(fun: MathFunction, metrics: &mut ShaderMetrics) {
    match fun {
        // Expensive transcendental operations
        MathFunction::Sin
        | MathFunction::Cos
        | MathFunction::Tan
        | MathFunction::Sinh
        | MathFunction::Cosh
        | MathFunction::Tanh
        | MathFunction::Asin
        | MathFunction::Acos
        | MathFunction::Atan
        | MathFunction::Atan2
        | MathFunction::Asinh
        | MathFunction::Acosh
        | MathFunction::Atanh
        | MathFunction::Exp
        | MathFunction::Exp2
        | MathFunction::Log
        | MathFunction::Log2
        | MathFunction::Pow => {
            metrics.transcendental_ops += 1;
        }

        // Moderate cost operations
        MathFunction::Sqrt
        | MathFunction::InverseSqrt
        | MathFunction::Length
        | MathFunction::Normalize
        | MathFunction::Reflect
        | MathFunction::Refract
        | MathFunction::Distance
        | MathFunction::Cross
        | MathFunction::FaceForward
        | MathFunction::Fma => {
            metrics.moderate_math_ops += 1;
        }

        // Cheap operations
        MathFunction::Abs
        | MathFunction::Min
        | MathFunction::Max
        | MathFunction::Clamp
        | MathFunction::Saturate
        | MathFunction::Floor
        | MathFunction::Ceil
        | MathFunction::Round
        | MathFunction::Fract
        | MathFunction::Trunc
        | MathFunction::Sign
        | MathFunction::Step
        | MathFunction::SmoothStep
        | MathFunction::Mix
        | MathFunction::Dot
        | MathFunction::Outer
        | MathFunction::Radians
        | MathFunction::Degrees
        | MathFunction::Modf
        | MathFunction::Frexp
        | MathFunction::Ldexp
        | MathFunction::Transpose
        | MathFunction::Determinant
        | MathFunction::Inverse
        | MathFunction::QuantizeToF16
        | MathFunction::CountTrailingZeros
        | MathFunction::CountLeadingZeros
        | MathFunction::CountOneBits
        | MathFunction::ReverseBits
        | MathFunction::ExtractBits
        | MathFunction::InsertBits
        | MathFunction::FirstTrailingBit
        | MathFunction::FirstLeadingBit
        | MathFunction::Pack4x8snorm
        | MathFunction::Pack4x8unorm
        | MathFunction::Pack2x16snorm
        | MathFunction::Pack2x16unorm
        | MathFunction::Pack2x16float
        | MathFunction::Pack4xI8
        | MathFunction::Pack4xU8
        | MathFunction::Pack4xI8Clamp
        | MathFunction::Pack4xU8Clamp
        | MathFunction::Unpack4x8snorm
        | MathFunction::Unpack4x8unorm
        | MathFunction::Unpack2x16snorm
        | MathFunction::Unpack2x16unorm
        | MathFunction::Unpack2x16float
        | MathFunction::Unpack4xI8
        | MathFunction::Unpack4xU8
        | MathFunction::Dot4I8Packed
        | MathFunction::Dot4U8Packed => {
            metrics.cheap_math_ops += 1;
        }
    }
}

/// Analyze a block of statements, tracking loop depth
fn analyze_block(block: &naga::Block, metrics: &mut ShaderMetrics, current_loop_depth: u32) {
    for statement in block.iter() {
        analyze_statement(statement, metrics, current_loop_depth);
    }
}

/// Analyze a single statement
fn analyze_statement(statement: &Statement, metrics: &mut ShaderMetrics, current_loop_depth: u32) {
    match statement {
        Statement::Loop {
            body, continuing, ..
        } => {
            metrics.loop_count += 1;
            let new_depth = current_loop_depth + 1;
            metrics.max_loop_depth = metrics.max_loop_depth.max(new_depth);

            analyze_block(body, metrics, new_depth);
            analyze_block(continuing, metrics, new_depth);
        }

        Statement::If { accept, reject, .. } => {
            metrics.branches += 1;
            analyze_block(accept, metrics, current_loop_depth);
            analyze_block(reject, metrics, current_loop_depth);
        }

        Statement::Switch { cases, .. } => {
            metrics.branches += cases.len() as u32;
            for case in cases {
                analyze_block(&case.body, metrics, current_loop_depth);
            }
        }

        Statement::Block(block) => {
            analyze_block(block, metrics, current_loop_depth);
        }

        _ => {}
    }
}

/// WGSL preamble for GlowBerry shaders (uniforms only, no texture)
const GLOWBERRY_PREAMBLE: &str = r#"
@group(0) @binding(0) var<uniform> iResolution: vec2f;
@group(0) @binding(1) var<uniform> iTime: f32;
"#;

/// WGSL preamble for GlowBerry shaders with texture support
const GLOWBERRY_PREAMBLE_WITH_TEXTURE: &str = r#"
@group(0) @binding(0) var<uniform> iResolution: vec2f;
@group(0) @binding(1) var<uniform> iTime: f32;
@group(0) @binding(2) var iTexture: texture_2d<f32>;
@group(0) @binding(3) var iTextureSampler: sampler;
"#;

/// Analyze a GlowBerry shader body (without preamble)
///
/// This function prepends the necessary uniforms to make the shader valid WGSL
/// before parsing. Use this when you have a shader body that expects GlowBerry's
/// standard uniforms (iResolution, iTime, etc.)
///
/// # Arguments
/// * `shader_body` - The shader code without GlowBerry uniforms
/// * `has_texture` - Whether the shader uses texture sampling (iTexture)
/// * `iteration_multiplier` - Optional multiplier for iteration-based parameters
pub fn analyze_glowberry_shader(
    shader_body: &str,
    has_texture: bool,
    iteration_multiplier: Option<f32>,
) -> Result<ShaderMetrics, String> {
    let preamble = if has_texture {
        GLOWBERRY_PREAMBLE_WITH_TEXTURE
    } else {
        GLOWBERRY_PREAMBLE
    };

    let full_source = format!("{preamble}\n{shader_body}");
    analyze_shader(&full_source, iteration_multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_SHADER: &str = r#"
        @fragment
        fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
            return vec4<f32>(uv.x, uv.y, 0.0, 1.0);
        }
    "#;

    const COMPLEX_SHADER: &str = r#"
        @fragment
        fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
            var color = vec3<f32>(0.0);
            for (var i = 0; i < 10; i++) {
                for (var j = 0; j < 10; j++) {
                    let angle = sin(f32(i) * 0.1) + cos(f32(j) * 0.1);
                    color += vec3<f32>(pow(angle, 2.0));
                }
            }
            return vec4<f32>(color, 1.0);
        }
    "#;

    #[test]
    fn test_simple_shader_low_complexity() {
        let metrics = analyze_shader(SIMPLE_SHADER, None).unwrap();
        assert_eq!(metrics.loop_count, 0);
        assert_eq!(metrics.transcendental_ops, 0);
        assert_eq!(metrics.complexity(), Complexity::Low);
    }

    #[test]
    fn test_complex_shader_high_complexity() {
        let metrics = analyze_shader(COMPLEX_SHADER, None).unwrap();
        assert_eq!(metrics.loop_count, 2);
        assert_eq!(metrics.max_loop_depth, 2);
        assert!(metrics.transcendental_ops >= 3); // sin, cos, pow
        assert!(matches!(
            metrics.complexity(),
            Complexity::Medium | Complexity::High
        ));
    }

    #[test]
    fn test_iteration_multiplier() {
        let metrics_1x = analyze_shader(COMPLEX_SHADER, Some(1.0)).unwrap();
        let metrics_2x = analyze_shader(COMPLEX_SHADER, Some(2.0)).unwrap();

        assert!(metrics_2x.score() > metrics_1x.score());
    }

    #[test]
    fn test_invalid_shader_returns_error() {
        let result = analyze_shader("this is not valid wgsl", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_glowberry_shader_body() {
        // This is a typical GlowBerry shader body that uses iResolution and iTime
        let shader_body = r#"
            @fragment
            fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
                let uv = pos.xy / iResolution;
                let color = sin(iTime + uv.x * 10.0) * 0.5 + 0.5;
                return vec4<f32>(color, color, color, 1.0);
            }
        "#;

        let metrics = analyze_glowberry_shader(shader_body, false, None).unwrap();
        assert!(metrics.transcendental_ops >= 1); // sin
        assert_eq!(metrics.complexity(), Complexity::Low);
    }

    #[test]
    fn test_glowberry_shader_with_texture() {
        let shader_body = r#"
            @fragment
            fn main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
                let uv = pos.xy / iResolution;
                return textureSample(iTexture, iTextureSampler, uv);
            }
        "#;

        let metrics = analyze_glowberry_shader(shader_body, true, None).unwrap();
        assert_eq!(metrics.texture_samples, 1);
    }

    #[test]
    fn test_frosted_glass_shader() {
        // Frosted Glass shader converted from Shadertoy
        let shader_body = r#"
            const speed: f32 = 1.0;
            const blur_amount: f32 = 0.13;
            const circle_size: f32 = 0.2;

            fn circle(uv: vec2<f32>, r: f32, blur: bool) -> f32 {
                var a: f32;
                var b: f32;
                if blur {
                    a = 0.01;
                    b = blur_amount;
                } else {
                    a = 0.0;
                    b = 5.0 / iResolution.y;
                }
                return smoothstep(a, b, length(uv) - r);
            }

            @fragment
            fn main(@builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
                let uv = (fragCoord.xy - 0.5 * iResolution) / iResolution.y;
                let t = vec2<f32>(
                    sin(iTime * 2.0 * speed),
                    cos(iTime * 3.0 * speed + cos(iTime * 0.5 * speed))
                ) * 0.1;
                
                let col0 = vec3<f32>(0.9);
                let col1 = vec3<f32>(0.1 + uv.y * 2.0, 0.4 + uv.x * -1.1, 0.8) * 0.828;
                let col2 = vec3<f32>(0.86);
                
                let cir1 = circle(uv - t, circle_size, false);
                let cir2 = circle(uv + t, circle_size, false);
                let cir2_blur = circle(uv + t, circle_size - 0.05, true);
                
                var col = mix(col1 + vec3<f32>(0.3, 0.1, 0.0), col2, cir2_blur);
                col = mix(col, col0, cir1);
                col = mix(col, col1, clamp(cir1 - cir2, 0.0, 1.0));
                
                return vec4<f32>(col, 1.0);
            }
        "#;

        let metrics = analyze_glowberry_shader(shader_body, false, None).unwrap();

        // Verify expected metrics
        assert_eq!(metrics.loop_count, 0, "No loops expected");
        assert_eq!(metrics.max_loop_depth, 0, "No loop nesting expected");
        assert!(
            metrics.transcendental_ops >= 3,
            "Should have sin/cos calls: {}",
            metrics.transcendental_ops
        );
        // length() is moderate, smoothstep/mix/clamp are cheap
        assert!(
            metrics.moderate_math_ops >= 1,
            "Should have length calls: {}",
            metrics.moderate_math_ops
        );
        assert!(
            metrics.cheap_math_ops >= 3,
            "Should have smoothstep/mix/clamp calls: {}",
            metrics.cheap_math_ops
        );
        assert!(
            metrics.branches >= 1,
            "Should have if branch in circle(): {}",
            metrics.branches
        );

        // Should be Low or Medium complexity (no loops, few ops)
        let complexity = metrics.complexity();
        println!(
            "Frosted Glass metrics: {:?}, score: {:.1}, complexity: {:?}",
            metrics,
            metrics.score(),
            complexity
        );
        assert!(
            matches!(complexity, Complexity::Low | Complexity::Medium),
            "Expected Low or Medium complexity, got {:?} (score: {:.1})",
            complexity,
            metrics.score()
        );
    }
}
