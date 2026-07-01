use crate::*;

pub struct WgpuLive2DRenderer {
    pub(crate) pipelines: PipelineCache,
    pub(crate) uniform_layout: wgpu::BindGroupLayout,
    pub(crate) texture_layout: wgpu::BindGroupLayout,
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) fallback_mask_bind_group: wgpu::BindGroup,
    pub(crate) fallback_blend_bind_group: wgpu::BindGroup,
    pub(crate) uniform_buffer: wgpu::Buffer,
    pub(crate) uniform_bind_group: wgpu::BindGroup,
    pub(crate) uniform_stride: u64,
    pub(crate) uniform_capacity: usize,
    pub(crate) uniform_staging: RefCell<Vec<u8>>,
    pub(crate) active_scene_key: Option<String>,
    pub(crate) scene_topologies: HashMap<String, SceneTopology>,
    pub(crate) texture_caches: HashMap<String, TextureCache>,
    pub(crate) mask_atlas: Option<MaskAtlas>,
    pub(crate) mask_atlas_dirty: bool,
    pub(crate) offscreen_target: Option<OffscreenTarget>,
    pub(crate) blend_copy_target: Option<BlendCopyTarget>,
    pub(crate) gpu_scenes: HashMap<String, GpuScene>,
    pub(crate) render_world: RenderWorld,
    #[cfg(feature = "probe")]
    pub(crate) pending_gpu_timestamps: Vec<GpuTimestampFrame>,
}
pub(crate) type TextureTopology = Vec<(u32, u32, usize)>;
pub(crate) type SceneTopology = u64;

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
                resource: uniform_binding(&uniform_buffer, uniform_stride),
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
                Some(&texture_layout),
            ],
            immediate_size: 0,
        });
        let pipelines = PipelineCache::new(device, &pipeline_layout, &shader, format);
        let fallback_mask_bind_group =
            create_empty_mask_bind_group(device, &texture_layout, &sampler);
        let fallback_blend_bind_group =
            create_empty_sampled_texture_bind_group(device, &texture_layout, &sampler);

        Self {
            pipelines,
            uniform_layout: bind_group_layout,
            texture_layout,
            sampler,
            fallback_mask_bind_group,
            fallback_blend_bind_group,
            uniform_buffer,
            uniform_bind_group,
            uniform_stride,
            uniform_capacity: 1,
            uniform_staging: RefCell::new(Vec::new()),
            active_scene_key: None,
            scene_topologies: HashMap::new(),
            texture_caches: HashMap::new(),
            mask_atlas: None,
            mask_atlas_dirty: true,
            offscreen_target: None,
            blend_copy_target: None,
            gpu_scenes: HashMap::new(),
            render_world: RenderWorld::new(),
            #[cfg(feature = "probe")]
            pending_gpu_timestamps: Vec::new(),
        }
    }

    #[cfg(feature = "probe")]
    pub fn new_with_probe<P>(device: &wgpu::Device, format: wgpu::TextureFormat, probe: &P) -> Self
    where
        P: ProbeSink,
    {
        measure(probe, Stage::WgpuRendererInit, Vec::new(), || {
            let renderer = Self::new(device, format);
            gauge(
                probe,
                Stage::WgpuGpuTimestampSupport,
                "gpu_timestamps",
                if device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
                    1.0
                } else {
                    0.0
                },
                Vec::new(),
            );
            counter(
                probe,
                Stage::WgpuPipelineCreation,
                "resource_rebuilds",
                renderer.pipelines.pipelines.len() as u64,
                Vec::new(),
            );
            renderer
        })
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

    #[cfg(feature = "probe")]
    pub fn prepare_render_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
        probe: &P,
    ) -> RenderPlan
    where
        P: ProbeSink,
    {
        measure(probe, Stage::WgpuPrepareRender, Vec::new(), || {
            let render_plan = self.prepare_scene_with_probe(device, queue, snapshot, probe);
            self.ensure_uniform_capacity_with_probe(device, uniform_slots(&render_plan), probe);
            render_plan
        })
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

    pub fn render_with_postprocess_to_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: WgpuLive2DTarget<'_>,
        snapshot: &ModelSnapshot,
        view: WgpuLive2DView,
        postprocess: &mut WgpuPostProcessChain,
    ) -> Result<(), WgpuPostProcessError> {
        let render_plan = self.prepare_render(device, queue, snapshot);
        self.encode_with_postprocess_to_view(
            device,
            queue,
            encoder,
            target,
            &render_plan,
            &snapshot.canvas,
            view,
            postprocess,
        )
    }

    #[cfg(feature = "probe")]
    pub fn render_to_view_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: WgpuLive2DTarget<'_>,
        snapshot: &ModelSnapshot,
        view: WgpuLive2DView,
        probe: &P,
    ) where
        P: ProbeSink,
    {
        let render_plan = self.prepare_render_with_probe(device, queue, snapshot, probe);
        self.encode_render_to_view_with_probe(
            device,
            queue,
            encoder,
            target,
            &render_plan,
            &snapshot.canvas,
            view,
            probe,
        );
    }

    #[cfg(feature = "probe")]
    pub fn render_with_postprocess_to_view_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: WgpuLive2DTarget<'_>,
        snapshot: &ModelSnapshot,
        view: WgpuLive2DView,
        postprocess: &mut WgpuPostProcessChain,
        probe: &P,
    ) -> Result<(), WgpuPostProcessError>
    where
        P: ProbeSink,
    {
        let render_plan = self.prepare_render_with_probe(device, queue, snapshot, probe);
        self.encode_with_postprocess_to_view_with_probe(
            device,
            queue,
            encoder,
            target,
            &render_plan,
            &snapshot.canvas,
            view,
            postprocess,
            probe,
        )
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
            self.prepare_mask_atlas(device, queue, encoder, render_plan, canvas, &view, None);
        }
        self.encode_main_draws_to_target(
            device,
            queue,
            encoder,
            target.texture,
            target.view,
            target.resolve_target,
            target.load_op,
            target.store_op,
            render_plan,
            canvas,
            view,
            mask_uniform_slots(render_plan),
        );
    }

    pub fn encode_with_postprocess_to_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: WgpuLive2DTarget<'_>,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: WgpuLive2DView,
        postprocess: &mut WgpuPostProcessChain,
    ) -> Result<(), WgpuPostProcessError> {
        if postprocess.is_empty() {
            self.encode_render_to_view(device, queue, encoder, target, render_plan, canvas, view);
            return Ok(());
        }
        if target.resolve_target.is_some() {
            return Err(WgpuPostProcessError::UnsupportedResolveTarget);
        }

        let width = view.width.max(1);
        let height = view.height.max(1);
        self.ensure_offscreen_target(device, width, height);
        let offscreen = self
            .offscreen_target
            .take()
            .expect("offscreen target is created before postprocess rendering");
        self.encode_render_to_view(
            device,
            queue,
            encoder,
            WgpuLive2DTarget::clear(&offscreen.texture, &offscreen.view, POST_PROCESS_CLEAR),
            render_plan,
            canvas,
            view,
        );
        let result = postprocess.encode(
            device,
            queue,
            encoder,
            &offscreen.view,
            target,
            width,
            height,
        );
        self.offscreen_target = Some(offscreen);
        result
    }

    #[cfg(feature = "probe")]
    pub fn encode_render_to_view_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: WgpuLive2DTarget<'_>,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: WgpuLive2DView,
        probe: &P,
    ) where
        P: ProbeSink,
    {
        self.ensure_uniform_capacity_with_probe(device, uniform_slots(render_plan), probe);
        let timestamp_frame = GpuTimestampFrame::new(device, !render_plan.masks.is_empty(), false);
        if !render_plan.masks.is_empty() {
            let mask_timestamp_writes = timestamp_frame
                .as_ref()
                .and_then(GpuTimestampFrame::mask_timestamp_writes);
            self.prepare_mask_atlas_with_probe(
                device,
                queue,
                encoder,
                render_plan,
                canvas,
                &view,
                probe,
                mask_timestamp_writes,
            );
        }
        let first_main_uniform_slot = mask_uniform_slots(render_plan);
        measure(
            probe,
            Stage::WgpuMainPassEncode,
            vec![ProbeAttr::new("draws", render_plan.draws.len())],
            || {
                self.encode_main_draws_to_target(
                    device,
                    queue,
                    encoder,
                    target.texture,
                    target.view,
                    target.resolve_target,
                    target.load_op,
                    target.store_op,
                    render_plan,
                    canvas,
                    view,
                    first_main_uniform_slot,
                );
                counter(
                    probe,
                    Stage::WgpuMainPassEncode,
                    "draw_calls",
                    render_plan.draws.len() as u64,
                    Vec::new(),
                );
            },
        );
        if let Some(timestamp_frame) = timestamp_frame {
            timestamp_frame.resolve(encoder);
            self.pending_gpu_timestamps.push(timestamp_frame);
        }
    }

    #[cfg(feature = "probe")]
    pub fn encode_with_postprocess_to_view_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: WgpuLive2DTarget<'_>,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: WgpuLive2DView,
        postprocess: &mut WgpuPostProcessChain,
        probe: &P,
    ) -> Result<(), WgpuPostProcessError>
    where
        P: ProbeSink,
    {
        if postprocess.is_empty() {
            self.encode_render_to_view_with_probe(
                device,
                queue,
                encoder,
                target,
                render_plan,
                canvas,
                view,
                probe,
            );
            return Ok(());
        }
        if target.resolve_target.is_some() {
            return Err(WgpuPostProcessError::UnsupportedResolveTarget);
        }

        let width = view.width.max(1);
        let height = view.height.max(1);
        self.ensure_offscreen_target_with_probe(device, width, height, probe);
        let offscreen = self
            .offscreen_target
            .take()
            .expect("offscreen target is created before postprocess rendering");
        self.encode_render_to_view_with_probe(
            device,
            queue,
            encoder,
            WgpuLive2DTarget::clear(&offscreen.texture, &offscreen.view, POST_PROCESS_CLEAR),
            render_plan,
            canvas,
            view,
            probe,
        );
        let result = postprocess.encode_with_probe(
            device,
            queue,
            encoder,
            &offscreen.view,
            target,
            width,
            height,
            probe,
        );
        self.offscreen_target = Some(offscreen);
        result
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
        let offscreen = self
            .offscreen_target
            .take()
            .expect("offscreen target is created before rendering");
        self.render_to_view(
            device,
            queue,
            encoder,
            WgpuLive2DTarget::clear(&offscreen.texture, &offscreen.view, clear_color),
            snapshot,
            view,
        );
        self.offscreen_target = Some(offscreen);
        &self
            .offscreen_target
            .as_ref()
            .expect("offscreen target is retained after rendering")
            .view
    }

    #[cfg(feature = "probe")]
    pub fn render_to_offscreen_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        snapshot: &ModelSnapshot,
        view: WgpuLive2DView,
        clear_color: wgpu::Color,
        probe: &P,
    ) -> &wgpu::TextureView
    where
        P: ProbeSink,
    {
        self.ensure_offscreen_target_with_probe(device, view.width, view.height, probe);
        let offscreen = self
            .offscreen_target
            .take()
            .expect("offscreen target is created before rendering");
        self.render_to_view_with_probe(
            device,
            queue,
            encoder,
            WgpuLive2DTarget::clear(&offscreen.texture, &offscreen.view, clear_color),
            snapshot,
            view,
            probe,
        );
        self.offscreen_target = Some(offscreen);
        &self
            .offscreen_target
            .as_ref()
            .expect("offscreen target is retained after rendering")
            .view
    }

    #[cfg(feature = "probe")]
    pub fn collect_gpu_timestamps_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        probe: &P,
    ) -> Result<(), String>
    where
        P: ProbeSink,
    {
        let timestamp_period = queue.get_timestamp_period() as f64;
        for pending in self.pending_gpu_timestamps.drain(..) {
            let values =
                read_timestamp_values(device, &pending.readback_buffer, pending.query_count)?;
            if let Some((start, end)) = pending.mask_indices {
                record_gpu_pass_nanos(
                    probe,
                    Stage::WgpuMaskPassEncode,
                    "mask_atlas",
                    &values,
                    (start, end),
                    timestamp_period,
                );
            }
            if let Some((start, end)) = pending.main_indices {
                record_gpu_pass_nanos(
                    probe,
                    Stage::WgpuMainPassEncode,
                    "main",
                    &values,
                    (start, end),
                    timestamp_period,
                );
            }
        }
        Ok(())
    }

    pub(crate) fn active_gpu_scene(&self) -> Option<&GpuScene> {
        self.active_scene_key
            .as_ref()
            .and_then(|key| self.gpu_scenes.get(key))
    }

    pub(crate) fn active_gpu_scene_mut(&mut self) -> Option<&mut GpuScene> {
        self.gpu_scenes.get_mut(self.active_scene_key.as_deref()?)
    }
}
