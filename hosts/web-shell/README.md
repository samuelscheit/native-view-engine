# Native View Engine — Web WASM Shell (`hosts/web-shell`)

Mounts any `native-view-engine` Dioxus app directly into browser DOM via `nve-backend-web` (`web-sys`) using the exact same `nve-abi` binary `CommandBuffer` and `NativeEvent` ring used by Skia and React Native Fabric.

## Architecture

1. **`DioxusSurface`** (`nve_studio_workspace::create_surface(1)` by default, or `nve_example_counter::create_surface(1)` via `?app=counter`) reconciles `rsx!` into a batched binary `CommandBuffer`.
2. **`WebBackend::mount_into(container)`** decodes the `CommandBuffer` and mutates real `web_sys::Element` nodes, applying Flexbox `LayoutProps` + `StyleProps` via `HtmlElement::style().set_css_text(...)`.
3. **Interactive DOM Events**: `EventMask::PRESS` (`click`) and `EventMask::TEXT_CHANGE` (`input`) attach real `wasm_bindgen::closure::Closure` listeners that push `NativeEvent`s into a shared `Rc<RefCell<Vec<NativeEvent>>>` and trigger an immediate incremental re-render (`drain_events -> DioxusSurface::dispatch_events -> render_frame -> submit_recorded_frame`).
4. **2-Tier Hot Reload (`nve dev`)**: Connects a browser `WebSocket` (default `ws://127.0.0.1:8089`, configurable via `?ws=<url>` or disabled via `?ws=off`) consuming `nve_engine::HotPatchMsg` JSON:
   - `template_patch`: Applies Tier-1 live text/style updates directly (<50ms, zero page reload).
   - `dylib_reload`: Logs generation & build time and cleanly reloads the page.
   - `dev_status`: Logs status and updates the top HUD badge.

## Prerequisites

- Rust target: `rustup target add wasm32-unknown-unknown` (`--dev` builds on stable)
- `wasm-bindgen` CLI matching the workspace `wasm-bindgen` crate version (`cargo install wasm-bindgen-cli --version <ver>`)
- Release builds additionally need: the pinned nightly with `rust-src` (auto-installed; override with
  `NVE_WASM_TOOLCHAIN`), binaryen `wasm-opt`, `wasm-tools`, `brotli`, `python3`, and optionally `npx`
  (terser JS minification). The release pipeline and its measured savings are documented in
  `docs/research/wasm-size/toolchain.md`; `pkg/` contains precompressed `.br`/`.gz` files to serve.

## Build & Run

```bash
# Build release WASM + JS bindings into hosts/web-shell/pkg/
./hosts/web-shell/build.sh

# Or fast dev build
./hosts/web-shell/build.sh --dev

# Serve locally
python3 -m http.server 8080 -d hosts/web-shell
```

Open `http://127.0.0.1:8080` in any modern browser. To connect live hot reload, run `cargo run -p nve-cli -- dev` in another terminal (serves `ws://127.0.0.1:8089`). Pass `?ws=off` to disable WebSocket hot reload.

## Host Unit Tests & Headless Browser E2E

Both `nve-backend-web` and `nve-web-shell` run headlessly on the host via `WebDomMirror`, and `e2e/run_e2e.mjs` drives the real compiled WASM bundle in headless Chrome over CDP with zero npm dependencies:

```bash
cargo test --locked -p nve-backend-web -p nve-web-shell
cargo check --locked -p nve-web-shell --tests --target wasm32-unknown-unknown
node hosts/web-shell/e2e/run_e2e.mjs
```
