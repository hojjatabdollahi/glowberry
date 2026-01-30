// SPDX-License-Identifier: MPL-2.0

//! Simplified fragment shader canvas for live wallpapers.
//!
//! This is a streamlined version of vibe's FragmentCanvas, providing:
//! - `iResolution` - screen dimensions
//! - `iTime` - elapsed time for animation
//! - Optional background texture sampling

use glowberry_config::{ShaderContent, ShaderLanguage, ShaderSource};
use image::DynamicImage;
use std::borrow::Cow;
use std::time::{Duration, Instant};

use crate::gpu::GpuRenderer;

/// WGSL preamble prepended to user shaders.
const WGSL_PREAMBLE: &str = r#"
// GlowBerry live wallpaper uniforms
@group(0) @binding(0) var<uniform> iResolution: vec2f;
@group(0) @binding(1) var<uniform> iTime: f32;
"#;

/// WGSL preamble with texture support.
const WGSL_PREAMBLE_WITH_TEXTURE: &str = r#"
// GlowBerry live wallpaper uniforms
@group(0) @binding(0) var<uniform> iResolution: vec2f;
@group(0) @binding(1) var<uniform> iTime: f32;
@group(0) @binding(2) var iTexture: texture_2d<f32>;
@group(0) @binding(3) var iTextureSampler: sampler;
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

/// Error when loading or compiling a shader.
#[derive(Debug, thiserror::Error)]
pub enum ShaderError {
    #[error("Failed to read shader file: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to load background image: {0}")]
    ImageLoad(#[from] image::ImageError),

    #[error("Unsupported shader language: {0:?}")]
    UnsupportedLanguage(ShaderLanguage),
}

pub fn detect_language(source: &ShaderSource) -> ShaderLanguage {
    if let ShaderContent::Path(path) = &source.shader {
        if path
            .extension()
            .map_or(false, |ext| ext == "glsl" || ext == "frag")
        {
            return ShaderLanguage::Glsl;
        }
    }

    source.language
}

fn aligned_bytes_per_row(width: u32, bytes_per_pixel: u32) -> u32 {
    let unpadded = width.saturating_mul(bytes_per_pixel);
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    ((unpadded + alignment - 1) / alignment) * alignment
}

fn texture_upload_data(rgba: &[u8], width: u32, height: u32) -> (Cow<'_, [u8]>, u32, u32) {
    let bytes_per_pixel = 4;
    let unpadded_bytes_per_row = width.saturating_mul(bytes_per_pixel);
    let bytes_per_row = aligned_bytes_per_row(width, bytes_per_pixel);

    if bytes_per_row == unpadded_bytes_per_row {
        return (Cow::Borrowed(rgba), bytes_per_row, height);
    }

    let mut padded = vec![0u8; (bytes_per_row * height) as usize];

    for row in 0..height {
        let src_offset = (row * unpadded_bytes_per_row) as usize;
        let dst_offset = (row * bytes_per_row) as usize;
        let src_end = src_offset + unpadded_bytes_per_row as usize;

        padded[dst_offset..dst_offset + unpadded_bytes_per_row as usize]
            .copy_from_slice(&rgba[src_offset..src_end]);
    }

    (Cow::Owned(padded), bytes_per_row, height)
}

fn build_shader_source(
    language: ShaderLanguage,
    preamble: &str,
    shader_code: &str,
) -> Result<wgpu::ShaderSource<'static>, ShaderError> {
    let full_shader = match language {
        ShaderLanguage::Wgsl => {
            let full_code = format!("{}\n{}", preamble, shader_code);
            wgpu::ShaderSource::Wgsl(Cow::Owned(full_code))
        }
        ShaderLanguage::Glsl => {
            // GLSL would need translation to WGSL, which is not supported yet.
            return Err(ShaderError::UnsupportedLanguage(ShaderLanguage::Glsl));
        }
    };

    Ok(full_shader)
}

/// A GPU-rendered fragment shader canvas for live wallpapers.
pub struct FragmentCanvas {
    // GPU resources
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,

    // Uniform buffers
    resolution_buffer: wgpu::Buffer,
    time_buffer: wgpu::Buffer,

    // Animation state
    start_time: Instant,
    last_frame: Instant,
    frame_interval: Duration,
    /// The configured (original) frame rate from the shader source.
    configured_frame_rate: u8,

    // Optional background texture
    _background_texture: Option<wgpu::Texture>,
}

