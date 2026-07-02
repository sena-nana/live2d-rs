use live2d_perf::{
    compare_reports, run_dispatch_null_backend, run_layered_motion, run_motion_update,
    run_physics_update, run_real_model_load, run_real_model_motion, run_real_model_motion_diff,
    run_real_model_physics, run_real_model_render, run_render_plan, run_render_world_switch,
    CompareSummary, RealModelRenderConfig, SyntheticBlendProfile, SyntheticConfig,
};
use live2d_probe::{RunReport, Stage, StageStats};
use serde::Serialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        print_help();
        return Ok(());
    }
    let scenario = args
        .first()
        .cloned()
        .unwrap_or_else(|| "synthetic-render-plan".to_owned());
    if !args.is_empty() {
        args.remove(0);
    }
    if scenario == "compare-revs" {
        return run_compare_revs(&args);
    }
    let profile = value_arg(&args, "--profile").unwrap_or_else(|| "medium".to_owned());
    let mut config = SyntheticConfig::from_profile(&profile);
    if let Some(blend_profile) = value_arg(&args, "--blend-profile") {
        config.blend_profile = SyntheticBlendProfile::parse(&blend_profile).ok_or_else(|| {
            format!(
                "--blend-profile must be one of classic-mix, advanced-colors, advanced-alphas, advanced-matrix, or all-modes; got `{blend_profile}`"
            )
        })?;
    }
    if let Some(frames) = value_arg(&args, "--frames") {
        config.frames = frames
            .parse::<usize>()
            .map_err(|_| "--frames must be a positive integer".to_owned())?;
    }

    let mut report = match scenario.as_str() {
        "synthetic-render-plan" => run_render_plan(&config),
        "render-world-switch" => run_render_world_switch(&config),
        "dispatch-null-backend" => {
            let (report, backend) = run_dispatch_null_backend(&config);
            println!(
                "backend counts: models={} mask_passes={} mask_draws={} main_draws={}",
                backend.begin_models, backend.mask_passes, backend.mask_draws, backend.main_draws
            );
            report
        }
        "motion-update" => run_motion_update(&config),
        "layered-motion" => run_layered_motion(&config),
        "physics-update" => run_physics_update(&config),
        "real-model-load" => {
            let model = value_arg(&args, "--model")
                .ok_or_else(|| "real-model-load requires --model <path>".to_owned())?;
            run_real_model_load(Path::new(&model))
        }
        "real-model-physics" => {
            let model = value_arg(&args, "--model")
                .ok_or_else(|| "real-model-physics requires --model <path>".to_owned())?;
            run_real_model_physics(Path::new(&model), config.frames)
        }
        "real-model-motion" => {
            let model = value_arg(&args, "--model")
                .ok_or_else(|| "real-model-motion requires --model <path>".to_owned())?;
            let motion_group = value_arg(&args, "--motion-group");
            run_real_model_motion(Path::new(&model), config.frames, motion_group.as_deref())
        }
        "real-model-motion-diff" => {
            let model = value_arg(&args, "--model")
                .ok_or_else(|| "real-model-motion-diff requires --model <path>".to_owned())?;
            let motion = value_arg(&args, "--motion");
            let expression = value_arg(&args, "--expression");
            let frame = usize_arg(&args, "--frame", 0)?;
            let dt = f32_arg(&args, "--dt", 1.0 / 60.0)?;
            run_real_model_motion_diff(
                Path::new(&model),
                motion.as_deref().map(Path::new),
                expression.as_deref().map(Path::new),
                frame,
                dt,
            )
        }
        "real-model-render" => {
            let model = value_arg(&args, "--model")
                .ok_or_else(|| "real-model-render requires --model <path>".to_owned())?;
            let motion_group = value_arg(&args, "--motion-group");
            let warmup_frames = usize_arg(&args, "--warmup-frames", 0)?;
            let width = u32_arg(&args, "--width", 1024)?;
            let height = u32_arg(&args, "--height", 1024)?;
            let render_config = RealModelRenderConfig::new(
                config.frames,
                warmup_frames,
                width,
                height,
                motion_group,
            );
            run_real_model_render(Path::new(&model), &render_config)
        }
        #[cfg(feature = "wgpu")]
        "wgpu-cold" | "wgpu-warm" | "wgpu-mask" | "wgpu-resize" | "wgpu-model-switch"
        | "wgpu-postprocess" => live2d_perf::wgpu_scenarios::run_wgpu_scenario(&scenario, &config)?,
        #[cfg(not(feature = "wgpu"))]
        "wgpu-cold" | "wgpu-warm" | "wgpu-mask" | "wgpu-resize" | "wgpu-model-switch"
        | "wgpu-postprocess" => {
            return Err("wgpu scenarios require `--features wgpu`".to_owned());
        }
        _ => {
            return Err(format!(
                "unknown scenario `{scenario}`; expected synthetic-render-plan, render-world-switch, dispatch-null-backend, motion-update, layered-motion, physics-update, real-model-load, real-model-physics, real-model-motion, real-model-motion-diff, real-model-render, wgpu-cold, wgpu-warm, wgpu-mask, wgpu-resize, wgpu-model-switch, or wgpu-postprocess"
            ));
        }
    };

    if uses_synthetic_config(&scenario) {
        report.config.insert("profile".to_owned(), profile.clone());
    }
    let label = report_label(&scenario, &profile, &report);
    let path = write_report(&label, &report)?;
    println!("report: {}", path.display());
    print_summary(&report);
    Ok(())
}

