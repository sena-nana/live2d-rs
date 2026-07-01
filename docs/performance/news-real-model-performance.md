# News Real Model Performance Report

Date: 2026-07-01

## Summary

The measured model is `C:\Files\workspace\VTSTemplate\assets\live2d\运作档\news.model3.json`.

The main performance hotspot is not steady-state motion or draw encoding. It is the first-use texture path:

- Load time is dominated by PNG decode: `RuntimeTextureDecode` took `67.650 ms` out of `RuntimeLoadSnapshot` `74.966 ms`.
- The model uses one 4096x4096 texture, stored as a 4.15 MiB PNG and expanded to `67,108,864` bytes of RGBA.
- First render uploads the same 64 MiB texture to wgpu: `WgpuTextureUpload` took `28.445 ms`, and the first submit-to-complete wait took `61.823 ms`.
- After 60 warmup frames, steady-state p90 values stay below 1 ms for the measured library-side stages: `RuntimeMotionUpdate` `0.659 ms`, `WgpuPrepareRender` `0.355 ms`, `WgpuMainPassEncode` `0.097 ms`, and `WgpuQueueSubmit` `0.848 ms`.

Conclusion: the obvious stutter is a cold-start/cold-cache spike caused by decoding and uploading a 4096 texture. The current `live2d-rs` library-side steady render loop does not show a persistent per-frame hotspot in this sample.

## Model And Commands

Model assets:

| Asset | Value |
| --- | --- |
| Model | `news.model3.json` |
| Moc | `news.moc3`, `1,324,032` bytes |
| Texture | `news.4096/texture_00.png`, 4096x4096 |
| Texture file size | `4,354,750` bytes |
| Decoded texture size | `67,108,864` bytes |
| Drawables | `271` in load snapshot, `188` rendered after visibility/filtering |
| Motions | `26`, default group `""`, sampled motion `motion/000_idle.motion3.json` |
| Canvas | `6000,9203` |

Environment from reports: Windows x86_64, release build, Rust debug assertions disabled. `LIVE2D_CUBISM_SDK_DIR` was loaded from `.env`.

Commands:

```powershell
cargo run -p live2d-perf --release --features live2d-cubism -- real-model-load --model "C:\Files\workspace\VTSTemplate\assets\live2d\运作档\news.model3.json"
cargo run -p live2d-perf --release --features live2d-cubism -- real-model-motion --frames 600 --model "C:\Files\workspace\VTSTemplate\assets\live2d\运作档\news.model3.json"
cargo run -p live2d-perf --release --features live2d-cubism,wgpu -- real-model-render --frames 1 --warmup-frames 0 --model "C:\Files\workspace\VTSTemplate\assets\live2d\运作档\news.model3.json"
cargo run -p live2d-perf --release --features live2d-cubism,wgpu -- real-model-render --frames 600 --warmup-frames 60 --model "C:\Files\workspace\VTSTemplate\assets\live2d\运作档\news.model3.json"
```

JSON evidence:

- `target/live2d-perf/real-model-load-news.model3.json`
- `target/live2d-perf/real-model-motion-news.model3.json`
- `target/live2d-perf/real-model-render-news.model3-frames-1-warmup-0-1024x1024.json`
- `target/live2d-perf/real-model-render-news.model3-frames-600-warmup-60-1024x1024.json`

## Results

Load path:

| Stage | Calls | Total ms | p90 ms | Bytes |
| --- | ---: | ---: | ---: | ---: |
| `RuntimeLoadSnapshot` | 1 | 74.966 | 74.966 | 0 |
| `RuntimeTextureDecode` | 1 | 67.650 | 67.650 | 67,108,864 |
| `RuntimeMocRead` | 1 | 3.947 | 3.947 | 1,324,032 |
| `RuntimeAssetResolve` | 1 | 1.526 | 1.526 | 0 |
| `RuntimeSnapshotExtract` | 1 | 0.525 | 0.525 | 0 |

Motion-only path:

| Stage | Calls | Total ms | p90 ms | Notes |
| --- | ---: | ---: | ---: | --- |
| `RuntimeMotionUpdate` | 605 | 292.606 | 0.603 | 600 changed frames, 0 motion events |

First real render frame, no warmup:

| Stage | Calls | Total ms | p90 ms | Key counters |
| --- | ---: | ---: | ---: | --- |
| `WgpuPrepareRender` | 1 | 29.092 | 29.092 | includes first GPU resource prep |
| `WgpuTextureUpload` | 1 | 28.445 | 28.445 | 67,108,864 bytes, 1 texture rebuild |
| `WgpuBufferRebuild` | 1 | 0.190 | 0.190 | 258,480 bytes, 3 resource rebuilds |
| `WgpuMaskPassEncode` | 1 | 0.097 | 0.097 | 12 mask draws, 1 cache miss |
| `WgpuMainPassEncode` | 1 | 0.069 | 0.069 | 188 main draws |
| `WgpuQueueSubmit` | 1 | 61.823 | 61.823 | submit-to-complete 61.903 ms |

Steady real render, 60 warmup frames then 600 measured frames:

| Stage | Calls | Total ms | p90 ms | Key counters |
| --- | ---: | ---: | ---: | --- |
| `RuntimeMotionUpdate` | 600 | 320.779 | 0.659 | 600 changed frames |
| `RenderPlanTotal` | 600 | 60.345 | 0.116 | 112,800 draw calls |
| `WgpuPrepareRender` | 600 | 172.952 | 0.355 | no texture upload stage recorded after warmup |
| `WgpuPositionUpload` | 600 | 34.931 | 0.090 | 301 writes, 25,117,848 bytes |
| `WgpuMaskPassEncode` | 600 | 13.778 | 0.031 | 3,612 mask draws, 299 hits, 301 misses |
| `WgpuMainPassEncode` | 600 | 42.814 | 0.097 | 112,800 main draws |
| `WgpuQueueSubmit` | 600 | 323.891 | 0.848 | submit-to-complete total 346.229 ms |

## Hotspot Analysis

The cold load and first render spikes are both texture-size driven.

The PNG file is only 4.15 MiB on disk, but decoding creates a 64 MiB RGBA allocation. Loading spends about 90% of total snapshot time inside `RuntimeTextureDecode`. First render then uploads the same 64 MiB texture to the GPU, which dominates `WgpuPrepareRender`; the queue submit wait is also a cold-frame cost because the upload and first draw have to complete.

The steady-state path is comparatively small. Motion p90 is below 1 ms, render-plan p90 is about 0.1 ms, main-pass encode p90 is below 0.1 ms, and queue submit p90 is below 1 ms. Mask work is present but not the primary cost in this run; misses track animated mask-affecting position uploads, not repeated atlas rebuilds.

## Recommendations

- Treat 4096 texture decode/upload as the root cause of visible first-use stutter.
- Prefer preloading models before presentation, or add an explicit warmup render before the model becomes visible.
- If startup latency still matters, add an asset pipeline option for smaller preview textures, such as 2048, while preserving 4096 for high-quality modes.
- Consider caching decoded texture data or an upload-ready representation only if memory budget allows it; the decoded texture is already 64 MiB for this model.
- If application-side stutter remains after library-side warmup, profile VTSTemplate separately because this report isolates `live2d-rs` library behavior only.
