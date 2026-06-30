use criterion::{criterion_group, criterion_main, Criterion};
use live2d_perf::{synthetic_snapshot, SyntheticConfig};
use live2d_probe::ProbeRecorder;
use live2d_render::RenderPlanner;

fn render_plan_bench(c: &mut Criterion) {
    let config = SyntheticConfig::medium();
    let snapshot = synthetic_snapshot(&config, 0);
    let planner = RenderPlanner::new();

    c.bench_function("synthetic_render_plan_medium", |b| {
        b.iter(|| {
            let recorder = ProbeRecorder::new();
            planner.build_with_probe(&snapshot, &recorder)
        })
    });
}

criterion_group!(benches, render_plan_bench);
criterion_main!(benches);
