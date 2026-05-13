// SPDX-License-Identifier: MPL-2.0

use image::{DynamicImage, Pixel, RgbaImage};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CropError {
    #[error("failed to load image: {0}")]
    ImageLoad(#[from] image::ImageError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no monitors provided")]
    NoMonitors,
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub name: String,
    pub position: (i32, i32),
    pub logical_size: (u32, u32),
    pub physical_size: (u32, u32),
    pub scale: f64,
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("glowberry")
        .join("extended")
}

pub fn ensure_cache_dir() -> std::io::Result<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn crop_for_monitors(
    image_path: &Path,
    monitors: &[MonitorInfo],
    offset: (f64, f64),
    img_scale: f64,
    cache_dir: &Path,
) -> Result<Vec<(String, PathBuf)>, CropError> {
    if monitors.is_empty() {
        return Err(CropError::NoMonitors);
    }

    std::fs::create_dir_all(cache_dir)?;

    let src_img = image::open(image_path)?;
    let (img_w, img_h) = (src_img.width() as f64, src_img.height() as f64);

    let mut results = Vec::with_capacity(monitors.len());

    for monitor in monitors {
        let mon_x = monitor.position.0 as f64;
        let mon_y = monitor.position.1 as f64;
        let mon_logical_w = monitor.logical_size.0 as f64;
        let mon_logical_h = monitor.logical_size.1 as f64;

        // Region of the image (in image pixels) that this monitor sees
        let crop_x = (mon_x - offset.0) / img_scale;
        let crop_y = (mon_y - offset.1) / img_scale;
        let crop_w = mon_logical_w / img_scale;
        let crop_h = mon_logical_h / img_scale;

        let output_img = extract_region(
            &src_img,
            crop_x,
            crop_y,
            crop_w,
            crop_h,
            img_w,
            img_h,
            monitor.physical_size.0,
            monitor.physical_size.1,
        );

        let out_path = cache_dir.join(format!("{}.png", monitor.name));
        output_img.save(&out_path)?;
        results.push((monitor.name.clone(), out_path));
    }

    Ok(results)
}

#[allow(clippy::too_many_arguments)]
fn extract_region(
    src: &DynamicImage,
    crop_x: f64,
    crop_y: f64,
    crop_w: f64,
    crop_h: f64,
    img_w: f64,
    img_h: f64,
    output_w: u32,
    output_h: u32,
) -> DynamicImage {
    // Clamp the crop region to the image bounds to find the actual overlap
    let src_x1 = crop_x.max(0.0);
    let src_y1 = crop_y.max(0.0);
    let src_x2 = (crop_x + crop_w).min(img_w);
    let src_y2 = (crop_y + crop_h).min(img_h);

    let overlap_w = (src_x2 - src_x1).max(0.0);
    let overlap_h = (src_y2 - src_y1).max(0.0);

    if overlap_w <= 0.0 || overlap_h <= 0.0 {
        // No overlap — return black image at physical resolution
        return DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            output_w,
            output_h,
            *image::Rgba::from_slice(&[0, 0, 0, 255]),
        ));
    }

    // Crop the overlapping region from the source
    let cropped = src.crop_imm(
        src_x1.round() as u32,
        src_y1.round() as u32,
        overlap_w.round().max(1.0) as u32,
        overlap_h.round().max(1.0) as u32,
    );

    // If the crop covers the full requested region, just resize to output
    let full_coverage =
        crop_x >= 0.0 && crop_y >= 0.0 && (crop_x + crop_w) <= img_w && (crop_y + crop_h) <= img_h;

    if full_coverage {
        return resize_image(&cropped, output_w, output_h);
    }

    // Partial coverage: place the cropped region onto a black canvas
    // First figure out where the overlap sits within the crop region (in 0..1 normalized coords)
    let norm_x = ((src_x1 - crop_x) / crop_w).clamp(0.0, 1.0);
    let norm_y = ((src_y1 - crop_y) / crop_h).clamp(0.0, 1.0);
    let norm_w = (overlap_w / crop_w).clamp(0.0, 1.0);
    let norm_h = (overlap_h / crop_h).clamp(0.0, 1.0);

    // Convert to output pixel coords
    let dest_x = (norm_x * output_w as f64).round() as u32;
    let dest_y = (norm_y * output_h as f64).round() as u32;
    let dest_w = (norm_w * output_w as f64).round().max(1.0) as u32;
    let dest_h = (norm_h * output_h as f64).round().max(1.0) as u32;

    let resized = resize_image(&cropped, dest_w, dest_h);

    let mut canvas = RgbaImage::from_pixel(
        output_w,
        output_h,
        *image::Rgba::from_slice(&[0, 0, 0, 255]),
    );
    image::imageops::overlay(
        &mut canvas,
        &resized.to_rgba8(),
        dest_x.into(),
        dest_y.into(),
    );

    DynamicImage::ImageRgba8(canvas)
}

fn resize_image(img: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    let mut resizer = fast_image_resize::Resizer::new();
    let options = fast_image_resize::ResizeOptions {
        algorithm: fast_image_resize::ResizeAlg::Convolution(
            fast_image_resize::FilterType::Lanczos3,
        ),
        ..Default::default()
    };
    let mut new_image = DynamicImage::new(width, height, img.color());
    if let Err(err) = resizer.resize(img, &mut new_image, &options) {
        tracing::warn!(?err, "fast_image_resize failed, falling back");
        new_image =
            image::imageops::resize(img, width, height, image::imageops::FilterType::Lanczos3)
                .into();
    }
    new_image
}
