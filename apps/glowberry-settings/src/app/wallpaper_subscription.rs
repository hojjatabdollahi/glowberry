// SPDX-License-Identifier: MPL-2.0

//! Wallpaper loading subscription

use cosmic::iced::Subscription;
use cosmic::iced::futures::{Stream, StreamExt as _};
use image::{ImageBuffer, Rgba, RgbaImage};
use std::path::PathBuf;
use std::pin::Pin;
use walkdir::WalkDir;

/// Events emitted by the wallpaper subscription
#[derive(Clone, Debug)]
pub enum WallpaperEvent {
    /// Started loading wallpapers
    Loading,
    /// A wallpaper was loaded
    Load {
        path: PathBuf,
        display: RgbaImage,
        selection: RgbaImage,
    },
    /// Finished loading wallpapers
    Loaded,
}

/// Create a subscription that loads wallpapers from the given directory
pub fn wallpapers(current_dir: PathBuf) -> Subscription<WallpaperEvent> {
    Subscription::run_with(current_dir, async_stream)
}

fn async_stream(current_dir: &PathBuf) -> Pin<Box<dyn Send + Stream<Item = WallpaperEvent>>> {
    Box::pin(futures_lite::stream::unfold(
        LoadState::Init(current_dir.clone()),
        |state| async move {
            match state {
                LoadState::Init(path) => Some((WallpaperEvent::Loading, LoadState::Loading(path))),
                LoadState::Loading(path) => {
                    let stream = load_wallpapers_from_path(path).await;
                    // Get first item or signal done
                    let mut stream = stream;
                    if let Some((path, display, selection)) = stream.next().await {
                        Some((
                            WallpaperEvent::Load {
                                path,
                                display,
                                selection,
                            },
                            LoadState::Streaming(stream),
                        ))
                    } else {
                        Some((WallpaperEvent::Loaded, LoadState::Done))
                    }
                }
                LoadState::Streaming(mut stream) => {
                    if let Some((path, display, selection)) = stream.next().await {
                        Some((
                            WallpaperEvent::Load {
                                path,
                                display,
                                selection,
                            },
                            LoadState::Streaming(stream),
                        ))
                    } else {
                        Some((WallpaperEvent::Loaded, LoadState::Done))
                    }
                }
                LoadState::Done => None,
            }
        },
    ))
}

enum LoadState {
    Init(PathBuf),
    Loading(PathBuf),
    Streaming(Pin<Box<dyn Send + Stream<Item = (PathBuf, RgbaImage, RgbaImage)>>>),
    Done,
}

/// Load wallpapers from a directory
async fn load_wallpapers_from_path(
    path: PathBuf,
) -> Pin<Box<dyn Send + Stream<Item = (PathBuf, RgbaImage, RgbaImage)>>> {
    let candidate_paths: Vec<_> = WalkDir::new(&path)
        .max_depth(3)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .collect();

    let stream = futures_lite::stream::iter(candidate_paths).filter_map(|path| async move {
        if is_image_file(&path) {
            load_image_with_thumbnail(path).await
        } else {
            None
        }
    });

    Box::pin(stream)
}

fn is_image_file(path: &PathBuf) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };

    let ext_lower = ext.to_lowercase();
    matches!(
        ext_lower.as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "jxl" | "avif"
    )
}

async fn load_image_with_thumbnail(path: PathBuf) -> Option<(PathBuf, RgbaImage, RgbaImage)> {
    tokio::task::spawn_blocking(move || load_image_with_thumbnail_sync(&path))
        .await
        .ok()
        .flatten()
}

fn load_image_with_thumbnail_sync(
    path: &PathBuf,
) -> Option<(
    PathBuf,
    ImageBuffer<Rgba<u8>, Vec<u8>>,
    ImageBuffer<Rgba<u8>, Vec<u8>>,
)> {
    // Try to load the image
    let image = if path.extension().is_some_and(|e| e == "jxl") {
        decode_jpegxl(path).ok()?
    } else {
        image::open(path).ok()?
    };

    // Create display thumbnail (300x169)
    let display_thumbnail = resize_thumbnail(&image, 300, 169);

    // Create selection thumbnail (158x105) with rounded corners
    let mut selection_thumbnail = image::imageops::resize(
        &display_thumbnail,
        158,
        105,
        image::imageops::FilterType::Lanczos3,
    );
    round(&mut selection_thumbnail, [8, 8, 8, 8]);

    Some((path.clone(), display_thumbnail, selection_thumbnail))
}

