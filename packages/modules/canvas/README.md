# `nve-canvas`

Pure-spec capability crate defining the `Canvas` (`nve.canvas`) word table, the object-safe `CanvasPainter` / `CanvasSink` / `GuardedPainter` drawing pipeline, optional in-process `DirectCanvasRegistry` + `write_canvas`, the canonical flat 4-byte-aligned `DrawList` (`nve.res.draw_list` `FlatResource`), and `DRAW_LIST_CODEC_DESC` for cross-language host decoder codegen.

## Role & API

- Exposes `Canvas` (`#[derive(NveTable)]`, 2 words: `draw_list: Res<DrawList>`, `generation: u32`) with zero cross-language ABI drift.
- Provides object-safe `CanvasPainter`, zero-alloc `PolylinePoints<'a>`, `CanvasSink`, `GuardedPainter<S>`, `DirectCanvasRegistry`, `write_canvas`, single-pass flat-word `DrawList` (`Arc<[u32]>`, `DrawList::record`, `DrawList::record_into_words`, `DrawList::as_view`, zero-copy `FlatResource::write_words`), `DrawListView::replay`, and stack-buffered `paint_sparkline`.
- Exports `DRAW_LIST_CODEC_DESC` (`nve_codec::FlatOpStreamDesc`) consumed by `nve-cli codegen abi` to generate C++, Swift, Java, Dart, and TypeScript decoders.
- Leaf backend rasterizers live in `web/` and `skia/`; C++20 host bindings live in `cxx/`.

## Invariants

- Pure specification crate with zero backend or platform dependencies.
- Canonical sanitization, NaN polyline splitting (0 allocs), and `MAX_CLIP_DEPTH = 64` RAII clip-stack balancing shared across `DrawListRecorder`, Skia, Web, and generated host decoders.

## Verification

```bash
cargo test -p nve-canvas
```
