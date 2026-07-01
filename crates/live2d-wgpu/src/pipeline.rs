use crate::*;

pub(crate) struct PipelineCache {
    pub(crate) target_format: wgpu::TextureFormat,
    pub(crate) pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PipelineKey {
    pub(crate) target_format: wgpu::TextureFormat,
    pub(crate) blend_mode: PipelineBlendMode,
    pub(crate) masked: bool,
    pub(crate) shader_variant: ShaderVariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PipelineBlendMode {
    Normal,
    Additive,
    Multiplicative,
    Advanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ShaderVariant {
    DefaultMesh,
    AdvancedBlend,
    MaskWriter,
}

impl PipelineCache {
    pub(crate) fn new(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let mut pipelines = HashMap::new();
        for masked in [false, true] {
            for blend_mode in [
                PipelineBlendMode::Normal,
                PipelineBlendMode::Additive,
                PipelineBlendMode::Multiplicative,
                PipelineBlendMode::Advanced,
            ] {
                let key = PipelineKey {
                    target_format,
                    blend_mode,
                    masked,
                    shader_variant: if blend_mode == PipelineBlendMode::Advanced {
                        ShaderVariant::AdvancedBlend
                    } else {
                        ShaderVariant::DefaultMesh
                    },
                };
                pipelines.insert(
                    key,
                    create_live2d_pipeline(
                        device,
                        layout,
                        shader,
                        key,
                        live2d_blend_state(blend_mode),
                    ),
                );
            }
        }
        let mask_key = PipelineKey {
            target_format: MASK_ATLAS_FORMAT,
            blend_mode: PipelineBlendMode::Normal,
            masked: false,
            shader_variant: ShaderVariant::MaskWriter,
        };
        pipelines.insert(
            mask_key,
            create_live2d_pipeline(
                device,
                layout,
                shader,
                mask_key,
                wgpu::BlendState::ALPHA_BLENDING,
            ),
        );

        Self {
            target_format,
            pipelines,
        }
    }

    pub(crate) fn mesh_key(&self, blend_mode: BlendMode, masked: bool) -> PipelineKey {
        let blend_mode = pipeline_blend_mode(blend_mode);
        PipelineKey {
            target_format: self.target_format,
            blend_mode,
            masked,
            shader_variant: if blend_mode == PipelineBlendMode::Advanced {
                ShaderVariant::AdvancedBlend
            } else {
                ShaderVariant::DefaultMesh
            },
        }
    }

    pub(crate) fn pipeline(&self, key: PipelineKey) -> &wgpu::RenderPipeline {
        self.pipelines
            .get(&key)
            .expect("default Live2D mesh pipeline is prebuilt")
    }

    pub(crate) fn mask_writer(&self) -> &wgpu::RenderPipeline {
        let key = PipelineKey {
            target_format: MASK_ATLAS_FORMAT,
            blend_mode: PipelineBlendMode::Normal,
            masked: false,
            shader_variant: ShaderVariant::MaskWriter,
        };
        self.pipelines
            .get(&key)
            .expect("Live2D mask writer pipeline is prebuilt")
    }
}
pub(crate) fn blend_uniform(blend_mode: BlendMode) -> [u32; 4] {
    match blend_mode {
        BlendMode::Advanced { color, alpha } => {
            [color_blend_code(color), alpha_blend_code(alpha), 0, 0]
        }
        BlendMode::Normal | BlendMode::Additive | BlendMode::Multiplicative => [0, 0, 0, 0],
    }
}

pub(crate) fn color_blend_code(mode: ColorBlendMode) -> u32 {
    match mode {
        ColorBlendMode::Normal => 0,
        ColorBlendMode::Add => 3,
        ColorBlendMode::AddGlow => 4,
        ColorBlendMode::Darken => 5,
        ColorBlendMode::Multiply => 6,
        ColorBlendMode::ColorBurn => 7,
        ColorBlendMode::LinearBurn => 8,
        ColorBlendMode::Lighten => 9,
        ColorBlendMode::Screen => 10,
        ColorBlendMode::ColorDodge => 11,
        ColorBlendMode::Overlay => 12,
        ColorBlendMode::SoftLight => 13,
        ColorBlendMode::HardLight => 14,
        ColorBlendMode::LinearLight => 15,
        ColorBlendMode::Hue => 16,
        ColorBlendMode::Color => 17,
    }
}

pub(crate) fn alpha_blend_code(mode: AlphaBlendMode) -> u32 {
    match mode {
        AlphaBlendMode::Over => 0,
        AlphaBlendMode::Atop => 1,
        AlphaBlendMode::Out => 2,
        AlphaBlendMode::ConjointOver => 3,
        AlphaBlendMode::DisjointOver => 4,
    }
}
pub(crate) fn create_live2d_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    key: PipelineKey,
    blend: wgpu::BlendState,
) -> wgpu::RenderPipeline {
    let label = pipeline_label(key);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuPosition>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x2,
                    }],
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuUv>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 1,
                        format: wgpu::VertexFormat::Float32x2,
                    }],
                },
            ],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry_point(key.shader_variant)),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: key.target_format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(crate) fn fragment_entry_point(shader_variant: ShaderVariant) -> &'static str {
    match shader_variant {
        ShaderVariant::DefaultMesh => "fs_main",
        ShaderVariant::AdvancedBlend => "fs_blend",
        ShaderVariant::MaskWriter => "fs_mask",
    }
}

pub(crate) fn pipeline_label(key: PipelineKey) -> String {
    format!(
        "Live2D {:?} {:?}{} Pipeline",
        key.shader_variant,
        key.blend_mode,
        if key.masked { " Masked" } else { "" }
    )
}

pub(crate) fn pipeline_blend_mode(blend_mode: BlendMode) -> PipelineBlendMode {
    match blend_mode {
        BlendMode::Normal => PipelineBlendMode::Normal,
        BlendMode::Additive => PipelineBlendMode::Additive,
        BlendMode::Multiplicative => PipelineBlendMode::Multiplicative,
        BlendMode::Advanced { .. } => PipelineBlendMode::Advanced,
    }
}

pub(crate) fn live2d_blend_state(blend_mode: PipelineBlendMode) -> wgpu::BlendState {
    match blend_mode {
        PipelineBlendMode::Normal => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        },
        PipelineBlendMode::Additive => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
        PipelineBlendMode::Multiplicative => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
        PipelineBlendMode::Advanced => wgpu::BlendState::REPLACE,
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
    fn legacy_blend_states_use_premultiplied_cubism_factors() {
        let normal = live2d_blend_state(PipelineBlendMode::Normal);
        assert_blend_component(
            normal.color,
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::OneMinusSrcAlpha,
        );
        assert_blend_component(
            normal.alpha,
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::OneMinusSrcAlpha,
        );

        let additive = live2d_blend_state(PipelineBlendMode::Additive);
        assert_blend_component(
            additive.color,
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::One,
        );
        assert_blend_component(
            additive.alpha,
            wgpu::BlendFactor::Zero,
            wgpu::BlendFactor::One,
        );

        let multiplicative = live2d_blend_state(PipelineBlendMode::Multiplicative);
        assert_blend_component(
            multiplicative.color,
            wgpu::BlendFactor::Dst,
            wgpu::BlendFactor::OneMinusSrcAlpha,
        );
        assert_blend_component(
            multiplicative.alpha,
            wgpu::BlendFactor::Zero,
            wgpu::BlendFactor::One,
        );
    }
}
