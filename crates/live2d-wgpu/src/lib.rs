use bytemuck::{Pod, Zeroable};
use live2d_core::{CanvasInfo, ModelSnapshot, TextureAsset};
use std::collections::HashSet;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WgpuPreviewUniform {
    pub viewport: [f32; 4],
    pub view_transform: [f32; 4],
    pub tint_a: [f32; 4],
    pub tint_b: [f32; 4],
    pub grad_lo: [f32; 4],
    pub grad_hi: [f32; 4],
    pub ptcl_color: [f32; 4],
    pub damage_fray_color: [f32; 4],
    pub params0: [f32; 4],
    pub params1: [f32; 4],
    pub params2: [f32; 4],
    pub params3: [f32; 4],
    pub params4: [f32; 4],
    pub params5: [f32; 4],
    pub params6: [f32; 4],
    pub params7: [f32; 4],
    pub params8: [f32; 4],
    pub params9: [f32; 4],
    pub picker: [f32; 4],
}

impl WgpuPreviewUniform {
    pub fn neutral(time_seconds: f32, width: u32, height: u32) -> Self {
        Self {
            viewport: [time_seconds, width.max(1) as f32, height.max(1) as f32, 0.0],
            view_transform: [0.0, 0.0, 1.0, 0.0],
            tint_a: [1.0, 1.0, 1.0, 1.0],
            tint_b: [1.0, 1.0, 1.0, 1.0],
            grad_lo: [0.0, 0.0, 0.0, 1.0],
            grad_hi: [1.0, 1.0, 1.0, 1.0],
            ptcl_color: [1.0, 1.0, 1.0, 1.0],
            damage_fray_color: [0.92, 0.88, 0.80, 1.0],
            params0: [0.0, 1.0, 0.0, 1.0],
            params1: [1.0, 0.0, 0.0, 0.0],
            params2: [0.0, 0.0, 2.0, 1.0],
            params3: [1.0, 1.0, 1.0, 0.0],
            params4: [0.0, 0.0, 0.35, 0.0],
            params5: [0.12, 1.25, 0.0, 2.0],
            params6: [1.0, 0.2, 1.0, 0.0],
            params7: [18.0, 0.15, 0.65, 0.0],
            params8: [0.0, 0.5, 0.6, 0.0],
            params9: [0.4, 0.4, 0.0, 0.0],
            picker: [0.0, 0.0, 0.0, 0.0],
        }
    }

    pub fn with_picker_hover(mut self, active: bool) -> Self {
        self.picker[0] = if active { 1.0 } else { 0.0 };
        self
    }

    pub fn with_view_transform(mut self, transform: [f32; 4]) -> Self {
        self.view_transform = transform;
        self
    }

    pub fn live2d_effect(self) -> [f32; 4] {
        let strength = self.params0[0].clamp(0.0, 1.0);
        let brightness = self.params0[1].clamp(0.0, 2.0);
        let opacity = self.params3[1].clamp(0.0, 1.0);
        [
            (1.0 * (1.0 - strength) + self.tint_a[0] * strength * brightness).clamp(0.0, 2.0),
            (1.0 * (1.0 - strength) + self.tint_a[1] * strength * brightness).clamp(0.0, 2.0),
            (1.0 * (1.0 - strength) + self.tint_a[2] * strength * brightness).clamp(0.0, 2.0),
            opacity,
        ]
    }
}

pub struct WgpuPreviewRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl WgpuPreviewRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Live2D Preview Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("preview.wgsl").into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Preview Uniform"),
            size: std::mem::size_of::<WgpuPreviewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Live2D Preview Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Live2D Preview Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Live2D Preview Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Live2D Preview Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            uniform_buffer,
            bind_group,
        }
    }

    pub fn render<'pass>(
        &'pass self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'pass>,
        uniform: WgpuPreviewUniform,
    ) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[derive(Debug, Clone)]
