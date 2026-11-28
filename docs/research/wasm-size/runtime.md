# WASM size research — TRACK `runtime`

Scope: Rust code and dependencies that end up inside `nve_web_shell_bg.wasm` (dioxus-core/-signals,
generational-box, subsecond, tracing, anyhow, futures, parking_lot, serde/serde_json, std pieces,
nve-interop/-engine/-dioxus/-abi/-components). The JS-binding layer and compiler/toolchain flags
belong to other tracks and are only listed under handoffs.

Branch `t3/wasm-runtime-size-research`, prototype commits `0a312f4..HEAD` (see "Prototype").

## Protocol

Every number comes from [`runtime/measure.sh`](runtime/measure.sh):
`cargo rustc --crate-type cdylib --target wasm32-unknown-unknown --profile wasm-release` (stable
1.98.1, `opt-level=z`, fat LTO, 1 CGU, `panic=abort`, `strip=symbols`) → `wasm-bindgen 0.2.130
--target web` → raw / `gzip -9` / `brotli -q 11` of the shipped `.wasm` and `.js`. "br" below means
brotli of the wasm-bindgen output **without** wasm-opt (the shipped baseline the task gave); the
`wasm-opt -Oz` column is in the logs and moves by ±2 KB (it makes gzip/brotli *worse* for the large
builds, see handoffs). Symbol builds (`SYMBOLS=1`, `strip=none`, `--keep-debug`) feed twiggy and
[`runtime/why.py`](runtime/why.py).

Baseline HEAD `4e046ca`, default features: **wasm 842,689 raw / 310,944 gz / 251,485 br, JS 64,404
raw / 8,630 br, total 260,115 br.**

## Headline result

| Build (all cheap/medium items applied, talc) | wasm raw | wasm gz | wasm br | JS br | **total br** | vs baseline |
|---|---:|---:|---:|---:|---:|---:|
| baseline HEAD (default features) | 842,689 | 310,944 | 251,485 | 8,630 | 260,115 | — |
| default features (`nve dev` shell, both apps) | 742,311 | 266,948 | 216,105 | 8,634 | 224,739 | −35.4 K (−14 %) |
| production studio (`app-studio`) | 601,551 | 213,252 | 174,456 | 5,533 | **179,989** | −80.1 K (−31 %) |
| production studio + popups (`app-studio,multi-window`) | 647,747 | 229,715 | 186,731 | 7,026 | 193,757 | −66.4 K (−26 %) |
| production counter (`app-counter`) | 283,667 | 112,332 | 94,667 | 5,523 | **100,190** | −160 K (−61 %) |
| counter + dev hot reload (`app-counter,dev`) | 368,743 | 145,678 | 122,424 | 7,318 | 129,742 | −130 K |
| counter, nightly `-Zbuild-std` + `-Cpanic=immediate-abort` (toolchain handoff, for scale; wasm-tools strip) | 237,448 | 96,282 | 81,628 | 5,473 | 87,101 | −13.1 K vs stable counter |

Reference points from the same protocol (probe crate [`runtime/probe`](runtime/probe), counter app,
one `render(i32)` import, two exports):

| Probe | wasm raw | wasm br | Δ br over empty |
|---|---:|---:|---:|
| empty wasm-bindgen cdylib (std, dlmalloc, panic=abort) | 12,935 | 5,113 | — |
| `sycamore-reactive 0.9` signal + memo + effect | 30,558 | 11,588 | +6.5 K |
| `reactive_graph 0.2` (Leptos 0.7/0.8 core) signal + memo + effect | 41,132 | 14,723 | +9.6 K |
| `dioxus-core 0.7.10` VirtualDom + 1 component + signal, no-op `WriteMutations` (upstream) | 187,366 | 65,897 | +60.8 K |
| same with this branch's dioxus patches (anyhow no-std, FxHash) | 143,594 | 51,642 | +46.5 K |

