use bytemuck::{Pod, Zeroable};
use live2d_core::{BlendMode, CanvasInfo, Drawable, ModelSnapshot, TextureAsset};
use live2d_render::{
    DrawCommand, Live2DRenderBackend, MaskPass, ModelRenderCtx, RenderPlan, RenderPlanner,
};
use std::collections::{HashMap, HashSet};

const MASK_ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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

pub struct WgpuLive2DTarget<'view> {
    pub view: &'view wgpu::TextureView,
    pub resolve_target: Option<&'view wgpu::TextureView>,
    pub load_op: wgpu::LoadOp<wgpu::Color>,
    pub store_op: wgpu::StoreOp,
}

impl<'view> WgpuLive2DTarget<'view> {
    pub fn load(view: &'view wgpu::TextureView) -> Self {
        Self {
            view,
            resolve_target: None,
            load_op: wgpu::LoadOp::Load,
            store_op: wgpu::StoreOp::Store,
        }
    }

    pub fn clear(view: &'view wgpu::TextureView, color: wgpu::Color) -> Self {
        Self {
            view,
            resolve_target: None,
            load_op: wgpu::LoadOp::Clear(color),
            store_op: wgpu::StoreOp::Store,
        }
    }
}

impl MaskAtlas {
    fn layout(&self) -> MaskAtlasLayout {
        MaskAtlasLayout {
            width: self.width,
            height: self.height,
            slot_width: self.slot_width,
            slot_height: self.slot_height,
            columns: self.columns,
            rows: self.slots.div_ceil(self.columns),
            slots: self.slots,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Live2dUniform {
    viewport: [f32; 4],
    view_transform: [f32; 4],
    canvas: [f32; 4],
    effect: [f32; 4],
    mask: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct GpuPosition {
    position: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct GpuUv {
    uv: [f32; 2],
}

pub struct WgpuLive2DRenderer {
    pipelines: PipelineCache,
    uniform_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    fallback_mask_bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    uniform_stride: u64,
    uniform_capacity: usize,
    scene_key: Option<String>,
    scene_topology: Option<SceneTopology>,
    texture_cache: Option<TextureCache>,
    mask_atlas: Option<MaskAtlas>,
    offscreen_target: Option<OffscreenTarget>,
    gpu_scene: Option<GpuScene>,
}

struct TextureCache {
    model_key: String,
    topology: TextureTopology,
    bind_groups: Vec<wgpu::BindGroup>,
}

struct GpuScene {
    position_buffer: wgpu::Buffer,
    uv_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    positions: Vec<GpuPosition>,
    vertex_count: u32,
    index_count: u32,
    textures: Vec<wgpu::BindGroup>,
}

struct MaskAtlas {
    width: u32,
    height: u32,
    slot_width: u32,
    slot_height: u32,
    columns: usize,
    slots: usize,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

struct OffscreenTarget {
    width: u32,
    height: u32,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaskAtlasLayout {
    width: u32,
    height: u32,
    slot_width: u32,
    slot_height: u32,
    columns: usize,
    rows: usize,
    slots: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PositionUpload {
    vertex_range: std::ops::Range<u32>,
    byte_offset: u64,
}

struct WgpuRenderBackend<'a, 'pass, 'view> {
    queue: &'a wgpu::Queue,
    pass: &'a mut wgpu::RenderPass<'pass>,
    pipelines: &'a PipelineCache,
    uniform_buffer: &'a wgpu::Buffer,
    uniform_bind_group: &'a wgpu::BindGroup,
    uniform_stride: u64,
    uniform_index: usize,
    mask_bind_group: &'a wgpu::BindGroup,
    mask_atlas: Option<&'a MaskAtlas>,
    gpu_scene: &'a GpuScene,
    canvas: [f32; 4],
    view: &'view WgpuLive2DView,
    target_ids: HashSet<&'view str>,
}

struct PipelineCache {
    target_format: wgpu::TextureFormat,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    target_format: wgpu::TextureFormat,
    blend_mode: BlendMode,
    masked: bool,
    shader_variant: ShaderVariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ShaderVariant {
    DefaultMesh,
    MaskWriter,
}

impl PipelineCache {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let mut pipelines = HashMap::new();
        for masked in [false, true] {
            for blend_mode in [
                BlendMode::Normal,
                BlendMode::Additive,
                BlendMode::Multiplicative,
            ] {
                let key = PipelineKey {
                    target_format,
                    blend_mode,
                    masked,
                    shader_variant: ShaderVariant::DefaultMesh,
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
            blend_mode: BlendMode::Normal,
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

    fn mesh(&self, blend_mode: BlendMode, masked: bool) -> &wgpu::RenderPipeline {
        let key = PipelineKey {
            target_format: self.target_format,
            blend_mode,
            masked,
            shader_variant: ShaderVariant::DefaultMesh,
        };
        self.pipelines
            .get(&key)
            .expect("default Live2D mesh pipeline is prebuilt")
    }

    fn mask_writer(&self) -> &wgpu::RenderPipeline {
        let key = PipelineKey {
            target_format: MASK_ATLAS_FORMAT,
            blend_mode: BlendMode::Normal,
            masked: false,
            shader_variant: ShaderVariant::MaskWriter,
        };
        self.pipelines
            .get(&key)
            .expect("Live2D mask writer pipeline is prebuilt")
    }
}

impl<'a, 'pass, 'view> Live2DRenderBackend for WgpuRenderBackend<'a, 'pass, 'view> {
    fn begin_model(&mut self, _ctx: &ModelRenderCtx) {
        self.pass.push_debug_group("live2d model");
        self.pass.set_bind_group(2, self.mask_bind_group, &[]);
        self.pass
            .set_vertex_buffer(0, self.gpu_scene.position_buffer.slice(..));
        self.pass
            .set_vertex_buffer(1, self.gpu_scene.uv_buffer.slice(..));
        self.pass.set_index_buffer(
            self.gpu_scene.index_buffer.slice(..),
            wgpu::IndexFormat::Uint16,
        );
    }

    fn begin_clip_masks(&mut self, _masks: &[MaskPass]) {
        self.pass.push_debug_group("live2d masks");
    }

    fn begin_clip_mask(&mut self, mask: &MaskPass) {
        self.pass.push_debug_group(&format!("mask {}", mask.id.0));
    }

    fn draw_mask_drawable(&mut self, mask: &MaskPass, call: &DrawCommand) {
        self.pass.insert_debug_marker(&format!(
            "mask {} drawable {}",
            mask.id.0,
            call.drawable_id.as_ref()
        ));
    }

    fn end_clip_mask(&mut self, _mask: &MaskPass) {
        self.pass.pop_debug_group();
    }

    fn end_clip_masks(&mut self) {
        self.pass.pop_debug_group();
    }

    fn begin_main_pass(&mut self) {
        self.pass.push_debug_group("live2d main pass");
    }

    fn draw_drawable(&mut self, draw: &DrawCommand) {
        let Some(texture) = self.gpu_scene.textures.get(draw.texture_index) else {
            return;
        };
        if draw.vertex_range.end > self.gpu_scene.vertex_count
            || draw.index_range.end > self.gpu_scene.index_count
        {
            return;
        }
        let Ok(base_vertex) = i32::try_from(draw.vertex_range.start) else {
            return;
        };
        let effect =
            if self.target_ids.is_empty() || self.target_ids.contains(draw.drawable_id.as_ref()) {
                [
                    self.view.effect[0],
                    self.view.effect[1],
                    self.view.effect[2],
                    self.view.effect[3] * draw.opacity.clamp(0.0, 1.0),
                ]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
        let mask = mask_uniform(draw, self.mask_atlas);
        let masked = mask[3] != 0.0;
        self.pass
            .set_pipeline(self.pipelines.mesh(draw.blend_mode, masked));
        let uniform = Live2dUniform {
            viewport: [
                self.view.width.max(1) as f32,
                self.view.height.max(1) as f32,
                0.0,
                0.0,
            ],
            view_transform: self.view.transform,
            canvas: self.canvas,
            effect,
            mask,
        };
        let uniform_offset = self.uniform_stride * self.uniform_index as u64;
        self.uniform_index += 1;
        self.queue.write_buffer(
            self.uniform_buffer,
            uniform_offset,
            bytemuck::bytes_of(&uniform),
        );
        self.pass
            .set_bind_group(0, self.uniform_bind_group, &[uniform_offset as u32]);
        self.pass.set_bind_group(1, texture, &[]);
        self.pass.insert_debug_marker(draw.drawable_id.as_ref());
        self.pass
            .draw_indexed(draw.index_range.clone(), base_vertex, 0..1);
    }

    fn end_model(&mut self) {
        self.pass.pop_debug_group();
        self.pass.pop_debug_group();
    }
}

type SceneTopology = (usize, Vec<(String, usize, usize, usize)>);
type TextureTopology = Vec<(u32, u32, usize)>;

impl WgpuLive2DRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Live2D Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("live2d.wgsl").into()),
        });
        let uniform_stride = aligned_uniform_stride(device);
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Uniform"),
            size: uniform_stride,
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
                    has_dynamic_offset: true,
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
            bind_group_layouts: &[
                Some(&bind_group_layout),
                Some(&texture_layout),
                Some(&texture_layout),
            ],
            immediate_size: 0,
        });
        let pipelines = PipelineCache::new(device, &pipeline_layout, &shader, format);
        let fallback_mask_bind_group =
            create_empty_mask_bind_group(device, &texture_layout, &sampler);

        Self {
            pipelines,
            uniform_layout: bind_group_layout,
            texture_layout,
            sampler,
            fallback_mask_bind_group,
            uniform_buffer,
            uniform_bind_group,
            uniform_stride,
            uniform_capacity: 1,
            scene_key: None,
            scene_topology: None,
            texture_cache: None,
            mask_atlas: None,
            offscreen_target: None,
            gpu_scene: None,
        }
    }

    pub fn prepare_model(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
    ) {
        let _ = self.prepare_render(device, queue, snapshot);
    }

    pub fn prepare_render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
    ) -> RenderPlan {
        let render_plan = self.prepare_scene(device, queue, snapshot);
        self.ensure_uniform_capacity(device, uniform_slots(&render_plan));
        render_plan
    }

    pub fn render<'pass>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'pass>,
        snapshot: &ModelSnapshot,
        view: WgpuLive2DView,
    ) {
        let render_plan = self.prepare_render(device, queue, snapshot);
        self.encode_render(queue, pass, &render_plan, &snapshot.canvas, view);
    }

    pub fn render_to_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: WgpuLive2DTarget<'_>,
        snapshot: &ModelSnapshot,
        view: WgpuLive2DView,
    ) {
        let render_plan = self.prepare_render(device, queue, snapshot);
        self.encode_render_to_view(
            device,
            queue,
            encoder,
            target,
            &render_plan,
            &snapshot.canvas,
            view,
        );
    }

    pub fn encode_render_to_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: WgpuLive2DTarget<'_>,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: WgpuLive2DView,
    ) {
        self.ensure_uniform_capacity(device, uniform_slots(render_plan));
        if !render_plan.masks.is_empty() {
            self.prepare_mask_atlas(device, queue, encoder, render_plan, canvas, &view);
        }
        let first_main_uniform_slot = mask_uniform_slots(render_plan);
        let mask_atlas = self.mask_atlas.as_ref();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Live2D Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: target.resolve_target,
                ops: wgpu::Operations {
                    load: target.load_op,
                    store: target.store_op,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.encode_render_from_uniform_slot(
            queue,
            &mut pass,
            render_plan,
            canvas,
            view,
            first_main_uniform_slot,
            mask_atlas,
        );
    }

    pub fn encode_render<'pass>(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'pass>,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: WgpuLive2DView,
    ) {
        self.encode_render_from_uniform_slot(queue, pass, render_plan, canvas, view, 0, None);
    }

    pub fn render_to_offscreen(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        snapshot: &ModelSnapshot,
        view: WgpuLive2DView,
        clear_color: wgpu::Color,
    ) -> &wgpu::TextureView {
        self.ensure_offscreen_target(device, view.width, view.height);
        let offscreen_view = self
            .offscreen_target
            .as_ref()
            .expect("offscreen target is created before rendering")
            .view
            .clone();
        self.render_to_view(
            device,
            queue,
            encoder,
            WgpuLive2DTarget::clear(&offscreen_view, clear_color),
            snapshot,
            view,
        );
        &self
            .offscreen_target
            .as_ref()
            .expect("offscreen target is retained after rendering")
            .view
    }

    fn encode_render_from_uniform_slot<'pass>(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'pass>,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: WgpuLive2DView,
        first_uniform_slot: usize,
        mask_atlas: Option<&MaskAtlas>,
    ) {
        let Some(gpu_scene) = &self.gpu_scene else {
            return;
        };
        let active_mask_atlas = if render_plan.masks.is_empty() {
            None
        } else {
            mask_atlas
        };
        let mask_bind_group = active_mask_atlas
            .map(|atlas| &atlas.bind_group)
            .unwrap_or(&self.fallback_mask_bind_group);
        let target_ids = view
            .target_drawable_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut backend = WgpuRenderBackend {
            queue,
            pass,
            pipelines: &self.pipelines,
            uniform_buffer: &self.uniform_buffer,
            uniform_bind_group: &self.uniform_bind_group,
            uniform_stride: self.uniform_stride,
            uniform_index: first_uniform_slot,
            mask_bind_group,
            mask_atlas: active_mask_atlas,
            gpu_scene,
            canvas: live2d_canvas_uniform(canvas),
            view: &view,
            target_ids,
        };
        render_plan.dispatch(&mut backend);
    }

