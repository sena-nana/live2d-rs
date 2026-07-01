use crate::{
    api::WgpuLive2DTarget,
    upload::{aligned_uniform_stride_for, uniform_binding},
    POST_PROCESS_CLEAR,
};
use bytemuck::{Pod, Zeroable};
#[cfg(feature = "probe")]
use live2d_probe::{measure, ProbeAttr, ProbeSink, Stage};
use live2d_render::{
    PostProcessParams, PostProcessPlan, PostProcessShaderId, POST_PROCESS_PARAM_VEC4S,
};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub enum WgpuPostProcessShaderSource<'a> {
    Wgsl(&'a str),
}

#[derive(Debug, Clone)]
pub struct WgpuPostProcessPlan {
    pub(crate) passes: Vec<WgpuPostProcessPassPlan>,
}

impl WgpuPostProcessPlan {
    pub fn from_render_plan<'a, I>(
        plan: &PostProcessPlan,
        shader_registry: I,
    ) -> Result<Self, WgpuPostProcessError>
    where
        I: IntoIterator<Item = (PostProcessShaderId, WgpuPostProcessShaderSource<'a>)>,
    {
        let shaders = shader_registry
            .into_iter()
            .map(|(id, source)| {
                let source = match source {
                    WgpuPostProcessShaderSource::Wgsl(source) => source,
                };
                (id, source)
            })
            .collect::<HashMap<_, _>>();
        let mut passes = Vec::with_capacity(plan.passes().len());
        for pass in plan.passes() {
            let source = shaders.get(&pass.shader_id).ok_or_else(|| {
                WgpuPostProcessError::MissingShader {
                    shader_id: pass.shader_id.clone(),
                }
            })?;
            if source.trim().is_empty() {
                return Err(WgpuPostProcessError::EmptyShaderSource {
                    shader_id: pass.shader_id.clone(),
                });
            }
            passes.push(WgpuPostProcessPassPlan {
                shader_id: pass.shader_id.clone(),
                wgsl: (*source).to_owned(),
                params: pass.params,
            });
        }
        Ok(Self { passes })
    }

    pub fn passes(&self) -> &[WgpuPostProcessPassPlan] {
        &self.passes
    }

    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct WgpuPostProcessPassPlan {
    pub shader_id: PostProcessShaderId,
    pub params: PostProcessParams,
    pub(crate) wgsl: String,
}

impl WgpuPostProcessPassPlan {
    pub fn wgsl(&self) -> &str {
        &self.wgsl
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WgpuPostProcessError {
    MissingShader { shader_id: PostProcessShaderId },
    EmptyShaderSource { shader_id: PostProcessShaderId },
    UnsupportedResolveTarget,
}

pub struct WgpuPostProcessChain {
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) layout: wgpu::BindGroupLayout,
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) uniform_buffer: wgpu::Buffer,
    pub(crate) uniform_stride: u64,
    pub(crate) uniform_capacity: usize,
    pub(crate) passes: Vec<WgpuPostProcessPipeline>,
    pub(crate) ping_pong: Vec<WgpuPostProcessTexture>,
}

pub(crate) struct WgpuPostProcessPipeline {
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) params: PostProcessParams,
}

