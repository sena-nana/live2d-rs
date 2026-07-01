use live2d_core::{
    AlphaBlendMode, BlendMode, CanvasInfo, ClippingInfo, ColorBlendMode, Drawable, DrawableId,
    ModelSnapshot, TextureAsset, Vertex,
};
use live2d_probe::{counter, ProbeAttr, ProbeRecorder, RunReport, Stage, StageStats};
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
            record_snapshot_blend_counters(&recorder, Stage::WgpuMainPassEncode, &snapshot);
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
    pub blend_profile: SyntheticBlendProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyntheticBlendProfile {
    ClassicMix,
    AdvancedColors,
    AdvancedAlphas,
    AdvancedMatrix,
    AllModes,
}

impl Default for SyntheticBlendProfile {
    fn default() -> Self {
        Self::ClassicMix
    }
}

impl SyntheticBlendProfile {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "classic-mix" => Some(Self::ClassicMix),
            "advanced-colors" => Some(Self::AdvancedColors),
            "advanced-alphas" => Some(Self::AdvancedAlphas),
            "advanced-matrix" => Some(Self::AdvancedMatrix),
            "all-modes" => Some(Self::AllModes),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClassicMix => "classic-mix",
            Self::AdvancedColors => "advanced-colors",
            Self::AdvancedAlphas => "advanced-alphas",
            Self::AdvancedMatrix => "advanced-matrix",
            Self::AllModes => "all-modes",
        }
    }

    pub fn minimum_coverage_drawables(self) -> usize {
        match self {
            Self::ClassicMix => 0,
            Self::AdvancedColors => COLOR_BLEND_MODES.len(),
            Self::AdvancedAlphas => ALPHA_BLEND_MODES.len(),
            Self::AdvancedMatrix => COLOR_BLEND_MODES.len() * ALPHA_BLEND_MODES.len(),
            Self::AllModes => 3 + COLOR_BLEND_MODES.len() * ALPHA_BLEND_MODES.len(),
        }
    }
}

const COLOR_BLEND_MODES: [ColorBlendMode; 16] = [
    ColorBlendMode::Normal,
    ColorBlendMode::Add,
    ColorBlendMode::AddGlow,
    ColorBlendMode::Darken,
    ColorBlendMode::Multiply,
    ColorBlendMode::ColorBurn,
    ColorBlendMode::LinearBurn,
    ColorBlendMode::Lighten,
    ColorBlendMode::Screen,
    ColorBlendMode::ColorDodge,
    ColorBlendMode::Overlay,
    ColorBlendMode::SoftLight,
    ColorBlendMode::HardLight,
    ColorBlendMode::LinearLight,
    ColorBlendMode::Hue,
    ColorBlendMode::Color,
];

const ALPHA_BLEND_MODES: [AlphaBlendMode; 5] = [
    AlphaBlendMode::Over,
    AlphaBlendMode::Atop,
    AlphaBlendMode::Out,
    AlphaBlendMode::ConjointOver,
    AlphaBlendMode::DisjointOver,
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BlendCoverage {
    pub normal: usize,
    pub additive: usize,
    pub multiplicative: usize,
    pub advanced: usize,
    pub advanced_color_modes: usize,
    pub advanced_alpha_modes: usize,
    colors: Vec<ColorBlendMode>,
    alphas: Vec<AlphaBlendMode>,
}

impl BlendCoverage {
    fn record(&mut self, blend_mode: BlendMode) {
        match blend_mode {
            BlendMode::Normal => self.normal += 1,
            BlendMode::Additive => self.additive += 1,
            BlendMode::Multiplicative => self.multiplicative += 1,
            BlendMode::Advanced { color, alpha } => {
                self.advanced += 1;
                if !self.colors.contains(&color) {
                    self.colors.push(color);
                    self.advanced_color_modes = self.colors.len();
                }
                if !self.alphas.contains(&alpha) {
                    self.alphas.push(alpha);
                    self.advanced_alpha_modes = self.alphas.len();
                }
            }
        }
    }

    fn as_counter_values(&self) -> [(&'static str, u64); 6] {
        [
            ("blend_normal_draws", self.normal as u64),
            ("blend_additive_draws", self.additive as u64),
            ("blend_multiplicative_draws", self.multiplicative as u64),
            ("blend_advanced_draws", self.advanced as u64),
            (
                "blend_advanced_color_modes",
                self.advanced_color_modes as u64,
            ),
            (
                "blend_advanced_alpha_modes",
                self.advanced_alpha_modes as u64,
            ),
        ]
    }
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
            blend_profile: SyntheticBlendProfile::ClassicMix,
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
            blend_profile: SyntheticBlendProfile::ClassicMix,
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
            blend_profile: SyntheticBlendProfile::ClassicMix,
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
        let coverage = self.blend_coverage();
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
            (
                "blend_profile".into(),
                self.blend_profile.as_str().to_owned(),
            ),
            ("blend_normal_draws".into(), coverage.normal.to_string()),
            ("blend_additive_draws".into(), coverage.additive.to_string()),
            (
                "blend_multiplicative_draws".into(),
                coverage.multiplicative.to_string(),
            ),
            ("blend_advanced_draws".into(), coverage.advanced.to_string()),
            (
                "blend_advanced_color_modes".into(),
                coverage.advanced_color_modes.to_string(),
            ),
            (
                "blend_advanced_alpha_modes".into(),
                coverage.advanced_alpha_modes.to_string(),
            ),
        ])
    }

    pub fn with_blend_profile(mut self, blend_profile: SyntheticBlendProfile) -> Self {
        self.blend_profile = blend_profile;
        self
    }

    pub fn blend_coverage(&self) -> BlendCoverage {
        blend_coverage_for_drawables(self.blend_profile, self.drawables)
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
                blend_mode: synthetic_blend_mode(config.blend_profile, index),
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
    let count = config
        .target_drawables
        .max(config.blend_profile.minimum_coverage_drawables())
        .min(config.drawables);
    (0..count)
        .map(|index| format!("drawable_{index:04}"))
        .collect()
}