fn print_help() {
    println!("usage: live2d-perf <scenario> [--profile <name>] [--frames <n>] [--blend-profile <name>] [--model <path>] [--motion-group <name>] [--motion <path>] [--expression <path>] [--frame <n>] [--dt <seconds>] [--warmup-frames <n>] [--width <px>] [--height <px>]");
    println!("scenarios: synthetic-render-plan, render-world-switch, dispatch-null-backend, motion-update, layered-motion, physics-update, real-model-load, real-model-physics, real-model-motion, real-model-motion-diff, real-model-render, wgpu-cold, wgpu-warm, wgpu-mask, wgpu-resize, wgpu-model-switch, wgpu-postprocess, compare-revs");
    println!("profiles: small, medium, large, mask-heavy, static-mask-heavy, texture-heavy, target-filter, physics-heavy");
    println!(
        "blend profiles: classic-mix, advanced-colors, advanced-alphas, advanced-matrix, all-modes"
    );
    println!("compare: live2d-perf compare-revs --before faccc70 --after HEAD --threshold 15 --frames 300 [--samples 2] [--model <path>] [--motion-group <name>]");
}

fn uses_synthetic_config(scenario: &str) -> bool {
    matches!(
        scenario,
        "synthetic-render-plan"
            | "render-world-switch"
            | "dispatch-null-backend"
            | "motion-update"
            | "layered-motion"
            | "physics-update"
            | "wgpu-cold"
            | "wgpu-warm"
            | "wgpu-mask"
            | "wgpu-resize"
            | "wgpu-model-switch"
            | "wgpu-postprocess"
    )
}

fn report_label(scenario: &str, profile: &str, report: &RunReport) -> String {
    if uses_synthetic_config(scenario) {
        if let Some(blend_profile) = report.config.get("blend_profile") {
            if blend_profile != "classic-mix" {
                return format!("{scenario}-{profile}-{blend_profile}");
            }
        }
        return format!("{scenario}-{profile}");
    }
    if scenario == "real-model-render" {
        return report
            .config
            .get("model")
            .and_then(|model| Path::new(model).file_stem())
            .and_then(|stem| stem.to_str())
            .map(|stem| {
                let frames = report
                    .config
                    .get("frames")
                    .map(String::as_str)
                    .unwrap_or("1");
                let warmup = report
                    .config
                    .get("warmup_frames")
                    .map(String::as_str)
                    .unwrap_or("0");
                let width = report
                    .config
                    .get("width")
                    .map(String::as_str)
                    .unwrap_or("0");
                let height = report
                    .config
                    .get("height")
                    .map(String::as_str)
                    .unwrap_or("0");
                format!("{scenario}-{stem}-frames-{frames}-warmup-{warmup}-{width}x{height}")
            })
            .unwrap_or_else(|| scenario.to_owned());
    }
    report
        .config
        .get("model")
        .and_then(|model| Path::new(model).file_stem())
        .and_then(|stem| stem.to_str())
        .map(|stem| format!("{scenario}-{stem}"))
        .unwrap_or_else(|| scenario.to_owned())
}