    fn prepare_mask_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: &WgpuLive2DView,
    ) {
        let Some(gpu_scene) = &self.gpu_scene else {
            return;
        };
        let slots = render_plan.masks.len();
        if slots == 0 {
            self.mask_atlas = None;
            return;
        }
        let layout = mask_atlas_layout(
            view.width,
            view.height,
            slots,
            device.limits().max_texture_dimension_2d,
        );
        let rebuild = self
            .mask_atlas
            .as_ref()
            .map(|atlas| {
                atlas.width != layout.width
                    || atlas.height != layout.height
                    || atlas.slot_width != layout.slot_width
                    || atlas.slot_height != layout.slot_height
                    || atlas.columns != layout.columns
                    || atlas.slots != layout.slots
            })
            .unwrap_or(true);
        if rebuild {
            self.mask_atlas = Some(create_mask_atlas(
                device,
                &self.texture_layout,
                &self.sampler,
                layout,
            ));
        }
        let Some(mask_atlas) = &self.mask_atlas else {
            return;
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Live2D Mask Atlas Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &mask_atlas.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.push_debug_group("live2d mask atlas");
        pass.set_pipeline(self.pipelines.mask_writer());
        pass.set_vertex_buffer(0, gpu_scene.position_buffer.slice(..));
        pass.set_vertex_buffer(1, gpu_scene.uv_buffer.slice(..));
        pass.set_index_buffer(gpu_scene.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

        let mut uniform_slot = 0;
        for (slot, mask) in render_plan.masks.iter().enumerate() {
            let slot_x = (slot % mask_atlas.columns) as f32 * mask_atlas.slot_width as f32;
            let slot_y = (slot / mask_atlas.columns) as f32 * mask_atlas.slot_height as f32;
            pass.push_debug_group(&format!("mask {}", mask.id.0));
            pass.set_viewport(
                slot_x,
                slot_y,
                mask_atlas.slot_width as f32,
                mask_atlas.slot_height as f32,
                0.0,
                1.0,
            );
            for drawable_id in &mask.drawable_ids {
                let Some(draw) = draw_command_for_id(render_plan, drawable_id.as_ref()) else {
                    continue;
                };
                let Some(texture) = gpu_scene.textures.get(draw.texture_index) else {
                    continue;
                };
                let Ok(base_vertex) = i32::try_from(draw.vertex_range.start) else {
                    continue;
                };
                let uniform = Live2dUniform {
                    viewport: [
                        mask_atlas.slot_width as f32,
                        mask_atlas.slot_height as f32,
                        0.0,
                        0.0,
                    ],
                    view_transform: view.transform,
                    canvas: live2d_canvas_uniform(canvas),
                    effect: [1.0, 1.0, 1.0, 1.0],
                    mask: [0.0, 0.0, 0.0, 0.0],
                };
                let uniform_offset = self.uniform_stride * uniform_slot as u64;
                uniform_slot += 1;
                queue.write_buffer(
                    &self.uniform_buffer,
                    uniform_offset,
                    bytemuck::bytes_of(&uniform),
                );
                pass.set_bind_group(0, &self.uniform_bind_group, &[uniform_offset as u32]);
                pass.set_bind_group(1, texture, &[]);
                pass.insert_debug_marker(draw.drawable_id.as_ref());
                pass.draw_indexed(draw.index_range.clone(), base_vertex, 0..1);
            }
            pass.pop_debug_group();
        }
        pass.pop_debug_group();
    }

    fn ensure_offscreen_target(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        let rebuild = self
            .offscreen_target
            .as_ref()
            .map(|target| target.width != width || target.height != height)
            .unwrap_or(true);
        if rebuild {
            self.offscreen_target = Some(create_offscreen_target(
                device,
                self.pipelines.target_format,
                width,
                height,
            ));
        }
    }

    fn prepare_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
    ) -> RenderPlan {
        let render_plan = RenderPlanner::new().build(snapshot);
        let topology = scene_topology(snapshot);
        if self.scene_key.as_deref() == Some(snapshot.model_key.as_str())
            && self.scene_topology.as_ref() == Some(&topology)
        {
            self.upload_scene_positions(queue, snapshot, &render_plan);
            return render_plan;
        }
        self.scene_key = Some(snapshot.model_key.clone());
        self.scene_topology = Some(topology);
        let textures = self.prepare_textures(device, queue, snapshot);
        let positions = gpu_scene_positions(snapshot, &render_plan);
        let uvs = gpu_scene_uvs(snapshot, &render_plan);
        let indices = gpu_scene_indices(snapshot, &render_plan);
        let position_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Model Positions"),
            size: buffer_size::<GpuPosition>(positions.len()),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&position_buffer, 0, bytemuck::cast_slice(&positions));
        let uv_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Model UVs"),
            size: buffer_size::<GpuUv>(uvs.len()),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uv_buffer, 0, bytemuck::cast_slice(&uvs));
        let index_bytes = padded_index_bytes(&indices);
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Model Indices"),
            size: index_bytes.len().max(1) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buffer, 0, &index_bytes);
        self.gpu_scene = Some(GpuScene {
            position_buffer,
            uv_buffer,
            index_buffer,
            positions,
            vertex_count: render_plan.model.vertex_count,
            index_count: render_plan.model.index_count,
            textures,
        });
        render_plan
    }

    fn prepare_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
    ) -> Vec<wgpu::BindGroup> {
        let topology = texture_topology(snapshot);
        if let Some(cache) = &self.texture_cache {
            if cache.model_key == snapshot.model_key && cache.topology == topology {
                return cache.bind_groups.clone();
            }
        }

        let bind_groups = snapshot
            .textures
            .iter()
            .map(|texture| self.create_texture_bind_group(device, queue, texture))
            .collect::<Vec<_>>();
        self.texture_cache = Some(TextureCache {
            model_key: snapshot.model_key.clone(),
            topology,
            bind_groups: bind_groups.clone(),
        });
        bind_groups
    }

    fn ensure_uniform_capacity(&mut self, device: &wgpu::Device, required_slots: usize) {
        let required_slots = required_slots.max(1);
        if self.uniform_capacity >= required_slots {
            return;
        }

        let new_capacity = required_slots.next_power_of_two();
        self.uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Uniform"),
            size: self.uniform_stride * new_capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Live2D Uniform Bind Group"),
            layout: &self.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniform_buffer.as_entire_binding(),
            }],
        });
        self.uniform_capacity = new_capacity;
    }

    fn upload_scene_positions(
        &mut self,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
        render_plan: &RenderPlan,
    ) {
        let Some(gpu_scene) = &mut self.gpu_scene else {
            return;
        };
        if gpu_scene.vertex_count != render_plan.model.vertex_count {
            return;
        }
        let positions = gpu_scene_positions(snapshot, render_plan);
        for upload in position_uploads(&gpu_scene.positions, &positions, render_plan) {
            let range = upload.vertex_range.start as usize..upload.vertex_range.end as usize;
            queue.write_buffer(
                &gpu_scene.position_buffer,
                upload.byte_offset,
                bytemuck::cast_slice(&positions[range]),
            );
        }
        gpu_scene.positions = positions;
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

fn aligned_uniform_stride(device: &wgpu::Device) -> u64 {
    align_to(
        std::mem::size_of::<Live2dUniform>() as u64,
        device.limits().min_uniform_buffer_offset_alignment.max(1) as u64,
    )
}

fn align_to(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

fn uniform_slots(render_plan: &RenderPlan) -> usize {
    render_plan.draws.len() + mask_uniform_slots(render_plan)
}

fn mask_uniform_slots(render_plan: &RenderPlan) -> usize {
    render_plan
        .masks
        .iter()
        .map(|mask| mask.drawable_ids.len())
        .sum()
}

fn create_empty_mask_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Live2D Empty Mask Texture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: MASK_ATLAS_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    create_mask_bind_group(device, layout, sampler, &view)
}

fn create_mask_atlas(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    atlas_layout: MaskAtlasLayout,
) -> MaskAtlas {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Live2D Mask Atlas Texture"),
        size: wgpu::Extent3d {
            width: atlas_layout.width,
            height: atlas_layout.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: MASK_ATLAS_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = create_mask_bind_group(device, layout, sampler, &view);
    MaskAtlas {
        width: atlas_layout.width,
        height: atlas_layout.height,
        slot_width: atlas_layout.slot_width,
        slot_height: atlas_layout.slot_height,
        columns: atlas_layout.columns,
        slots: atlas_layout.slots,
        view,
        bind_group,
    }
}

fn create_offscreen_target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> OffscreenTarget {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Live2D Offscreen Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    OffscreenTarget {
        width,
        height,
        _texture: texture,
        view,
    }
}

fn create_mask_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Live2D Mask Bind Group"),
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
        ],
    })
}

