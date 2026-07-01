use live2d_core::{
    AlphaBlendMode, BlendMode, CanvasInfo, ClippingInfo, ColorBlendMode, Drawable, DrawableId,
    ModelSnapshot, TextureAsset, Vertex,
};
use live2d_probe::{ProbeRecorder, RunReport};
use live2d_render::{
    DrawCommand, Live2DRenderBackend, MaskPass, ModelRenderCtx, RenderPlanner, RenderWorld,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[cfg(feature = "wgpu")]
pub mod wgpu_scenarios {
    use super::*;
    use live2d_probe::{counter, measure, ProbeRecorder, Stage};
    use live2d_render::{PostProcessPlan, PostProcessShaderId};
    use live2d_wgpu::{
        WgpuLive2DRenderer, WgpuLive2DView, WgpuPostProcessChain, WgpuPostProcessPlan,
        WgpuPostProcessShaderSource,
    };
    use std::time::Instant;

    const TONE_POSTPROCESS_WGSL: &str = r#"
fn pp_apply(fragment: PpFragment) -> vec4<f32> {
    let color = pp_sample(fragment.uv);
    let gain = 1.0 + pp_param(0u).x;
    return vec4<f32>(color.rgb * gain, color.a);
}
"#;

    const NEIGHBOR_POSTPROCESS_WGSL: &str = r#"
fn pp_apply(fragment: PpFragment) -> vec4<f32> {
    let c = pp_sample(fragment.uv);
    let l = pp_sample(fragment.uv - vec2<f32>(fragment.texel.x, 0.0));
    let r = pp_sample(fragment.uv + vec2<f32>(fragment.texel.x, 0.0));
    let u = pp_sample(fragment.uv - vec2<f32>(0.0, fragment.texel.y));
    let d = pp_sample(fragment.uv + vec2<f32>(0.0, fragment.texel.y));
    return (c * 0.5) + ((l + r + u + d) * 0.125);
}
"#;

    pub fn run_wgpu_scenario(
        scenario: &str,
        config: &SyntheticConfig,
    ) -> Result<RunReport, String> {
        pollster::block_on(run_wgpu_scenario_async(scenario, config))
    }

    async fn run_wgpu_scenario_async(
        scenario: &str,
        config: &SyntheticConfig,
    ) -> Result<RunReport, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|err| format!("wgpu adapter unavailable: {err}"))?;
        let adapter_features = adapter.features();
        let features = adapter_features & wgpu::Features::TIMESTAMP_QUERY;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Live2D Perf Device"),
                required_features: features,
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|err| format!("wgpu device unavailable: {err}"))?;
        let recorder = ProbeRecorder::new();
        let mut renderer = WgpuLive2DRenderer::new_with_probe(
            &device,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &recorder,
        );
        let mut postprocess = if scenario == "wgpu-postprocess" {
            let plan = PostProcessPlan::linear(["tone", "neighbor"]);
            let wgpu_plan = WgpuPostProcessPlan::from_render_plan(
                &plan,
                [
                    (
                        PostProcessShaderId::from("tone"),
                        WgpuPostProcessShaderSource::Wgsl(TONE_POSTPROCESS_WGSL),
                    ),
                    (
                        PostProcessShaderId::from("neighbor"),
                        WgpuPostProcessShaderSource::Wgsl(NEIGHBOR_POSTPROCESS_WGSL),
                    ),
                ],
            )
            .map_err(|err| format!("failed to build postprocess plan: {err:?}"))?;
            Some(WgpuPostProcessChain::new(
                &device,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                &wgpu_plan,
            ))
        } else {
            None
        };
        let frames = match scenario {
            "wgpu-cold" => 1,
            "wgpu-resize" => config.frames.max(2).min(8),
            _ => config.frames.max(1),
        };
        let warnings = if features.contains(wgpu::Features::TIMESTAMP_QUERY) {
            Vec::new()
        } else {
            vec!["gpu timestamp queries are unsupported; report uses CPU encode and submit-to-complete timing".to_owned()]
        };

        for frame in 0..frames {
            let mut scenario_config = config.clone();
            if scenario == "wgpu-mask" {
                scenario_config.mask_groups = scenario_config.mask_groups.max(16);
                scenario_config.mask_members = scenario_config.mask_members.max(6);
            }
            let mut snapshot = synthetic_snapshot(&scenario_config, frame);
            if scenario == "wgpu-model-switch" {
                snapshot.model_key = format!("synthetic-wgpu-switch-{}", frame % 2);
            }
            let extent = if scenario == "wgpu-resize" {
                256 + frame as u32 * 32
            } else {
                config.canvas_size[0].max(1.0) as u32 * 256
            };
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Live2D Perf Target"),
                size: wgpu::Extent3d {
                    width: extent.max(1),
                    height: extent.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view_texture = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Live2D Perf Encoder"),
            });
            let view = WgpuLive2DView {
                transform: [1.0, 0.0, 0.0, 1.0],
                width: extent.max(1),
                height: extent.max(1),
                effect: [1.0, 1.0, 1.0, 1.0],
                target_drawable_ids: if scenario == "wgpu-warm" {
                    Vec::new()
                } else {
                    target_drawable_ids(config)
                },
            };
            if scenario == "wgpu-resize" {
                renderer.render_to_offscreen_with_probe(
                    &device,
                    &queue,
                    &mut encoder,
                    &snapshot,
                    view,
                    wgpu::Color::TRANSPARENT,
                    &recorder,
                );
            } else if let Some(postprocess) = postprocess.as_mut() {
                renderer
                    .render_with_postprocess_to_view_with_probe(
                        &device,
                        &queue,
                        &mut encoder,
                        live2d_wgpu::WgpuLive2DTarget::clear(
                            &target,
                            &view_texture,
                            wgpu::Color::TRANSPARENT,
                        ),
                        &snapshot,
                        view,
                        postprocess,
                        &recorder,
                    )
                    .map_err(|err| format!("postprocess render failed: {err:?}"))?;
            } else {
                renderer.render_to_view_with_probe(
                    &device,
                    &queue,
                    &mut encoder,
                    live2d_wgpu::WgpuLive2DTarget::clear(
                        &target,
                        &view_texture,
                        wgpu::Color::TRANSPARENT,
                    ),
                    &snapshot,
                    view,
                    &recorder,
                );
            }
            let command_buffer = encoder.finish();
            let started = Instant::now();
            measure(&recorder, Stage::WgpuQueueSubmit, Vec::new(), || {
                queue.submit([command_buffer]);
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
            });
            renderer.collect_gpu_timestamps_with_probe(&device, &queue, &recorder)?;
            counter(
                &recorder,
                Stage::WgpuQueueSubmit,
                "submit_to_complete_nanos",
                started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                Vec::new(),
            );
        }

        Ok(recorder.report(scenario, config.as_report_config(), warnings))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticConfig {
    pub drawables: usize,
    pub vertices_per_drawable: usize,
    pub indices_per_drawable: usize,
    pub textures: usize,
    pub texture_size: u32,
    pub mask_groups: usize,
    pub mask_members: usize,
    pub animated_ratio: f32,
    pub static_masks: bool,
    pub target_drawables: usize,
    pub frames: usize,
    pub canvas_size: [f32; 2],
}