#[derive(Debug, Serialize)]
struct CompareRevsSummary {
    before: String,
    after: String,
    profile: String,
    frames: usize,
    samples: usize,
    threshold_percent: f64,
    results: Vec<ScenarioCompareResult>,
    warnings: Vec<String>,
    regressions: Vec<String>,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct ScenarioCompareResult {
    scenario: String,
    classic: CompareSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    advanced: Option<AdvancedCheck>,
    reports: ScenarioReportPaths,
}

#[derive(Debug, Serialize)]
struct ScenarioReportPaths {
    before_classic: String,
    after_classic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_all_modes: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct AdvancedCheck {
    color_modes: u64,
    alpha_modes: u64,
    advanced_draws: u64,
    wgpu_advanced_draw_calls: u64,
    wgpu_advanced_copy_operations: u64,
    wgpu_pipeline_count: u64,
    passed: bool,
    warnings: Vec<String>,
}

fn run_compare_revs(args: &[String]) -> Result<(), String> {
    let before = value_arg(args, "--before").unwrap_or_else(|| "faccc70".to_owned());
    let after = value_arg(args, "--after").unwrap_or_else(|| "HEAD".to_owned());
    let profile = value_arg(args, "--profile").unwrap_or_else(|| "medium".to_owned());
    let model = value_arg(args, "--model");
    let motion_group = value_arg(args, "--motion-group");
    if motion_group.is_some() && model.is_none() {
        return Err("--motion-group requires --model <path> for compare-revs".to_owned());
    }
    let frames = value_arg(args, "--frames")
        .unwrap_or_else(|| "300".to_owned())
        .parse::<usize>()
        .map_err(|_| "--frames must be a positive integer".to_owned())?;
    let samples = value_arg(args, "--samples")
        .unwrap_or_else(|| "2".to_owned())
        .parse::<usize>()
        .map_err(|_| "--samples must be a positive integer".to_owned())?
        .max(1);
    let threshold_percent = value_arg(args, "--threshold")
        .unwrap_or_else(|| "15".to_owned())
        .parse::<f64>()
        .map_err(|_| "--threshold must be a number".to_owned())?;

    let root = env::current_dir().map_err(|err| format!("failed to read cwd: {err}"))?;
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("system clock is before UNIX_EPOCH: {err}"))?
        .as_secs();
    let out_dir = root
        .join("target")
        .join("live2d-perf")
        .join("compare")
        .join(format!("blend-{run_id}"));
    fs::create_dir_all(&out_dir).map_err(|err| format!("failed to create compare dir: {err}"))?;

    let before_worktree = TempWorktree::add(&root, &out_dir.join("before-worktree"), &before)?;
    let after_worktree = if after == "HEAD" {
        None
    } else {
        Some(TempWorktree::add(
            &root,
            &out_dir.join("after-worktree"),
            &after,
        )?)
    };
    let after_dir = after_worktree
        .as_ref()
        .map(|worktree| worktree.path.as_path())
        .unwrap_or(root.as_path());

