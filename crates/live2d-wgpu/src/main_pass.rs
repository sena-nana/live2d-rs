use crate::{
    api::WgpuLive2DView,
    mask::mask_uniform,
    pipeline::{PipelineCache, PipelineKey},
    renderer::WgpuLive2DRenderer,
    resources::{GpuScene, MaskAtlas},
    upload::upload_main_uniforms,
};
use live2d_core::{BlendMode, CanvasInfo};
use live2d_render::{DrawCommand, Live2DRenderBackend, MaskPass, ModelRenderCtx, RenderPlan};

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
