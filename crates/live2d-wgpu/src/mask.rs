use crate::*;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaskAtlasLayout {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) slot_width: u32,
    pub(crate) slot_height: u32,
    pub(crate) columns: usize,
    pub(crate) rows: usize,
    pub(crate) slots: usize,
}
pub(crate) struct WgpuMaskRenderBackend<'a, 'pass> {
    pub(crate) pass: &'a mut wgpu::RenderPass<'pass>,
    pub(crate) pipelines: &'a PipelineCache,
    pub(crate) uniform_bind_group: &'a wgpu::BindGroup,
    pub(crate) uniform_stride: u64,
    pub(crate) uniform_index: usize,
    pub(crate) fallback_mask_bind_group: &'a wgpu::BindGroup,
    pub(crate) fallback_blend_bind_group: &'a wgpu::BindGroup,
    pub(crate) gpu_scene: &'a GpuScene,
    pub(crate) layout: MaskAtlasLayout,
    pub(crate) skip_mask: bool,
    pub(crate) last_texture_index: Option<usize>,
    pub(crate) draw_calls: usize,
}
pub(crate) enum MaskDrawLookup<'a> {
    Linear(&'a [DrawCommand]),
    Indexed(HashMap<&'a str, &'a DrawCommand>),
}

impl<'a> MaskDrawLookup<'a> {
    pub(crate) fn new(render_plan: &'a RenderPlan) -> Self {
        let potential_comparisons = render_plan
            .mask_draws
            .len()
            .saturating_mul(raw_mask_drawable_count(render_plan));
        if potential_comparisons <= MASK_DRAW_LOOKUP_INDEX_THRESHOLD {
            return Self::Linear(&render_plan.mask_draws);
        }

        let lookup = render_plan
            .mask_draws
            .iter()
            .map(|draw| (draw.drawable_id.as_ref(), draw))
            .collect();
        Self::Indexed(lookup)
    }

    fn get(&self, drawable_id: &str) -> Option<&'a DrawCommand> {
        match self {
            Self::Linear(draws) => draws
                .iter()
                .find(|draw| draw.drawable_id.as_ref() == drawable_id),
            Self::Indexed(lookup) => lookup.get(drawable_id).copied(),
        }
    }
}

impl<'a, 'pass> Live2DRenderBackend for WgpuMaskRenderBackend<'a, 'pass> {
    fn begin_model(&mut self, _ctx: &ModelRenderCtx) {
        self.pass.set_pipeline(self.pipelines.mask_writer());
        self.pass
            .set_bind_group(3, self.fallback_blend_bind_group, &[]);
        self.pass
            .set_vertex_buffer(0, self.gpu_scene.position_buffer.slice(..));
        self.pass
            .set_vertex_buffer(1, self.gpu_scene.uv_buffer.slice(..));
        self.pass.set_index_buffer(
            self.gpu_scene.index_buffer.slice(..),
            wgpu::IndexFormat::Uint16,
        );
        self.pass
            .set_bind_group(2, self.fallback_mask_bind_group, &[]);
    }

    fn begin_clip_mask(&mut self, mask: &MaskPass) {
        self.skip_mask = mask.id.0 >= self.layout.slots;
        if self.skip_mask {
            return;
        }
        let slot_x = (mask.id.0 % self.layout.columns) as f32 * self.layout.slot_width as f32;
        let slot_y = (mask.id.0 / self.layout.columns) as f32 * self.layout.slot_height as f32;
        self.pass.set_viewport(
            slot_x,
            slot_y,
            self.layout.slot_width as f32,
            self.layout.slot_height as f32,
            0.0,
            1.0,
        );
    }

    fn draw_mask_drawable(&mut self, _mask: &MaskPass, draw: &DrawCommand) {
        if self.skip_mask {
            return;
        }
        let uniform_offset = self.uniform_stride * self.uniform_index as u64;
        self.uniform_index += 1;
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
        self.pass
            .set_bind_group(0, self.uniform_bind_group, &[uniform_offset as u32]);
        if self.last_texture_index != Some(draw.texture_index) {
            self.pass.set_bind_group(1, texture, &[]);
            self.last_texture_index = Some(draw.texture_index);
        }
        self.pass.insert_debug_marker(draw.drawable_id.as_ref());
        self.pass
            .draw_indexed(draw.index_range.clone(), base_vertex, 0..1);
        self.draw_calls += 1;
    }