    let mut results = Vec::new();
    let mut warnings = Vec::new();
    let mut regressions = Vec::new();
    let params = CompareRunParams {
        profile: &profile,
        frames,
        model: model.as_deref(),
        motion_group: motion_group.as_deref(),
    };
    for scenario in compare_scenarios(model.is_some()) {
        let scenario_name = scenario.name;
        let before_copy = run_best_report_sample(
            before_worktree.path.as_path(),
            false,
            scenario,
            &params,
            None,
            samples,
            &out_dir,
            &format!("{scenario_name}-before-classic"),
        )?;
        let after_classic_copy = run_best_report_sample(
            after_dir,
            after_worktree.is_none(),
            scenario,
            &params,
            scenario.classic_blend_profile(),
            samples,
            &out_dir,
            &format!("{scenario_name}-after-classic"),
        )?;
        let after_all_copy = run_advanced_report(
            after_dir,
            after_worktree.is_none(),
            scenario,
            &params,
            &out_dir,
        )?;

        let before_report = read_report_with_warnings(
            &before_copy,
            &mut warnings,
            &format!("{scenario_name}:before:{before}"),
        )?;
        let after_classic_name = scenario.after_classic_name(&after);
        let after_classic_report =
            read_report_with_warnings(&after_classic_copy, &mut warnings, &after_classic_name)?;
        let classic = compare_reports(
            format!("{scenario_name}:before:{before}"),
            &before_report,
            after_classic_name,
            &after_classic_report,
            threshold_percent,
            scenario.stages,
        );
        warnings.extend(classic.warnings.iter().cloned());
        regressions.extend(classic.regressions.iter().cloned());

        let advanced = if let Some(after_all_copy) = &after_all_copy {
            let after_all_report = read_report_with_warnings(
                after_all_copy,
                &mut warnings,
                &format!("{scenario_name}:after:{after}:all-modes"),
            )?;
            let advanced = advanced_check(scenario_name, &after_all_report);
            warnings.extend(advanced.warnings.iter().cloned());
            if !advanced.passed {
                regressions.push(format!(
                    "{scenario_name} all-modes coverage/probe check failed"
                ));
            }
            Some(advanced)
        } else {
            None
        };

        results.push(ScenarioCompareResult {
            scenario: scenario_name.to_owned(),
            classic,
            advanced,
            reports: ScenarioReportPaths {
                before_classic: before_copy.display().to_string(),
                after_classic: after_classic_copy.display().to_string(),
                after_all_modes: after_all_copy.map(|path| path.display().to_string()),
            },
        });
    }

    let passed = warnings.is_empty() && regressions.is_empty();
    let summary = CompareRevsSummary {
        before,
        after,
        profile,
        frames,
        samples,
        threshold_percent,
        results,
        warnings,
        regressions,
        passed,
    };
    let summary_json = out_dir.join("summary.json");
    fs::write(
        &summary_json,
        serde_json::to_string_pretty(&summary)
            .map_err(|err| format!("failed to encode compare summary: {err}"))?,
    )
    .map_err(|err| format!("failed to write compare summary: {err}"))?;
    let summary_md = out_dir.join("summary.md");
    fs::write(&summary_md, compare_markdown(&summary))
        .map_err(|err| format!("failed to write compare markdown: {err}"))?;

    println!("compare summary: {}", summary_json.display());
    println!("compare markdown: {}", summary_md.display());
    if summary.passed {
        Ok(())
    } else {
        Err(format!(
            "compare-revs found {} warning(s) and {} regression(s)",
            summary.warnings.len(),
            summary.regressions.len()
        ))
    }
}

struct ChildReport {
    report_path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
struct CompareScenario {
    name: &'static str,
    stages: &'static [Stage],
    kind: CompareScenarioKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompareScenarioKind {
    Synthetic,
    #[cfg(feature = "wgpu")]
    Wgpu,
    RealModel,
}

impl CompareScenario {
    fn classic_blend_profile(self) -> Option<&'static str> {
        self.run_advanced_check().then_some("classic-mix")
    }

    fn after_classic_name(self, after: &str) -> String {
        match self.classic_blend_profile() {
            Some(blend_profile) => format!("{}:after:{after}:{blend_profile}", self.name),
            None => format!("{}:after:{after}", self.name),
        }
    }