impl SyntheticConfig {
    pub fn small() -> Self {
        Self {
            drawables: 32,
            vertices_per_drawable: 8,
            indices_per_drawable: 18,
            textures: 1,
            texture_size: 64,
            mask_groups: 2,
            mask_members: 2,
            animated_ratio: 0.25,
            static_masks: false,
            target_drawables: 4,
            frames: 60,
            canvas_size: [2.0, 2.0],
        }
    }

    pub fn medium() -> Self {
        Self {
            drawables: 128,
            vertices_per_drawable: 16,
            indices_per_drawable: 42,
            textures: 4,
            texture_size: 256,
            mask_groups: 8,
            mask_members: 4,
            animated_ratio: 0.5,
            static_masks: false,
            target_drawables: 12,
            frames: 180,
            canvas_size: [2.0, 2.0],
        }
    }

    pub fn large() -> Self {
        Self {
            drawables: 512,
            vertices_per_drawable: 24,
            indices_per_drawable: 72,
            textures: 8,
            texture_size: 512,
            mask_groups: 32,
            mask_members: 8,
            animated_ratio: 0.75,
            static_masks: false,
            target_drawables: 32,
            frames: 300,
            canvas_size: [2.0, 2.0],
        }
    }

    pub fn mask_heavy() -> Self {
        Self {
            mask_groups: 64,
            mask_members: 12,
            ..Self::medium()
        }
    }

    pub fn static_mask_heavy() -> Self {
        Self {
            static_masks: true,
            ..Self::mask_heavy()
        }
    }

    pub fn texture_heavy() -> Self {
        Self {
            textures: 12,
            texture_size: 1024,
            ..Self::medium()
        }
    }

    pub fn target_filter() -> Self {
        Self {
            target_drawables: 48,
            ..Self::medium()
        }
    }

    pub fn from_profile(profile: &str) -> Self {
        match profile {
            "small" => Self::small(),
            "large" => Self::large(),
            "mask-heavy" => Self::mask_heavy(),
            "static-mask-heavy" => Self::static_mask_heavy(),
            "texture-heavy" => Self::texture_heavy(),
            "target-filter" => Self::target_filter(),
            _ => Self::medium(),
        }
    }

