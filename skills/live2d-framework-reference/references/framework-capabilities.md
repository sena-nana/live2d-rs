# Cubism Native Framework Capability Map

## Version And Sources

- Local SDK anchor: `.env` sets `LIVE2D_CUBISM_SDK_DIR=C:/Files/workspace/_CubismSdkForNative-5-r.5_backup`.
- Local version: `cubism-info.yml` reports `version: 5-r.5`, created `20260401T135000+0900`.
- Official docs: Native overview, SDK manual index, 5.3 compatibility, R5 official compatibility, blend mode, offscreen drawing, Native multiply/screen color, and model calculation order reordering:
  - https://docs.live2d.com/en/cubism-sdk-manual/cubism-sdk-for-native/
  - https://docs.live2d.com/en/cubism-sdk-manual/top/
  - https://docs.live2d.com/en/cubism-sdk-manual/compatibility-with-cubism-5-3/
  - https://docs.live2d.com/en/cubism-sdk-manual/compatibility-with-cubism-5-3-official/
  - https://docs.live2d.com/en/cubism-sdk-manual/blend-mode-ow/
  - https://docs.live2d.com/en/cubism-sdk-manual/offscreen-drawing-alias-ow/
  - https://docs.live2d.com/en/cubism-sdk-manual/multiply-color-screen-color-native/
  - https://docs.live2d.com/en/cubism-sdk-manual/model-param-updater/

Status labels: `covered`, `partial`, `missing`, `out-of-scope`.

## Capability Matrix

| Framework area | Official capability | Local SDK evidence | live2d-rs status | Crate boundary |
| --- | --- | --- | --- | --- |
| Effect | Eye blink, breath, look, pose parameter updates. | `Framework/src/Effect/*`, `Framework/src/Motion/*Updater.hpp` | `partial`: motion layers exist; effect parity is not complete. | `live2d-runtime` |
| Id | Typed parameter, part, drawable IDs. | `Framework/src/Id/*`, `CubismDefaultParameterId.hpp` | `partial`: Rust identifiers exist; no full ID manager/default catalog. | `live2d-core`, `live2d-runtime` |
| Math | Matrix, vector, model/view transforms, target point. | `Framework/src/Math/*` | `partial`: canvas/view data exist; Framework helpers are not standalone APIs. | `live2d-core`, `live2d-render` |
| Model | Moc/model lifecycle, settings, drawable data, user data, multiply/screen color. | `Framework/src/Model/*`, `CubismModelSettingJson.hpp` | `partial`: loading, model3, snapshots, textures, motions, ArtMesh inspection exist; user data and override colors are incomplete. | `live2d-sys`, `live2d-core`, `live2d-runtime` |
| Motion | Motion playback, queues, fade, expression motion, events, R5 scheduler. | `Framework/src/Motion/*` | `partial`: motion3 playback, queues, fade, layers, idle/request APIs, and events exist; expression parity and scheduler ordering are incomplete. | `live2d-runtime`, `live2d-perf` |
| Physics | Physics JSON, simulation, stabilization, scheduled updates. | `Framework/src/Physics/*`, `CubismPhysicsUpdater.hpp` | `missing`: no equivalent evaluator or stabilization path. | `live2d-runtime`, `live2d-cubism-perf` |
| Rendering | Masks, blend modes, drawing, model-internal offscreen, backend renderers. | `Framework/src/Rendering/*` | `partial`: render plans, masks, material grouping, and wgpu backend exist; Cubism model-internal offscreen is not covered. Native D3D/OpenGL/Metal/Vulkan ports are out of scope. | `live2d-render`, `live2d-wgpu` |
| Type | C++ containers and primitive aliases. | `Framework/src/Type/*` | `out-of-scope` unless raw FFI layout requires it. | `live2d-core`, `live2d-sys` |
| Utils | JSON, strings, debug/logging, framework helpers. | `Framework/src/Utils/*`, `CubismFramework.hpp` | `partial`: Rust serde/logging paths exist; CDI/supplementary parsing is not baseline. | `live2d-runtime`, `live2d-sys` |

## Cubism 5.3 And R5 Items

- Blend mode: live2d-rs has `BlendMode` data and wgpu pipeline selection; verify Core extraction, render-plan data, and shader behavior before claiming parity.
- Model-internal offscreen drawing: hierarchy-aware model rendering feature, not the same as rendering the whole model to an application-owned offscreen target.
- Multiply/screen color: check model-authored colors and SDK-side overrides separately through Core extraction, snapshot data, render planning, and shader inputs.
- R5 update order: when implementing eye blink, expression, breath, physics, lip sync, pose, or look, account for Framework Motion scheduler ordering.
