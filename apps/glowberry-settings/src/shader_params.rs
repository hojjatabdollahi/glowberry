// SPDX-License-Identifier: MPL-2.0

//! Shader parameter parsing and management

use std::collections::HashMap;
use std::path::Path;

/// Shader metadata from header
#[derive(Debug, Clone, Default)]
pub struct ShaderMetadata {
    pub name: String,
    pub author: String,
    pub source: String,
    pub license: String,
}

/// A shader parameter definition
#[derive(Debug, Clone)]
pub struct ShaderParam {
    pub name: String,
    pub param_type: ParamType,
    pub default: ParamValue,
    pub min: ParamValue,
    pub max: ParamValue,
    pub step: ParamValue,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamType {
    F32,
    I32,
}

#[derive(Debug, Clone, Copy)]
pub enum ParamValue {
    F32(f32),
    I32(i32),
}

impl ParamValue {
    pub fn as_f32(&self) -> f32 {
        match self {
            ParamValue::F32(v) => *v,
            ParamValue::I32(v) => *v as f32,
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            ParamValue::F32(v) => *v as i32,
            ParamValue::I32(v) => *v,
        }
    }
}

/// Shader complexity level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Complexity {
    Low,
    Medium,
    High,
}

impl Complexity {
    /// Get a display string for the complexity level
    pub fn as_str(&self) -> &'static str {
        match self {
            Complexity::Low => "Low",
            Complexity::Medium => "Medium",
            Complexity::High => "High",
        }
    }
}

/// Parsed shader with metadata and parameters
#[derive(Debug, Clone)]
pub struct ParsedShader {
    pub metadata: ShaderMetadata,
    pub params: Vec<ShaderParam>,
    /// The shader source after the header (without comments)
    pub source_body: String,
}

impl ParsedShader {
    /// Parse a shader file and extract metadata and parameters
    pub fn parse(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        Self::parse_content(&content)
    }

    /// Parse shader content string
    pub fn parse_content(content: &str) -> Option<Self> {
        let mut metadata = ShaderMetadata::default();
        let mut params = Vec::new();
        let mut in_params_section = false;
        let mut source_lines = Vec::new();
        let mut header_ended = false;

        for line in content.lines() {
            let trimmed = line.trim();

            // Check for section markers
            if trimmed == "// [SHADER]" {
                continue;
            }
            if trimmed == "// [PARAMS]" {
                in_params_section = true;
                continue;
            }
            if trimmed == "// [/PARAMS]" {
                in_params_section = false;
                header_ended = true;
                continue;
            }

            // Parse metadata
            if !header_ended && trimmed.starts_with("// ") && !in_params_section {
                let rest = &trimmed[3..];
                if let Some((key, value)) = rest.split_once(": ") {
                    match key {
                        "name" => metadata.name = value.to_string(),
                        "author" => metadata.author = value.to_string(),
                        "source" => metadata.source = value.to_string(),
                        "license" => metadata.license = value.to_string(),
                        _ => {}
                    }
                }
            }

            // Parse parameters
            if in_params_section && trimmed.starts_with("// ") {
                let rest = &trimmed[3..];
                if let Some(param) = parse_param_line(rest) {
                    params.push(param);
                }
            }

            // Collect source body (after header or non-comment lines)
            if header_ended || (!trimmed.starts_with("//") && !trimmed.is_empty()) {
                if header_ended {
                    source_lines.push(line.to_string());
                }
            }
        }

        // If no explicit header end, take everything after first non-comment
        if !header_ended {
            source_lines.clear();
            let mut found_code = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.starts_with("//") && !trimmed.is_empty() {
                    found_code = true;
                }
                if found_code {
                    source_lines.push(line.to_string());
                }
            }
        }

