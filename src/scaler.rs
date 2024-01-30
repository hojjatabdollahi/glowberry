// SPDX-License-Identifier: MPL-2.0-only

//! Background scaling methods such as fit, stretch, and zoom.

use image::imageops::FilterType;
use image::{DynamicImage, Pixel};

pub fn fit(
    img: &image::DynamicImage,
    color: &[f32; 3],
    layer_width: u32,
    layer_height: u32,
) -> image::DynamicImage {
    // TODO: convert color to the same format as the input image.
    let mut filled_image =
        image::ImageBuffer::from_pixel(layer_width, layer_height, *image::Rgb::from_slice(color));

    let (w, h) = (img.width(), img.height());

    let ratio = (layer_width as f64 / w as f64).min(layer_height as f64 / h as f64);

    let (new_width, new_height) = (
        (w as f64 * ratio).round() as u32,
        (h as f64 * ratio).round() as u32,
    );

    let resized_image = img.resize(new_width, new_height, FilterType::Lanczos3);

    image::imageops::replace(
        &mut filled_image,
        &resized_image.to_rgb32f(),
        ((layer_width - new_width) / 2).into(),
        ((layer_height - new_height) / 2).into(),
    );

    DynamicImage::from(filled_image)
}

pub fn stretch(
    img: &image::DynamicImage,
    layer_width: u32,
    layer_height: u32,
) -> image::DynamicImage {
    img.resize_exact(layer_width, layer_height, FilterType::Lanczos3)
}

pub fn zoom(img: &image::DynamicImage, layer_width: u32, layer_height: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());

    let ratio = (layer_width as f64 / w as f64).max(layer_height as f64 / h as f64);

    let (new_width, new_height) = (
        (w as f64 * ratio).round() as u32,
        (h as f64 * ratio).round() as u32,
    );

    let mut new_image = image::imageops::resize(img, new_width, new_height, FilterType::Lanczos3);

    image::imageops::crop(
        &mut new_image,
        (new_width - layer_width) / 2,
        (new_height - layer_height) / 2,
        layer_width,
        layer_height,
    )
    .to_image()
    .into()
}
