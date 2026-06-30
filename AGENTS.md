<!-- CODEGRAPH_START -->
## CodeGraph

If this repository has a `.codegraph/` directory, use CodeGraph before grep/find or manual file reads when locating code or understanding flows.
<!-- CODEGRAPH_END -->

# Repository Agent Workflow

## General Rules

- Keep `live2d-sys`, `live2d-core`, `live2d-runtime`, and `live2d-render` free of `wgpu`, `winit`, and Tauri types.
- Do not create a `live2d-tauri` crate or a `tauri` feature in this phase.
- Do not vendor the official Cubism SDK or generated SDK downloads. Use `LIVE2D_CUBISM_SDK_DIR` for local linking.
- Preserve the NanaVTS display HTTP protocol when changing `crates/nanavts-display`.
- Prefer functional tests over log/string matching. Add tests only when behavior changes.

## Crate Boundaries

- `live2d-sys`: raw Cubism Core FFI only.
- `live2d-core`: safe shared data types and identifiers.
- `live2d-runtime`: asset resolving, model3 parsing, Cubism-backed loading, snapshots, ArtMesh inspect.
- `live2d-render`: platform-neutral render plans and mask/material grouping.
- `live2d-wgpu`: wgpu resources, pipeline cache, buffers, textures, shaders, Live2D renderer, and built-in preview renderer.
- `live2d`: facade re-exports with opt-in `wgpu`.
- `nanavts-display`: NanaVTS preview process, HTTP session protocol, winit event loop, and session-to-backend parameter adaptation.

`nanavts-display` must call `live2d-wgpu` for wgpu rendering. Do not add NanaVTS-local WGSL files, render pipelines, uniform buffers, bind groups, or renderer state there.

## Validation

- Baseline: `cargo check --workspace --no-default-features`
- Behavior: `cargo test --workspace --no-default-features`
- Built-in renderer: `cargo check -p live2d --features wgpu`
- Previewer: `cargo check -p nanavts-display --features wgpu,winit`