pub(crate) struct WgpuPostProcessTexture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) _texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct WgpuPostProcessUniform {
    pub(crate) viewport: [f32; 4],
    pub(crate) params: [[f32; 4]; POST_PROCESS_PARAM_VEC4S],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WgpuPostProcessStep {
    pub(crate) source: WgpuPostProcessSource,
    pub(crate) target: WgpuPostProcessDestination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WgpuPostProcessSource {
    Scene,
    PingPong(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WgpuPostProcessDestination {
    PingPong(usize),
    Final,
}
impl WgpuPostProcessChain {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        plan: &WgpuPostProcessPlan,
    ) -> Self {
        let uniform_stride = aligned_uniform_stride_for::<WgpuPostProcessUniform>(device);
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D PostProcess Uniform"),
            size: uniform_stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Live2D PostProcess Bind Group Layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Live2D PostProcess Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Live2D PostProcess Pipeline Layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let passes = plan
            .passes()
            .iter()
            .map(|pass| WgpuPostProcessPipeline {
                pipeline: create_post_process_pipeline(device, &pipeline_layout, format, pass),
                params: pass.params,
            })
            .collect();

        Self {
            format,
            layout,
            sampler,
            uniform_buffer,
            uniform_stride,
            uniform_capacity: 1,
            passes,
            ping_pong: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input_view: &wgpu::TextureView,
        target: WgpuLive2DTarget<'_>,
        width: u32,
        height: u32,
    ) -> Result<(), WgpuPostProcessError> {
        self.encode_inner(device, queue, encoder, input_view, target, width, height)
    }

    #[cfg(feature = "probe")]
    pub fn encode_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input_view: &wgpu::TextureView,
        target: WgpuLive2DTarget<'_>,
        width: u32,
        height: u32,
        probe: &P,
    ) -> Result<(), WgpuPostProcessError>
    where
        P: ProbeSink,
    {
        measure(
            probe,
            Stage::WgpuPostProcessPassEncode,
            vec![ProbeAttr::new("passes", self.passes.len())],
            || self.encode_inner(device, queue, encoder, input_view, target, width, height),
        )
    }

    fn encode_inner(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input_view: &wgpu::TextureView,
        target: WgpuLive2DTarget<'_>,
        width: u32,
        height: u32,
    ) -> Result<(), WgpuPostProcessError> {
        if target.resolve_target.is_some() {
            return Err(WgpuPostProcessError::UnsupportedResolveTarget);
        }
        if self.passes.is_empty() {
            return Ok(());
        }

        let width = width.max(1);
        let height = height.max(1);
        self.ensure_uniform_capacity(device, self.passes.len());
        self.ensure_ping_pong_targets(device, width, height);
        upload_post_process_uniforms(
            queue,
            &self.uniform_buffer,
            self.uniform_stride,
            width,
            height,
            &self.passes,
        );

        let steps = post_process_steps(self.passes.len());
        for (index, step) in steps.iter().enumerate() {
            let source_view = match step.source {
                WgpuPostProcessSource::Scene => input_view,
                WgpuPostProcessSource::PingPong(slot) => &self.ping_pong[slot].view,
            };
            let target_view = match step.target {
                WgpuPostProcessDestination::PingPong(slot) => &self.ping_pong[slot].view,
                WgpuPostProcessDestination::Final => target.view,
            };
            let bind_group = create_post_process_bind_group(
                device,
                &self.layout,
                &self.sampler,
                source_view,
                &self.uniform_buffer,
                self.uniform_stride,
            );
            let ops = match step.target {
                WgpuPostProcessDestination::PingPong(_) => wgpu::Operations {
                    load: wgpu::LoadOp::Clear(POST_PROCESS_CLEAR),
                    store: wgpu::StoreOp::Store,
                },
                WgpuPostProcessDestination::Final => wgpu::Operations {
                    load: target.load_op,
                    store: target.store_op,
                },
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Live2D PostProcess Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let pipeline = &self.passes[index];
            pass.set_pipeline(&pipeline.pipeline);
            let uniform_offset = self.uniform_stride * index as u64;
            pass.set_bind_group(0, &bind_group, &[uniform_offset as u32]);
            pass.draw(0..3, 0..1);
        }
        Ok(())
    }

    fn ensure_uniform_capacity(&mut self, device: &wgpu::Device, needed: usize) {
        let needed = needed.max(1);
        if self.uniform_capacity >= needed {
            return;
        }
        self.uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D PostProcess Uniform"),
            size: self.uniform_stride * needed as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.uniform_capacity = needed;
    }

    fn ensure_ping_pong_targets(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let needed = self.passes.len().saturating_sub(1).min(2);
        if self.ping_pong.len() != needed {
            self.ping_pong = (0..needed)
                .map(|slot| create_post_process_texture(device, self.format, width, height, slot))
                .collect();
            return;
        }
        for (slot, texture) in self.ping_pong.iter_mut().enumerate() {
            if texture.width != width || texture.height != height || texture.format != self.format {
                *texture = create_post_process_texture(device, self.format, width, height, slot);
            }
        }
    }
}
pub(crate) fn wrapped_post_process_wgsl(user_source: &str) -> String {
    format!(
        r#"
pub(crate) struct PpUniform {{
    pub(crate) viewport: vec4<f32>,
    pub(crate) params: array<vec4<f32>, {POST_PROCESS_PARAM_VEC4S}>,
}};

@group(0) @binding(0) var pp_input: texture_2d<f32>;
@group(0) @binding(1) var pp_sampler: sampler;
@group(0) @binding(2) var<uniform> pp: PpUniform;

pub(crate) struct PpVertexOut {{
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}};

pub(crate) struct PpFragment {{
    pub(crate) uv: vec2<f32>,
    pub(crate) pixel: vec2<f32>,
    pub(crate) texel: vec2<f32>,
}};

@vertex
pub(crate) fn pp_vs(@builtin(vertex_index) vertex_index: u32) -> PpVertexOut {{
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let pos = positions[vertex_index];
    var out: PpVertexOut;
    out.pos = vec4<f32>(pos, 0.0, 1.0);
    out.uv = pos * 0.5 + vec2<f32>(0.5, 0.5);
    return out;
}}

pub(crate) fn pp_sample(uv: vec2<f32>) -> vec4<f32> {{
    return textureSample(pp_input, pp_sampler, uv);
}}

pub(crate) fn pp_param(index: u32) -> vec4<f32> {{
    return pp.params[index];
}}

{user_source}

@fragment
pub(crate) fn pp_fs(input: PpVertexOut) -> @location(0) vec4<f32> {{
    let fragment = PpFragment(
        input.uv,
        input.uv * pp.viewport.xy,
        pp.viewport.zw,
    );
    return pp_apply(fragment);
}}
"#
    )
}

pub(crate) fn create_post_process_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    pass: &WgpuPostProcessPassPlan,
) -> wgpu::RenderPipeline {
    let source = wrapped_post_process_wgsl(pass.wgsl());
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Live2D PostProcess Shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let label = format!("Live2D PostProcess Pipeline {}", pass.shader_id.as_ref());
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("pp_vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
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
            entry_point: Some("pp_fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

pub(crate) fn post_process_steps(pass_count: usize) -> Vec<WgpuPostProcessStep> {
    (0..pass_count)
        .map(|index| WgpuPostProcessStep {
            source: if index == 0 {
                WgpuPostProcessSource::Scene
            } else {
                WgpuPostProcessSource::PingPong((index - 1) % 2)
            },
            target: if index + 1 == pass_count {
                WgpuPostProcessDestination::Final
            } else {
                WgpuPostProcessDestination::PingPong(index % 2)
            },
        })
        .collect()
}

pub(crate) fn upload_post_process_uniforms(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    uniform_stride: u64,
    width: u32,
    height: u32,
    passes: &[WgpuPostProcessPipeline],
) {
    let uniform_stride = uniform_stride as usize;
    let uniform_size = std::mem::size_of::<WgpuPostProcessUniform>();
    debug_assert!(uniform_stride >= uniform_size);
    let mut bytes = vec![0; uniform_stride * passes.len().max(1)];
    for (index, pass) in passes.iter().enumerate() {
        let uniform = WgpuPostProcessUniform {
            viewport: [
                width as f32,
                height as f32,
                1.0 / width.max(1) as f32,
                1.0 / height.max(1) as f32,
            ],
            params: pass.params.values,
        };
        let offset = index * uniform_stride;
        bytes[offset..offset + uniform_size].copy_from_slice(bytemuck::bytes_of(&uniform));
    }
    queue.write_buffer(buffer, 0, &bytes);
}

pub(crate) fn create_post_process_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
    uniform_buffer: &wgpu::Buffer,
    uniform_stride: u64,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Live2D PostProcess Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform_binding(uniform_buffer, uniform_stride),
            },
        ],
    })
}

pub(crate) fn create_post_process_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    slot: usize,
) -> WgpuPostProcessTexture {
    let label = format!("Live2D PostProcess PingPong {slot}");
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    WgpuPostProcessTexture {
        width,
        height,
        format,
        _texture: texture,
        view,
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
    fn post_process_steps_skip_work_for_empty_chain() {
        assert!(post_process_steps(0).is_empty());
    }

    #[test]
    fn post_process_steps_write_single_pass_to_final_target() {
        assert_eq!(
            post_process_steps(1),
            vec![WgpuPostProcessStep {
                source: WgpuPostProcessSource::Scene,
                target: WgpuPostProcessDestination::Final,
            }]
        );
    }

    #[test]
    fn post_process_steps_use_ping_pong_for_multi_pass_chain() {
        assert_eq!(
            post_process_steps(4),
            vec![
                WgpuPostProcessStep {
                    source: WgpuPostProcessSource::Scene,
                    target: WgpuPostProcessDestination::PingPong(0),
                },
                WgpuPostProcessStep {
                    source: WgpuPostProcessSource::PingPong(0),
                    target: WgpuPostProcessDestination::PingPong(1),
                },
                WgpuPostProcessStep {
                    source: WgpuPostProcessSource::PingPong(1),
                    target: WgpuPostProcessDestination::PingPong(0),
                },
                WgpuPostProcessStep {
                    source: WgpuPostProcessSource::PingPong(0),
                    target: WgpuPostProcessDestination::Final,
                },
            ]
        );
    }

    #[test]
    fn wgpu_post_process_plan_lowers_shader_ids_to_sources() {
        let plan = PostProcessPlan::linear(["tone", "composite"]);
        let wgpu_plan = WgpuPostProcessPlan::from_render_plan(
            &plan,
            [
                (
                    PostProcessShaderId::from("tone"),
                    WgpuPostProcessShaderSource::Wgsl(
                        "fn pp_apply(f: PpFragment) -> vec4<f32> { return pp_sample(f.uv); }",
                    ),
                ),
                (
                    PostProcessShaderId::from("composite"),
                    WgpuPostProcessShaderSource::Wgsl(
                        "fn pp_apply(f: PpFragment) -> vec4<f32> { return pp_sample(f.uv); }",
                    ),
                ),
            ],
        )
        .unwrap();

        assert_eq!(wgpu_plan.passes().len(), 2);
        assert_eq!(
            wgpu_plan.passes()[0].shader_id,
            PostProcessShaderId::from("tone")
        );
        assert!(wgpu_plan.passes()[1].wgsl().contains("pp_apply"));
    }

    #[test]
    fn wgpu_post_process_plan_rejects_missing_shader() {
        let plan = PostProcessPlan::linear(["tone"]);

        assert_eq!(
            WgpuPostProcessPlan::from_render_plan(&plan, []).unwrap_err(),
            WgpuPostProcessError::MissingShader {
                shader_id: PostProcessShaderId::from("tone"),
            }
        );
    }
}
