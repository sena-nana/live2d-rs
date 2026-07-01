---
name: live2d-cubism-perf
description: Validate live2d-rs Cubism SDK setup and performance behavior. Use when working on LIVE2D_CUBISM_SDK_DIR, .env SDK configuration, live2d-cubism feature linking, real model loading performance, live2d-perf scenarios, before/after benchmark comparisons, or unexplained framework performance regressions.
---

# Live2D Cubism Perf

## Scope

- Own `.env`/`LIVE2D_CUBISM_SDK_DIR`, SDK linking, `live2d-cubism`, real model loading, `live2d-perf`, and performance regression checks.
- Use existing `live2d-perf` scenarios: `synthetic-render-plan`, `render-world-switch`, `dispatch-null-backend`, `real-model-load`, and `wgpu-*`.

## Rules

- Treat `.env` as the local environment-variable file; use `LIVE2D_CUBISM_SDK_DIR` from `.env` or the current environment.
- Do not hard-code SDK paths or vendor SDK binaries, extracted SDK directories, downloads, or generated `.cargo/config.toml`.
- Run the same scenario before and after changes with the same SDK path, model path, profile, frame count, feature set, and practical machine state.
- When validating SDK sample models, every `*.model3.json` under `Samples\Resources` must pass `real-model-load`; treat any `real model load failed` output as failure even if the command exits successfully.
- Compare `target/live2d-perf/*.json` for `total_ms`, `p90_ms`, draw counts, cache hits/misses, bytes, and resource rebuilds; investigate unexplained regressions.

## Validation

```powershell
cargo run -p live2d-perf -- synthetic-render-plan --profile medium --frames 300
cargo run -p live2d-perf -- render-world-switch --profile mask-heavy --frames 300
cargo run -p live2d-perf -- dispatch-null-backend --profile medium --frames 300
cargo run -p live2d-perf --features wgpu -- wgpu-warm --profile medium --frames 300
cargo check -p live2d --features live2d-cubism,wgpu
cargo run -p live2d-perf --features live2d-cubism -- real-model-load --model <path-to-model3.json>
```
