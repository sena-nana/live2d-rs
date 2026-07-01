use crate::{
    api::WgpuLive2DView,
    mask::mask_uniform,
    pipeline::{PipelineCache, PipelineKey},
    renderer::WgpuLive2DRenderer,
    resources::{GpuScene, MaskAtlas},
    upload::upload_main_uniforms,
};
use live2d_core::{BlendMode, CanvasInfo};
use live2d_render::{
    DrawCommand, Live2DRenderBackend, MaskPass, ModelRenderCtx, RenderCommand, RenderPlan,
};

pub(crate) struct WgpuRenderBackend<'a, 'pass> {
    pass: &'a mut wgpu::RenderPass<'pass>,
    pipelines: &'a PipelineCache,
    uniform_bind_group: &'a wgpu::BindGroup,
    uniform_stride: u64,
    uniform_index: usize,
    mask_bind_group: &'a wgpu::BindGroup,
    blend_bind_group: &'a wgpu::BindGroup,
    mask_atlas: Option<&'a MaskAtlas>,
    gpu_scene: &'a GpuScene,
    last_pipeline_key: Option<PipelineKey>,
    last_texture_index: Option<usize>,
}
impl<'a, 'pass> Live2DRenderBackend for WgpuRenderBackend<'a, 'pass> {
    fn begin_model(&mut self, _ctx: &ModelRenderCtx) {
        self.pass.push_debug_group("live2d model");
        self.pass.set_bind_group(2, self.mask_bind_group, &[]);
        self.pass.set_bind_group(3, self.blend_bind_group, &[]);
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
        let mask = mask_uniform(draw, self.mask_atlas);
        let masked = mask[3] != 0.0;
        let pipeline_key = self.pipelines.mesh_key(draw.blend_mode, masked);
        if self.last_pipeline_key != Some(pipeline_key) {
            self.pass
                .set_pipeline(self.pipelines.pipeline(pipeline_key));
            self.last_pipeline_key = Some(pipeline_key);
        }
        let uniform_offset = self.uniform_stride * self.uniform_index as u64;
        self.uniform_index += 1;
        self.pass
            .set_bind_group(0, self.uniform_bind_group, &[uniform_offset as u32]);
        if self.last_texture_index != Some(draw.texture_index) {
            self.pass.set_bind_group(1, texture, &[]);
            self.last_texture_index = Some(draw.texture_index);
        }
        self.pass.insert_debug_marker(draw.drawable_id.as_ref());
        self.pass
            .draw_indexed(draw.index_range.clone(), base_vertex, 0..1);
    }

    fn end_model(&mut self) {
        self.pass.pop_debug_group();
        self.pass.pop_debug_group();
    }
}

