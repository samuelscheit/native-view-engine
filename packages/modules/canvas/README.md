# `nve-canvas`

Pure-spec capability crate defining the `Canvas` (`nve.canvas`) word table, the canonical flat 4-byte-aligned `DrawList` (`nve.res.draw_list` `FlatResource`), and `DRAW_LIST_CODEC_DESC` for cross-language host decoder codegen.

## Role & API

- Exposes `Canvas` (`#[derive(NveTable)]`) referencing a `DrawList` flat word-op stream resource (`DrawListRecorder`, `DrawListView`, `CanvasOp`, `ClipDepthTracker`).
- Exports `DRAW_LIST_CODEC_DESC` (`nve_codec::FlatOpStreamDesc`) consumed by `nve-cli codegen abi` to generate C++, Swift, Java, Dart, and TypeScript decoders.
- Leaf backend rasterizers live in `web/` and `skia/`; C++20 host bindings live in `cxx/`.

## Invariants

- Pure specification crate with zero backend or platform dependencies.
- Bounds-checked, finite/non-negative float-validating flat word-op stream with `MAX_CLIP_DEPTH = 64` clip-stack balancing shared across Rust and generated host decoders.

## Verification

```bash
cargo test -p nve-canvas
```
