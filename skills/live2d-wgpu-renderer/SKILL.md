---
name: live2d-wgpu-renderer
description: Maintain the optional live2d-rs wgpu renderer and built-in preview renderer. Use when working on live2d-wgpu, wgpu buffers, textures, bind groups, pipeline cache, WGSL shaders, mask atlas rendering, post-process execution, GPU resource rebuilds, resize/model-switch behavior, or preview renderer state.
---

# Live2D wgpu Renderer

## Scope

- Own `live2d-wgpu`: pipelines, WGSL, buffers, textures, bind groups, mask atlas resources, post-process execution, and renderer state.
- Consume `live2d-render` plans rather than duplicating draw ordering, mask grouping, or material classification.

## Rules

- Keep GPU resources out of lower crates.
- Keep application display protocols outside this workspace.
- Treat resize, model switch, mask atlas, and post-process behavior as functional surfaces.
- For renderer performance changes, capture a matching `live2d-perf` wgpu baseline before editing and compare after editing.

## Validation

```powershell
cargo check -p live2d --features wgpu
cargo run -p live2d-perf --features wgpu -- wgpu-warm --profile medium --frames 300
cargo run -p live2d-perf --features wgpu -- wgpu-mask --profile mask-heavy --frames 300
```
