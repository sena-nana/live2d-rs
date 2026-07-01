---
name: live2d-framework-reference
description: Reference official Live2D Cubism Native Framework capabilities for live2d-rs planning and implementation. Use when checking Cubism Framework modules, local SDK evidence, official documentation links, Rust crate coverage, missing capabilities, or whether a requested feature belongs in runtime, render, wgpu, perf, or out of scope.
---

# Live2D Framework Reference

## Scope

Use this as the source map for official Cubism Native Framework capabilities before planning or implementing related live2d-rs work. Treat `.env`/`LIVE2D_CUBISM_SDK_DIR` as the local SDK version anchor and official Live2D docs as compatibility notes.

## Workflow

1. Read `references/framework-capabilities.md` for the capability matrix and current Rust coverage.
2. Use CodeGraph before grep or manual source reads when checking the current implementation.
3. Route implementation to the existing area skill: `live2d-core-runtime`, `live2d-render-plan`, `live2d-wgpu-renderer`, or `live2d-cubism-perf`.

## Notes

- Do not vendor SDK files, binaries, downloads, or generated SDK output.
- Keep application protocols, Tauri, winit, and NanaVTS-specific display behavior outside this workspace.
- Distinguish Cubism model-internal offscreen drawing from `live2d-wgpu` render-to-offscreen targets.