        Some(Self {
            metadata,
            params,
            source_body: source_lines.join("\n"),
        })
    }

    /// Generate shader source with parameter values substituted
    pub fn generate_source(&self, values: &HashMap<String, ParamValue>) -> String {
        let mut result = String::new();

        // Add const declarations for parameters with custom values
        for param in &self.params {
            let value = values.get(&param.name).unwrap_or(&param.default);
            match param.param_type {
                ParamType::F32 => {
                    result.push_str(&format!(
                        "const {}: f32 = {:.6};\n",
                        param.name,
                        value.as_f32()
                    ));
                }
                ParamType::I32 => {
                    result.push_str(&format!(
                        "const {}: i32 = {};\n",
                        param.name,
                        value.as_i32()
                    ));
                }
            }
        }

        result.push('\n');

        // Filter out existing const declarations for parameters we're overriding
        // to avoid duplicate definitions
        let param_names: std::collections::HashSet<&str> =
            self.params.iter().map(|p| p.name.as_str()).collect();

        for line in self.source_body.lines() {
            let trimmed = line.trim();
            // Check if this line is a const declaration for one of our parameters
            let is_param_const = trimmed.starts_with("const ")
                && param_names.iter().any(|name| {
                    trimmed.starts_with(&format!("const {name}:"))
                        || trimmed.starts_with(&format!("const {name} :"))
                });

            if !is_param_const {
                result.push_str(line);
                result.push('\n');
            }
        }

        result
    }

    /// Estimate shader complexity based on static analysis
    ///
    /// This uses heuristics to estimate GPU load:
    /// - Loop count and nesting
    /// - Iteration parameters (params that control loop counts)
    /// - Expensive math operations (sin, cos, exp, pow, sqrt, etc.)
    /// - Texture sampling (if present)
    pub fn estimate_complexity(
        &self,
        param_values: Option<&HashMap<String, ParamValue>>,
    ) -> Complexity {
        let source = &self.source_body;
        let mut score: f32 = 0.0;

        // Count loops
        let for_loops = source.matches("for ").count() + source.matches("for(").count();
        let while_loops = source.matches("loop ").count() + source.matches("loop{").count();
        let total_loops = for_loops + while_loops;
        score += total_loops as f32 * 10.0;

        // Check for nested loops (very expensive)
        // Simple heuristic: if we have 2+ loops, assume some nesting
        if total_loops >= 2 {
            score += 15.0;
        }

        // Count expensive math operations
        let expensive_ops = [
            ("sin(", 1.0),
            ("cos(", 1.0),
            ("tan(", 1.5),
            ("exp(", 1.5),
            ("exp2(", 1.2),
            ("log(", 1.5),
            ("log2(", 1.2),
            ("pow(", 2.0),
            ("sqrt(", 0.8),
            ("inverseSqrt(", 0.8),
            ("length(", 0.5),
            ("normalize(", 0.8),
            ("dot(", 0.3),
            ("cross(", 0.5),
            ("reflect(", 1.0),
            ("refract(", 1.5),
            ("atan(", 1.5),
            ("atan2(", 1.5),
            ("asin(", 1.5),
            ("acos(", 1.5),
            ("sinh(", 2.0),
            ("cosh(", 2.0),
            ("tanh(", 1.5),
            ("smoothstep(", 0.5),
            ("mix(", 0.3),
        ];

        for (op, weight) in expensive_ops {
            let count = source.matches(op).count();
            score += count as f32 * weight;
        }

        // Check for texture sampling (expensive)
        if source.contains("textureSample") || source.contains("iTexture") {
            score += 5.0;
        }

        // Check for iteration-controlling parameters
        // These multiply the base cost
        let iteration_params: Vec<&ShaderParam> = self
            .params
            .iter()
            .filter(|p| {
                let name_lower = p.name.to_lowercase();
                name_lower.contains("iteration")
                    || name_lower.contains("layers")
                    || name_lower.contains("steps")
                    || name_lower.contains("samples")
                    || (name_lower == "zoom" && p.param_type == ParamType::I32)
                    || (name_lower.contains("num_") || name_lower.contains("count"))
            })
            .collect();

        // Get current or default iteration values
        for param in iteration_params {
            let value = param_values
                .and_then(|v| v.get(&param.name))
                .unwrap_or(&param.default);

            let iter_count = value.as_i32().max(1) as f32;
            // Iteration params have multiplicative effect
            // Normalize: assume default of ~10 iterations is "normal"
            let multiplier = (iter_count / 10.0).max(0.5);
            score *= multiplier;
        }

        // Classify based on score
        // These thresholds are tuned based on the existing shaders
        if score < 15.0 {
            Complexity::Low
        } else if score < 40.0 {
            Complexity::Medium
        } else {
            Complexity::High
        }
    }
}

/// Parse a parameter line like:
/// speed: f32 = 0.5 | min: 0.1 | max: 2.0 | step: 0.1 | label: Speed
fn parse_param_line(line: &str) -> Option<ShaderParam> {
    // Split by |
    let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    // Parse first part: name: type = default
    let first = parts[0];
    let (name_type, default_str) = first.split_once('=')?;
    let (name, type_str) = name_type.trim().split_once(':')?;
    let name = name.trim().to_string();
    let type_str = type_str.trim();
    let default_str = default_str.trim();

    let param_type = match type_str {
        "f32" => ParamType::F32,
        "i32" => ParamType::I32,
        _ => return None,
    };

    let default = parse_value(default_str, param_type)?;
    let mut min = default;
    let mut max = default;
    let mut step = match param_type {
        ParamType::F32 => ParamValue::F32(0.1),
        ParamType::I32 => ParamValue::I32(1),
    };
    let mut label = name.clone();

    // Parse remaining parts
    for part in parts.iter().skip(1) {
        if let Some((key, value)) = part.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "min" => min = parse_value(value, param_type).unwrap_or(min),
                "max" => max = parse_value(value, param_type).unwrap_or(max),
                "step" => step = parse_value(value, param_type).unwrap_or(step),
                "label" => label = value.to_string(),
                _ => {}
            }
        }
    }

    Some(ShaderParam {
        name,
        param_type,
        default,
        min,
        max,
        step,
        label,
    })
}

fn parse_value(s: &str, param_type: ParamType) -> Option<ParamValue> {
    match param_type {
        ParamType::F32 => s.parse::<f32>().ok().map(ParamValue::F32),
        ParamType::I32 => s.parse::<i32>().ok().map(ParamValue::I32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_param_line() {
        let line = "speed: f32 = 0.5 | min: 0.1 | max: 2.0 | step: 0.1 | label: Speed";
        let param = parse_param_line(line).unwrap();
        assert_eq!(param.name, "speed");
        assert_eq!(param.param_type, ParamType::F32);
        assert_eq!(param.label, "Speed");
    }
}
