---
name: live2d-rs-dev
description: Maintain the live2d-rs Rust workspace for Live2D Cubism FFI, safe runtime snapshots, platform-neutral render planning, optional wgpu rendering, and the NanaVTS preview process. Use when working on live2d-rs crates, Git dependency consumption from NanaVTS/VTSTemplate, Cubism SDK link setup, model3 parsing, ArtMesh inspection, RenderPlan design, wgpu preview rendering, or NanaVTS display HTTP protocol compatibility.
---

# live2d-rs Development

## Workflow

1. Identify the layer first: `sys`, `core`, `runtime`, `render`, `wgpu`, facade, or `nanavts-display`.
2. Keep dependency direction one-way. Do not let `wgpu`, `winit`, NanaVTS protocol types, or Tauri concepts enter `live2d-sys`, `live2d-core`, `live2d-runtime`, or `live2d-render`.
3. Treat `live2d-render::RenderPlan` as the renderer boundary. Downstream renderers should be able to use `Live2DInstance::update`, `snapshot`, and `RenderPlanner::build` without enabling `wgpu`.
4. Keep Cubism SDK local-only. Do not commit SDK binaries, extracted SDK directories, generated `.cargo/config.toml`, or downloaded archives.
5. Preserve the NanaVTS preview HTTP surface unless the caller explicitly requests a breaking protocol change.

## Crate Boundaries

- `live2d-sys`: raw Cubism Core FFI and link probing only.
- `live2d-core`: identifiers, canvas, texture, drawable, ArtMesh, and snapshot types.
- `live2d-runtime`: `AssetResolver`, model3 resource parsing, Cubism-backed loading, update, snapshot, ArtMesh inspect.
- `live2d-render`: draw ordering, mask plan, material keys, render commands; no GPU or window types.
- `live2d-wgpu`: wgpu buffers, textures, bind groups, pipelines, shaders, Live2D renderer state, and built-in preview renderer state.
- `live2d`: facade re-exports; default features stay platform-independent.
- `nanavts-display`: NanaVTS session protocol, schema, picker, replay, winit executable, and session-to-backend parameter adaptation.

## Guardrails

- Do not add `live2d-tauri` or a `tauri` feature in this phase.
- Do not fake Live2D model data when Cubism is unavailable; return an explicit runtime error after validating model file shape.
- Do not add tests that only match log text or exact error strings unless the string is a public protocol field.
- Keep public API examples in `README.md` aligned with the facade crate.
- Keep `nanavts-display` as a consumer of the facade, not as a source of core runtime types.
- Keep wgpu shader, pipeline, uniform-buffer, bind-group, and renderer state in `live2d-wgpu`; `nanavts-display` should call backend renderers instead of owning a local preview renderer.

## Validation

Use the smallest set that proves the change:

```powershell
cargo check --workspace --no-default-features
cargo test --workspace --no-default-features
cargo check -p live2d --features wgpu
cargo check -p nanavts-display --features wgpu,winit
```

When touching Cubism-backed loading, also check with a local SDK:

```powershell
$env:LIVE2D_CUBISM_SDK_DIR = "C:\path\to\CubismSdkForNative"
cargo check -p nanavts-display --features live2d-cubism,wgpu,winit
```
