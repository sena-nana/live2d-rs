# live2d-rs

Rust workspace for Live2D model loading, runtime snapshots, platform-neutral render planning, and optional wgpu preview rendering.

This repository intentionally does not vendor the official Live2D Cubism SDK. Set `LIVE2D_CUBISM_SDK_DIR` to a local Cubism SDK for Native checkout when enabling Cubism-backed model evaluation.

## Crates

```text
live2d-sys      Cubism Core FFI and SDK link probing
live2d-core     Platform-independent model, drawable, canvas, texture, and identifier types
live2d-runtime  Asset resolving, model3 parsing, model loading, update, snapshot, ArtMesh inspect
live2d-render   Snapshot-to-RenderPlan conversion without wgpu or window-system types
live2d-wgpu     Optional wgpu backend and built-in WGSL renderer
live2d          Facade crate with opt-in features
nanavts-display NanaVTS preview HTTP protocol and winit/wgpu display process
```

The dependency direction is one way:

```text
live2d-sys -> live2d-core -> live2d-runtime -> live2d-render -> live2d-wgpu
                                                        \-> nanavts-display
```

No Tauri crate is provided in this phase. Tauri applications should launch or embed their own process/window integration and call the normal Rust APIs.

## Features

The facade crate defaults to the platform-independent runtime only:

```toml
[dependencies]
live2d = { git = "https://github.com/sena-nana/live2d-rs.git", package = "live2d" }
```

Enable wgpu when the app wants the built-in renderer:

```toml
live2d = { git = "https://github.com/sena-nana/live2d-rs.git", package = "live2d", features = ["wgpu"] }
```

Enable Cubism-backed loading in crates that need real `.moc3` evaluation:

```powershell
$env:LIVE2D_CUBISM_SDK_DIR = "C:\path\to\CubismSdkForNative"
cargo check -p nanavts-display --features live2d-cubism,wgpu,winit
```

## Runtime API

Applications that provide their own renderer should stop at the render plan boundary:

```rust
use live2d::{
    render::RenderPlanner,
    runtime::{FsAssetResolver, Live2DInstance},
};

let resolver = FsAssetResolver::new("assets/live2d/hiyori");
let mut instance = Live2DInstance::load(&resolver, "hiyori.model3.json")?;
instance.update(1.0 / 60.0);

let snapshot = instance.snapshot();
let plan = RenderPlanner::new().build(snapshot);
```

Applications that want the built-in wgpu path enable `features = ["wgpu"]` and use `live2d::wgpu::WgpuLive2DRenderer`.

## NanaVTS Preview

The preview process preserves the NanaVTS display HTTP surface:

- `GET /schema`
- `POST /session`
- `POST /model/inspect`
- `POST /artmesh-picker/sessions`
- `GET /artmesh-picker/sessions/{id}`

Run it directly:

```powershell
cargo run -p nanavts-display --features wgpu,winit
```

Replay a session fixture:

```powershell
.\crates\nanavts-display\replay.ps1
```

## Validation

```powershell
cargo check --workspace --no-default-features
cargo test --workspace --no-default-features
cargo check -p live2d --features wgpu
cargo check -p nanavts-display --features wgpu,winit
```
