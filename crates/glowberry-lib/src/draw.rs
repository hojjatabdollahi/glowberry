// SPDX-License-Identifier: MPL-2.0

use image::{DynamicImage, GenericImageView};
use sctk::{
    reexports::{
        client::{
            Dispatch, QueueHandle, protocol::wl_callback, protocol::wl_shm, protocol::wl_surface,
        },
        protocols::wp::viewporter::client::wp_viewport,
    },
    shell::{WaylandSurface, wlr_layer::LayerSurface},
    shm::slot::{Buffer, CreateBufferError, SlotPool},
};

pub fn canvas(
    pool: &mut SlotPool,
    image: &DynamicImage,
    width: i32,
    height: i32,
    stride: i32,
) -> Result<Buffer, CreateBufferError> {
    let (buffer, canvas) =
        pool.create_buffer(width, height, stride, wl_shm::Format::Xrgb8888)?;

    xrgb888_canvas(canvas, image);

    Ok(buffer)
}

pub fn layer_surface<T>(
    layer_surface: &LayerSurface,
    viewport: &wp_viewport::WpViewport,
    queue_handle: &QueueHandle<T>,
    buffer: &Buffer,
    buffer_damage: (i32, i32),
    size: (u32, u32),
) where
    T: Dispatch<wl_callback::WlCallback, wl_surface::WlSurface> + 'static,
{
    let (width, height) = size;

    let wl_surface = layer_surface.wl_surface();

    // Damage the entire window
    wl_surface.damage_buffer(0, 0, buffer_damage.0, buffer_damage.1);

    // Request our next frame
    layer_surface
        .wl_surface()
        .frame(queue_handle, wl_surface.clone());

    // Attach and commit to present.
    if let Err(why) = buffer.attach_to(wl_surface) {
        tracing::error!(?why, "buffer attachment failed");
    }

    viewport.set_destination(width as i32, height as i32);

    wl_surface.commit();
}

/// Draws the image on an 8-bit canvas.
pub fn xrgb888_canvas(canvas: &mut [u8], image: &DynamicImage) {
    for (pos, (_, _, pixel)) in image.pixels().enumerate() {
        let indice = pos * 4;

        let [r, g, b, _] = pixel.0;

        let r = u32::from(r) << 16;
        let g = u32::from(g) << 8;
        let b = u32::from(b);

        canvas[indice..indice + 4].copy_from_slice(&(r | g | b).to_le_bytes());
    }
}
