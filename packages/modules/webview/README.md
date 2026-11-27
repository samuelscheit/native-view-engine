# `nve-webview`

Pure-spec capability crate defining the `WebView` (`nve.web_view`) word table and navigation command opcodes.

## Role & API

- Exposes `WebView` (`#[derive(NveTable)]`) with `url` and `allows_back_forward`, plus navigation commands (`Navigate`, `Reload`, `GoBack`, `GoForward`, `EvaluateScript`).
- Platform adapters live in sibling subdirectories (`web/`, `apple/`).

## Invariants

- Pure specification crate with zero backend or platform dependencies.
- Enforces safe URL schemes (`https://`, `http://`, `about:blank`).

## Verification

```bash
cargo test -p nve-webview
```