impl WgpuLive2DRenderer {
    pub(crate) fn encode_main_draws_to_target(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        resolve_target: Option<&wgpu::TextureView>,
        first_load_op: wgpu::LoadOp<wgpu::Color>,
        store_op: wgpu::StoreOp,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: WgpuLive2DView,
        first_uniform_slot: usize,
    ) {
        if render_plan
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::BeginOffscreen { .. }))
        {
            self.encode_command_draws_to_target(
                device,
                queue,
                encoder,
                target_view,
                resolve_target,
                first_load_op,
                store_op,
                render_plan,
                canvas,
                view,
                first_uniform_slot,
            );
            return;
        }

        if render_plan.draws.is_empty() {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Live2D Empty Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: first_load_op,
                        store: store_op,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            return;
        }

        if render_plan_has_advanced_blend(render_plan) {
            self.ensure_blend_copy_target(
                device,
                self.pipelines.target_format,
                view.width,
                view.height,
            );
        }

        let Some(gpu_scene) = self.active_gpu_scene() else {
            return;
        };
        let active_mask_atlas = if render_plan.masks.is_empty() {
            None
        } else {
            self.mask_atlas.as_ref()
        };
        let mask_bind_group = active_mask_atlas
            .map(|atlas| &atlas.bind_group)
            .unwrap_or(&self.fallback_mask_bind_group);
        {
            let mut uniform_staging = self.uniform_staging.borrow_mut();
            upload_main_uniforms(
                queue,
                &self.uniform_buffer,
                self.uniform_stride,
                first_uniform_slot,
                render_plan,
                canvas,
                &view,
                self.texture_sampling,
                active_mask_atlas,
                &mut uniform_staging,
            );
        }

        let extent = wgpu::Extent3d {
            width: view.width.max(1),
            height: view.height.max(1),
            depth_or_array_layers: 1,
        };
        let mut load_op = first_load_op;
        if matches!(render_plan.draws[0].blend_mode, BlendMode::Advanced { .. }) {
            if let wgpu::LoadOp::Clear(color) = load_op {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Live2D Advanced Blend Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        depth_slice: None,
                        resolve_target,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(color),
                            store: store_op,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                load_op = wgpu::LoadOp::Load;
            }
        }
        let mut start = 0;
        while start < render_plan.draws.len() {
            let advanced = matches!(
                render_plan.draws[start].blend_mode,
                BlendMode::Advanced { .. }
            );
            let end = if advanced {
                start + 1
            } else {
                render_plan.draws[start..]
                    .iter()
                    .position(|draw| matches!(draw.blend_mode, BlendMode::Advanced { .. }))
                    .map(|offset| start + offset)
                    .unwrap_or(render_plan.draws.len())
            };

            let blend_bind_group = if advanced {
                let blend_target = self
                    .blend_copy_target
                    .as_ref()
                    .expect("advanced blend copy target is created before encoding");
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: target_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &blend_target.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    extent,
                );
                &blend_target.bind_group
            } else {
                &self.fallback_blend_bind_group
            };

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Live2D Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: load_op,
                        store: store_op,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut backend = WgpuRenderBackend {
                pass: &mut pass,
                pipelines: &self.pipelines,
                uniform_bind_group: &self.uniform_bind_group,
                uniform_stride: self.uniform_stride,
                uniform_index: first_uniform_slot + start,
                mask_bind_group,
                blend_bind_group,
                mask_atlas: active_mask_atlas,
                gpu_scene,
                last_pipeline_key: None,
                last_texture_index: None,
            };
            backend.begin_model(&render_plan.model);
            backend.begin_main_pass();
            for draw in &render_plan.draws[start..end] {
                backend.draw_drawable(draw);
            }
            backend.end_model();
            drop(pass);

            load_op = wgpu::LoadOp::Load;
            start = end;
        }
    }

    fn encode_command_draws_to_target(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        resolve_target: Option<&wgpu::TextureView>,
        first_load_op: wgpu::LoadOp<wgpu::Color>,
        store_op: wgpu::StoreOp,
        render_plan: &RenderPlan,
        canvas: &CanvasInfo,
        view: WgpuLive2DView,
        first_uniform_slot: usize,
    ) {
        if render_plan.draws.is_empty() {
            return;
        }
        self.ensure_model_offscreen_targets(
            device,
            view.width,
            view.height,
            render_plan.offscreens.len(),
        );

        let Some(gpu_scene) = self.active_gpu_scene() else {
            return;
        };
        let active_mask_atlas = if render_plan.masks.is_empty() {
            None
        } else {
            self.mask_atlas.as_ref()
        };
        let mask_bind_group = active_mask_atlas
            .map(|atlas| &atlas.bind_group)
            .unwrap_or(&self.fallback_mask_bind_group);
        {
            let mut uniform_staging = self.uniform_staging.borrow_mut();
            upload_main_uniforms(
                queue,
                &self.uniform_buffer,
                self.uniform_stride,
                first_uniform_slot,
                render_plan,
                canvas,
                &view,
                self.texture_sampling,
                active_mask_atlas,
                &mut uniform_staging,
            );
        }

        let mut active_offscreens = Vec::new();
        let mut pending_draws = Vec::new();
        let mut root_loaded = false;
        for command in &render_plan.commands {
            match *command {
                RenderCommand::BeginOffscreen { offscreen_index } => {
                    flush_command_draws(
                        encoder,
                        target_view,
                        resolve_target,
                        first_load_op,
                        store_op,
                        &mut root_loaded,
                        active_offscreens.last().copied(),
                        &self.model_offscreen_targets,
                        &mut pending_draws,
                        render_plan,
                        first_uniform_slot,
                        &self.pipelines,
                        &self.uniform_bind_group,
                        self.uniform_stride,
                        mask_bind_group,
                        &self.fallback_blend_bind_group,
                        active_mask_atlas,
                        gpu_scene,
                    );
                    if let Some(offscreen) = self.model_offscreen_targets.get(&offscreen_index) {
                        clear_offscreen(encoder, &offscreen.view);
                        active_offscreens.push(offscreen_index);
                    }
                }
                RenderCommand::Draw { draw_index } => {
                    if render_plan.draws.get(draw_index).is_some() {
                        pending_draws.push(draw_index);
                    }
                }
                RenderCommand::CompositeOffscreen { offscreen_index } => {
                    flush_command_draws(
                        encoder,
                        target_view,
                        resolve_target,
                        first_load_op,
                        store_op,
                        &mut root_loaded,
                        active_offscreens.last().copied(),
                        &self.model_offscreen_targets,
                        &mut pending_draws,
                        render_plan,
                        first_uniform_slot,
                        &self.pipelines,
                        &self.uniform_bind_group,
                        self.uniform_stride,
                        mask_bind_group,
                        &self.fallback_blend_bind_group,
                        active_mask_atlas,
                        gpu_scene,
                    );
                    if active_offscreens.last().copied() == Some(offscreen_index) {
                        active_offscreens.pop();
                    }
                    let Some(offscreen) = self.model_offscreen_targets.get(&offscreen_index) else {
                        continue;
                    };
                    let Some(offscreen_plan) = render_plan.offscreens.get(offscreen_index) else {
                        continue;
                    };
                    if offscreen_plan.opacity <= 1e-6 {
                        continue;
                    }
                    let composite_pipeline = self
                        .offscreen_composite_pipelines
                        .pipeline(offscreen_plan.blend_mode);
                    if let Some(parent_index) = active_offscreens.last().copied() {
                        if let Some(parent) = self.model_offscreen_targets.get(&parent_index) {
                            encode_offscreen_composite(
                                queue,
                                encoder,
                                &parent.view,
                                None,
                                wgpu::LoadOp::Load,
                                wgpu::StoreOp::Store,
                                composite_pipeline,
                                &offscreen.bind_group,
                                &offscreen.composite_uniform_buffer,
                                &offscreen.composite_uniform_bind_group,
                                offscreen_plan.opacity,
                            );
                        }
                    } else {
                        let load_op = root_load_op(first_load_op, &mut root_loaded);
                        encode_offscreen_composite(
                            queue,
                            encoder,
                            target_view,
                            resolve_target,
                            load_op,
                            store_op,
                            composite_pipeline,
                            &offscreen.bind_group,
                            &offscreen.composite_uniform_buffer,
                            &offscreen.composite_uniform_bind_group,
                            offscreen_plan.opacity,
                        );
                    }
                }
            }
        }
        flush_command_draws(
            encoder,
            target_view,
            resolve_target,
            first_load_op,
            store_op,
            &mut root_loaded,
            active_offscreens.last().copied(),
            &self.model_offscreen_targets,
            &mut pending_draws,
            render_plan,
            first_uniform_slot,
            &self.pipelines,
            &self.uniform_bind_group,
            self.uniform_stride,
            mask_bind_group,
            &self.fallback_blend_bind_group,
            active_mask_atlas,
            gpu_scene,
        );

        if !root_loaded {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Live2D Empty Root Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: first_load_op,
                        store: store_op,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
    }
}

