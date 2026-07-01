# live2d-rs

Rust workspace for Live2D model loading, runtime snapshots, platform-neutral render planning, and optional wgpu preview rendering.

This repository intentionally does not vendor the official Live2D Cubism SDK. Set `LIVE2D_CUBISM_SDK_DIR` to a local Cubism SDK for Native checkout when enabling Cubism-backed model evaluation.

## Crates

```text
live2d-sys      Cubism Core FFI and SDK link probing
live2d-core     Platform-independent model, drawable, canvas, texture, and identifier types
live2d-runtime  Asset resolving, model3 parsing, model loading, update, snapshot, ArtMesh inspect
live2d-render   Snapshot-to-RenderPlan conversion without wgpu or window-system types
live2d-wgpu     Optional wgpu backend, Live2D renderer, preview renderer, and WGSL assets
live2d          Facade crate with opt-in features
```

The dependency direction is one way:

```text
live2d-sys -> live2d-core -> live2d-runtime -> live2d-render -> live2d-wgpu
```

No Tauri or application-specific display crate is provided. Applications should own their process/window/protocol integration and call the normal Rust APIs.

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
cargo check -p live2d --features live2d-cubism,wgpu
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

Motion playback can use `.model3.json` motion metadata directly, including fade times:

```rust
use live2d::{resolve_model_files, update_instances_into, Live2DInstance};

let files = resolve_model_files("assets/live2d/hiyori/hiyori.model3.json")?;
let idle = files
    .motion_groups
    .iter()
    .find(|group| group.name == "Idle")
    .and_then(|group| group.motions.first())
    .ok_or("idle_motion_missing")?;

let mut instance = Live2DInstance::load_file(&files.model_json_path)?;
instance.set_idle_motion_file(idle)?;

if instance.update_motion(1.0 / 60.0)? {
    // Rebuild the render plan or upload dynamic buffers for this model.
}
for event in instance.motion_events() {
    println!("motion event: {}", event.value);
}

let tap = files
    .motion_groups
    .iter()
    .find(|group| group.name == "TapBody")
    .and_then(|group| group.motions.first())
    .ok_or("tap_motion_missing")?;
instance.request_motion_file(tap, false)?;

let mut first = instance;
let mut second = Live2DInstance::load_file(&files.model_json_path)?;
let mut changed_indices = Vec::new();
update_instances_into([&mut first, &mut second], 1.0 / 60.0, &mut changed_indices)?;
```

Applications that want the built-in wgpu path enable `features = ["wgpu"]` and use `live2d::wgpu::WgpuLive2DRenderer`. The same backend owns the built-in `WgpuPreviewRenderer`; application preview code should call that backend instead of keeping local wgpu shader or pipeline state.

## Validation

```powershell
cargo check --workspace --no-default-features
cargo test --workspace --no-default-features
cargo run -p live2d-perf -- motion-update --profile medium --frames 300
cargo check -p live2d --features wgpu
cargo check -p live2d --features live2d-cubism,wgpu
cargo run -p live2d-perf --features live2d-cubism -- real-model-motion --model <path-to-model3.json> --frames 300
```