    pub fn as_report_config(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("drawables".into(), self.drawables.to_string()),
            (
                "vertices_per_drawable".into(),
                self.vertices_per_drawable.to_string(),
            ),
            (
                "indices_per_drawable".into(),
                self.indices_per_drawable.to_string(),
            ),
            ("textures".into(), self.textures.to_string()),
            ("texture_size".into(), self.texture_size.to_string()),
            ("mask_groups".into(), self.mask_groups.to_string()),
            ("mask_members".into(), self.mask_members.to_string()),
            ("animated_ratio".into(), self.animated_ratio.to_string()),
            ("static_masks".into(), self.static_masks.to_string()),
            ("target_drawables".into(), self.target_drawables.to_string()),
            ("frames".into(), self.frames.to_string()),
            (
                "canvas_size".into(),
                format!("{},{}", self.canvas_size[0], self.canvas_size[1]),
            ),
        ])
    }
}

pub fn synthetic_snapshot(config: &SyntheticConfig, frame: usize) -> ModelSnapshot {
    let texture_count = config.textures.max(1);
    let mask_count = config.mask_groups.min(config.drawables);
    let animated_drawables = ((config.drawables as f32 * config.animated_ratio.clamp(0.0, 1.0))
        .round() as usize)
        .min(config.drawables);
    let drawables = (0..config.drawables)
        .map(|index| {
            let id = DrawableId::from(format!("drawable_{index:04}"));
            let mask_group = if index >= mask_count && mask_count > 0 {
                Some(index % mask_count)
            } else {
                None
            };
            let clipping = mask_group.map(|group| ClippingInfo {
                drawable_ids: (0..config.mask_members.max(1))
                    .map(|member| {
                        DrawableId::from(format!("drawable_{:04}", (group + member) % mask_count))
                    })
                    .collect(),
                inverted: group % 3 == 0,
            });
            Drawable {
                id,
                render_order: index as i32,
                texture_index: index % texture_count,
                vertices: synthetic_vertices(
                    config.vertices_per_drawable,
                    index,
                    frame,
                    index < animated_drawables && !(config.static_masks && index < mask_count),
                ),
                indices: synthetic_indices(
                    config.indices_per_drawable,
                    config.vertices_per_drawable,
                ),
                visible: true,
                opacity: 1.0,
                blend_mode: match index % 11 {
                    0 => BlendMode::Additive,
                    1 => BlendMode::Multiplicative,
                    2 => BlendMode::Advanced {
                        color: ColorBlendMode::Multiply,
                        alpha: AlphaBlendMode::Over,
                    },
                    _ => BlendMode::Normal,
                },
                clipping,
            }
        })
        .collect();
    ModelSnapshot {
        model_key: format!("synthetic-{}", config.drawables),
        canvas: CanvasInfo {
            size: config.canvas_size,
            origin: [0.0, 0.0],
            pixels_per_unit: 1.0,
        },
        art_meshes: Vec::new(),
        drawables,
        textures: synthetic_textures(texture_count, config.texture_size),
    }
}

pub fn target_drawable_ids(config: &SyntheticConfig) -> Vec<String> {
    (0..config.target_drawables.min(config.drawables))
        .map(|index| format!("drawable_{index:04}"))
        .collect()
}

pub fn run_render_plan(config: &SyntheticConfig) -> RunReport {
    let recorder = ProbeRecorder::new();
    let planner = RenderPlanner::new();
    for frame in 0..config.frames.max(1) {
        let snapshot = synthetic_snapshot(config, frame);
        let _ = planner.build_with_probe(&snapshot, &recorder);
    }
    recorder.report(
        "synthetic-render-plan",
        config.as_report_config(),
        Vec::new(),
    )
}

pub fn run_render_world_switch(config: &SyntheticConfig) -> RunReport {
    let recorder = ProbeRecorder::new();
    let mut world = RenderWorld::new();
    for frame in 0..config.frames.max(1) {
        let mut snapshot = synthetic_snapshot(config, frame);
        snapshot.model_key = format!("synthetic-switch-{}", frame % 2);
        let _ = world.build_with_probe(&snapshot, &recorder);
    }
    recorder.report("render-world-switch", config.as_report_config(), Vec::new())
}

