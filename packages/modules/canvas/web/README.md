# `nve-canvas-web`

Thin Web (`nve-backend-web`) leaf adapter rendering direct `DirectCanvasRegistry` callbacks and flat `nve.res.draw_list` (`FlatResource`) draw lists to HTML5 `<canvas>` 2D contexts and SSR attributes.

## Role & API

- Exports `CanvasWebSystem::{new, with_direct}` implementing `WebTableSystem` (`watches_slot(Layout::slot())`, `commit_node`, `sync_connected`, `remove_node`, `reset`) for `Canvas`.
- Exports `WebCanvas2dPainter` and `WebCanvas2dSink` on `wasm32` backed by `GuardedPainter` and a stack-allocated CSS color formatter.

## Invariants

- Leaf backend adapter only; `nve-backend-web` has zero compile-time dependency on `nve-canvas`.
- Emits deterministic `<canvas>` markup for zero-mismatch SSR hydration and repaints in Step 6 (`sync_connected`) with true connected layout dimensions.

## Verification

```bash
cargo test -p nve-canvas-web
```