fn mask_atlas_layout(
    view_width: u32,
    view_height: u32,
    slots: usize,
    max_texture_dimension: u32,
) -> MaskAtlasLayout {
    let slots = slots.max(1);
    let view_width = view_width.max(1);
    let view_height = view_height.max(1);
    let max_dimension = max_texture_dimension.max(1) as u64;
    let mut best = None;

    for columns in 1..=slots {
        let rows = slots.div_ceil(columns);
        let width_scale = max_dimension as f64 / (columns as f64 * view_width as f64);
        let height_scale = max_dimension as f64 / (rows as f64 * view_height as f64);
        let scale = width_scale.min(height_scale).min(1.0);
        if scale <= 0.0 {
            continue;
        }
        let slot_width = ((view_width as f64 * scale).floor() as u32).max(1);
        let slot_height = ((view_height as f64 * scale).floor() as u32).max(1);
        let width = slot_width as u64 * columns as u64;
        let height = slot_height as u64 * rows as u64;
        if width > max_dimension || height > max_dimension {
            continue;
        }
        let candidate = MaskAtlasLayout {
            width: width as u32,
            height: height as u32,
            slot_width,
            slot_height,
            columns,
            rows,
            slots,
        };
        best = match best {
            Some(current) if mask_layout_score(candidate) <= mask_layout_score(current) => {
                Some(current)
            }
            _ => Some(candidate),
        };
    }

    best.unwrap_or(MaskAtlasLayout {
        width: 1,
        height: slots.min(max_texture_dimension.max(1) as usize) as u32,
        slot_width: 1,
        slot_height: 1,
        columns: 1,
        rows: slots,
        slots,
    })
}

