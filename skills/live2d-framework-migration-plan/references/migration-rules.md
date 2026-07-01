# Framework Migration Planning Rules

## Required First Pass

1. Read `../live2d-framework-reference/references/framework-capabilities.md`.
2. Identify the target Framework area and official evidence path.
3. Use CodeGraph before grep or manual file reads to inspect current live2d-rs symbols and call paths.
4. Classify the work as `covered`, `partial`, `missing`, or `out-of-scope`.

## Crate Boundary Rules

- `live2d-sys`: raw Cubism Core FFI only.
- `live2d-core`: safe shared data types and identifiers.
- `live2d-runtime`: asset resolving, model3, Cubism-backed loading/evaluation, snapshots, ArtMesh, motions, and future physics/effect state.
- `live2d-render`: platform-neutral render plans, draw order, masks, materials, and backend contracts.
- `live2d-wgpu`: wgpu resources, shaders, pipelines, buffers, textures, mask atlas, post-process, and preview rendering.
- `live2d`: facade re-exports with opt-in features.
- `live2d-perf`: before/after checks, real model scenarios, and probes.

## Cubism 5.3/R5 Checklist

- Blend mode: Core extraction, snapshot data, render-plan grouping, pipeline keys, WGSL behavior.
- Model-internal offscreen drawing: hierarchy-aware render target planning; not equivalent to `WgpuLive2DRenderer::render_to_offscreen*`.
- Multiply/screen color: keep model-authored colors and SDK-side overrides distinct.
- Framework Motion update order: account for eye blink, expression, breath, physics, lip sync, pose, and look ordering.

## Planning Shape

Produce migration plans with these sections:

- Goal: one official capability and visible behavior.
- Current state: exact existing Rust coverage and gap.
- Implementation: crate-by-crate steps in dependency order.
- Public API: changed types, methods, feature flags, or facade exports.
- Validation: functional tests, cargo commands, SDK-backed checks, and perf baselines when relevant.
- Non-goals: SDK vendoring, application protocols, platform renderer ports, Editor-only features.

## Validation Defaults

- Baseline: `cargo check --workspace --no-default-features`.
- Behavior: `cargo test --workspace --no-default-features`, only when behavior changes.
- Renderer: `cargo check -p live2d --features wgpu`.
- SDK-backed checks must load `.env`/`LIVE2D_CUBISM_SDK_DIR`; never hard-code SDK paths.
- Perf-sensitive work needs the same `live2d-perf` scenario before and after.

## Implementation Constraints

- Prefer simple data-flow additions over copying official class structure into Rust.
- Do not add low-value tests that only match strings or logs.
- Do not expose placeholder UI or APIs that imply connected behavior before the behavior is implemented.
- Keep official SDK code out of the repository; summarize behavior and cite source paths instead.
