// SPDX-License-Identifier: MPL-2.0

//! Utilities for loading background images for session lock screens.
//!
//! Session lock surfaces block all layer surfaces underneath them, so the normal
//! cosmic-bg layer surface approach doesn't work for lock screens. This module
//! provides functions to load the user's background image/color for rendering
//! directly on the lock surface.

use crate::user_context::UserContext;
use cosmic_bg_config::{Color, Config, Source};
use image::DynamicImage;
use std::path::PathBuf;

/// Background source information for static rendering.
#[derive(Debug, Clone)]
pub enum BackgroundSource {
    /// A static image loaded from a path.
    Image(PathBuf),
    /// A solid color [R, G, B] in 0.0-1.0 range.
    SolidColor([f32; 3]),
    /// A gradient with colors and radius.
    Gradient { colors: Vec<[f32; 3]>, radius: f32 },
    /// A shader background (animated, requires special handling).
    Shader,
}

/// Errors that can occur when loading backgrounds.
#[derive(Debug, thiserror::Error)]
pub enum ExternalSurfaceError {
    #[error("Failed to load config: {0}")]
    Config(String),

    #[error("Failed to create shader: {0}")]
    Shader(String),

    #[error("Failed to load image: {0}")]
    Image(String),
}

/// Load the background source for a user.
///
/// This reads the user's cosmic-bg configuration and returns the appropriate
/// background source. For image paths, it returns the first image in a directory
/// if the path is a directory.
pub fn load_background_source(user_context: &UserContext) -> Option<BackgroundSource> {
    let _env_guard = user_context.apply();

    let config_context = cosmic_bg_config::context().ok()?;
    let config = Config::load(&config_context).ok()?;

    // Use the default background entry
    source_to_background(&config.default_background.source)
}

/// Load a background image for a user, scaling it to the given dimensions.
///
/// Returns `None` if the background is not an image or if loading fails.
pub fn load_background_image(
    user_context: &UserContext,
    width: u32,
    height: u32,
) -> Option<DynamicImage> {
    let source = load_background_source(user_context)?;

    match source {
        BackgroundSource::Image(path) => {
            let img_path = if path.is_dir() {
                // Find first image in directory
                find_first_image_in_dir(&path)?
            } else {
                path
            };

            let img = image::open(&img_path).ok()?;
            // Scale to fit the target dimensions
            Some(crate::scaler::zoom(&img, width, height))
        }
        BackgroundSource::SolidColor(color) => {
            // Create a solid color image
            Some(create_solid_color_image(color, width, height))
        }
        BackgroundSource::Gradient { colors, radius } => {
            // Create a gradient image
            Some(create_gradient_image(&colors, radius, width, height))
        }
        BackgroundSource::Shader => {
            // Shader backgrounds need special handling
            // For now, return None and let the caller handle the fallback
            None
        }
    }
}

/// Check if the user has a shader background configured.
pub fn has_shader_background(user_context: &UserContext) -> bool {
    matches!(
        load_background_source(user_context),
        Some(BackgroundSource::Shader)
    )
}

fn source_to_background(source: &Source) -> Option<BackgroundSource> {
    match source {
        Source::Path(path) => Some(BackgroundSource::Image(path.clone())),
        Source::Color(Color::Single(color)) => Some(BackgroundSource::SolidColor(*color)),
        Source::Color(Color::Gradient(gradient)) => Some(BackgroundSource::Gradient {
            colors: gradient.colors.to_vec(),
            radius: gradient.radius,
        }),
        Source::Shader(_) => Some(BackgroundSource::Shader),
    }
}

fn find_first_image_in_dir(dir: &PathBuf) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut images: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|ext| {
                    let ext = ext.to_str().unwrap_or("").to_lowercase();
                    matches!(
                        ext.as_str(),
                        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"
                    )
                })
                .unwrap_or(false)
        })
        .collect();

    images.sort();
    images.into_iter().next()
}

fn create_solid_color_image(color: [f32; 3], width: u32, height: u32) -> DynamicImage {
    use image::{Rgb, RgbImage};

    let r = (color[0] * 255.0) as u8;
    let g = (color[1] * 255.0) as u8;
    let b = (color[2] * 255.0) as u8;

    let img = RgbImage::from_fn(width, height, |_, _| Rgb([r, g, b]));
    DynamicImage::ImageRgb8(img)
}

fn create_gradient_image(
    colors: &[[f32; 3]],
    radius: f32,
    width: u32,
    height: u32,
) -> DynamicImage {
    use image::{Rgb, RgbImage};

    if colors.is_empty() {
        return create_solid_color_image([0.0, 0.0, 0.0], width, height);
    }

    if colors.len() == 1 {
        return create_solid_color_image(colors[0], width, height);
    }

    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;
    let max_dist = (center_x * center_x + center_y * center_y).sqrt() * radius;

    let img = RgbImage::from_fn(width, height, |x, y| {
        let dx = x as f32 - center_x;
        let dy = y as f32 - center_y;
        let dist = (dx * dx + dy * dy).sqrt();
        let t = (dist / max_dist).min(1.0);

        // Interpolate between colors
        let color_idx = t * (colors.len() - 1) as f32;
        let idx1 = color_idx.floor() as usize;
        let idx2 = (idx1 + 1).min(colors.len() - 1);
        let frac = color_idx.fract();

        let c1 = colors[idx1];
        let c2 = colors[idx2];

        let r = ((c1[0] + (c2[0] - c1[0]) * frac) * 255.0) as u8;
        let g = ((c1[1] + (c2[1] - c1[1]) * frac) * 255.0) as u8;
        let b = ((c1[2] + (c2[2] - c1[2]) * frac) * 255.0) as u8;

        Rgb([r, g, b])
    });

    DynamicImage::ImageRgb8(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_surface_error_display() {
        let err = ExternalSurfaceError::Config("test".to_string());
        assert!(err.to_string().contains("test"));
    }

    #[test]
    fn solid_color_image_creation() {
        let img = create_solid_color_image([1.0, 0.0, 0.0], 10, 10);
        let rgb = img.to_rgb8();
        let pixel = rgb.get_pixel(5, 5);
        assert_eq!(pixel.0, [255, 0, 0]);
    }

    #[test]
    fn gradient_single_color() {
        let img = create_gradient_image(&[[0.0, 1.0, 0.0]], 1.0, 10, 10);
        let rgb = img.to_rgb8();
        let pixel = rgb.get_pixel(5, 5);
        assert_eq!(pixel.0, [0, 255, 0]);
    }
}