fn mask_layout_score(layout: MaskAtlasLayout) -> (u64, u32, std::cmp::Reverse<usize>) {
    (
        layout.slot_width as u64 * layout.slot_height as u64,
        layout.slot_width.min(layout.slot_height),
        std::cmp::Reverse(layout.rows),
    )
}

fn mask_uniform(draw: &DrawCommand, mask_atlas: Option<&MaskAtlas>) -> [f32; 4] {
    mask_uniform_for_layout(draw, mask_atlas.map(MaskAtlas::layout))
}

fn mask_uniform_for_layout(draw: &DrawCommand, layout: Option<MaskAtlasLayout>) -> [f32; 4] {
    let Some(mask) = draw.mask else {
        return [0.0, 0.0, 0.0, 0.0];
    };
    let Some(layout) = layout else {
        return [0.0, 0.0, 0.0, 0.0];
    };
    if mask.0 >= layout.slots {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let column = mask.0 % layout.columns;
    let row = mask.0 / layout.columns;
    let slot_scale_x = layout.slot_width as f32 / layout.width as f32;
    let slot_scale_y = layout.slot_height as f32 / layout.height as f32;
    [
        column as f32 * slot_scale_x,
        slot_scale_x,
        row as f32 * slot_scale_y,
        if draw.inverted_mask {
            -slot_scale_y
        } else {
            slot_scale_y
        },
    ]
}

fn draw_command_for_id<'a>(
    render_plan: &'a RenderPlan,
    drawable_id: &str,
) -> Option<&'a DrawCommand> {
    render_plan
        .draws
        .iter()
        .find(|draw| draw.drawable_id.as_ref() == drawable_id)
}

