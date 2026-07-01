use bytemuck::{Pod, Zeroable};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;
    use crate::*;
    use live2d_core::{
        AlphaBlendMode, BlendMode, CanvasInfo, ClippingInfo, ColorBlendMode, DrawableId, MaskRef,
        MaterialKey, TextureAsset, Vertex,
    };
    use live2d_render::RenderPlanner;

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
