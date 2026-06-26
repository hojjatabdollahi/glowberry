// SPDX-License-Identifier: MPL-2.0

use image::{DynamicImage, Pixel, RgbaImage};
use std::collections::HashMap;
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

#[derive(Debug, Clone)]
pub struct LayerInfo {
    pub source_path: PathBuf,
    pub offset: (f64, f64),
    pub img_scale: f64,
    pub z_index: usize,
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

pub fn composite_for_monitors(
    layers: &mut [LayerInfo],
    monitors: &[MonitorInfo],
    cache_dir: &Path,
) -> Result<Vec<(String, PathBuf)>, CropError> {
    if monitors.is_empty() {
        return Err(CropError::NoMonitors);
    }

    std::fs::create_dir_all(cache_dir)?;

    // Pre-load all unique source images
    let mut image_cache: HashMap<PathBuf, DynamicImage> = HashMap::new();
    for layer in layers.iter() {
        if !image_cache.contains_key(&layer.source_path) {
            let img = image::open(&layer.source_path)?;
            image_cache.insert(layer.source_path.clone(), img);
        }
    }

    // Sort layers by z_index (bottom to top)
    layers.sort_by_key(|l| l.z_index);

    let mut results = Vec::with_capacity(monitors.len());

    for monitor in monitors {
        let mon_x = monitor.position.0 as f64;
        let mon_y = monitor.position.1 as f64;
        let mon_logical_w = monitor.logical_size.0 as f64;
        let mon_logical_h = monitor.logical_size.1 as f64;
        let phys_w = monitor.physical_size.0;
        let phys_h = monitor.physical_size.1;

        // Start with black canvas
        let mut canvas =
            RgbaImage::from_pixel(phys_w, phys_h, *image::Rgba::from_slice(&[0, 0, 0, 255]));

        // Composite each layer bottom-to-top
        for layer in layers.iter() {
            let Some(src_img) = image_cache.get(&layer.source_path) else {
                continue;
            };
            let img_w = src_img.width() as f64;
            let img_h = src_img.height() as f64;

            let crop_x = (mon_x - layer.offset.0) / layer.img_scale;
            let crop_y = (mon_y - layer.offset.1) / layer.img_scale;
            let crop_w = mon_logical_w / layer.img_scale;
            let crop_h = mon_logical_h / layer.img_scale;

            let layer_img = extract_region(
                src_img, crop_x, crop_y, crop_w, crop_h, img_w, img_h, phys_w, phys_h, true,
            );

            image::imageops::overlay(&mut canvas, &layer_img.to_rgba8(), 0, 0);
        }

        // Name the file by a hash of its pixels so the path changes whenever the
        // composited image changes (and stays the same when it doesn't). Downstream
        // consumers — cosmic-bg for the desktop, cosmic-greeter for the lock screen —
        // cache wallpapers by path and won't reload a file whose path is unchanged.
        // A stable per-output filename therefore leaves the desktop/lock screen
        // showing the previous image after the user picks a new one.
        let digest = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash_slice(canvas.as_raw().as_slice(), &mut hasher);
            std::hash::Hasher::finish(&hasher)
        };
        let out_path = cache_dir.join(format!("{}-{:016x}.png", monitor.name, digest));

        DynamicImage::ImageRgba8(canvas).save(&out_path)?;

        // Prune old composites for this monitor, keeping the few most recent so
        // the cache stays bounded. We keep more than one because the previously
        // applied file is still referenced by the cosmic-bg state (lock screen)
        // until the next export — deleting it eagerly would make the lock screen
        // fall back to its default wallpaper.
        prune_old_composites(cache_dir, &monitor.name);

        results.push((monitor.name.clone(), out_path));
    }

    Ok(results)
}

/// Keep only the most recent composites for `monitor_name` (both the legacy
/// `<name>.png` and `<name>-<hash>.png` files), pruning older ones.
fn prune_old_composites(cache_dir: &Path, monitor_name: &str) {
    const KEEP: usize = 3;
    let legacy = format!("{monitor_name}.png");
    let prefix = format!("{monitor_name}-");
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    let mut matches: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let fname = path.file_name()?.to_str()?.to_owned();
            if fname == legacy || (fname.starts_with(&prefix) && fname.ends_with(".png")) {
                let mtime = entry.metadata().ok()?.modified().ok()?;
                Some((mtime, path))
            } else {
                None
            }
        })
        .collect();
    // Newest first; drop everything past the KEEP most recent.
    matches.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in matches.into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(&path);
    }
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
    transparent_bg: bool,
) -> DynamicImage {
    let bg_pixel = if transparent_bg {
        *image::Rgba::from_slice(&[0, 0, 0, 0])
    } else {
        *image::Rgba::from_slice(&[0, 0, 0, 255])
    };

    let src_x1 = crop_x.max(0.0);
    let src_y1 = crop_y.max(0.0);
    let src_x2 = (crop_x + crop_w).min(img_w);
    let src_y2 = (crop_y + crop_h).min(img_h);

    let overlap_w = (src_x2 - src_x1).max(0.0);
    let overlap_h = (src_y2 - src_y1).max(0.0);

    if overlap_w <= 0.0 || overlap_h <= 0.0 {
        return DynamicImage::ImageRgba8(RgbaImage::from_pixel(output_w, output_h, bg_pixel));
    }

    let cropped = src.crop_imm(
        src_x1.round() as u32,
        src_y1.round() as u32,
        overlap_w.round().max(1.0) as u32,
        overlap_h.round().max(1.0) as u32,
    );

    let full_coverage =
        crop_x >= 0.0 && crop_y >= 0.0 && (crop_x + crop_w) <= img_w && (crop_y + crop_h) <= img_h;

    if full_coverage {
        return resize_image(&cropped, output_w, output_h);
    }

    let norm_x = ((src_x1 - crop_x) / crop_w).clamp(0.0, 1.0);
    let norm_y = ((src_y1 - crop_y) / crop_h).clamp(0.0, 1.0);
    let norm_w = (overlap_w / crop_w).clamp(0.0, 1.0);
    let norm_h = (overlap_h / crop_h).clamp(0.0, 1.0);

    let dest_x = (norm_x * output_w as f64).round() as u32;
    let dest_y = (norm_y * output_h as f64).round() as u32;
    let dest_w = (norm_w * output_w as f64).round().max(1.0) as u32;
    let dest_h = (norm_h * output_h as f64).round().max(1.0) as u32;

    let resized = resize_image(&cropped, dest_w, dest_h);

    let mut canvas = RgbaImage::from_pixel(output_w, output_h, bg_pixel);
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