fn clear_offscreen(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Live2D Offscreen Clear Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
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
}

#[allow(clippy::too_many_arguments)]
fn flush_command_draws(
    encoder: &mut wgpu::CommandEncoder,
    root_target_view: &wgpu::TextureView,
    root_resolve_target: Option<&wgpu::TextureView>,
    first_load_op: wgpu::LoadOp<wgpu::Color>,
    store_op: wgpu::StoreOp,
    root_loaded: &mut bool,
    active_offscreen: Option<usize>,
    offscreen_targets: &std::collections::HashMap<usize, crate::resources::OffscreenTarget>,
    pending_draws: &mut Vec<usize>,
    render_plan: &RenderPlan,
    first_uniform_slot: usize,
    pipelines: &PipelineCache,
    uniform_bind_group: &wgpu::BindGroup,
    uniform_stride: u64,
    mask_bind_group: &wgpu::BindGroup,
    blend_bind_group: &wgpu::BindGroup,
    mask_atlas: Option<&MaskAtlas>,
    gpu_scene: &GpuScene,
) {
    if pending_draws.is_empty() {
        return;
    }
    if let Some(offscreen_index) = active_offscreen {
        if let Some(offscreen) = offscreen_targets.get(&offscreen_index) {
            encode_draw_run(
                encoder,
                &offscreen.view,
                None,
                wgpu::LoadOp::Load,
                wgpu::StoreOp::Store,
                pending_draws,
                render_plan,
                first_uniform_slot,
                pipelines,
                uniform_bind_group,
                uniform_stride,
                mask_bind_group,
                blend_bind_group,
                mask_atlas,
                gpu_scene,
            );
        }
    } else {
        let load_op = root_load_op(first_load_op, root_loaded);
        encode_draw_run(
            encoder,
            root_target_view,
            root_resolve_target,
            load_op,
            store_op,
            pending_draws,
            render_plan,
            first_uniform_slot,
            pipelines,
            uniform_bind_group,
            uniform_stride,
            mask_bind_group,
            blend_bind_group,
            mask_atlas,
            gpu_scene,
        );
    }
    pending_draws.clear();
}