fn create_live2d_pipeline(
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

fn fragment_entry_point(shader_variant: ShaderVariant) -> &'static str {
    match shader_variant {
        ShaderVariant::DefaultMesh => "fs_main",
        ShaderVariant::MaskWriter => "fs_mask",
    }
}

fn pipeline_label(key: PipelineKey) -> String {
    format!(
        "Live2D {:?} {:?}{} Pipeline",
        key.shader_variant,
        key.blend_mode,
        if key.masked { " Masked" } else { "" }
    )
}

fn live2d_blend_state(blend_mode: BlendMode) -> wgpu::BlendState {
    match blend_mode {
        BlendMode::Normal => wgpu::BlendState::ALPHA_BLENDING,
        BlendMode::Additive => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        },
        BlendMode::Multiplicative => wgpu::BlendState {
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
    }
}

fn scene_topology(snapshot: &ModelSnapshot) -> SceneTopology {
    (
        snapshot.textures.len(),
        renderable_drawables(snapshot)
            .map(|drawable| {
                (
                    drawable.id.as_ref().to_owned(),
                    drawable.vertices.len(),
                    drawable.indices.len(),
                    drawable.texture_index,
                )
            })
            .collect(),
    )
}

fn texture_topology(snapshot: &ModelSnapshot) -> TextureTopology {
    snapshot
        .textures
        .iter()
        .map(|texture| (texture.width, texture.height, texture.rgba.len()))
        .collect()
}

