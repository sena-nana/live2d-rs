---
name: live2d-rs-dev
description: Maintain the live2d-rs Rust workspace for Live2D Cubism FFI, safe runtime snapshots, platform-neutral render planning, and optional wgpu rendering. Use when working on live2d-rs crates, Git dependency consumption from NanaVTS/VTSTemplate, Cubism SDK link setup, model3 parsing, ArtMesh inspection, RenderPlan design, or wgpu preview rendering.
---

# live2d-rs Development

## Workflow

1. Identify the layer first: `sys`, `core`, `runtime`, `render`, `wgpu`, or facade.
2. Keep dependency direction one-way. Do not let `wgpu`, `winit`, NanaVTS protocol types, or Tauri concepts enter `live2d-sys`, `live2d-core`, `live2d-runtime`, or `live2d-render`.
3. Treat `live2d-render::RenderPlan` as the renderer boundary. Downstream renderers should be able to use `Live2DInstance::update`, `snapshot`, and `RenderPlanner::build` without enabling `wgpu`.
4. Keep Cubism SDK local-only. Do not commit SDK binaries, extracted SDK directories, generated `.cargo/config.toml`, or downloaded archives.
5. Keep application-specific preview protocols in the application repository, not in `live2d-rs`.

## Crate Boundaries

- `live2d-sys`: raw Cubism Core FFI and link probing only.
- `live2d-core`: identifiers, canvas, texture, drawable, ArtMesh, and snapshot types.
- `live2d-runtime`: `AssetResolver`, model3 resource parsing, Cubism-backed loading, update, snapshot, ArtMesh inspect.
- `live2d-render`: draw ordering, mask plan, material keys, render commands; no GPU or window types.
- `live2d-wgpu`: wgpu buffers, textures, bind groups, pipelines, shaders, Live2D renderer state, and built-in preview renderer state.
- `live2d`: facade re-exports; default features stay platform-independent.

## Guardrails

- Do not add `live2d-tauri` or a `tauri` feature in this phase.
- Do not fake Live2D model data when Cubism is unavailable; return an explicit runtime error after validating model file shape.
- Do not add tests that only match log text or exact error strings unless the string is a public protocol field.
- Keep public API examples in `README.md` aligned with the facade crate.
- Keep application display crates as consumers of the facade, not as sources of core runtime types.
- Keep wgpu shader, pipeline, uniform-buffer, bind-group, and renderer state in `live2d-wgpu`; application preview code should call backend renderers instead of owning local renderer state.

## Validation

Use the smallest set that proves the change:

```powershell
cargo check --workspace --no-default-features
cargo test --workspace --no-default-features
cargo check -p live2d --features wgpu
```

When touching Cubism-backed loading, also check with a local SDK:

```powershell
$env:LIVE2D_CUBISM_SDK_DIR = "C:\path\to\CubismSdkForNative"
cargo check -p live2d --features live2d-cubism,wgpu
```
