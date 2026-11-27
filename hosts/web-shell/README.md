# Native View Engine — Web WASM Shell (`hosts/web-shell`)

Mounts any `native-view-engine` Dioxus app directly into browser DOM via `nve-backend-web` (`web-sys`) using the exact same `nve-abi` binary `NVE2` frames and 64-byte `NveEvent` batches used by Skia and React Native Fabric.

## Architecture

1. **`DioxusSurface`** (`nve_studio_workspace::create_surface(1)` by default, or `nve_example_kitchen_sink::create_surface(1)` via `?app=kitchen-sink`) reconciles `rsx!` into a batched binary `CommandBuffer`.
2. **`WebBackend::mount_into(container)`** decodes the `CommandBuffer` and mutates real `web_sys::Element` nodes, applying Flexbox `LayoutProps` + `StyleProps` via `HtmlElement::style().set_css_text(...)`.
3. **Interactive DOM Events**: Table-scoped `EventSet` listeners (`InteractionEvent::Press`, `TextInputEvent::Change`, etc.) drive lazy `0 -> 1` / `1 -> 0` root DOM delegation via `SurfaceEventTracker`, pushing 64-byte `NveEvent`s into a shared `Rc<RefCell<EventBatchBuf>>` and triggering an immediate incremental re-render (`drain_events -> DioxusSurface::dispatch_events -> commit_to`).
4. **2-Tier Hot Reload (`nve dev`, `dev` bundle only)**: `?dev` connects a browser `WebSocket` to `ws://<host>:8089` (`?ws=<url>` picks another endpoint) consuming binary `NVP1` `nve_engine::HotPatchMsg` frames (text frames are ignored):
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
  Release bundles keep panic messages and locations: a production panic logs
  `[NVE Web Panic] panicked at <file>:<line>:<col>: <message>` (the hook is installed when the module
  is instantiated, in every bundle).

## Build & Run

```bash
# Release bundles: pkg/studio, pkg/kitchen-sink (one app each, production) and pkg/dev
# (both apps + NVP1 hot reload, pairing, popups, scene view, E2E probes)
./hosts/web-shell/build.sh

# Only some bundles, or the fast stable debug build (what `nve dev` and the E2E use)
./hosts/web-shell/build.sh kitchen-sink
./hosts/web-shell/build.sh --dev

# Serve locally
python3 -m http.server 8080 -d hosts/web-shell
```

Open `http://127.0.0.1:8080` (studio) or `?app=kitchen-sink` in any modern browser; each loads its own
production bundle. `index.html` loads `pkg/dev` instead for `?dev` / `?ws=<url>` (hot reload) and
`?pair=...` (thin client), or any bundle via `?bundle=<studio|kitchen-sink|dev>`.

For live hot reload just run `cargo run -p nve-cli -- dev` and open the printed
`http://127.0.0.1:8088/` (`--web-port`). `nve dev` serves `hosts/web-shell` itself, rebuilds
`pkg/dev` on save, and redirects a bare `/` or `/?app=kitchen-sink` to the same URL plus
`ws=ws://<host>:<--port>`, so the dev bundle connects to the daemon with no flags to remember
(LAN clients get the host they used). Explicit `ws`, `dev`, `pair` or `bundle` parameters are
never rewritten.

## Caching

`build.sh` publishes every bundle (release and `--dev`) under a content-hashed directory:

```text
pkg/<bundle>/manifest.json                  {"js":"<hash>/nve_web_shell.js","wasm":"<hash>/nve_web_shell_bg.wasm"}
pkg/<bundle>/<hash>/nve_web_shell.js        JS glue        (+ .br / .gz in release builds)
pkg/<bundle>/<hash>/nve_web_shell_bg.wasm   module         (+ .br / .gz)
pkg/<bundle>/<hash>/snippets/...            wasm-bindgen inline JS, imported relative to the glue
```

`<hash>` is the first 16 hex digits of a SHA-256 over the path and bytes of every shipped file, so
any change to the glue, the wasm or a snippet yields a new directory, and a directory's content never
changes. `index.html` fetches `pkg/<bundle>/manifest.json` with `cache: "no-cache"` (always
revalidated), then starts the wasm download in parallel with the glue import and compiles it while
streaming. There is no `?v=` cache-buster.

Serve it like this in production:

| Path | `Cache-Control` |
| --- | --- |
| `pkg/<bundle>/<hash>/**` | `public, max-age=31536000, immutable` |
| `index.html`, `pkg/<bundle>/manifest.json` | `no-cache` (revalidate with `ETag`/`Last-Modified`) |

Serve the precompressed siblings instead of compressing per request: when `Accept-Encoding` lists
`br`, answer with `<file>.br` and `Content-Encoding: br`, else with `gzip` and `<file>.gz`, else the
file itself. Always send the original type (`application/wasm` for `.wasm`, which streaming
compilation requires, `text/javascript` for `.js`) and `Vary: Accept-Encoding`. For example, nginx:
`brotli_static on; gzip_static on;` plus a `location ~ ^/pkg/[^/]+/[0-9a-f]{16}/` block with the
immutable header; Caddy: `file_server { precompressed br gzip }`.

`nve dev` and the E2E server send `Cache-Control: no-store` for everything, and every `--dev` rebuild
publishes a new hash directory, so the browser always loads the fresh build.

## Budgets

Release bundles are checked against per-bundle brotli budgets (wasm + JS, override with
`WASM_BROTLI_BUDGET_{STUDIO,COUNTER,DEV}`); `build.sh` fails when one is exceeded.

## Host Unit Tests & Headless Browser E2E

Both `nve-backend-web` and `nve-web-shell` run headlessly on the host via `WebDomMirror`, and `e2e/run_e2e.mjs` drives the real compiled WASM bundle in headless Chrome over CDP with zero npm dependencies:

```bash
cargo test --locked -p nve-backend-web -p nve-web-shell
cargo check --locked -p nve-web-shell --tests --target wasm32-unknown-unknown
node hosts/web-shell/e2e/run_e2e.mjs
```
