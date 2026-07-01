---
name: live2d-core-runtime
description: Maintain the live2d-rs FFI, shared core data, runtime loading, snapshots, ArtMesh inspection, and facade APIs. Use when working on live2d-sys, live2d-core, live2d-runtime, live2d facade exports, model3 parsing, asset resolving, Cubism-backed runtime state, or Git dependency consumption that needs platform-independent APIs.
---

# Live2D Core Runtime

## Scope

- `live2d-sys`: raw Cubism Core FFI and link probing only.
- `live2d-core`: shared identifiers, canvas, texture, drawable, ArtMesh, and snapshot types.
- `live2d-runtime`: `AssetResolver`, model3 parsing, Cubism-backed loading, update, snapshot, and ArtMesh inspection.
- `live2d`: facade exports and README-aligned public API examples.

## Rules

- Keep this layer platform-independent: no `wgpu`, `winit`, Tauri, NanaVTS protocol types, renderer state, or shader code.
- Do not fake model data when Cubism is unavailable; validate file shape and return an explicit runtime error.
- Use `.env`/`LIVE2D_CUBISM_SDK_DIR` for SDK-backed commands; never hard-code SDK paths.
- Add functional tests only for behavior changes.

## Validation

```powershell
cargo check --workspace --no-default-features
cargo test --workspace --no-default-features
cargo check -p live2d --features live2d-cubism
```
