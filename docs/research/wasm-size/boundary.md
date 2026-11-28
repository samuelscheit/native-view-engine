# WASM size research: the Rust ↔ JS boundary (track `boundary`)

Scope: wasm-bindgen, web-sys, js-sys, Closures, string marshalling, JS glue, and where DOM-apply
logic should live. Baseline HEAD `4e046ca`, stable rustc 1.98.1, wasm-bindgen 0.2.130,
binaryen 133, Chrome headless (macOS).

## TL;DR

- **The boundary can cost essentially zero bytes.** The prototype ships with **0 imports and
  5 exports** (`memory`, `nve_init`, `nve_events`, `nve_dispatch`, `nve_frame_len`). A
  hand-written 17 KB (9 KB minified, **3.5 KB brotli**) `nve_dom.js` decodes the binary
  `CommandBuffer` straight out of linear memory and writes DOM events back as an EventRing.
- Measured on the same app and profile, compared with wasm-bindgen + `nve-backend-web`:
  - **Counter:** 105,304 B vs 124,443 B total brotli: **−19.1 KB (−15.4 %)**.
  - **Studio-workspace (the real demo):** 187,574 B vs 204,056 B: **−16.5 KB (−8.1 %)**.
  - DOM output is **byte-identical** at every step of a 10-step real-input studio scenario and a
    4-step counter scenario (headless Chrome, `hosts/web-raw/e2e.mjs`).
- It is also faster: counter press round-trip 8.5 µs vs 10.4 µs (−18 %). Studio is 154 µs vs
  159 µs, because Dioxus diffing in Rust dominates there.
- **Hard blocker found:** `dioxus-core 0.7.10` → `subsecond 0.7.10` *unconditionally* depends on
  wasm-bindgen/js-sys/web-sys/wasm-bindgen-futures on wasm32. Without the wasm-bindgen CLI, every
  js-sys `__wbindgen_describe_*` function stays exported, which makes the "raw" wasm **+52 % raw /
  +38 % brotli** *larger* than the wasm-bindgen build. The fix is a 10-line patch that puts the
  wasm patch loader behind an opt-in feature (vendored here, to be upstreamed).
- **Migration step with no upstream dependency:** "hybrid" keeps wasm-bindgen only to export the
  four raw functions, and still moves DOM apply to `nve_dom.js`. It measures **188,141 B vs
  205,041 B (−16.9 KB)** on studio, the same win as pure raw.