## Findings — why each heavy item is retained

twiggy's dominator/paths view stops at `table[0]` for every address-taken function (vtables,
`fmt::Arguments` function pointers, `Box<dyn Fn>`), which is exactly where most of the weight hides.
`runtime/why.py` re-adds those edges (`i32.const <slot>` and rodata vtable words) on the
`wasm-tools print` output; its fn-pointer edges are over-approximate (small table indices collide),
so every lead below was confirmed by an experiment that removes it.

1. **Everything is always linked.** `nve-web-shell` depended unconditionally on both demo apps,
   the hot-reload socket, the NveWireFrame pairing client, popups/`BroadcastChannel` window sync and
   the per-frame `NveSceneTable` mirror, and `wizer.initialize` renders *both* apps, so nothing could
   be dead-stripped. Measured costs (br, after the other fixes): `dev` (hot reload + pairing)
   +25.9 K on the counter build (+41.5 K before NVP1-only/anyhow fixes), `multi-window` +12.5 K,
   counter app alongside studio ≈ +3 K, `scene-view` +2.0–2.5 K, JS glue 8.6 K → 5.5 K br.
2. **`rustc-demangle` (19 KB raw) and `std::backtrace` printing come from `anyhow`'s `std`
   feature**, enabled by `dioxus-core 0.7.10` (`anyhow = "1"` with default features). With `std`,
   anyhow captures a `std::backtrace::Backtrace` in every error and its `Debug` impl
   (`ErrorImpl::debug`, 5.8 KB) prints it; `CapturedError`'s `Debug` is reachable from the error
   boundary. Turning `std` off (no API dioxus-core uses needs it on Rust ≥ 1.81, `core::error::Error`)
   removes demangle, the backtrace printer and most `ErrorImpl::debug`: **−12.1 K br** on the
   counter. Not std's panic hook: after this fix no `rustc_demangle` symbol remains.
3. **Float formatting and parsing (≈ 20 KB code + ≈ 10 KB of `dec2flt` power tables) come from a
   string round-trip in the style path**: RSX writes `"width: 12px"` → `nve-components::parse_dim`
   → `str::parse::<f32>` (Eisel-Lemire + big-decimal fallback) → typed f32 in the `CommandBuffer` →
   `nve-backend-web::css` re-formats with `write!("{v}px")` / `"{:.3}"` (Grisu + Dragon4) for every
   style write. Also `AttributeValue::Float(f).to_string()` in `nve-dioxus` and `{elapsed_ms:.1}`
   in the hot-reload HUD. A 150-line fixed-point codec (`nve_abi::num`, max 3 decimals, bit-exact
   with std on every plain decimal tested) replaces both directions: **−12.7 K br**. With `dev` off
   no `flt2dec`/`dec2flt` symbol is left.
4. **serde_json in the dev wasm is the JSON fallback of `HotPatchMsg::from_wire`**
   (`HotPatchMsg::deserialize` alone is 11 KB) even though the binary NVP1 decoder exists; `nve dev`
   still broadcast JSON text frames. NVP1-only on wasm32 + `nve dev` sending binary: **−16.7 K br**
   on the dev build. In production studio serde_json (≈ 17.5 KB + `BTreeMap<String, Value>` 3 KB +
   `zmij` 1 KB) remains, from `serde_json::json!` in `ui_islands`, `NveChannelBus` payloads
   (`nve-interop/src/channel.rs`), `IslandSpec::validate` and JSON `props_bytes` — JSON across an
   NVE boundary, which AGENTS.md forbids anyway.
5. **`subsecond` and `tracing` cost nothing in release.** `HotFn::try_call` starts with
   `if !cfg!(debug_assertions) { return Ok(self.inner.call_it(args)) }` and `register_handler` is
   `#[cfg(debug_assertions)]`; the 11–13 KB twiggy attributes to subsecond are component bodies
   named `<Comp as subsecond::HotFunction>::call_it`. `tracing` has 0 bytes after LTO (no
   subscriber, callsites fold away). No `[patch]` needed for either.