impl FragmentCanvas {
    /// Create a new fragment canvas from a shader source.
    pub fn new(
        renderer: &GpuRenderer,
        source: &ShaderSource,
        format: wgpu::TextureFormat,
    ) -> Result<Self, ShaderError> {
        let device = renderer.device();
        let queue = renderer.queue();

        // Load shader code
        let shader_code = match &source.shader {
            ShaderContent::Path(path) => std::fs::read_to_string(path)?,
            ShaderContent::Code(code) => code.clone(),
        };

        let language = detect_language(source);

        // Load optional background texture
        let (background_texture, has_texture) = if let Some(img_path) = &source.background_image {
            let img = image::open(img_path)?;
            let texture = Self::create_texture(device, queue, &img);
            (Some(texture), true)
        } else {
            (None, false)
        };

        // Create uniform buffers
        let resolution_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glowberry: iResolution buffer"),
            size: std::mem::size_of::<[f32; 2]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let time_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glowberry: iTime buffer"),
            size: std::mem::size_of::<f32>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create bind group layout
        let bind_group_layout = if has_texture {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("glowberry: bind group layout (with texture)"),
                entries: &[
                    // iResolution
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
                    // iTime
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
                    // iTexture
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // iTextureSampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            })
        } else {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("glowberry: bind group layout"),
                entries: &[
                    // iResolution
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
                    // iTime
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
            })
        };

        // Create bind group
        let bind_group = if has_texture {
            let texture = background_texture.as_ref().unwrap();
            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("glowberry: bind group (with texture)"),
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
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        } else {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("glowberry: bind group"),
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
            })
        };

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glowberry: pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            ..Default::default()
        });

        // Create vertex shader module
        let vertex_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glowberry: vertex shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(VERTEX_SHADER)),
        });

        // Create fragment shader module with preamble
        let preamble = if has_texture {
            WGSL_PREAMBLE_WITH_TEXTURE
        } else {
            WGSL_PREAMBLE
        };

        let full_shader = build_shader_source(language, preamble, &shader_code)?;

        let fragment_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glowberry: fragment shader"),
            source: full_shader,
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glowberry: render pipeline"),
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
            multiview: None,
            cache: None,
        });

        // Calculate frame interval
        let configured_frame_rate = source.frame_rate.clamp(1, 60);
        let frame_interval = Duration::from_secs_f64(1.0 / f64::from(configured_frame_rate));

        Ok(Self {
            pipeline,
            bind_group,
            resolution_buffer,
            time_buffer,
            start_time: Instant::now(),
            last_frame: Instant::now(),
            frame_interval,
            configured_frame_rate,
            _background_texture: background_texture,
        })
    }

    /// Create a GPU texture from an image.
    fn create_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &DynamicImage,
    ) -> wgpu::Texture {
        let rgba = image.to_rgba8();
        let dimensions = rgba.dimensions();

        let size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glowberry: background texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let (upload_data, bytes_per_row, rows_per_image) =
            texture_upload_data(&rgba, dimensions.0, dimensions.1);

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &upload_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows_per_image),
            },
            size,
        );

        texture
    }

    /// Update the resolution uniform.
    pub fn update_resolution(&self, queue: &wgpu::Queue, width: u32, height: u32) {
        let data = [width as f32, height as f32];
        queue.write_buffer(&self.resolution_buffer, 0, bytemuck::cast_slice(&data));
    }

    /// Check if enough time has passed for the next frame.
    pub fn should_render(&self) -> bool {
        self.last_frame.elapsed() >= self.frame_interval
    }

    /// Mark that a frame was rendered.
    pub fn mark_frame_rendered(&mut self) {
        self.last_frame = Instant::now();
    }

    /// Get the configured (original) frame rate.
    pub fn configured_frame_rate(&self) -> u8 {
        self.configured_frame_rate
    }

    /// Get the current effective frame rate.
    pub fn current_frame_rate(&self) -> u8 {
        (1.0 / self.frame_interval.as_secs_f64()).round() as u8
    }

    /// Set a temporary frame rate override.
    /// Pass `None` to restore the configured frame rate.
    pub fn set_frame_rate_override(&mut self, frame_rate: Option<u8>) {
        let effective_rate = frame_rate
            .unwrap_or(self.configured_frame_rate)
            .clamp(1, 60);
        self.frame_interval = Duration::from_secs_f64(1.0 / f64::from(effective_rate));
    }

    /// Render the shader to a texture view.
    pub fn render(&self, renderer: &GpuRenderer, view: &wgpu::TextureView) {
        let device = renderer.device();
        let queue = renderer.queue();

        // Update time uniform
        let elapsed = self.start_time.elapsed().as_secs_f32();
        queue.write_buffer(&self.time_buffer, 0, bytemuck::bytes_of(&elapsed));

        // Create command encoder
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("glowberry: render encoder"),
        });

        // Begin render pass
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glowberry: render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.draw(0..4, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use glowberry_config::{ShaderContent, ShaderLanguage, ShaderSource};

    #[test]
    fn detects_glsl_language_for_frag_extension() {
        let source = ShaderSource {
            shader: ShaderContent::Path("/tmp/test.frag".into()),
            background_image: None,
            language: ShaderLanguage::Wgsl,
            frame_rate: 30,
        };

        assert_eq!(super::detect_language(&source), ShaderLanguage::Glsl);
    }

    #[test]
    fn aligns_bytes_per_row_to_wgpu_requirement() {
        let bytes_per_pixel = 4;
        let aligned = super::aligned_bytes_per_row(1, bytes_per_pixel);

        assert_eq!(aligned, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    }

    #[test]
    fn pads_texture_upload_rows_when_needed() {
        let width = 1;
        let height = 2;
        let rgba = vec![1u8; (width * height * 4) as usize];

        let (upload_data, bytes_per_row, rows_per_image) =
            super::texture_upload_data(&rgba, width, height);

        assert_eq!(bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        assert_eq!(rows_per_image, height);
        assert_eq!(upload_data.len(), (bytes_per_row * height) as usize);
    }

    #[test]
    fn glsl_is_rejected_when_building_shader_source() {
        let result = super::build_shader_source(ShaderLanguage::Glsl, "preamble", "void main(){}");

        assert!(matches!(
            result,
            Err(super::ShaderError::UnsupportedLanguage(
                ShaderLanguage::Glsl
            ))
        ));
    }
}