    fn cargo_feature(self) -> Option<&'static str> {
        match self.kind {
            CompareScenarioKind::Synthetic => None,
            #[cfg(feature = "wgpu")]
            CompareScenarioKind::Wgpu => Some("wgpu"),
            CompareScenarioKind::RealModel => Some("live2d-cubism"),
        }
    }

    fn uses_real_model(self) -> bool {
        self.kind == CompareScenarioKind::RealModel
    }

    fn run_advanced_check(self) -> bool {
        self.kind != CompareScenarioKind::RealModel
    }
}

struct CompareRunParams<'a> {
    profile: &'a str,
    frames: usize,
    model: Option<&'a str>,
    motion_group: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
enum PerfCommandMode {
    Cargo,
    CurrentExe,
}

fn run_perf_child(
    cwd: &Path,
    mode: PerfCommandMode,
    scenario: CompareScenario,
    params: &CompareRunParams<'_>,
    blend_profile: Option<&str>,
) -> Result<ChildReport, String> {
    let mut command = match mode {
        PerfCommandMode::Cargo => {
            let mut command = Command::new("cargo");
            command.arg("run").arg("-p").arg("live2d-perf");
            if let Some(feature) = scenario.cargo_feature() {
                command.arg("--features").arg(feature);
            }
            command.arg("--");
            command
        }
        PerfCommandMode::CurrentExe => Command::new(
            env::current_exe().map_err(|err| format!("failed to locate current exe: {err}"))?,
        ),
    };
    command.current_dir(cwd).arg(scenario.name);
    command.arg("--profile").arg(params.profile);
    command.arg("--frames").arg(params.frames.to_string());
    if let Some(blend_profile) = blend_profile {
        command.arg("--blend-profile").arg(blend_profile);
    }
    if scenario.uses_real_model() {
        let model = params
            .model
            .ok_or_else(|| format!("{} requires --model <path>", scenario.name))?;
        command.arg("--model").arg(model);
        if let Some(motion_group) = params.motion_group {
            command.arg("--motion-group").arg(motion_group);
        }
    }
    let output = command.output().map_err(|err| {
        format!(
            "failed to run live2d-perf {mode:?} for {}: {err}",
            scenario.name
        )
    })?;
    if !output.status.success() {
        return Err(format_command_failure("live2d-perf child run", &output));
    }
    Ok(ChildReport {
        report_path: child_report_path(cwd, scenario.name, &output)?,
    })
}

fn run_best_report_sample(
    cwd: &Path,
    use_current_exe: bool,
    scenario: CompareScenario,
    params: &CompareRunParams<'_>,
    blend_profile: Option<&str>,
    samples: usize,
    out_dir: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    let mut best: Option<(u128, PathBuf)> = None;
    let mode = if use_current_exe {
        PerfCommandMode::CurrentExe
    } else {
        PerfCommandMode::Cargo
    };
    for sample in 0..samples.max(1) {
        let child = run_perf_child(cwd, mode, scenario, params, blend_profile)?;
        let sample_path = copy_report(
            &child.report_path,
            out_dir,
            &format!("{label}-sample-{}.json", sample + 1),
        )?;
        let report = read_report(&sample_path)?;
        let score = report_score(&report, scenario.stages);
        if best
            .as_ref()
            .map(|(best_score, _)| score < *best_score)
            .unwrap_or(true)
        {
            best = Some((score, sample_path));
        }
    }
    best.map(|(_, path)| path)
        .ok_or_else(|| format!("{label} did not produce any report samples"))
}

fn run_advanced_report(
    cwd: &Path,
    use_current_exe: bool,
    scenario: CompareScenario,
    params: &CompareRunParams<'_>,
    out_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    if !scenario.run_advanced_check() {
        return Ok(None);
    }
    run_best_report_sample(
        cwd,
        use_current_exe,
        scenario,
        params,
        Some("all-modes"),
        1,
        out_dir,
        &format!("{}-after-all-modes", scenario.name),
    )
    .map(Some)
}

fn child_report_path(cwd: &Path, scenario: &str, output: &Output) -> Result<PathBuf, String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report_path = stdout
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("report: "))
        .ok_or_else(|| format!("{scenario} did not print a report path"))?;
    let path = PathBuf::from(report_path);
    Ok(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

fn report_score(report: &RunReport, stages: &[Stage]) -> u128 {
    stages
        .iter()
        .map(|stage| {
            report
                .analysis
                .stages
                .get(stage)
                .filter(|stats| stats.calls > 0)
                .map(|stats| stats.total_nanos as u128)
                .unwrap_or(u128::MAX / 4)
        })
        .sum()
}

fn read_report(path: &Path) -> Result<RunReport, String> {
    let json = fs::read_to_string(path)
        .map_err(|err| format!("failed to read report {}: {err}", path.display()))?;
    serde_json::from_str(&json)
        .map_err(|err| format!("failed to parse report {}: {err}", path.display()))
}

fn read_report_with_warnings(
    path: &Path,
    warnings: &mut Vec<String>,
    label: &str,
) -> Result<RunReport, String> {
    let report = read_report(path)?;
    collect_report_warnings(warnings, label, &report);
    Ok(report)
}

fn copy_report(src: &Path, out_dir: &Path, file_name: &str) -> Result<PathBuf, String> {
    let dst = out_dir.join(sanitize_filename(file_name));
    fs::copy(src, &dst).map_err(|err| {
        format!(
            "failed to copy report {} to {}: {err}",
            src.display(),
            dst.display()
        )
    })?;
    Ok(dst)
}

fn collect_report_warnings(warnings: &mut Vec<String>, label: &str, report: &RunReport) {
    for warning in &report.warnings {
        warnings.push(format!("{label}: {warning}"));
    }
}

#[cfg(feature = "wgpu")]
fn append_wgpu_compare_scenarios(scenarios: &mut Vec<CompareScenario>) {
    scenarios.push(CompareScenario {
        name: "wgpu-warm",
        stages: &[
            Stage::WgpuPrepareRender,
            Stage::WgpuMainPassEncode,
            Stage::WgpuQueueSubmit,
        ],
        kind: CompareScenarioKind::Wgpu,
    });
}

#[cfg(not(feature = "wgpu"))]
fn append_wgpu_compare_scenarios(_scenarios: &mut Vec<CompareScenario>) {}

fn compare_scenarios(include_real_model_motion: bool) -> Vec<CompareScenario> {
    let mut scenarios = vec![
        CompareScenario {
            name: "synthetic-render-plan",
            stages: &[
                Stage::RenderPlanTotal,
                Stage::RenderMaskDedup,
                Stage::RenderDrawCommandBuild,
            ],
            kind: CompareScenarioKind::Synthetic,
        },
        CompareScenario {
            name: "dispatch-null-backend",
            stages: &[Stage::RenderPlanTotal, Stage::RenderDispatchTotal],
            kind: CompareScenarioKind::Synthetic,
        },
        CompareScenario {
            name: "motion-update",
            stages: &[Stage::RuntimeMotionUpdate],
            kind: CompareScenarioKind::Synthetic,
        },
        CompareScenario {
            name: "physics-update",
            stages: &[Stage::RuntimePhysicsUpdate],
            kind: CompareScenarioKind::Synthetic,
        },
    ];
    append_wgpu_compare_scenarios(&mut scenarios);
    if include_real_model_motion {
        scenarios.push(CompareScenario {
            name: "real-model-physics",
            stages: &[Stage::RuntimePhysicsParse, Stage::RuntimePhysicsUpdate],
            kind: CompareScenarioKind::RealModel,
        });
        scenarios.push(CompareScenario {
            name: "real-model-motion",
            stages: &[Stage::RuntimeMotionUpdate],
            kind: CompareScenarioKind::RealModel,
        });
    }
    scenarios
}

fn advanced_check(scenario: &str, report: &RunReport) -> AdvancedCheck {
    let color_modes = config_u64(report, "blend_advanced_color_modes");
    let alpha_modes = config_u64(report, "blend_advanced_alpha_modes");
    let advanced_draws = config_u64(report, "blend_advanced_draws");
    let wgpu_advanced_draw_calls = counter_value(
        report.analysis.stages.get(&Stage::WgpuMainPassEncode),
        "advanced_draw_calls",
    );
    let wgpu_advanced_copy_operations = counter_value(
        report.analysis.stages.get(&Stage::WgpuMainPassEncode),
        "advanced_copy_operations",
    );
    let wgpu_pipeline_count = counter_value(
        report.analysis.stages.get(&Stage::WgpuPipelineCreation),
        "resource_rebuilds",
    );
    let mut warnings = Vec::new();
    if color_modes < 16 {
        warnings.push(format!(
            "{scenario} all-modes covered {color_modes}/16 color blend modes"
        ));
    }
    if alpha_modes < 5 {
        warnings.push(format!(
            "{scenario} all-modes covered {alpha_modes}/5 alpha blend modes"
        ));
    }
    if advanced_draws == 0 {
        warnings.push(format!(
            "{scenario} all-modes did not generate advanced drawables"
        ));
    }
    if scenario.starts_with("wgpu-") && wgpu_advanced_draw_calls == 0 {
        warnings.push(format!(
            "{scenario} did not record advanced wgpu draw calls"
        ));
    }
    if scenario.starts_with("wgpu-") && wgpu_advanced_copy_operations == 0 {
        warnings.push(format!(
            "{scenario} did not record advanced blend copy operations"
        ));
    }
    if scenario.starts_with("wgpu-") && wgpu_pipeline_count == 0 {
        warnings.push(format!("{scenario} did not record wgpu pipeline count"));
    }
    if scenario.starts_with("wgpu-") && wgpu_pipeline_count > 9 {
        warnings.push(format!(
            "{scenario} built {wgpu_pipeline_count} pipelines; expected advanced blends to share the prebuilt Advanced pipeline"
        ));
    }
    AdvancedCheck {
        color_modes,
        alpha_modes,
        advanced_draws,
        wgpu_advanced_draw_calls,
        wgpu_advanced_copy_operations,
        wgpu_pipeline_count,
        passed: warnings.is_empty(),
        warnings,
    }
}

fn config_u64(report: &RunReport, key: &str) -> u64 {
    report
        .config
        .get(key)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default()
}

fn counter_value(stats: Option<&StageStats>, name: &str) -> u64 {
    stats
        .and_then(|stats| stats.counters.get(name))
        .copied()
        .unwrap_or_default()
}

fn compare_markdown(summary: &CompareRevsSummary) -> String {
    let mut output = String::new();
    output.push_str("# live2d-perf compare-revs\n\n");
    output.push_str(&format!(
        "- before: `{}`\n- after: `{}`\n- profile: `{}`\n- frames: `{}`\n- samples: `{}`\n- threshold: `{:.2}%`\n- passed: `{}`\n\n",
        summary.before,
        summary.after,
        summary.profile,
        summary.frames,
        summary.samples,
        summary.threshold_percent,
        summary.passed
    ));
    for result in &summary.results {
        output.push_str(&format!("## {}\n\n", result.scenario));
        output.push_str("| stage | total ratio | p90 ratio | regressed |\n");
        output.push_str("| --- | ---: | ---: | --- |\n");
        for comparison in &result.classic.comparisons {
            output.push_str(&format!(
                "| {:?} | {:.3} | {:.3} | {} |\n",
                comparison.stage,
                comparison.total_ratio,
                comparison.p90_ratio,
                comparison.regressed
            ));
        }
        output.push('\n');
        if let Some(advanced) = &result.advanced {
            output.push_str(&format!(
                "Advanced coverage: colors={}/16, alphas={}/5, advanced_draws={}, wgpu_draws={}, wgpu_copies={}, wgpu_pipelines={}, passed={}\n\n",
                advanced.color_modes,
                advanced.alpha_modes,
                advanced.advanced_draws,
                advanced.wgpu_advanced_draw_calls,
                advanced.wgpu_advanced_copy_operations,
                advanced.wgpu_pipeline_count,
                advanced.passed
            ));
        }
    }
    if !summary.warnings.is_empty() {
        output.push_str("## Warnings\n\n");
        for warning in &summary.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
        output.push('\n');
    }
    if !summary.regressions.is_empty() {
        output.push_str("## Regressions\n\n");
        for regression in &summary.regressions {
            output.push_str(&format!("- {regression}\n"));
        }
    }
    output
}

struct TempWorktree {
    root: PathBuf,
    path: PathBuf,
}

impl TempWorktree {
    fn add(root: &Path, path: &Path, rev: &str) -> Result<Self, String> {
        if path.exists() {
            let _ = fs::remove_dir_all(path);
        }
        let output = Command::new("git")
            .current_dir(root)
            .arg("worktree")
            .arg("add")
            .arg("--detach")
            .arg(path)
            .arg(rev)
            .output()
            .map_err(|err| format!("failed to create git worktree for {rev}: {err}"))?;
        if !output.status.success() {
            return Err(format_command_failure("git worktree add", &output));
        }
        Ok(Self {
            root: root.to_path_buf(),
            path: path.to_path_buf(),
        })
    }
}

impl Drop for TempWorktree {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .current_dir(&self.root)
            .arg("worktree")
            .arg("remove")
            .arg("--force")
            .arg(&self.path)
            .output();
    }
}