6. **SipHash / `RandomState`** came from `std::collections::HashMap` in nve crates and from
   `HashSet<ReactiveContext>` in dioxus-core/-signals. Swapping to FxHash everywhere removes
   `RandomState` and SipHash but saves only **−0.6 K br**: the bulk of `hashbrown` (6–22 KB) is
   per-`(K,V)` monomorphization, which FxHash does not change. The real fix is dense-id storage
   (below).
7. **`dlmalloc` (7.1 KB)**: talc 5.1 `WasmDynamicTalc` **−2.5 K br**, lol_alloc `FreeListAllocator`
   −2.9 K (but O(n) and fragmenting), lol_alloc `LeakingAllocator` −9.0 K (never frees; shows how
   much free/realloc/coalescing code exists — relevant for a per-frame bump arena, not as a global
   allocator).
8. **`BTreeMap`/`BTreeSet` ≈ 37 KB raw in production studio**: each `(K,V)` instantiation costs
   2–4 KB. Instances: dioxus-core scheduler (`BTreeSet<ScopeOrder>`, `BTreeSet<Effect>`, ≈ 8 KB),
   `BTreeMap<String, serde_json::Value>`, `nve-interop` surface/channel/window maps, web-shell
   `ACTIVE_SHELLS`/popups, studio CRDT state. Most are tiny maps keyed by `u32`.
9. **Sorting ≈ 14.6 KB**: two identical `ipnsort` copies (`u32`, `usize`, 3 KB each), stable
   `driftsort` for the command palette and for `VirtualDom::drop` (`sort_by_key` on scope height).
   Most `u32` sorts exist because node maps are hash maps that must be iterated in order.
10. **Panic location strings ≈ 4.8–9 KB of `.rodata`** (75–123 `*.rs` paths; registry crates carry
    `/kache/.cargo/registry/src/index.crates.io-…/` prefixes — vendoring dioxus with a short
    relative path alone saved 0.5 K br). Toolchain handoff.
11. **wasm-bindgen closure descriptors survive in the shipped wasm**: `.rodata` of the counter
    build contains ≈ 2 KB of strings such as `Closure(Closure { owned: true, function: Function {
    arguments: [NamedExternref("MouseEvent")], shim_idx: 1, … })`. Binding-track handoff.
12. **What the counter floor is made of** (production counter, talc, symbol build, 283 KB
    code+data): Dioxus runtime (dioxus-core + signals + generational-box + parking_lot + anyhow +
    futures) ≈ 92 KB (32 %), NVE stack (nve-dioxus 25 K, nve-backend-web 30 K, web-shell 18 K,
    nve-components 10 K, nve-abi 4 K) ≈ 88 KB (31 %), core/alloc/std/hashbrown/talc ≈ 47 KB,
    `.rodata` 29 KB, wasm-bindgen/web-sys glue + imports ≈ 12 KB, **the counter app itself 3.4 KB
    (1 %)**. Inside dioxus-core (probe): `nodes` (diff/create) 16 K, `virtual_dom` 15 K, scheduler
    6.5 K, effects 5.6 K, reactive_context 4.9 K, scopes 4.8 K, runtime 4.5 K, error boundary 3.8 K,
    suspense 1.8 K, tasks 1.5 K, plus generational-box/parking_lot 7.8 K.

## Ranked changes

Savings are brotli bytes of the shipped wasm (JS unchanged unless noted). "Measured" = built and
measured in this branch; "est." = derived from symbol sizes (≈ 3:1 raw:br for code in these builds).

