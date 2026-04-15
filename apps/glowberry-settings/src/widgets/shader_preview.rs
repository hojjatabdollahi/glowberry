// SPDX-License-Identifier: MPL-2.0

//! Shader preview renderer for generating thumbnails of live wallpaper shaders.
//!
//! This module provides GPU-accelerated shader preview rendering that outputs
//! RGBA pixel data suitable for display in iced widgets.

use std::borrow::Cow;
use std::path::Path;
use std::time::Instant;

use pollster::FutureExt;

/// WGSL preamble prepended to user shaders (must match glowberry).
const WGSL_PREAMBLE: &str = r#"
// GlowBerry live wallpaper uniforms
@group(0) @binding(0) var<uniform> iResolution: vec2f;
@group(0) @binding(1) var<uniform> iTime: f32;
"#;

/// Full-screen vertex shader.
const VERTEX_SHADER: &str = r#"
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

/// Error type for shader preview rendering.
#[derive(Debug)]
pub enum PreviewError {
    /// Failed to read shader file.
    Io(std::io::Error),
    /// Failed to create GPU resources.
    Gpu(String),
    /// Shader compilation failed.
    ShaderCompilation(String),
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Gpu(e) => write!(f, "GPU error: {e}"),
            Self::ShaderCompilation(e) => write!(f, "Shader compilation error: {e}"),
        }
    }
}

impl std::error::Error for PreviewError {}

impl From<std::io::Error> for PreviewError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Shader preview renderer that renders shaders to RGBA pixel data.
#[allow(dead_code)] // resolution_buffer is read by the GPU, not Rust code
pub struct ShaderPreviewRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    resolution_buffer: wgpu::Buffer,
    time_buffer: wgpu::Buffer,
    render_texture: wgpu::Texture,
    output_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    start_time: Instant,
}

impl ShaderPreviewRenderer {
    /// Create a new shader preview renderer.
    ///
    /// # Arguments
    /// * `shader_path` - Path to the WGSL shader file
    /// * `width` - Preview width in pixels
    /// * `height` - Preview height in pixels
    pub fn new(shader_path: &Path, width: u32, height: u32) -> Result<Self, PreviewError> {
        // Read shader code
        let shader_code = std::fs::read_to_string(shader_path)?;

        // Check if shader requires texture resources (which we don't provide in preview)
        if shader_code.contains("iTexture") || shader_code.contains("iTextureSampler") {
            return Err(PreviewError::ShaderCompilation(
                "Shader requires texture resources not available in preview".into(),
            ));
        }

        // Create wgpu instance
        let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_desc.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        let instance = wgpu::Instance::new(instance_desc);

        // Request adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .block_on()
            .map_err(|e| PreviewError::Gpu(format!("No suitable GPU adapter found: {e}")))?;

        // Request device
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .block_on()
            .map_err(|e| PreviewError::Gpu(format!("Failed to create device: {e}")))?;

        // Create uniform buffers
        let resolution_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader-preview: iResolution buffer"),
            size: std::mem::size_of::<[f32; 2]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let time_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader-preview: iTime buffer"),
            size: std::mem::size_of::<f32>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader-preview: bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader-preview: bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: resolution_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: time_buffer.as_entire_binding(),
                },
            ],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader-preview: pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            ..Default::default()
        });

        // Create vertex shader
        let vertex_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader-preview: vertex shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(VERTEX_SHADER)),
        });

        // Create fragment shader with preamble, using error scope to catch validation errors
        let full_shader = format!("{WGSL_PREAMBLE}\n{shader_code}");

        let fragment_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader-preview: fragment shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(full_shader)),
        });

        // Create render pipeline
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader-preview: render pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_module,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Create render texture
        let render_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shader-preview: render texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        // Create output buffer for reading back pixels
        let bytes_per_row = aligned_bytes_per_row(width, 4);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader-preview: output buffer"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // Initialize resolution uniform
        let resolution_data = [width as f32, height as f32];
        queue.write_buffer(
            &resolution_buffer,
            0,
            bytemuck::cast_slice(&resolution_data),
        );

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group,
            resolution_buffer,
            time_buffer,
            render_texture,
            output_buffer,
            width,
            height,
            start_time: Instant::now(),
        })
    }

    /// Render a single frame and return the RGBA pixel data.
    ///
    /// Returns a tuple of (width, height, rgba_data).
    pub fn render_frame(&self) -> Result<(u32, u32, Vec<u8>), PreviewError> {
        // Update time uniform
        let elapsed = self.start_time.elapsed().as_secs_f32();
        self.queue
            .write_buffer(&self.time_buffer, 0, bytemuck::bytes_of(&elapsed));

        // Create texture view
        let view = self
            .render_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Create command encoder
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shader-preview: render encoder"),
            });

        // Render pass
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shader-preview: render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.draw(0..4, 0..1);
        }

        // Copy texture to buffer
        let bytes_per_row = aligned_bytes_per_row(self.width, 4);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.render_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        // Submit commands
        self.queue.submit(std::iter::once(encoder.finish()));

        // Map buffer and read pixels
        let buffer_slice = self.output_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        rx.recv()
            .map_err(|_| PreviewError::Gpu("Failed to receive map result".into()))?
            .map_err(|e| PreviewError::Gpu(format!("Buffer mapping failed: {e}")))?;

        // Read data and unmap
        let data = buffer_slice.get_mapped_range();

        // Remove row padding if present
        let unpadded_bytes_per_row = self.width * 4;
        let rgba_data = if bytes_per_row == unpadded_bytes_per_row {
            data.to_vec()
        } else {
            let mut result = Vec::with_capacity((unpadded_bytes_per_row * self.height) as usize);
            for row in 0..self.height {
                let start = (row * bytes_per_row) as usize;
                let end = start + unpadded_bytes_per_row as usize;
                result.extend_from_slice(&data[start..end]);
            }
            result
        };

        drop(data);
        self.output_buffer.unmap();

        Ok((self.width, self.height, rgba_data))
    }
}

/// Calculate aligned bytes per row for wgpu buffer operations.
fn aligned_bytes_per_row(width: u32, bytes_per_pixel: u32) -> u32 {
    let unpadded = width.saturating_mul(bytes_per_pixel);
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    unpadded.div_ceil(alignment) * alignment
}

/// Render a single preview frame for a shader.
///
/// This is a convenience function that creates a temporary renderer,
/// renders one frame, and returns the RGBA data.
///
/// # Arguments
/// * `shader_path` - Path to the WGSL shader file
/// * `width` - Preview width in pixels  
/// * `height` - Preview height in pixels
///
/// # Returns
/// A tuple of (width, height, rgba_data) on success.
pub fn render_shader_preview(
    shader_path: &Path,
    width: u32,
    height: u32,
) -> Result<(u32, u32, Vec<u8>), PreviewError> {
    let renderer = ShaderPreviewRenderer::new(shader_path, width, height)?;
    renderer.render_frame()
}