pub fn run_render_plan(config: &SyntheticConfig) -> RunReport {
    let recorder = ProbeRecorder::new();
    let planner = RenderPlanner::new();
    for frame in 0..config.frames.max(1) {
        let snapshot = synthetic_snapshot(config, frame);
        record_snapshot_blend_counters(&recorder, Stage::RenderDrawCommandBuild, &snapshot);
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
        record_snapshot_blend_counters(&recorder, Stage::RenderDrawCommandBuild, &snapshot);
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
        record_snapshot_blend_counters(&recorder, Stage::RenderDrawCommandBuild, &snapshot);
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

pub fn synthetic_blend_mode(profile: SyntheticBlendProfile, index: usize) -> BlendMode {
    match profile {
        SyntheticBlendProfile::ClassicMix => match index % 11 {
            0 => BlendMode::Additive,
            1 => BlendMode::Multiplicative,
            _ => BlendMode::Normal,
        },
        SyntheticBlendProfile::AdvancedColors => BlendMode::Advanced {
            color: COLOR_BLEND_MODES[index % COLOR_BLEND_MODES.len()],
            alpha: AlphaBlendMode::Over,
        },
        SyntheticBlendProfile::AdvancedAlphas => BlendMode::Advanced {
            color: ColorBlendMode::Multiply,
            alpha: ALPHA_BLEND_MODES[index % ALPHA_BLEND_MODES.len()],
        },
        SyntheticBlendProfile::AdvancedMatrix => advanced_matrix_blend_mode(index),
        SyntheticBlendProfile::AllModes => match index {
            0 => BlendMode::Normal,
            1 => BlendMode::Additive,
            2 => BlendMode::Multiplicative,
            _ => advanced_matrix_blend_mode(index - 3),
        },
    }
}

fn advanced_matrix_blend_mode(index: usize) -> BlendMode {
    let alpha_len = ALPHA_BLEND_MODES.len();
    let combo = index % (COLOR_BLEND_MODES.len() * alpha_len);
    BlendMode::Advanced {
        color: COLOR_BLEND_MODES[combo / alpha_len],
        alpha: ALPHA_BLEND_MODES[combo % alpha_len],
    }
}

fn blend_coverage_for_drawables(profile: SyntheticBlendProfile, drawables: usize) -> BlendCoverage {
    let mut coverage = BlendCoverage::default();
    for index in 0..drawables {
        coverage.record(synthetic_blend_mode(profile, index));
    }
    coverage
}

fn record_snapshot_blend_counters<P>(probe: &P, stage: Stage, snapshot: &ModelSnapshot)
where
    P: live2d_probe::ProbeSink,
{
    let coverage = blend_coverage_for_snapshot(snapshot);
    for (name, value) in coverage.as_counter_values() {
        counter(
            probe,
            stage,
            name,
            value,
            vec![ProbeAttr::new("source", "synthetic_snapshot")],
        );
    }
}

fn blend_coverage_for_snapshot(snapshot: &ModelSnapshot) -> BlendCoverage {
    let mut coverage = BlendCoverage::default();
    for drawable in &snapshot.drawables {
        coverage.record(drawable.blend_mode);
    }
    coverage
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareSummary {
    pub before: String,
    pub after: String,
    pub threshold_percent: f64,
    pub comparisons: Vec<StageComparison>,
    pub warnings: Vec<String>,
    pub regressions: Vec<String>,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageComparison {
    pub stage: Stage,
    pub before_total_nanos: u64,
    pub after_total_nanos: u64,
    pub total_ratio: f64,
    pub before_p90_nanos: u64,
    pub after_p90_nanos: u64,
    pub p90_ratio: f64,
    pub regressed: bool,
}

pub fn compare_reports(
    before_name: impl Into<String>,
    before: &RunReport,
    after_name: impl Into<String>,
    after: &RunReport,
    threshold_percent: f64,
    stages: &[Stage],
) -> CompareSummary {
    let before_name = before_name.into();
    let after_name = after_name.into();
    let mut comparisons = Vec::new();
    let mut warnings = Vec::new();
    let mut regressions = Vec::new();
    let threshold_ratio = 1.0 + (threshold_percent.max(0.0) / 100.0);

    for stage in stages {
        let Some(before_stats) = non_empty_stage(before, *stage) else {
            warnings.push(format!("{before_name} missing non-empty stage {stage:?}"));
            continue;
        };
        let Some(after_stats) = non_empty_stage(after, *stage) else {
            warnings.push(format!("{after_name} missing non-empty stage {stage:?}"));
            continue;
        };
        let total_ratio = ratio(after_stats.total_nanos, before_stats.total_nanos);
        let p90_ratio = ratio(after_stats.p90_nanos, before_stats.p90_nanos);
        let regressed = exceeds_threshold(
            after_stats.total_nanos,
            before_stats.total_nanos,
            threshold_ratio,
        ) || exceeds_threshold(
            after_stats.p90_nanos,
            before_stats.p90_nanos,
            threshold_ratio,
        );
        if regressed {
            regressions.push(format!(
                "{stage:?} regressed: total x{total_ratio:.3}, p90 x{p90_ratio:.3}"
            ));
        }
        comparisons.push(StageComparison {
            stage: *stage,
            before_total_nanos: before_stats.total_nanos,
            after_total_nanos: after_stats.total_nanos,
            total_ratio,
            before_p90_nanos: before_stats.p90_nanos,
            after_p90_nanos: after_stats.p90_nanos,
            p90_ratio,
            regressed,
        });
    }

    let passed = warnings.is_empty() && regressions.is_empty();
    CompareSummary {
        before: before_name,
        after: after_name,
        threshold_percent,
        comparisons,
        warnings,
        regressions,
        passed,
    }
}

fn non_empty_stage(report: &RunReport, stage: Stage) -> Option<&StageStats> {
    report
        .analysis
        .stages
        .get(&stage)
        .filter(|stats| stats.calls > 0)
}

fn ratio(after: u64, before: u64) -> f64 {
    if before == 0 {
        if after == 0 {
            1.0
        } else {
            f64::INFINITY
        }
    } else {
        after as f64 / before as f64
    }
}

fn exceeds_threshold(after: u64, before: u64, threshold_ratio: f64) -> bool {
    if before == 0 {
        after > 0
    } else {
        ratio(after, before) > threshold_ratio
    }
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
            blend_profile: SyntheticBlendProfile::ClassicMix,
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

    #[test]
    fn classic_mix_matches_pre_blend_support_distribution() {
        let config = SyntheticConfig {
            drawables: 22,
            ..SyntheticConfig::small()
        };

        let coverage = config.blend_coverage();

        assert_eq!(coverage.additive, 2);
        assert_eq!(coverage.multiplicative, 2);
        assert_eq!(coverage.normal, 18);
        assert_eq!(coverage.advanced, 0);
    }

    #[test]
    fn advanced_matrix_covers_every_advanced_combination() {
        let config = SyntheticConfig {
            drawables: SyntheticBlendProfile::AdvancedMatrix.minimum_coverage_drawables(),
            ..SyntheticConfig::small().with_blend_profile(SyntheticBlendProfile::AdvancedMatrix)
        };

        let coverage = config.blend_coverage();

        assert_eq!(
            coverage.advanced,
            COLOR_BLEND_MODES.len() * ALPHA_BLEND_MODES.len()
        );
        assert_eq!(coverage.advanced_color_modes, COLOR_BLEND_MODES.len());
        assert_eq!(coverage.advanced_alpha_modes, ALPHA_BLEND_MODES.len());
        assert_eq!(
            coverage.normal + coverage.additive + coverage.multiplicative,
            0
        );
    }

    #[test]
    fn all_modes_covers_classic_and_advanced_modes() {
        let config = SyntheticConfig {
            drawables: SyntheticBlendProfile::AllModes.minimum_coverage_drawables(),
            ..SyntheticConfig::small().with_blend_profile(SyntheticBlendProfile::AllModes)
        };

        let coverage = config.blend_coverage();

        assert_eq!(coverage.normal, 1);
        assert_eq!(coverage.additive, 1);
        assert_eq!(coverage.multiplicative, 1);
        assert_eq!(
            coverage.advanced,
            COLOR_BLEND_MODES.len() * ALPHA_BLEND_MODES.len()
        );
        assert_eq!(coverage.advanced_color_modes, COLOR_BLEND_MODES.len());
        assert_eq!(coverage.advanced_alpha_modes, ALPHA_BLEND_MODES.len());
    }

    #[test]
    fn target_drawables_include_coverage_modes() {
        let config = SyntheticConfig {
            drawables: SyntheticBlendProfile::AllModes.minimum_coverage_drawables(),
            target_drawables: 4,
            ..SyntheticConfig::small().with_blend_profile(SyntheticBlendProfile::AllModes)
        };

        let ids = target_drawable_ids(&config);

        assert_eq!(
            ids.len(),
            SyntheticBlendProfile::AllModes.minimum_coverage_drawables()
        );
    }

    #[test]
    fn compare_reports_allows_values_inside_threshold() {
        let before = report_with_stage(Stage::RenderPlanTotal, 1_000, 100);
        let after = report_with_stage(Stage::RenderPlanTotal, 1_149, 114);

        let summary = compare_reports(
            "before",
            &before,
            "after",
            &after,
            15.0,
            &[Stage::RenderPlanTotal],
        );

        assert!(summary.passed);
        assert!(summary.regressions.is_empty());
    }

    #[test]
    fn compare_reports_flags_total_or_p90_regressions() {
        let before = report_with_stage(Stage::RenderPlanTotal, 1_000, 100);
        let total_regression = report_with_stage(Stage::RenderPlanTotal, 1_151, 100);
        let p90_regression = report_with_stage(Stage::RenderPlanTotal, 1_000, 116);

        let total = compare_reports(
            "before",
            &before,
            "after-total",
            &total_regression,
            15.0,
            &[Stage::RenderPlanTotal],
        );
        let p90 = compare_reports(
            "before",
            &before,
            "after-p90",
            &p90_regression,
            15.0,
            &[Stage::RenderPlanTotal],
        );

        assert!(!total.passed);
        assert_eq!(total.regressions.len(), 1);
        assert!(!p90.passed);
        assert_eq!(p90.regressions.len(), 1);
    }

    #[test]
    fn compare_reports_warns_when_stage_is_missing() {
        let before = report_with_stage(Stage::RenderPlanTotal, 1_000, 100);
        let after = report_with_stage(Stage::RenderDispatchTotal, 1_000, 100);

        let summary = compare_reports(
            "before",
            &before,
            "after",
            &after,
            15.0,
            &[Stage::RenderPlanTotal],
        );

        assert!(!summary.passed);
        assert!(summary.regressions.is_empty());
        assert_eq!(summary.warnings.len(), 1);
    }

    fn report_with_stage(stage: Stage, total_nanos: u64, p90_nanos: u64) -> RunReport {
        let mut stages = BTreeMap::new();
        stages.insert(
            stage,
            StageStats {
                calls: 1,
                total_nanos,
                self_nanos: total_nanos,
                p90_nanos,
                ..StageStats::default()
            },
        );
        RunReport {
            scenario: "test".to_owned(),
            config: BTreeMap::new(),
            environment: live2d_probe::EnvironmentReport::current(),
            data: live2d_probe::ProbeData::default(),
            analysis: live2d_probe::ProbeAnalysis {
                stages,
                gauges: BTreeMap::new(),
            },
            warnings: Vec::new(),
        }
    }
}