- **Projection for today's full web-shell (260 KB br):** about **−25…30 KB brotli** from the
  boundary alone (the studio delta plus the shell's own wasm-bindgen glue and the 64 KB JS glue).
  About −12…20 KB more is possible once the dev-only code (hot-reload JSON, pairing, popup,
  WebSocket) leaves the production wasm (cross-track, see Handoffs).
- After this change the remaining ~100 KB (counter) / ~184 KB (studio) brotli is Dioxus, std
  (fmt/panic/demangle), allocator, CSS-string parsing and app code. That is **not boundary**: the
  other tracks own it.

## 1. What the boundary costs today

### 1.1 Full web-shell attribution (symbol build, twiggy, raw code bytes)

| Item | Raw bytes | Notes |
|---|---:|---|
| `nve_backend_web` (DOM apply via web-sys, Closures, CSS strings) | 28,315 | only exists to drive DOM through web-sys |
| `nve_web_shell::wasm_api` (controller, WebSocket hot reload, resize) | 14,881 | |
| `nve_web_shell::wasm_popup` | 6,683 | popup + BroadcastChannel glue |
| `nve_web_shell::wasm_pair` | 4,679 | pairing WebSocket glue |
| Import section (120 imports, hashed names `__wbg_createElement_de4a31ed2fb252be`) | 7,057 | |
| Export section (69 exports) | 2,002 | |
| `wasm_bindgen` runtime + `web_sys` + `js_sys` | 6,975 | |
| Closure invoke/drop shims | 3,192 | one per closure signature |
| `__wbg_*`/`__wbindgen_*`/externref table fns | 3,066 | |
| `IntoWasmAbi`/`FromWasmAbi`/`RefFromWasmAbi` impls | 1,248 | |
| Table/elem | 1,146 | |
| **Sum** | **≈ 79 KB raw (9.4 % of 842 KB)** | plus float-formatting and hashbrown share, below |
| JS glue `nve_web_shell.js` + 2 snippets | 64,716 raw / 8,826 br | |

### 1.2 Clean A/B on the counter (same app, only the boundary differs)

twiggy on symbol builds, crates aggregated with hashes stripped. Raw build = 299,695 B, wb build =
345,905 B (non-custom sections). The +46 KB of the wasm-bindgen build is:

| Crate / item | wb − raw (bytes) | Why |
|---|---:|---|
| `nve_backend_web` | +25,317 | `submit_frame` 16 KB, `sync_event_listeners` 3 KB, DOM mirror |
| `core` | +9,642 | float→string formatting for CSS (`{:.3}`, `{}px`: `format_exact`, grisu/dragon exact) |
| `hashbrown` | +5,573 | `HashMap<NodeId, Element>`, listeners, `raw_props` maps |
| `nve_components` | +4,555 | NVE_DRAW1 `DrawListDecoder` (canvas), base64 |
| import section | +4,492 | |
| `wasm_bindgen` + `web_sys` + `js_sys` | +5,570 | |
| `start` + `wb::start` closures + `Rc<RefCell<App>>` drop | +13,000 | host wiring (raw: `nve_init`/`nve_dispatch` 12.6 KB, which inline the render path instead) |
| `alloc`, `.rodata` (CSS keywords), exports, externref tables | ≈ +8,000 | |

Float formatting in the wasm: wb 14,480 B vs raw 7,746 B. What raw still carries comes from
`AttributeValue::Float.to_string()` in nve-dioxus and the runtime CSS parsing `f32::from_str`.
JS gets number→string for free.

### 1.3 Upstream quirks hit (also in `docs/UPSTREAM.md`)

1. **wasm-bindgen 0.2.130 + `strip = "symbols"`:** `wasm-ld --strip-all` drops the
   `target_features` section, so wasm-bindgen cannot see multivalue and falls back to the
   shadow-stack `retptr` return ABI (`__wbindgen_add_to_stack_pointer`, `getDataViewMemory0`
   reads). JS grows 64,404 → 58,186 B raw (−6.2 KB) with `strip = "debuginfo"`, which keeps the
   section. That is only −0.1 KB brotli because it compresses well. Tracked upstream in
   wasm-bindgen PRs #5205/#5382.
2. **`subsecond 0.7.10` unconditional wasm32 deps** (see TL;DR). Measured without the patch,
   counter raw: 521,130 raw / 143,148 br. With the patch: 298,923 / 104,148.
3. **wasm-ld padded LEB128 relocations:** rustc's wasm-ld output pads every relocated LEB128 to
   5 bytes. wasm-bindgen (walrus) and wasm-opt silently re-encode, so wasm-bindgen users never
   see it. A raw pipeline must re-encode explicitly. Counter: 299,197 → 276,929 B raw (−7.4 %),
   104,167 → 102,915 B brotli (−1.25 KB), either via a binaryen round trip with no passes or
   `-C link-arg=--compress-relocations`. This is why the "hybrid" wasm came out *smaller* than
   pure raw before the fix.
4. **Browser sync events during DOM mutation:** removing a focused `<input>` fires `blur`
   synchronously inside the mutation, which re-enters the host mid-frame. web-shell hides this
   with `try_borrow_mut` (the event waits until the next pump). The prototype's first baseline
   version panicked (`unreachable` under panic=abort). `nve_dom.js` instead queues events raised
   while a frame is applied and flushes them right after, so Rust is never re-entered while JS
   reads the frame buffer.

## 2. Zero-overhead design (prototype: `hosts/web-raw`)

```
Rust (wasm, 0 imports)                         nve_dom.js (≈3.5 KB br)
nve_init(surface)  → frame_ptr  ────────────▶  decode frame in place (DataView, one TextDecoder)
                                               apply: createElement / replaceChildren / textContent /
                                               style.cssText (layout+style → CSS in JS) / attrs /
                                               ARIA map / object-fit / canvas draw_list
DOM event (1 delegated listener per type)  ◀── click, input, composition*, focusin/out,
nve_events(cap) → ptr                           scroll(capture), keydown (submit + global key)
  JS writes EventRing (encode_events format)   straight into linear memory (TextEncoder.encodeInto)
nve_dispatch(len)  → frame_ptr | 0  ───────▶   apply next frame (re-entrancy queued)
```

- **One crossing per frame and one per DOM event.** wasm-bindgen + web-sys makes one import call
  per DOM operation (createElement, setAttribute, set_css_text, addEventListener…), plus a
  `TextDecoder` per string and a JS `Closure` (externref table slot plus invoke shim) per listener
  per node.
- **No closures, no externref table.** Event delegation means O(1) listeners. A node is
  identified by an expando `el.n` on the element, so JS needs no handle table and Rust never sees
  a DOM handle. Integer `NodeId`s are the only identity crossing the boundary, which is already
  the repo's invariant.
- **Event bubbling semantics preserved:** a press on a nested `<span>` emits one Press per
  ancestor with `EventMask::PRESS`, exactly like per-node listeners (asserted in the e2e).
- **`NveApp` is the only Rust API.** The four exports are generic over `NveApp`
  (`CounterModule` or `StudioWorkspaceModule`), so this is `export_nve_app!` for the web. EventRing
  bytes go to `NveApp::dispatch_events` unchanged: no host-side decode or copy.
- **Strings:** decoded once per op straight from memory. For JS→Rust text, `encodeInto` writes
  directly into the reserved wasm buffer.
- **Generated-from-spec:** `hosts/web-shell/nve_core_abi.generated.ts` only carries constants
  (opcodes, kinds, value tags, event kinds), **no decoder**. `crates/nve-codegen/specs/
  nve_core_abi.nve.rs` has no per-opcode field layouts (e.g. SetLayout's 82-byte record). The
  `nve_dom.js` "ABI tables" block (opcodes, kind→tag, enum→CSS keyword arrays, EventMask bits,
  record offsets) is the part to generate: add `#[nve_hybrid_record]` op layouts to the spec and
  emit `nve_dom_abi.generated.js`, keeping the DOM-apply logic hand-written.

Prior art:

- Dioxus's `sledgehammer_bindgen` + `dioxus-interpreter-js` batch all mutations into one byte
  buffer and decode strings once per batch. That is the same idea, but it keeps wasm-bindgen for
  the call and generates the JS interpreter from Rust macros.
  ([sledgehammer_bindgen](https://github.com/ealmloff/sledgehammer_bindgen),
  [Dioxus project structure](https://dioxuslabs.com/learn/0.7/beyond/project_structure/),
  [dioxus-interpreter-js features](https://lib.rs/crates/dioxus-interpreter-js/features))
- NVE already has the binary op buffer (`CommandBuffer`), so it only needs the interpreter half.
- Raw exports without wasm-bindgen are a known pattern
  ([users.rust-lang.org](https://users.rust-lang.org/t/export-functions-from-wasm-library-w-o-using-wasm-bindgen/51778),
  [Rust→Wasm guide](https://www.webassembly-wasm.com/compilation-pipelines-toolchain-setup/rust-to-wasm-compilation-guide/)).
  Reported speedups for raw vs wasm-bindgen on array work are about 4.5×
  ([DEV benchmark](https://dev.to/bence_rcz_fe471c168707c1/rust-webassembly-performance-javascript-vs-wasm-bindgen-vs-raw-wasm-with-simd-4pco)).
- Allocator: `lol_alloc` is 356–656 B vs ~5 KB for dlmalloc but "not production ready"
  ([crates.io](https://crates.io/crates/lol_alloc/0.1.2)). That belongs to the toolchain track.
- Published framework sizes are inconsistent (Leptos ~86–100 KB gz, Yew ~110–180 KB, Dioxus
  ~100–300 KB gz, no controlled method;
  [pistack](https://www.pistack.xyz/posts/2026-08-29-dioxus-vs-yew-vs-leptos-rust-wasm-frameworks-comparison/),
  [rustify](https://rustify.rs/articles/leptos-vs-dioxus-rust-frontend-2026)). Our counter at
  101.8 KB brotli wasm is in that band, and the boundary is no longer what dominates it.

## 3. Measurements

Protocol: `docs/research/wasm-size/boundary/measure.sh` reports wasm raw / `gzip -9` /
`brotli -q 11`, and JS raw / gzip / brotli. "Total br" = wasm br + JS br. Build:
`hosts/web-raw/build.sh` (`STRIP=debuginfo`, custom sections stripped with
`wasm-tools strip -a`; `WASM_OPT=1` adds `wasm-opt -Oz`; `HYBRID=1` adds the hybrid variant).
"min JS" = `bun build --minify`.

| Experiment | wasm raw | wasm gz | wasm br | JS raw | JS br | **total br** |
|---|---:|---:|---:|---:|---:|---:|
| Full web-shell, baseline (`strip=symbols`, no wasm-opt) | 842,689 | 310,944 | 251,485 | 64,716 | 8,826 | **260,311** |
| Full web-shell, `strip=debuginfo` (multivalue glue) | 840,740 | 311,312 | 251,416 | 58,186 | 8,730 | 260,146 |
| Full web-shell, same pipeline, unminified names (binaryen -O0) | 837,358 | 310,358 | 250,824 | 64,404 | 8,630 | 259,454 |
| Full web-shell, `--minify-imports-and-exports-and-modules` + JS rename | 829,499 | 307,368 | 248,709 | 56,464 | 7,219 | **255,928** |
| Counter wb (wasm-bindgen + nve-backend-web) | 349,757 | 144,953 | 122,436 | 24,221 | 4,299 | 126,735 |
| Counter raw, **unpatched subsecond** (no wasm-bindgen CLI) | 521,130 | 173,942 | 143,148 | 12,806 | 4,193 | 147,341 |
| Counter raw (patched subsecond) | 299,197 | 125,599 | 104,167 | 16,956 | 5,516 | 109,683 |
| Counter raw + binaryen round trip (no passes) | 276,929 | 122,820 | 102,915 | — | — | — |
| Counter raw + `-C link-arg=--compress-relocations` | 276,885 | 123,133 | 103,008 | — | — | — |
| Counter hybrid (wasm-bindgen exports only + nve_dom.js) | 277,748 | 120,477 | 102,888 | 21,585 | 6,639 | 109,527 |
| Counter wb, `-Oz` + min JS | 324,905 | 144,369 | 121,129 | 10,918 | 3,314 | 124,443 |
| **Counter raw, `-Oz` + min JS** | 260,700 | 120,348 | 101,767 | 8,938 | 3,537 | **105,304** |
| Studio wb | 663,344 | 245,039 | 200,742 | 24,221 | 4,299 | 205,041 |
| Studio raw | 636,978 | 230,239 | 184,037 | 16,956 | 5,516 | 189,553 |
| Studio hybrid | 592,362 | 220,454 | 181,502 | 21,585 | 6,639 | 188,141 |
| Studio wb, `-Oz` + min JS | 615,172 | 248,949 | 202,874 | 10,918 | 3,314 | 206,188 |
| Studio raw, `-Oz` + min JS | 550,591 | 226,662 | 184,086 | 8,938 | 3,537 | 187,623 |
| **Studio wb + min JS / raw + min JS (no -Oz)** | | | 200,742 / 184,037 | | 3,314 / 3,537 | **204,056 / 187,574** |

`wasm-opt -Oz` makes brotli *worse* on the studio builds (+2.1 KB for wb). That is the gzip/brotli
regression the toolchain track is chasing.

Runtime (headless Chrome, synchronous `click()` loop, includes DOM event → dispatch → Dioxus
render → DOM apply):

| | raw | wb |
|---|---:|---:|
| Counter, 2,000 presses | 8.5 µs | 10.4 µs |
| Studio task-status pill, 300 presses | 154.3 µs | 158.7 µs |

Correctness: `node hosts/web-raw/e2e.mjs` runs both apps on both variants with real CDP input.
The studio steps are search typing, clear, composer Enter-submit, status pill, Cmd+K palette with
filter/ArrowDown/Enter, nav metrics (canvas), stream with scroll, and docs. It asserts app
behaviour, **byte-identical `outerHTML` raw vs wb after every step** (up to 121 K chars),
sparkline canvas ink in both variants, and no page errors. Passing for raw, hybrid and `-Oz`
builds.

## 4. What moves where (nve-backend-web split and JS budget)

| Concern | Today (Rust, wasm) | Proposed | JS cost (in the measured 3.5 KB br) |
|---|---|---|---|
| CommandBuffer decode | `CommandDecoder` + `Vec<Command>` alloc | JS DataView, in place | included |
| CSS translation (layout/style → cssText) | `css.rs` + core float fmt | JS (native number→string; shortest-f32 helper keeps Rust `{}` output) | included |
| ARIA / input attr mapping, object-fit | `dom.rs` | JS tables | included |
| Event listeners, IME guard, submit, global keydown | `events.rs` (8 Closures per input node) | JS delegated listeners; `EventRing` bytes built in JS | included |
| `<canvas>` NVE_DRAW1 paint | `canvas.rs` + `DrawListDecoder` + base64 (Rust) | JS (`atob`, DataView, 2D context) | included (~1 KB br) |
| Scroll / virtual lists | Rust app logic, scroll events | unchanged: JS only reports scroll offsets | included |
| `WebDomMirror` (native) | test oracle + SSR `render_html` | **stays in Rust** as the semantic spec and SSR | 0 |
| Hot reload (WebSocket, `HotPatchMsg` JSON) | wasm_api + serde_json (~30 KB raw) | dev-only JS module (WebSocket) + Rust `nve_hot_patch(ptr,len)` behind a cargo `dev` feature, binary `NVP1` only | dev bundle only |
| Pairing (`NveWireFrame` protocol, `PairClientMachine`) | `pairing.rs` (pure logic) + `wasm_pair.rs` glue | protocol stays Rust (testable) behind `pair` feature; WebSocket pump in a dev JS module | dev bundle only |
| Popup / multi-window (`window.open`, BroadcastChannel) | `wasm_popup.rs`, `multi_window.rs` | JS shell (~1 KB) + raw `nve_window_*` exports | <1 KB |
| Viewport/resize | ResizeObserver Closures | JS ResizeObserver → Layout event (already in prototype for init) | included |
| `NveSceneView` ptr export | wasm-bindgen fn | plain export, since memory is exported anyway | 0 |

Production JS budget for full parity with today's web host: **≤ 5 KB brotli**, including a
viewport observer and popup shell. That replaces 8.8 KB brotli of glue plus about 25 KB brotli of
wasm-side bindings and backend code.

## 5. Ranked changes

| # | Change | Saving (brotli) | Effort | Risk | What breaks |
|---|---|---:|---|---|---|
| 1 | Raw ABI + `nve_dom.js` interpreter (delete wasm DOM path of nve-backend-web and wasm-bindgen from prod) | **−16.5 KB studio (measured), −19.1 KB counter; full shell projected −25…30 KB** | L | Medium: DOM semantics exist twice (JS ship, Rust `WebDomMirror` spec); mitigated by parity tests | `WebAppController` JS API (`__nveController` in e2e → JS host object), `build.sh`, index.html boot, wizer flow (simpler: raw exports) |
| 2 | Upstream/patch `subsecond` (`wasm-patching` opt-in feature) | prerequisite for #1 (−38 KB br vs naive raw) | S | Low | wasm subsecond hot-patching needs the feature (unused by this repo) |
| 3 | Hybrid first step: keep wasm-bindgen only for the 4 exports + dev APIs, DOM apply in JS | **−16.9 KB studio (measured)** | M | Low | same as #1 minus the build pipeline change; needs no upstream patch |
| 4 | Dev code out of prod wasm (`dev`/`pair` features, dev JS module) | est. −12…20 KB (HotPatchMsg JSON 16.6 KB + serde_json 13 KB + pairing 11.9 KB + glue raw) | M | Low | hot reload/pair only in dev builds |
| 5 | Re-encode step in the raw pipeline (binaryen round trip or `--compress-relocations`) | −1.25 KB counter | S | None | — |
| 6 | Minify host JS | −2.0 KB (nve_dom 5.5 → 3.5 KB) | S | None | — |
| 7 | (If staying on wasm-bindgen) minify import/export/module names | −3.5 KB full shell | S/M | Low (post-process JS rename) | debugging names |
| 8 | `strip = "debuginfo"` + strip in a post step (keeps `target_features`) | −0.1 KB br (−6.2 KB raw JS) | S | None | — |
| 9 | ABI v2 for JS decode: interned string table (define-once atom ids for attr keys/CSS strings), CSS-ready enum bytes | perf (fewer TextDecoder calls); size neutral | M | Low | ABI version bump |

## 6. Feasibility verdict

**Feasible, and the right direction.** The repo's invariant (one flat binary CommandBuffer per
frame) already *is* the boundary. wasm-bindgen + web-sys re-explodes it into hundreds of
per-operation calls, Closures and externref slots. Removing that layer is a clean subtraction:
the CommandBuffer encoding, `NveApp`, EventRing and `WebDomMirror` are unchanged, and the
prototype reaches byte-identical DOM on the full studio app.

Effects on testing:

- **`WebDomMirror` stays** as the semantic spec and SSR renderer, and native unit tests stay
  sub-second. The shipped DOM logic moves to JS, so add a fast Node parity test: feed the
  CommandBuffer fixtures used by `crates/nve-backend-web/src/tests.rs` into `nve_dom.js` with a
  ~40-line fake DOM serializer, and compare against `WebDomMirror::render_html()`. The real-browser
  parity check already exists in `hosts/web-raw/e2e.mjs`.
- **`hosts/web-shell/e2e/run_e2e.mjs`** drives the DOM through testids and real input, so it
  ports unchanged. Only `window.__nveController.{node_count,app_name}` need the JS host object
  (`nve.nodes.size`).

**Boundary floor ("hand-written WAT"):** 0 imports, 5 exports (65-byte export section), ~3.5 KB
brotli JS. Everything else in the wasm is app, framework and std. For that, see the toolchain,
framework and app tracks.

## 7. Handoffs (other tracks)

- **toolchain:** `rustc_demangle` (~20 KB raw) and panic/fmt machinery are in *every* variant,
  raw too (std's panic hook prints a backtrace even with panic=abort), so try
  `-Zbuild-std` + `panic_immediate_abort`. `anyhow` 7.8 KB comes via dioxus-core. dlmalloc 7.7 KB
  could be replaced by `lol_alloc`/talc. `wasm-opt -Oz` raises brotli on studio (+2.1 KB).
- **framework/app:** runtime CSS-string parsing (`f32::from_str` 3.8 KB, `parse_color`,
  `apply_str_attr`) could move to compile time (rsx macro → typed props). `.rodata` is 44.7 KB in
  the *counter* raw build.
- **Bug found along the way (out of scope):** `nve-dioxus` forwards CSS-only raw style props
  (`font-weight`, `text-align`, `box-shadow`) as `SetRawProp`. `nve_backend_web::sync_dom_raw_prop`
  then applies them with `setAttribute("font-weight", "700")`, not as style, so they have no effect
  on the web. `nve_dom.js` mirrors this to keep parity; the fix belongs in both.

## 8. Artifacts

- Prototype crate: [hosts/web-raw](../../../hosts/web-raw): `src/raw.rs` (raw exports),
  `src/wb.rs` (baseline), `nve_dom.js` (interpreter), `build.sh`, `e2e.mjs`.
- Vendored patch: `hosts/web-raw/patches/subsecond` (wired through the root `[patch.crates-io]`).
- Measure script: `docs/research/wasm-size/boundary/measure.sh`.

## Sources

- https://github.com/ealmloff/sledgehammer_bindgen
- https://dioxuslabs.com/learn/0.7/beyond/project_structure/
- https://lib.rs/crates/dioxus-interpreter-js/features
- https://news.ycombinator.com/item?id=42389185 (Dioxus maintainer on the sledgehammer batch layer)
- https://github.com/wasm-bindgen/wasm-bindgen/pull/5205 , https://github.com/wasm-bindgen/wasm-bindgen/pull/5382 (strip drops target_features)
- https://github.com/DataDog/pup/pull/1045 (`STRIP=debuginfo` preserves the section)
- https://lld.llvm.org/WebAssembly.html (`--compress-relocations`, padded LEB128)
- https://discourse.llvm.org/t/lld-adding-webassembly-support-to-lld/45447
- https://users.rust-lang.org/t/export-functions-from-wasm-library-w-o-using-wasm-bindgen/51778
- https://www.webassembly-wasm.com/compilation-pipelines-toolchain-setup/rust-to-wasm-compilation-guide/
- https://dev.to/bence_rcz_fe471c168707c1/rust-webassembly-performance-javascript-vs-wasm-bindgen-vs-raw-wasm-with-simd-4pco
- https://crates.io/crates/lol_alloc/0.1.2
- https://rustwasm.github.io/docs/wasm-bindgen/print.html
- https://www.pistack.xyz/posts/2026-08-29-dioxus-vs-yew-vs-leptos-rust-wasm-frameworks-comparison/
- https://rustify.rs/articles/leptos-vs-dioxus-rust-frontend-2026