fn resize_thumbnail(img: &image::DynamicImage, new_width: u32, new_height: u32) -> RgbaImage {
    use fast_image_resize::{ResizeAlg, ResizeOptions, Resizer, SrcCropping};

    let mut resizer = Resizer::new();
    let options = ResizeOptions {
        algorithm: ResizeAlg::Convolution(fast_image_resize::FilterType::Lanczos3),
        cropping: SrcCropping::FitIntoDestination((new_width as f64, new_height as f64)),
        ..Default::default()
    };

    let mut new_image = image::DynamicImage::new(new_width, new_height, img.color());

    if resizer.resize(img, &mut new_image, &options).is_err() {
        // Fallback to standard resize
        return image::imageops::resize(
            img,
            new_width,
            new_height,
            image::imageops::FilterType::Lanczos3,
        );
    }

    new_image.to_rgba8()
}

fn decode_jpegxl(path: &PathBuf) -> eyre::Result<image::DynamicImage> {
    use jxl_oxide::integration::JxlDecoder;
    use std::fs::File;

    let file = File::open(path)?;
    let decoder = JxlDecoder::new(file)?;
    Ok(image::DynamicImage::from_decoder(decoder)?)
}

// Rounded corner implementation from cosmic-settings-wallpaper
fn round(img: &mut RgbaImage, radius: [u32; 4]) {
    let (width, height) = img.dimensions();
    if radius[0] + radius[1] > width
        || radius[3] + radius[2] > width
        || radius[0] + radius[3] > height
        || radius[1] + radius[2] > height
    {
        return;
    }

    border_radius(img, radius[0], |x, y| (x - 1, y - 1));
    border_radius(img, radius[1], |x, y| (width - x, y - 1));
    border_radius(img, radius[2], |x, y| (width - x, height - y));
    border_radius(img, radius[3], |x, y| (x - 1, height - y));
}

fn border_radius(img: &mut RgbaImage, r: u32, coordinates: impl Fn(u32, u32) -> (u32, u32)) {
    if r == 0 {
        return;
    }
    let r0 = r;
    let r = 16 * r;

    let mut x = 0;
    let mut y = r - 1;
    let mut p: i32 = 2 - r as i32;

    let mut alpha: u16 = 0;
    let mut skip_draw = true;

    let draw = |img: &mut RgbaImage, alpha: u16, x: u32, y: u32| {
        if alpha == 0 || alpha > 256 {
            return;
        }
        let (cx, cy) = coordinates(r0 - x, r0 - y);
        if cx < img.width() && cy < img.height() {
            let pixel_alpha = &mut img[(cx, cy)].0[3];
            *pixel_alpha = ((alpha * *pixel_alpha as u16 + 128) / 256) as u8;
        }
    };

    'l: loop {
        {
            let i = x / 16;
            for j in y / 16 + 1..r0 {
                let (cx, cy) = coordinates(r0 - i, r0 - j);
                if cx < img.width() && cy < img.height() {
                    img[(cx, cy)].0[3] = 0;
                }
            }
        }
        {
            let j = x / 16;
            for i in y / 16 + 1..r0 {
                let (cx, cy) = coordinates(r0 - i, r0 - j);
                if cx < img.width() && cy < img.height() {
                    img[(cx, cy)].0[3] = 0;
                }
            }
        }

        if !skip_draw {
            draw(img, alpha, x / 16 - 1, y / 16);
            draw(img, alpha, y / 16, x / 16 - 1);
            alpha = 0;
        }

        for _ in 0..16 {
            skip_draw = false;

            if x >= y {
                break 'l;
            }

            alpha += y as u16 % 16 + 1;
            if p < 0 {
                x += 1;
                p += (2 * x + 2) as i32;
            } else {
                if y % 16 == 0 {
                    draw(img, alpha, x / 16, y / 16);
                    draw(img, alpha, y / 16, x / 16);
                    skip_draw = true;
                    alpha = (x + 1) as u16 % 16 * 16;
                }

                x += 1;
                p -= (2 * (y - x) + 2) as i32;
                y -= 1;
            }
        }
    }

    if x / 16 == y / 16 {
        if x == y {
            alpha += y as u16 % 16 + 1;
        }
        let s = y as u16 % 16 + 1;
        let alpha = 2 * alpha - s * s;
        draw(img, alpha, x / 16, y / 16);
    }

    let range = y / 16 + 1..r0;
    for i in range.clone() {
        for j in range.clone() {
            let (cx, cy) = coordinates(r0 - i, r0 - j);
            if cx < img.width() && cy < img.height() {
                img[(cx, cy)].0[3] = 0;
            }
        }
    }
}
