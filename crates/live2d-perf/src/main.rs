use live2d_perf::{
    compare_reports, run_dispatch_null_backend, run_real_model_load, run_render_plan,
    run_render_world_switch, CompareSummary, SyntheticBlendProfile, SyntheticConfig,
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
        "real-model-load" => {
            let model = value_arg(&args, "--model")
                .ok_or_else(|| "real-model-load requires --model <path>".to_owned())?;
            run_real_model_load(Path::new(&model))
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
                "unknown scenario `{scenario}`; expected synthetic-render-plan, render-world-switch, dispatch-null-backend, real-model-load, wgpu-cold, wgpu-warm, wgpu-mask, wgpu-resize, wgpu-model-switch, or wgpu-postprocess"
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
    println!("usage: live2d-perf <scenario> [--profile <name>] [--frames <n>] [--blend-profile <name>] [--model <path>]");
    println!("scenarios: synthetic-render-plan, render-world-switch, dispatch-null-backend, real-model-load, wgpu-cold, wgpu-warm, wgpu-mask, wgpu-resize, wgpu-model-switch, wgpu-postprocess, compare-revs");
    println!("profiles: small, medium, large, mask-heavy, static-mask-heavy, texture-heavy, target-filter");
    println!(
        "blend profiles: classic-mix, advanced-colors, advanced-alphas, advanced-matrix, all-modes"
    );
    println!("compare: live2d-perf compare-revs --before faccc70 --after HEAD --threshold 15 --frames 300 [--samples 2]");
}

fn uses_synthetic_config(scenario: &str) -> bool {
    matches!(
        scenario,
        "synthetic-render-plan"
            | "render-world-switch"
            | "dispatch-null-backend"
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
    advanced: AdvancedCheck,
    reports: ScenarioReportPaths,
}

#[derive(Debug, Serialize)]
struct ScenarioReportPaths {
    before_classic: String,
    after_classic: String,
    after_all_modes: String,
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
    for &scenario in compare_scenarios() {
        let stages = stages_for_scenario(scenario);
        let before_copy = run_best_report_sample(
            before_worktree.path.as_path(),
            false,
            scenario,
            &profile,
            frames,
            None,
            samples,
            &out_dir,
            &format!("{scenario}-before-classic"),
            stages,
        )?;
        let after_classic_copy = run_best_report_sample(
            after_dir,
            after_worktree.is_none(),
            scenario,
            &profile,
            frames,
            Some("classic-mix"),
            samples,
            &out_dir,
            &format!("{scenario}-after-classic"),
            stages,
        )?;
        let after_all_copy = run_best_report_sample(
            after_dir,
            after_worktree.is_none(),
            scenario,
            &profile,
            frames,
            Some("all-modes"),
            1,
            &out_dir,
            &format!("{scenario}-after-all-modes"),
            stages,
        )?;

        let before_report = read_report(&before_copy)?;
        let after_classic_report = read_report(&after_classic_copy)?;
        let after_all_report = read_report(&after_all_copy)?;
        let classic = compare_reports(
            format!("{scenario}:before:{before}"),
            &before_report,
            format!("{scenario}:after:{after}:classic-mix"),
            &after_classic_report,
            threshold_percent,
            stages,
        );
        warnings.extend(classic.warnings.iter().cloned());
        regressions.extend(classic.regressions.iter().cloned());

        let advanced = advanced_check(scenario, &after_all_report);
        warnings.extend(advanced.warnings.iter().cloned());
        if !advanced.passed {
            regressions.push(format!("{scenario} all-modes coverage/probe check failed"));
        }

        results.push(ScenarioCompareResult {
            scenario: scenario.to_owned(),
            classic,
            advanced,
            reports: ScenarioReportPaths {
                before_classic: before_copy.display().to_string(),
                after_classic: after_classic_copy.display().to_string(),
                after_all_modes: after_all_copy.display().to_string(),
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
enum PerfCommandMode {
    Cargo,
    CurrentExe,
}

fn run_perf_child(
    cwd: &Path,
    mode: PerfCommandMode,
    scenario: &str,
    profile: &str,
    frames: usize,
    blend_profile: Option<&str>,
) -> Result<ChildReport, String> {
    let mut command = match mode {
        PerfCommandMode::Cargo => {
            let mut command = Command::new("cargo");
            command.arg("run").arg("-p").arg("live2d-perf");
            if scenario.starts_with("wgpu-") {
                command.arg("--features").arg("wgpu");
            }
            command.arg("--");
            command
        }
        PerfCommandMode::CurrentExe => Command::new(
            env::current_exe().map_err(|err| format!("failed to locate current exe: {err}"))?,
        ),
    };
    command.current_dir(cwd).arg(scenario);
    command.arg("--profile").arg(profile);
    command.arg("--frames").arg(frames.to_string());
    if let Some(blend_profile) = blend_profile {
        command.arg("--blend-profile").arg(blend_profile);
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run live2d-perf {mode:?} for {scenario}: {err}"))?;
    if !output.status.success() {
        return Err(format_command_failure("live2d-perf child run", &output));
    }
    Ok(ChildReport {
        report_path: child_report_path(cwd, scenario, &output)?,
    })
}

fn run_best_report_sample(
    cwd: &Path,
    use_current_exe: bool,
    scenario: &str,
    profile: &str,
    frames: usize,
    blend_profile: Option<&str>,
    samples: usize,
    out_dir: &Path,
    label: &str,
    stages: &[Stage],
) -> Result<PathBuf, String> {
    let mut best: Option<(u128, PathBuf)> = None;
    let mode = if use_current_exe {
        PerfCommandMode::CurrentExe
    } else {
        PerfCommandMode::Cargo
    };
    for sample in 0..samples.max(1) {
        let child = run_perf_child(cwd, mode, scenario, profile, frames, blend_profile)?;
        let sample_path = copy_report(
            &child.report_path,
            out_dir,
            &format!("{label}-sample-{}.json", sample + 1),
        )?;
        let report = read_report(&sample_path)?;
        let score = report_score(&report, stages);
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

fn stages_for_scenario(scenario: &str) -> &'static [Stage] {
    match scenario {
        "synthetic-render-plan" => &[
            Stage::RenderPlanTotal,
            Stage::RenderMaskDedup,
            Stage::RenderDrawCommandBuild,
        ],
        "dispatch-null-backend" => &[Stage::RenderPlanTotal, Stage::RenderDispatchTotal],
        "wgpu-warm" => &[
            Stage::WgpuPrepareRender,
            Stage::WgpuMainPassEncode,
            Stage::WgpuQueueSubmit,
        ],
        _ => &[Stage::RenderPlanTotal],
    }
}

#[cfg(feature = "wgpu")]
fn compare_scenarios() -> &'static [&'static str] {
    &[
        "synthetic-render-plan",
        "dispatch-null-backend",
        "wgpu-warm",
    ]
}

#[cfg(not(feature = "wgpu"))]
fn compare_scenarios() -> &'static [&'static str] {
    &["synthetic-render-plan", "dispatch-null-backend"]
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
        output.push_str(&format!(
            "Advanced coverage: colors={}/16, alphas={}/5, advanced_draws={}, wgpu_draws={}, wgpu_copies={}, wgpu_pipelines={}, passed={}\n\n",
            result.advanced.color_modes,
            result.advanced.alpha_modes,
            result.advanced.advanced_draws,
            result.advanced.wgpu_advanced_draw_calls,
            result.advanced.wgpu_advanced_copy_operations,
            result.advanced.wgpu_pipeline_count,
            result.advanced.passed
        ));
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
    }
}