    fn draw_drawable(&mut self, _call: &DrawCommand) {}
}

impl WgpuLive2DRenderer {
    pub(crate) fn prepare_mask_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: &WgpuLive2DView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> MaskAtlasUpdate {
        let Some(active_scene_key) = self.active_scene_key.clone() else {
            return MaskAtlasUpdate::default();
        };
        let Some(gpu_scene) = self.gpu_scenes.get(&active_scene_key) else {
            return MaskAtlasUpdate::default();
        };
        let slots = render_plan.masks.len();
        if slots == 0 {
            self.mask_atlas = None;
            return MaskAtlasUpdate::default();
        }
        let layout = mask_atlas_layout(
            view.width,
            view.height,
            slots,
            device.limits().max_texture_dimension_2d,
        );
        let rebuild = mask_atlas_needs_rebuild(self.mask_atlas.as_ref(), layout);
        if rebuild {
            self.mask_atlas_dirty = true;
            self.mask_atlas = Some(create_mask_atlas(
                device,
                &self.texture_layout,
                &self.sampler,
                layout,
            ));
        }
        let draw_lookup = MaskDrawLookup::new(render_plan);
        let signature =
            mask_atlas_static_signature(render_plan, &draw_lookup, canvas, view, layout);
        let cache_hit = !self.mask_atlas_dirty
            && self.mask_atlas.as_ref().and_then(|atlas| atlas.signature) == Some(signature);
        if cache_hit {
            if let Some(timestamp_writes) = timestamp_writes {
                let mask_atlas = self
                    .mask_atlas
                    .as_ref()
                    .expect("mask atlas exists for cached mask pass");
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Live2D Mask Atlas Cache Hit"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &mask_atlas.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: Some(timestamp_writes),
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            return MaskAtlasUpdate::default();
        }
        let Some(mask_atlas) = &self.mask_atlas else {
            return MaskAtlasUpdate::default();
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
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let wrote_uniforms = {
            let mut uniform_staging = self.uniform_staging.borrow_mut();
            fill_mask_uniform_upload_bytes(
                render_plan,
                &draw_lookup,
                canvas,
                view,
                layout,
                self.uniform_stride,
                &mut uniform_staging,
            );
            let wrote_uniforms = !uniform_staging.is_empty();
            if wrote_uniforms {
                queue.write_buffer(&self.uniform_buffer, 0, &uniform_staging);
            }
            wrote_uniforms
        };
        pass.push_debug_group("live2d mask atlas");
        let mut backend = WgpuMaskRenderBackend {
            pass: &mut pass,
            pipelines: &self.pipelines,
            uniform_bind_group: &self.uniform_bind_group,
            uniform_stride: self.uniform_stride,
            uniform_index: 0,
            fallback_mask_bind_group: &self.fallback_mask_bind_group,
            fallback_blend_bind_group: &self.fallback_blend_bind_group,
            gpu_scene,
            layout,
            skip_mask: false,
            last_texture_index: None,
            draw_calls: 0,
        };
        render_plan.dispatch(&mut backend);
        let draw_calls = backend.draw_calls;
        pass.pop_debug_group();
        drop(pass);
        if let Some(mask_atlas) = &mut self.mask_atlas {
            mask_atlas.signature = Some(signature);
        }
        self.mask_atlas_dirty = false;
        MaskAtlasUpdate {
            encoded: true,
            draw_calls,
            uniform_writes: usize::from(wrote_uniforms),
        }
    }

    #[cfg(feature = "probe")]
    pub(crate) fn prepare_mask_atlas_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: &WgpuLive2DView,
        probe: &P,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) where
        P: ProbeSink,
    {
        let slots = render_plan.masks.len();
        if slots == 0 {
            self.prepare_mask_atlas(
                device,
                queue,
                encoder,
                render_plan,
                canvas,
                view,
                timestamp_writes,
            );
            return;
        }
        let layout = measure(
            probe,
            Stage::WgpuMaskAtlasLayout,
            vec![
                ProbeAttr::new("view_width", view.width),
                ProbeAttr::new("view_height", view.height),
                ProbeAttr::new("slots", slots),
            ],
            || {
                mask_atlas_layout(
                    view.width,
                    view.height,
                    slots,
                    device.limits().max_texture_dimension_2d,
                )
            },
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
            counter(
                probe,
                Stage::WgpuMaskAtlasRebuild,
                "resource_rebuilds",
                1,
                vec![ProbeAttr::new("resource", "mask_atlas")],
            );
        }
        let update = measure(
            probe,
            Stage::WgpuMaskPassEncode,
            vec![
                ProbeAttr::new("masks", render_plan.masks.len()),
                ProbeAttr::new("mask_draw_calls", mask_draw_call_count(render_plan)),
            ],
            || {
                self.prepare_mask_atlas(
                    device,
                    queue,
                    encoder,
                    render_plan,
                    canvas,
                    view,
                    timestamp_writes,
                )
            },
        );
        if update.encoded {
            counter(
                probe,
                Stage::WgpuMaskPassEncode,
                "cache_misses",
                1,
                vec![ProbeAttr::new("cache", "mask_atlas")],
            );
        } else {
            counter(
                probe,
                Stage::WgpuMaskPassEncode,
                "cache_hits",
                1,
                vec![ProbeAttr::new("cache", "mask_atlas")],
            );
        }
        counter(
            probe,
            Stage::WgpuMaskPassEncode,
            "draw_calls",
            update.draw_calls as u64,
            Vec::new(),
        );
        counter(
            probe,
            Stage::WgpuMaskPassEncode,
            "buffer_writes",
            update.uniform_writes as u64,
            vec![ProbeAttr::new("buffer", "uniform")],
        );
    }
}

pub(crate) fn mask_uniform_slots(render_plan: &RenderPlan) -> usize {
    mask_draw_call_count(render_plan)
}

pub(crate) fn mask_draw_call_count(render_plan: &RenderPlan) -> usize {
    let draw_lookup = MaskDrawLookup::new(render_plan);
    render_plan
        .masks
        .iter()
        .map(|mask| {
            mask.drawable_ids
                .iter()
                .filter(|drawable_id| draw_lookup.get(drawable_id.as_ref()).is_some())
                .count()
        })
        .sum()
}

pub(crate) fn raw_mask_drawable_count(render_plan: &RenderPlan) -> usize {
    render_plan
        .masks
        .iter()
        .map(|mask| mask.drawable_ids.len())
        .sum()
}

pub(crate) fn mask_writer_uniform(
    _draw: &DrawCommand,
    canvas: &CanvasInfo,
    view: &WgpuLive2DView,
    layout: MaskAtlasLayout,
) -> Live2dUniform {
    Live2dUniform {
        viewport: [
            layout.slot_width as f32,
            layout.slot_height as f32,
            layout.slot_width as f32 / layout.slot_height as f32,
            0.0,
        ],
        view_transform: view.transform,
        canvas: live2d_canvas_uniform(canvas),
        effect: [1.0, 1.0, 1.0, 1.0],
        mask: [0.0, 0.0, 0.0, 0.0],
        blend: [0, 0, 0, 0],
    }
}

pub(crate) fn fill_mask_uniform_upload_bytes(
    render_plan: &RenderPlan,
    draw_lookup: &MaskDrawLookup<'_>,
    canvas: &CanvasInfo,
    view: &WgpuLive2DView,
    layout: MaskAtlasLayout,
    uniform_stride: u64,
    bytes: &mut Vec<u8>,
) {
    let uniform_stride = uniform_stride as usize;
    let uniform_size = std::mem::size_of::<Live2dUniform>();
    debug_assert!(uniform_stride >= uniform_size);
    bytes.clear();

    for mask in &render_plan.masks {
        if mask.id.0 >= layout.slots {
            continue;
        }
        for drawable_id in &mask.drawable_ids {
            let Some(draw) = draw_lookup.get(drawable_id.as_ref()) else {
                continue;
            };
            let offset = bytes.len();
            bytes.resize(offset + uniform_stride, 0);
            let uniform = mask_writer_uniform(draw, canvas, view, layout);
            bytes[offset..offset + uniform_size].copy_from_slice(bytemuck::bytes_of(&uniform));
        }
    }
}

pub(crate) fn create_empty_mask_bind_group(
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

pub(crate) fn create_mask_atlas(
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
        signature: None,
        view,
        bind_group,
    }
}
pub(crate) fn create_mask_bind_group(
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

pub(crate) fn mask_atlas_needs_rebuild(
    mask_atlas: Option<&MaskAtlas>,
    layout: MaskAtlasLayout,
) -> bool {
    mask_atlas
        .map(|atlas| {
            atlas.width != layout.width
                || atlas.height != layout.height
                || atlas.slot_width != layout.slot_width
                || atlas.slot_height != layout.slot_height
                || atlas.columns != layout.columns
                || atlas.slots != layout.slots
        })
        .unwrap_or(true)
}

pub(crate) fn mask_atlas_static_signature(
    render_plan: &RenderPlan,
    draw_lookup: &MaskDrawLookup<'_>,
    canvas: &CanvasInfo,
    view: &WgpuLive2DView,
    layout: MaskAtlasLayout,
) -> u64 {
    let mut signature = 0xcbf2_9ce4_8422_2325;
    mix_u64(&mut signature, layout.width as u64);
    mix_u64(&mut signature, layout.height as u64);
    mix_u64(&mut signature, layout.slot_width as u64);
    mix_u64(&mut signature, layout.slot_height as u64);
    mix_u64(&mut signature, layout.columns as u64);
    mix_u64(&mut signature, layout.rows as u64);
    mix_u64(&mut signature, layout.slots as u64);
    mix_f32_slice(&mut signature, &view.transform);
    mix_f32_slice(&mut signature, &live2d_canvas_uniform(canvas));

    mix_u64(&mut signature, render_plan.masks.len() as u64);
    for (mask_index, mask) in render_plan.masks.iter().enumerate() {
        mix_u64(&mut signature, mask_index as u64);
        mix_u64(&mut signature, mask.drawable_ids.len() as u64);
        for drawable_id in &mask.drawable_ids {
            let Some(draw) = draw_lookup.get(drawable_id.as_ref()) else {
                mix_u64(&mut signature, 0);
                continue;
            };
            mix_u64(&mut signature, 1);
            mix_u64(&mut signature, draw.texture_index as u64);
            mix_u64(&mut signature, draw.vertex_range.start as u64);
            mix_u64(&mut signature, draw.vertex_range.end as u64);
            mix_u64(&mut signature, draw.index_range.start as u64);
            mix_u64(&mut signature, draw.index_range.end as u64);
        }
    }

    signature
}

pub(crate) fn position_uploads_touch_masks(
    uploads: &[PositionUpload],
    render_plan: &RenderPlan,
) -> bool {
    if uploads.is_empty() || render_plan.masks.is_empty() {
        return false;
    }
    let draw_lookup = MaskDrawLookup::new(render_plan);
    render_plan
        .masks
        .iter()
        .flat_map(|mask| &mask.drawable_ids)
        .filter_map(|drawable_id| draw_lookup.get(drawable_id.as_ref()))
        .any(|draw| {
            uploads.iter().any(|upload| {
                upload.vertex_range.start < draw.vertex_range.end
                    && draw.vertex_range.start < upload.vertex_range.end
            })
        })
}
pub(crate) fn mask_atlas_layout(
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

pub(crate) fn mask_layout_score(layout: MaskAtlasLayout) -> (u64, u32, std::cmp::Reverse<usize>) {
    (
        layout.slot_width as u64 * layout.slot_height as u64,
        layout.slot_width.min(layout.slot_height),
        std::cmp::Reverse(layout.rows),
    )
}

pub(crate) fn mask_uniform(draw: &DrawCommand, mask_atlas: Option<&MaskAtlas>) -> [f32; 4] {
    mask_uniform_for_layout(draw, mask_atlas.map(MaskAtlas::layout))
}

pub(crate) fn mask_uniform_for_layout(
    draw: &DrawCommand,
    layout: Option<MaskAtlasLayout>,
) -> [f32; 4] {
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
    fn mask_writer_uniform_slots_track_mask_draws() {
        let mut snapshot = masked_snapshot();
        snapshot.drawables.push(Drawable {
            id: DrawableId::from("mask_extra"),
            render_order: 2,
            texture_index: 0,
            vertices: vec![Vertex {
                position: [2.0, 2.0],
                uv: [0.0, 0.0],
            }],
            indices: vec![0],
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clipping: None,
        });
        snapshot.drawables[1]
            .clipping
            .as_mut()
            .expect("masked drawable has clipping")
            .drawable_ids
            .push(DrawableId::from("mask_extra"));
        let render_plan = RenderPlanner::new().build(&snapshot);

        assert_eq!(mask_draw_call_count(&render_plan), 2);
        assert_eq!(mask_uniform_slots(&render_plan), 2);
        assert_eq!(uniform_slots(&render_plan), render_plan.draws.len() + 2);
    }

    #[test]
    fn mask_writer_uniform_upload_bytes_ignore_mask_draw_opacity() {
        let mut snapshot = masked_snapshot();
        snapshot.drawables[0].opacity = 0.0;
        let render_plan = RenderPlanner::new().build(&snapshot);
        let draw_lookup = MaskDrawLookup::new(&render_plan);
        let layout = mask_atlas_layout(160, 120, render_plan.masks.len(), 512);
        let stride = align_to(std::mem::size_of::<Live2dUniform>() as u64, 256);
        let view = WgpuLive2DView {
            transform: [0.1, 0.2, 1.5, 0.0],
            width: 160,
            height: 120,
            effect: [0.4, 0.5, 0.6, 0.7],
            target_drawable_ids: Vec::new(),
        };

        let mut bytes = Vec::new();
        fill_mask_uniform_upload_bytes(
            &render_plan,
            &draw_lookup,
            &CanvasInfo::default(),
            &view,
            layout,
            stride,
            &mut bytes,
        );
        let uniform =
            bytemuck::from_bytes::<Live2dUniform>(&bytes[..std::mem::size_of::<Live2dUniform>()]);

        assert_eq!(bytes.len(), stride as usize);
        assert_eq!(uniform.viewport, [160.0, 120.0, 160.0 / 120.0, 0.0]);
        assert_eq!(uniform.view_transform, view.transform);
        assert_eq!(uniform.effect, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn mask_atlas_signature_ignores_mask_draw_opacity() {
        let base = masked_snapshot();
        let mut changed = base.clone();
        changed.drawables[0].opacity = 0.0;
        let base_plan = RenderPlanner::new().build(&base);
        let changed_plan = RenderPlanner::new().build(&changed);
        let view = WgpuLive2DView {
            transform: [0.0, 0.0, 1.0, 0.0],
            width: 160,
            height: 120,
            effect: [1.0; 4],
            target_drawable_ids: Vec::new(),
        };
        let layout = mask_atlas_layout(160, 120, base_plan.masks.len(), 512);
        let base_lookup = MaskDrawLookup::new(&base_plan);
        let changed_lookup = MaskDrawLookup::new(&changed_plan);

        assert_eq!(
            mask_atlas_static_signature(
                &base_plan,
                &base_lookup,
                &CanvasInfo::default(),
                &view,
                layout,
            ),
            mask_atlas_static_signature(
                &changed_plan,
                &changed_lookup,
                &CanvasInfo::default(),
                &view,
                layout,
            )
        );
    }

    #[test]
    fn mask_atlas_dirty_ranges_track_only_mask_inputs() {
        let snapshot = masked_snapshot();
        let render_plan = RenderPlanner::new().build(&snapshot);

        assert!(position_uploads_touch_masks(
            &[PositionUpload {
                vertex_range: 0..1,
                byte_offset: 0,
            }],
            &render_plan,
        ));
        assert!(!position_uploads_touch_masks(
            &[PositionUpload {
                vertex_range: 1..2,
                byte_offset: std::mem::size_of::<GpuPosition>() as u64,
            }],
            &render_plan,
        ));
    }
}
