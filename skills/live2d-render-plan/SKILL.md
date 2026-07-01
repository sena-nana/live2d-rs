---
name: live2d-render-plan
description: Maintain platform-neutral live2d-rs render planning. Use when working on live2d-render, RenderPlan design, draw ordering, clipping mask plans, mask/material grouping, post-process plan structure, backend dispatch contracts, or renderer-neutral performance behavior.
---

# Live2D Render Plan

## Scope

- Own renderer-neutral `RenderPlan` behavior: draw ordering, clipping masks, material keys, texture indices, post-process plan shape, and backend dispatch contracts.
- Keep output deterministic from `ModelSnapshot` and inspectable without a GPU backend.

## Rules

- Treat `live2d-render::RenderPlan` as the boundary between runtime snapshots and renderer backends.
- Keep `live2d-render` free of GPU, window, Tauri, NanaVTS protocol, and application display types.
- Prefer cache/data-structure changes only when invalidation remains clear.
- For performance-sensitive changes, capture a matching `live2d-perf` baseline before editing and compare after editing.

## Validation

```powershell
cargo check --workspace --no-default-features
cargo test --workspace --no-default-features
cargo run -p live2d-perf -- synthetic-render-plan --profile medium --frames 300
cargo run -p live2d-perf -- render-world-switch --profile mask-heavy --frames 300
```