pub struct WgpuLive2DView {
    pub transform: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub effect: [f32; 4],
    pub target_drawable_ids: Vec<String>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Live2dUniform {
    viewport: [f32; 4],
    view_transform: [f32; 4],
    canvas: [f32; 4],
    effect: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

pub struct WgpuLive2DRenderer {
    pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    scene_key: Option<String>,
    gpu_scene: Option<GpuScene>,
}

struct GpuScene {
    drawables: Vec<GpuDrawable>,
    textures: Vec<wgpu::BindGroup>,
}

struct GpuDrawable {
    id: String,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    texture_index: usize,
}

impl WgpuLive2DRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Live2D Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("live2d.wgsl").into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Uniform"),
            size: std::mem::size_of::<Live2dUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Live2D Uniform Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Live2D Uniform Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Live2D Texture Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Live2D Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Live2D Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Live2D Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 8,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            texture_layout,
            sampler,
            uniform_buffer,
            uniform_bind_group,
            scene_key: None,
            gpu_scene: None,
        }
    }

    pub fn prepare_model(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
    ) {
        self.ensure_scene(device, queue, snapshot);
    }

    pub fn render<'pass>(
        &'pass mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'pass>,
        snapshot: &ModelSnapshot,
        view: WgpuLive2DView,
    ) {
        self.ensure_scene(device, queue, snapshot);
        let Some(gpu_scene) = &self.gpu_scene else {
            return;
        };
        let target_ids = view
            .target_drawable_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        for drawable in &gpu_scene.drawables {
            let Some(texture) = gpu_scene.textures.get(drawable.texture_index) else {
                continue;
            };
            let effect = if target_ids.is_empty() || target_ids.contains(drawable.id.as_str()) {
                view.effect
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            let uniform = Live2dUniform {
                viewport: [
                    view.width.max(1) as f32,
                    view.height.max(1) as f32,
                    0.0,
                    0.0,
                ],
                view_transform: view.transform,
                canvas: live2d_canvas_uniform(&snapshot.canvas),
                effect,
            };
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
            pass.set_bind_group(1, texture, &[]);
            pass.set_vertex_buffer(0, drawable.vertex_buffer.slice(..));
            pass.set_index_buffer(drawable.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..drawable.index_count, 0, 0..1);
        }
    }

    fn ensure_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
    ) {
        if self.scene_key.as_deref() == Some(snapshot.model_key.as_str()) {
            return;
        }
        self.scene_key = Some(snapshot.model_key.clone());
        let textures = snapshot
            .textures
            .iter()
            .map(|texture| self.create_texture_bind_group(device, queue, texture))
            .collect::<Vec<_>>();
        let drawables = snapshot
            .drawables
            .iter()
            .filter(|drawable| !drawable.vertices.is_empty() && !drawable.indices.is_empty())
            .map(|drawable| {
                let vertices = drawable
                    .vertices
                    .iter()
                    .map(|vertex| GpuVertex {
                        position: vertex.position,
                        uv: vertex.uv,
                    })
                    .collect::<Vec<_>>();
                let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Live2D Drawable Vertices"),
                    size: (vertices.len() * std::mem::size_of::<GpuVertex>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));
                let index_bytes = padded_index_bytes(&drawable.indices);
                let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Live2D Drawable Indices"),
                    size: index_bytes.len() as u64,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&index_buffer, 0, &index_bytes);
                GpuDrawable {
                    id: drawable.id.as_ref().to_owned(),
                    vertex_buffer,
                    index_buffer,
                    index_count: drawable.indices.len() as u32,
                    texture_index: drawable.texture_index,
                }
            })
            .collect();
        self.gpu_scene = Some(GpuScene {
            drawables,
            textures,
        });
    }

    fn create_texture_bind_group(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &TextureAsset,
    ) -> wgpu::BindGroup {
        let size = wgpu::Extent3d {
            width: texture.width.max(1),
            height: texture.height.max(1),
            depth_or_array_layers: 1,
        };
        let gpu_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Live2D Texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texture.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * texture.width.max(1)),
                rows_per_image: Some(texture.height.max(1)),
            },
            size,
        );
        let view = gpu_texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Live2D Texture Bind Group"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

fn live2d_canvas_uniform(canvas: &CanvasInfo) -> [f32; 4] {
    let pixels_per_unit = canvas.pixels_per_unit.max(0.0001);
    [
        (canvas.size[0] / pixels_per_unit).max(0.0001),
        (canvas.size[1] / pixels_per_unit).max(0.0001),
        canvas.origin[0] / pixels_per_unit,
        canvas.origin[1] / pixels_per_unit,
    ]
}

fn padded_index_bytes(indices: &[u16]) -> Vec<u8> {
    let bytes = bytemuck::cast_slice(indices);
    let aligned_len = bytes
        .len()
        .next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT as usize);
    let mut padded = Vec::with_capacity(aligned_len);
    padded.extend_from_slice(bytes);
    padded.resize(aligned_len, 0);
    padded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_odd_index_upload_bytes_without_changing_draw_count() {
        let indices = [0_u16, 1, 2];
        let bytes = padded_index_bytes(&indices);
        let raw_bytes: &[u8] = bytemuck::cast_slice(&indices);

        assert_eq!(bytes.len() % wgpu::COPY_BUFFER_ALIGNMENT as usize, 0);
        assert_eq!(
            &bytes[..indices.len() * std::mem::size_of::<u16>()],
            raw_bytes
        );
        assert_eq!(indices.len(), 3);
    }

    #[test]
    fn preview_uniform_derives_live2d_effect() {
        let mut uniform = WgpuPreviewUniform::neutral(0.0, 800, 600);
        uniform.tint_a = [0.25, 0.5, 1.0, 1.0];
        uniform.params0 = [0.5, 1.2, 0.0, 0.0];
        uniform.params3[1] = 0.75;

        let effect = uniform.live2d_effect();

        assert_eq!(effect, [0.65, 0.8, 1.1, 0.75]);
    }
}
