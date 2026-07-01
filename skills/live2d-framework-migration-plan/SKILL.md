---
name: live2d-framework-migration-plan
description: Plan live2d-rs work to migrate or implement official Live2D Cubism Native Framework capabilities. Use before implementing or scoping framework parity, missing capability backlogs, Cubism 5.3/R5 features, updater ordering, physics, expression, pose, offscreen drawing, blend modes, multiply/screen color, or renderer/runtime boundary changes.
---

# Live2D Framework Migration Plan

## Workflow

1. Read `../live2d-framework-reference/references/framework-capabilities.md` first.
2. Read `references/migration-rules.md`.
3. Use CodeGraph to inspect the current symbols and files named by the capability area before proposing or editing code.
4. Select the existing area skill for implementation details: `live2d-core-runtime`, `live2d-render-plan`, `live2d-wgpu-renderer`, or `live2d-cubism-perf`.

## Planning Output

Produce a phased plan with the target official capability, current coverage/gap, crate-by-crate sequence, public API or data-shape changes, validation commands, and explicit non-goals.

Keep the plan grounded in repository boundaries. Do not introduce a Tauri crate, `tauri` feature, application display protocol, or SDK vendoring.