pub fn run_dispatch_null_backend(config: &SyntheticConfig) -> (RunReport, CountingBackend) {
    let recorder = ProbeRecorder::new();
    let planner = RenderPlanner::new();
    let mut backend = CountingBackend::default();
    for frame in 0..config.frames.max(1) {
        let snapshot = synthetic_snapshot(config, frame);
        let plan = planner.build_with_probe(&snapshot, &recorder);
        plan.dispatch_with_probe(&mut backend, &recorder);
    }
    (
        recorder.report(
            "dispatch-null-backend",
            config.as_report_config(),
            Vec::new(),
        ),
        backend,
    )
}

pub fn run_real_model_load(model_path: &Path) -> RunReport {
    let recorder = ProbeRecorder::new();
    let warnings = match live2d_runtime::load_snapshot_with_probe(model_path, &recorder) {
        Ok(_) => Vec::new(),
        Err(err) => vec![format!("real model load failed: {err}")],
    };
    recorder.report(
        "real-model-load",
        BTreeMap::from([("model".into(), model_path.display().to_string())]),
        warnings,
    )
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountingBackend {
    pub begin_models: u64,
    pub mask_passes: u64,
    pub mask_draws: u64,
    pub main_draws: u64,
}

impl Live2DRenderBackend for CountingBackend {
    fn begin_model(&mut self, _ctx: &ModelRenderCtx) {
        self.begin_models += 1;
    }

    fn begin_clip_mask(&mut self, _mask: &MaskPass) {
        self.mask_passes += 1;
    }

    fn draw_mask_drawable(&mut self, _mask: &MaskPass, _call: &DrawCommand) {
        self.mask_draws += 1;
    }

    fn draw_drawable(&mut self, _call: &DrawCommand) {
        self.main_draws += 1;
    }
}

fn synthetic_vertices(
    count: usize,
    drawable_index: usize,
    frame: usize,
    animated: bool,
) -> Vec<Vertex> {
    let count = count.max(1);
    let phase = if animated {
        frame as f32 * 0.01 + drawable_index as f32 * 0.001
    } else {
        0.0
    };
    (0..count)
        .map(|index| {
            let t = index as f32 / count as f32;
            Vertex {
                position: [
                    (drawable_index as f32 * 0.001) + t + phase.sin() * 0.01,
                    (t * std::f32::consts::TAU + phase).sin() * 0.5,
                ],
                uv: [t, 1.0 - t],
            }
        })
        .collect()
}

fn synthetic_indices(count: usize, vertex_count: usize) -> Vec<u16> {
    let vertex_count = vertex_count.max(1).min(u16::MAX as usize);
    (0..count.max(1))
        .map(|index| (index % vertex_count) as u16)
        .collect()
}

fn synthetic_textures(count: usize, size: u32) -> Vec<TextureAsset> {
    let size = size.max(1);
    (0..count.max(1))
        .map(|texture_index| {
            let mut rgba = Vec::with_capacity((size * size * 4) as usize);
            for pixel in 0..(size * size) {
                rgba.push(((pixel + texture_index as u32) % 255) as u8);
                rgba.push(((pixel / size) % 255) as u8);
                rgba.push((texture_index as u8).wrapping_mul(31));
                rgba.push(255);
            }
            TextureAsset {
                width: size,
                height: size,
                rgba,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_snapshot_matches_requested_scale() {
        let config = SyntheticConfig {
            drawables: 10,
            vertices_per_drawable: 7,
            indices_per_drawable: 9,
            textures: 3,
            texture_size: 4,
            mask_groups: 2,
            mask_members: 2,
            animated_ratio: 0.5,
            static_masks: false,
            target_drawables: 4,
            frames: 1,
            canvas_size: [3.0, 4.0],
        };

        let snapshot = synthetic_snapshot(&config, 0);

        assert_eq!(snapshot.drawables.len(), 10);
        assert_eq!(snapshot.textures.len(), 3);
        assert_eq!(snapshot.textures[0].rgba.len(), 4 * 4 * 4);
        assert!(snapshot
            .drawables
            .iter()
            .all(|drawable| drawable.vertices.len() == 7));
        assert!(snapshot
            .drawables
            .iter()
            .all(|drawable| drawable.indices.len() == 9));
        assert_eq!(
            snapshot
                .drawables
                .iter()
                .filter(|drawable| drawable.clipping.is_some())
                .count(),
            8
        );
    }

    #[test]
    fn dispatch_null_backend_counts_render_plan_work() {
        let config = SyntheticConfig {
            drawables: 12,
            mask_groups: 2,
            mask_members: 2,
            frames: 1,
            ..SyntheticConfig::small()
        };

        let (_report, backend) = run_dispatch_null_backend(&config);

        assert_eq!(backend.begin_models, 1);
        assert_eq!(backend.main_draws, 12);
        assert_eq!(backend.mask_passes, 2);
        assert_eq!(backend.mask_draws, 4);
    }
}