| # | Change | Saving (br) | Effort | Risk | What breaks / needs care | Status |
|---|---|---:|---|---|---|---|
| 1 | Cargo features in `nve-web-shell`: ship production bundles with `--no-default-features --features app-studio` (drop `dev`, `multi-window`, the other app, `scene-view`) | **−41.6 K** (studio) … **−121 K** (counter) wasm vs default, −3.1 K JS | S | low | `index.html` must load the right bundle per mode; `nve dev` keeps default features | measured |
| 2 | `nve_abi::num` fixed-point float codec instead of `core::fmt` floats and `str::parse::<f32>` | **−12.7 K** | S | low | CSS gets ≤ 3 decimals (`rgba(..,1)` instead of `1.000`); tests updated | measured |
| 3 | dioxus-core: `anyhow` without `std` (vendored patch; upstream PR) | **−12.1 K** | S | low | anyhow errors carry no backtrace (none is printable on wasm anyway) | measured |
| 4 | Hot reload NVP1-only on wasm, `nve dev` sends binary frames | **−16.7 K** (dev builds) | S | low | Apple host still applies JSON via `applyHotPatchJson` (boundary track) | measured |
| 5 | Remove JSON from production paths: binary channel-bus payloads, island props, `props_bytes` | est. −7 … −8 K | M | med | `NveChannelBus`/`IslandSpec` wire formats change on every host | estimated |
| 6 | Dense-id slabs instead of `HashMap<NodeId,_>` / `HashMap<ElementId,_>` (writer, web backend, scene table) | est. −3 … −5 K (+ removes per-frame sorts) | M | med | `NodeId` must stay dense (it is allocated sequentially today); needs free-list reuse | estimated |
| 7 | `BTreeMap` → shared sorted `VecMap<K,V>` for small `u32`-keyed maps (interop, web-shell, studio state); dioxus scheduler sets via patch | est. −3 … −5 K | M | low | O(n) insert for large maps (all are tiny) | estimated |
| 8 | Allocator: talc `WasmDynamicTalc` | **−2.5 K** | S | low | none (faster than dlmalloc per talc's benchmarks) | measured (feature `alloc-talc`) |
| 9 | Per-frame scratch arena (bump allocator reset per frame) for CommandBuffer/CSS strings | est. −1 … −3 K + perf | M | med | lifetimes of frame data must not escape the frame | idea (LeakingAllocator −9 K bounds it) |
| 10 | Gate `NveSceneTable` mirror (`scene-view`) | **−2.0 … −2.5 K** | S | low | `nve_web_scene_view_ptr` returns an empty view in production | measured |
| 11 | One sort instantiation (`u32` everywhere), no stable sorts in hot code | est. −1 K | S | low | — | estimated |
| 12 | FxHash for all maps (nve crates + dioxus patch) | −0.6 K | S | low | HashDoS resistance (irrelevant for internal ids) | measured |
| 13 | Replace dioxus-core with an NVE-native fine-grained runtime (signals + compile-time templates emitting `CommandBuffer` ops) | est. **−35 … −45 K** (dioxus floor 46.5 K vs sycamore-reactive 6.5 K / reactive_graph 9.6 K), plus part of nve-dioxus' 25 KB writer | XL | high | new `rsx!`/`#[component]` macros, all examples, Tier-1 NVP1 patching re-targeted, Tier-2 state archive | feasibility below |
| 14 | `no_std + alloc` for the wasm crates | est. ≤ −1 K after #2–#3 | L | med | `thread_local!`, `std::sync`, dioxus-core | not worth it (below) |

Cumulative measured (items 1–4, 8, 10, 12): default 251.5 K → 216.1 K br; production studio
179,989 B total br (−31 %); production counter 100,190 B total br (−61 %, and −13 K more with the
nightly immediate-abort panic strategy from the toolchain track).

## Feasibility: how far can the runtime go?

- **Hand-written-WAT level** for this architecture means: a fixed binary `CommandBuffer` decoder,
  one import per DOM op, no reconciler in wasm at all — the `wasm-bindgen` row of the
  js-framework-benchmark (14.5 KB compressed for the full 1k-row app, vanillajs 2.5 KB) is the
  realistic floor for "Rust + DOM". NVE cannot get there while a VDOM reconciler, a typed style
  model and an encode→decode round trip all live in the same wasm.
- **With Dioxus kept** (items 1–12): production counter ≈ 94.7 K br wasm today (81.6 K with
  immediate-abort, ≈ 75 K after #6/#7/#9), production studio ≈ 160–167 K br after #5–#7. The Dioxus runtime is then the largest fixed cost (≈ 46 K br floor measured in
  isolation).
- **With an NVE-native runtime** (#13): the probe numbers put a signals core at 6.5–9.6 K br.
  Leptos 0.7 / Sycamore 0.9 / Dominator land at 47–49 / 47 / 40 KB compressed vs Dioxus 0.7.2 at
  115 KB for the identical js-framework-benchmark app — the same ≈ 2.4× gap we measure in
  isolation. A counter on NVE-native runtime + current backend would be ≈ 50–55 K br; with #6/#7
  and a leaner backend ≈ 35–40 K br.
- **Feasibility of #13.** What `rsx!` would need: compile each static template into (a) a
  `&'static [u8]` CommandBuffer fragment (create nodes, static props already typed — parse
  `"width: 12px"` at compile time, which deletes the runtime style parser entirely) and (b) a list
  of dynamic slots bound to signal getters that emit `SetProp/SetText` ops. Components become plain
  functions run once (no re-render, no diff); lists need a keyed reconciler (LIS) for `For`. Tier-1
  NVP1 hot patching keeps working because it already targets static template literals by
  `template_id`/`node_path`/`test_id` — the template table becomes the patch target directly
  (simpler than today's Dioxus template re-registration). Tier-2 state preservation maps to
  serializing signal slots (the existing `signal_archive` already does this keyed by slot id).
  Effort: XL (macro crate, runtime, port of `examples/*`, `nve-ui`, `nve-router`, `nve-mdx`,
  Skia/headless backends unaffected because they consume the same `CommandBuffer`). Risk: losing
  Dioxus ecosystem compatibility (hooks, router) — acceptable under the zero-compat rule but it is
  a product decision.
- **no_std + alloc.** Blockers in the wasm graph: `thread_local!` (web-shell `ACTIVE_SHELLS`,
  session caches, dioxus `Runtime`), `std::sync::{Mutex, Arc}` in dioxus-core/-signals/
  generational-box, `std::collections::HashMap` (→ `hashbrown`), `std::time::Instant` (already
  cfg'd off on wasm), `std::error::Error` (→ `core::error`). `Rc`, `RefCell`, `BTreeMap`, `Vec`,
  `String` are fine in `alloc`. What it would buy after items #2–#3: the symbol build attributes
  1.6 KB to `std`; the rest of std's weight (panic formatting, location strings) is removed by
  `-Cpanic=immediate-abort` with `-Zbuild-std` without giving up std (see the nightly row). Verdict:
  not worth doing on its own; it falls out naturally if #13 is built `no_std`-first.

## Prototype (branch `t3/wasm-runtime-size-research`)

- `crates/nve-abi/src/{hash,num}.rs`: FxHash aliases; fixed-point float/int codec with tests
  (bit-exact vs `str::parse::<f32>` on edge cases, roundtrip, rejection cases).
- `nve-components`, `nve-backend-web` (css, canvas, dom), `nve-dioxus` writer: use `nve_abi::num`
  and `nve_abi::hash`.
- `third_party/dioxus-core`, `third_party/dioxus-signals` (0.7.10 + two patches) via
  `[patch.crates-io]`: FxHash subscriber sets; `anyhow` without default features.
- `hosts/web-shell`: features `app-studio`, `app-counter`, `dev`, `multi-window`, `scene-view`
  (default = all), `alloc-talc` / `alloc-lol` / `alloc-lol-leak`.
- `nve-engine` `HotPatchMsg::from_wire`: NVP1-only on wasm32; `nve-cli dev` broadcasts NVP1 binary.
- Tools: `docs/research/wasm-size/runtime/measure.sh`, `why.py`, `probe/`.
- Verified: `cargo test -p nve-abi -p nve-components -p nve-backend-web -p nve-web-shell`,
  `nve-engine` hotreload tests, `cargo check -p nve-cli`, wasm32 builds of every feature combo.

## Upstream bugs / quirks

- **anyhow 1.0.105 (`std` feature) + dioxus-core 0.7.10**: dioxus-core enables anyhow's default
  `std` feature; on `wasm32-unknown-unknown` that links `std::backtrace` printing and
  `rustc-demangle` (~19 KB raw / 12 KB br) although backtraces are never captured there.
  Mitigation: `[patch.crates-io]` dioxus-core with `anyhow = { default-features = false }`
  (upstream PR candidate).
- **dioxus-core/-signals 0.7.10**: `HashSet<ReactiveContext>` with std `RandomState` pulls SipHash
  + random-key init into wasm although `rustc-hash` is already a dependency. Mitigation: FxHashSet
  patch.
- **subsecond 0.7.10 / tracing 0.1**: zero release cost — no action needed (documented to stop
  people from chasing them because twiggy names functions `subsecond::HotFunction::call_it`).
- **twiggy 0.7**: dominators/paths stop at `table[0]` for all indirect calls, so retention of
  vtable/fn-pointer reached code (fmt impls, closures) is invisible. Mitigation: `why.py`.
- **wasm-opt (binaryen 133) -Oz** on these builds lowers raw bytes 7–8 % but raises gzip/brotli by
  1–2 % (toolchain track).
- **wasm-bindgen 0.2.130**: closure descriptor `Debug` strings remain in `.rodata` of the final
  module (binding track).

## Handoffs (other tracks)

- Toolchain: `-Zlocation-detail=none` / `-Cpanic=immediate-abort` + `-Zbuild-std` (panic strings,
  `unwrap` `Debug` impls, location paths ≈ 5–9 KB); `--remap-path-prefix`; why `wasm-opt -Oz`
  worsens compression (try `-Oz --converge`, `--gufa`, or skipping it).
- Binding: closure descriptor strings in `.rodata`; one `Closure` per listener vs. one delegated
  listener; JS glue for dev/multi-window exports is now feature-gated (8.6 K → 5.5 K br).
- Boundary: JSON in `NveApp::apply_hot_patch(&str)` / Apple `applyHotPatchJson`, `PairControlMsg::
  Tier1Patch(String)`, `NveChannelBus` and island props → NVP1/binary everywhere; the web backend
  rebuilds and re-sends the full style string per element on every style change.

## Sources

- js-framework-benchmark results (sizes for dioxus 0.7.2, leptos 0.7.0, sycamore 0.9.0-beta.2,
  dominator 0.5.0, yew 0.23.0, sauron 0.61.4, wasm-bindgen 0.2.84, vanillajs):
  https://github.com/krausest/js-framework-benchmark (data:
  https://raw.githubusercontent.com/krausest/js-framework-benchmark/master/webdriver-ts-results/src/results.ts,
  viewer https://krausest.github.io/js-framework-benchmark/current.html)
- Dioxus 0.7 optimizing guide: http://dioxuslabs.com/learn/0.7/guides/tips/optimizing/
- Dioxus bundle size issue: https://github.com/DioxusLabs/dioxus/issues/732
- talc WebAssembly guide: https://github.com/SFBdragon/talc/blob/master/talc/README_WASM.md
- lol_alloc: https://crates.io/crates/lol_alloc
- anyhow no_std support: https://docs.rs/anyhow/latest/anyhow/#no-std-support
- min-sized-rust: https://github.com/johnthagen/min-sized-rust
- Leptos binary size: https://book.leptos.dev/deployment/binary_size.html