fn format_command_failure(label: &str, output: &Output) -> String {
    format!(
        "{label} failed with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn value_arg(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].clone())
}

fn usize_arg(args: &[String], name: &str, default: usize) -> Result<usize, String> {
    value_arg(args, name)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| format!("{name} must be a positive integer"))
        })
        .unwrap_or(Ok(default))
}

fn u32_arg(args: &[String], name: &str, default: u32) -> Result<u32, String> {
    value_arg(args, name)
        .map(|value| {
            value
                .parse::<u32>()
                .map(|parsed| parsed.max(1))
                .map_err(|_| format!("{name} must be a positive integer"))
        })
        .unwrap_or(Ok(default.max(1)))
}

fn f32_arg(args: &[String], name: &str, default: f32) -> Result<f32, String> {
    value_arg(args, name)
        .map(|value| {
            value
                .parse::<f32>()
                .map(|parsed| {
                    if parsed.is_finite() {
                        parsed.max(0.0)
                    } else {
                        default
                    }
                })
                .map_err(|_| format!("{name} must be a finite number"))
        })
        .unwrap_or(Ok(default))
}

fn write_report(label: &str, report: &RunReport) -> Result<PathBuf, String> {
    let dir = PathBuf::from("target").join("live2d-perf");
    fs::create_dir_all(&dir).map_err(|err| format!("failed to create report dir: {err}"))?;
    let path = dir.join(format!("{}.json", sanitize_filename(label)));
    let json = serde_json::to_string_pretty(report)
        .map_err(|err| format!("failed to encode report: {err}"))?;
    fs::write(&path, json).map_err(|err| format!("failed to write report: {err}"))?;
    Ok(path)
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn print_summary(report: &RunReport) {
    println!("scenario: {}", report.scenario);
    if !report.warnings.is_empty() {
        for warning in &report.warnings {
            println!("warning: {warning}");
        }
    }
    for (stage, stats) in &report.analysis.stages {
        if stats.calls == 0 {
            continue;
        }
        println!(
            "{stage:?}: calls={} total_ms={:.3} p90_ms={:.3} draws={} bytes={} cache_hit={} cache_miss={} rebuilds={}",
            stats.calls,
            stats.total_nanos as f64 / 1_000_000.0,
            stats.p90_nanos as f64 / 1_000_000.0,
            stats.draw_calls,
            stats.bytes,
            stats.cache_hits,
            stats.cache_misses,
            stats.resource_rebuilds
        );
        for (name, value) in &stats.counters {
            println!("{stage:?}.{name}={value}");
        }
    }
}
