// SPDX-License-Identifier: MPL-2.0

//! GPU rendering support for live shader wallpapers.

use pollster::FutureExt;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use sctk::reexports::client::{Connection, Proxy};
use std::ptr::NonNull;
use wgpu::SurfaceTargetUnsafe;

/// GPU renderer for shader-based live wallpapers.
///
/// This is lazily initialized only when a shader wallpaper is configured.
#[derive(Debug)]
pub struct GpuRenderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl GpuRenderer {
    /// Create a new GPU renderer.
    ///
    /// # Panics
    ///
    /// Panics if no suitable GPU adapter is found.
    pub fn new() -> Self {
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        let instance = wgpu::Instance::new(instance_desc);

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .block_on()
            .expect("Failed to find GPU adapter for live wallpapers");

        tracing::info!(
            "GPU renderer using: {} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .block_on()
            .expect("Failed to create GPU device");

        Self {
            instance,
            adapter,
            device,
            queue,
        }
    }

    /// Create a wgpu surface from a Wayland surface.
    ///
    /// # Safety
    ///
    /// The connection and surface must remain valid for the lifetime of the returned surface.
    pub unsafe fn create_surface(
        &self,
        conn: &Connection,
        wl_surface: &sctk::reexports::client::protocol::wl_surface::WlSurface,
    ) -> wgpu::Surface<'static> {
        let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(conn.backend().display_ptr() as *mut _)
                .expect("Wayland display pointer is null"),
        ));

        let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(wl_surface.id().as_ptr() as *mut _)
                .expect("Wayland surface pointer is null"),
        ));

        // SAFETY: The caller guarantees that conn and wl_surface remain valid
        unsafe {
            self.instance
                .create_surface_unsafe(SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(raw_display_handle),
                    raw_window_handle,
                })
                .expect("Failed to create GPU surface")
        }
    }

    /// Configure a surface for rendering.
    pub fn configure_surface(
        &self,
        surface: &wgpu::Surface<'_>,
        width: u32,
        height: u32,
    ) -> wgpu::SurfaceConfiguration {
        let capabilities = surface.get_capabilities(&self.adapter);

        // Prefer non-sRGB formats for better color accuracy
        let format = capabilities
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(capabilities.formats[0]);

        let alpha_mode = if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            capabilities.alpha_modes[0]
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };

        surface.configure(&self.device, &config);
        config
    }

    #[inline]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    #[inline]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

impl Default for GpuRenderer {
    fn default() -> Self {
        Self::new()
    }
}