fn root_load_op(
    first_load_op: wgpu::LoadOp<wgpu::Color>,
    root_loaded: &mut bool,
) -> wgpu::LoadOp<wgpu::Color> {
    if *root_loaded {
        wgpu::LoadOp::Load
    } else {
        *root_loaded = true;
        first_load_op
    }
}

#[allow(clippy::too_many_arguments)]
fn encode_draw_run(
    encoder: &mut wgpu::CommandEncoder,
    target_view: &wgpu::TextureView,
    resolve_target: Option<&wgpu::TextureView>,
    load_op: wgpu::LoadOp<wgpu::Color>,
    store_op: wgpu::StoreOp,
    draw_indices: &[usize],
    render_plan: &RenderPlan,
    first_uniform_slot: usize,
    pipelines: &PipelineCache,
    uniform_bind_group: &wgpu::BindGroup,
    uniform_stride: u64,
    mask_bind_group: &wgpu::BindGroup,
    blend_bind_group: &wgpu::BindGroup,
    mask_atlas: Option<&MaskAtlas>,
    gpu_scene: &GpuScene,
) {
    if draw_indices.is_empty() {
        return;
    }
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Live2D Command Draw Run Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target_view,
            depth_slice: None,
            resolve_target,
            ops: wgpu::Operations {
                load: load_op,
                store: store_op,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let mut backend = WgpuRenderBackend {
        pass: &mut pass,
        pipelines,
        uniform_bind_group,
        uniform_stride,
        uniform_index: first_uniform_slot,
        mask_bind_group,
        blend_bind_group,
        mask_atlas,
        gpu_scene,
        last_pipeline_key: None,
        last_texture_index: None,
    };
    backend.begin_model(&render_plan.model);
    backend.begin_main_pass();
    for draw_index in draw_indices {
        if let Some(draw) = render_plan.draws.get(*draw_index) {
            backend.uniform_index = first_uniform_slot + *draw_index;
            backend.draw_drawable(draw);
        }
    }
    backend.end_model();
}

fn encode_offscreen_composite(
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    target_view: &wgpu::TextureView,
    resolve_target: Option<&wgpu::TextureView>,
    load_op: wgpu::LoadOp<wgpu::Color>,
    store_op: wgpu::StoreOp,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    uniform_buffer: &wgpu::Buffer,
    uniform_bind_group: &wgpu::BindGroup,
    opacity: f32,
) {
    queue.write_buffer(
        uniform_buffer,
        0,
        bytemuck::cast_slice(&[opacity.clamp(0.0, 1.0), 0.0, 0.0, 0.0]),
    );
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Live2D Offscreen Composite Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target_view,
            depth_slice: None,
            resolve_target,
            ops: wgpu::Operations {
                load: load_op,
                store: store_op,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.set_bind_group(1, uniform_bind_group, &[]);
    pass.draw(0..3, 0..1);
}

pub(crate) fn render_plan_has_advanced_blend(render_plan: &RenderPlan) -> bool {
    render_plan
        .draws
        .iter()
        .any(|draw| matches!(draw.blend_mode, BlendMode::Advanced { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{blend_uniform, pipeline_blend_mode, PipelineBlendMode};
    use crate::tests::*;
    use live2d_core::{AlphaBlendMode, BlendMode, ColorBlendMode};
    use live2d_render::RenderPlanner;

    #[test]
    fn render_plan_reports_advanced_blend_draws() {
        let mut snapshot = masked_snapshot();
        snapshot.drawables[1].blend_mode = BlendMode::Advanced {
            color: ColorBlendMode::Multiply,
            alpha: AlphaBlendMode::Atop,
        };
        let render_plan = RenderPlanner::new().build(&snapshot);
        let advanced_draw = render_plan
            .draws
            .iter()
            .find(|draw| matches!(draw.blend_mode, BlendMode::Advanced { .. }))
            .expect("advanced drawable is present");

        assert!(render_plan_has_advanced_blend(&render_plan));
        assert_eq!(
            pipeline_blend_mode(advanced_draw.blend_mode),
            PipelineBlendMode::Advanced
        );
        assert_eq!(blend_uniform(advanced_draw.blend_mode), [6, 1, 0, 0]);
    }
}