fn gpu_scene_positions(snapshot: &ModelSnapshot, render_plan: &RenderPlan) -> Vec<GpuPosition> {
    let mut positions = Vec::with_capacity(render_plan.model.vertex_count as usize);
    for drawable in renderable_drawables(snapshot) {
        positions.extend(drawable.vertices.iter().map(|vertex| GpuPosition {
            position: vertex.position,
        }));
    }
    positions
}

fn position_uploads(
    previous: &[GpuPosition],
    next: &[GpuPosition],
    render_plan: &RenderPlan,
) -> Vec<PositionUpload> {
    if previous.len() != next.len() {
        return vec![PositionUpload {
            vertex_range: 0..next.len() as u32,
            byte_offset: 0,
        }];
    }

    let mut uploads = Vec::new();
    for drawable in &render_plan.model.drawables {
        let start = drawable.ranges.vertex_range.start as usize;
        let end = drawable.ranges.vertex_range.end as usize;
        if end > previous.len() || previous[start..end] == next[start..end] {
            continue;
        }
        uploads.push(PositionUpload {
            vertex_range: drawable.ranges.vertex_range.clone(),
            byte_offset: start as u64 * std::mem::size_of::<GpuPosition>() as u64,
        });
    }
    uploads
}

fn gpu_scene_uvs(snapshot: &ModelSnapshot, render_plan: &RenderPlan) -> Vec<GpuUv> {
    let mut uvs = Vec::with_capacity(render_plan.model.vertex_count as usize);
    for drawable in renderable_drawables(snapshot) {
        uvs.extend(
            drawable
                .vertices
                .iter()
                .map(|vertex| GpuUv { uv: vertex.uv }),
        );
    }
    uvs
}

fn gpu_scene_indices(snapshot: &ModelSnapshot, render_plan: &RenderPlan) -> Vec<u16> {
    let mut indices = Vec::with_capacity(render_plan.model.index_count as usize);
    for drawable in renderable_drawables(snapshot) {
        indices.extend_from_slice(&drawable.indices);
    }
    indices
}

