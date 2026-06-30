use live2d_perf::{
    run_dispatch_null_backend, run_real_model_load, run_render_plan, run_render_world_switch,
    SyntheticConfig,
};
use live2d_probe::RunReport;
use std::{
    env, fs,
    path::{Path, PathBuf},
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
    let profile = value_arg(&args, "--profile").unwrap_or_else(|| "medium".to_owned());
    let mut config = SyntheticConfig::from_profile(&profile);
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
        "wgpu-cold" | "wgpu-warm" | "wgpu-mask" | "wgpu-resize" => {
            live2d_perf::wgpu_scenarios::run_wgpu_scenario(&scenario, &config)?
        }
        #[cfg(not(feature = "wgpu"))]
        "wgpu-cold" | "wgpu-warm" | "wgpu-mask" | "wgpu-resize" => {
            return Err("wgpu scenarios require `--features wgpu`".to_owned());
        }
        _ => {
            return Err(format!(
                "unknown scenario `{scenario}`; expected synthetic-render-plan, render-world-switch, dispatch-null-backend, real-model-load, wgpu-cold, wgpu-warm, wgpu-mask, or wgpu-resize"
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
    println!("usage: live2d-perf <scenario> [--profile <name>] [--frames <n>] [--model <path>]");
    println!("scenarios: synthetic-render-plan, render-world-switch, dispatch-null-backend, real-model-load, wgpu-cold, wgpu-warm, wgpu-mask, wgpu-resize");
    println!("profiles: small, medium, large, mask-heavy, static-mask-heavy, texture-heavy, target-filter");
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
    )
}

fn report_label(scenario: &str, profile: &str, report: &RunReport) -> String {
    if uses_synthetic_config(scenario) {
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