fn renderable_drawables(snapshot: &ModelSnapshot) -> impl Iterator<Item = &Drawable> {
    snapshot
        .drawables
        .iter()
        .filter(|drawable| !drawable.vertices.is_empty() && !drawable.indices.is_empty())
}

fn buffer_size<T>(len: usize) -> u64 {
    (len * std::mem::size_of::<T>()).max(1) as u64
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
    use live2d_core::{
        BlendMode, CanvasInfo, ClippingInfo, DrawableId, MaskRef, MaterialKey, TextureAsset, Vertex,
    };

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

    #[test]
    fn mask_uniform_maps_mask_ref_to_atlas_slot() {
        let mut draw = draw_command("masked");
        draw.mask = Some(MaskRef(2));
        let layout = mask_atlas_layout(100, 50, 4, 200);

        assert_eq!(
            mask_uniform_for_layout(&draw, Some(layout)),
            [0.0, 0.5, 0.5, 0.5]
        );

        draw.inverted_mask = true;
        assert_eq!(
            mask_uniform_for_layout(&draw, Some(layout)),
            [0.0, 0.5, 0.5, -0.5]
        );
    }

    #[test]
    fn mask_uniform_disables_mask_without_slot_or_mask_ref() {
        let layout = mask_atlas_layout(100, 50, 1, 200);

        assert_eq!(
            mask_uniform_for_layout(&draw_command("plain"), Some(layout)),
            [0.0; 4]
        );

        let mut draw = draw_command("masked");
        draw.mask = Some(MaskRef(0));
        assert_eq!(mask_uniform_for_layout(&draw, None), [0.0; 4]);
        draw.mask = Some(MaskRef(1));
        assert_eq!(mask_uniform_for_layout(&draw, Some(layout)), [0.0; 4]);
    }

    #[test]
    fn mask_atlas_layout_wraps_slots_within_texture_limit() {
        let layout = mask_atlas_layout(100, 50, 3, 200);

        assert_eq!(
            layout,
            MaskAtlasLayout {
                width: 200,
                height: 100,
                slot_width: 100,
                slot_height: 50,
                columns: 2,
                rows: 2,
                slots: 3,
            }
        );
    }

    #[test]
    fn mask_atlas_layout_scales_slots_when_full_size_grid_would_overflow() {
        let layout = mask_atlas_layout(100, 100, 9, 250);

        assert_eq!(layout.columns, 3);
        assert_eq!(layout.rows, 3);
        assert!(layout.width <= 250);
        assert!(layout.height <= 250);
        assert_eq!(layout.slot_width, layout.slot_height);
    }

    #[test]
    fn align_to_rounds_uniform_stride_to_required_boundary() {
        assert_eq!(align_to(80, 256), 256);
        assert_eq!(align_to(512, 256), 512);
    }

    #[test]
    fn uniform_slots_include_mask_writer_and_main_draws() {
        let render_plan = RenderPlanner::new().build(&masked_snapshot());

        assert_eq!(mask_uniform_slots(&render_plan), 1);
        assert_eq!(uniform_slots(&render_plan), 3);
    }

    #[test]
    fn scene_topology_allows_dynamic_vertex_upload_without_rebuild() {
        let mut next = snapshot_with_drawable("mesh", 0, 2, 3, 0, 1);
        next.drawables[0].vertices[0].position = [2.0, 3.0];
        let base_plan = RenderPlanner::new().build(&snapshot_with_drawable("mesh", 0, 2, 3, 0, 1));
        let next_plan = RenderPlanner::new().build(&next);

        assert_eq!(
            scene_topology(&snapshot_with_drawable("mesh", 0, 2, 3, 0, 1)),
            scene_topology(&next)
        );
        assert_ne!(
            gpu_scene_positions(&snapshot_with_drawable("mesh", 0, 2, 3, 0, 1), &base_plan),
            gpu_scene_positions(&next, &next_plan)
        );
        assert_eq!(
            gpu_scene_uvs(&snapshot_with_drawable("mesh", 0, 2, 3, 0, 1), &base_plan),
            gpu_scene_uvs(&next, &next_plan)
        );
    }

    #[test]
    fn scene_topology_changes_for_static_gpu_resource_shape() {
        let base = snapshot_with_drawable("mesh", 0, 2, 3, 0, 1);

        assert_ne!(
            scene_topology(&base),
            scene_topology(&snapshot_with_drawable("mesh", 0, 3, 3, 0, 1))
        );
        assert_ne!(
            scene_topology(&base),
            scene_topology(&snapshot_with_drawable("mesh", 0, 2, 4, 0, 1))
        );
        assert_ne!(
            scene_topology(&base),
            scene_topology(&snapshot_with_drawable("mesh", 0, 2, 3, 1, 2))
        );
    }

    #[test]
    fn texture_topology_is_independent_from_drawable_buffer_shape() {
        let base = snapshot_with_drawable("mesh", 0, 2, 3, 0, 1);
        let changed_drawable = snapshot_with_drawable("mesh", 0, 4, 6, 0, 1);

        assert_eq!(texture_topology(&base), texture_topology(&changed_drawable));
    }

    #[test]
    fn texture_topology_changes_for_texture_resource_shape() {
        let base = snapshot_with_drawable("mesh", 0, 2, 3, 0, 1);
        let mut resized = base.clone();
        resized.textures[0].width = 2;
        resized.textures[0].rgba.resize(2 * 1 * 4, 255);
        let mut changed_bytes = base.clone();
        changed_bytes.textures[0].rgba.push(0);

        assert_ne!(texture_topology(&base), texture_topology(&resized));
        assert_ne!(texture_topology(&base), texture_topology(&changed_bytes));
    }

    #[test]
    fn position_uploads_only_cover_changed_drawable_ranges() {
        let base = snapshot_with_drawables(&[("a", 0, 2, 3), ("b", 1, 3, 3)]);
        let mut next = base.clone();
        next.drawables[1].vertices[0].position = [20.0, 21.0];
        let plan = RenderPlanner::new().build(&base);
        let previous = gpu_scene_positions(&base, &plan);
        let next_positions = gpu_scene_positions(&next, &plan);

        assert_eq!(
            position_uploads(&previous, &next_positions, &plan),
            vec![PositionUpload {
                vertex_range: 2..5,
                byte_offset: 2 * std::mem::size_of::<GpuPosition>() as u64,
            }]
        );
    }

    #[test]
    fn position_uploads_fall_back_to_full_range_when_lengths_differ() {
        let base = snapshot_with_drawables(&[("a", 0, 2, 3)]);
        let next = snapshot_with_drawables(&[("a", 0, 3, 3)]);
        let base_plan = RenderPlanner::new().build(&base);
        let next_plan = RenderPlanner::new().build(&next);
        let previous = gpu_scene_positions(&base, &base_plan);
        let next_positions = gpu_scene_positions(&next, &next_plan);

        assert_eq!(
            position_uploads(&previous, &next_positions, &next_plan),
            vec![PositionUpload {
                vertex_range: 0..3,
                byte_offset: 0,
            }]
        );
    }

    fn snapshot_with_drawable(
        id: &str,
        render_order: i32,
        vertex_count: usize,
        index_count: usize,
        texture_index: usize,
        texture_count: usize,
    ) -> ModelSnapshot {
        ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            drawables: vec![Drawable {
                id: DrawableId::from(id),
                render_order,
                texture_index,
                vertices: (0..vertex_count)
                    .map(|index| Vertex {
                        position: [index as f32, index as f32 + 1.0],
                        uv: [0.0, 0.0],
                    })
                    .collect(),
                indices: (0..index_count).map(|index| index as u16).collect(),
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                clipping: None,
            }],
            textures: (0..texture_count)
                .map(|_| TextureAsset {
                    width: 1,
                    height: 1,
                    rgba: vec![255, 255, 255, 255],
                })
                .collect(),
        }
    }

    fn snapshot_with_drawables(drawables: &[(&str, i32, usize, usize)]) -> ModelSnapshot {
        ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            drawables: drawables
                .iter()
                .map(|(id, render_order, vertex_count, index_count)| Drawable {
                    id: DrawableId::from(*id),
                    render_order: *render_order,
                    texture_index: 0,
                    vertices: (0..*vertex_count)
                        .map(|index| Vertex {
                            position: [index as f32, index as f32 + 1.0],
                            uv: [0.0, 0.0],
                        })
                        .collect(),
                    indices: (0..*index_count).map(|index| index as u16).collect(),
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    clipping: None,
                })
                .collect(),
            textures: vec![TextureAsset {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
            }],
        }
    }

    fn masked_snapshot() -> ModelSnapshot {
        ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            drawables: vec![
                Drawable {
                    id: DrawableId::from("mask"),
                    render_order: 0,
                    texture_index: 0,
                    vertices: vec![Vertex {
                        position: [0.0, 0.0],
                        uv: [0.0, 0.0],
                    }],
                    indices: vec![0],
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    clipping: None,
                },
                Drawable {
                    id: DrawableId::from("masked"),
                    render_order: 1,
                    texture_index: 0,
                    vertices: vec![Vertex {
                        position: [1.0, 1.0],
                        uv: [0.0, 0.0],
                    }],
                    indices: vec![0],
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    clipping: Some(ClippingInfo {
                        drawable_ids: vec![DrawableId::from("mask")],
                        inverted: false,
                    }),
                },
            ],
            textures: vec![TextureAsset {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
            }],
        }
    }

    fn draw_command(id: &str) -> DrawCommand {
        DrawCommand {
            drawable_id: DrawableId::from(id),
            texture_index: 0,
            vertex_range: 0..3,
            index_range: 0..3,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            inverted_mask: false,
            material: MaterialKey::Default,
        }
    }
}
