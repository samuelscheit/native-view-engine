# WASM size research: architecture track

How small can the NVE web bundle get, which architectural changes get it there, and is it worth leaving Rust? Every number below was measured on branch `t3/wasm-size-architecture`, starting from `4e046ca` (stable rustc 1.98.1, wasm-bindgen 0.2.130, binaryen 133), unless it is marked *est.* (estimate) or *cited*.

**Protocol.** I measure the shipped artifacts after `wasm-bindgen --target web`: the wasm as raw bytes, `gzip -9` and `brotli -q 11`, and the JS glue the same way. **total br** is wasm br plus JS br. The tooling is in [`architecture/`](architecture/):

- `measure.sh` and `build-crate.sh` (build, then bindgen, then optional `wasm-opt -Oz`, then measure);
- `build-std-probe.sh` (nightly build-std with immediate-abort panics);
- `crate-attrib.py` (groups twiggy output by crate);
- `rodata-classify.py` (classifies the strings in the data segment).

## TL;DR

| What | wasm br | total br (wasm+JS) |
|---|---:|---:|
| **Hand-written WAT counter** (same UI as `examples/counter`) | **430** | **652** |
| **no_std Rust counter**, raw imports, CommandBuffer passed to a JS interpreter | **629** | **1,025** |
| Leptos 0.8 CSR counter (same UI, same profile) | 28,616 | 33,670 |
| dioxus-web 0.7.10 counter (same UI, same profile) | 100,995 | 108,500 |
| NVE today: minimal counter-only shell (`hosts/web-min`) | 122,829 | 127,207 |
| NVE today: minimal studio-only shell | 201,609 | 205,975 |
| NVE today: full `nve-web-shell` (both apps + dev/infra) | 251,485 | 260,115 |

- **The gap is architecture, not language.** Rust reaches 1.6× of hand-written WAT, for the same design. The 120–200× gap to today's bundle comes from five things: the VDOM runtime (dioxus-core), Rust std's panic/fmt/backtrace machinery, decoding the CommandBuffer into the DOM inside wasm (web-sys plus closures), runtime parsing of style strings, and shipping dev features and both apps in one bundle.
- **Do not leave Rust.** No other language delivers an engine like this one at a smaller size with the same shared native engine (details under "Change compiler/language?" below).
- **Realistic lower bounds:**
  - counter: about 3–6 KB br total with a compiled fine-grained renderer on no_std (the demo floor is 1 KB);
  - studio-workspace: about 40–60 KB br total, or 25–40 KB initial with per-panel splitting. Today it is 260 KB.
- **Wizer makes things worse.** It removes no code and adds a 379 KB data section: +62 KB br, measured. It also does not run in today's pipeline at all (upstream quirk below). Remove it from `build.sh` and use SSR HTML for frame 0 instead: 2.9 KB br for studio.

## 1. Theoretical floor

### 1.1 Hand-written WAT counter (measured)

[`architecture/floor-wat/counter.wat`](architecture/floor-wat/counter.wat) is the full counter UI: the same five styled nodes, "Count: N", Increment and Reset.

- **Design:** the static UI is one HTML string in a data segment. It is mounted once, with `innerHTML`. The only dynamic slot is a `<u>` text node. Itoa runs in wasm. Events are delegated through `data-e` attributes and arrive as one `i32`.
- **Interface:** 2 imports (`h(ptr,len)` mount, `t(ptr,len)` set text) and 3 exports. Code is 113 B, data 746 B.
- **Result:** 935 B raw / 550 gz / **430 br**. The [loader](architecture/floor-wat/loader.js) is 342 B / **222 br**. **Total 652 B br.**
- **Verified:** with Node (`verify.mjs`: 1, 2, 3, reset to 0, 1, and 1235 after 1,234 clicks) and in real headless Chrome (`browser-check.html` clicks the DOM buttons; the observed text sequence was `["1","2","3","0","1"]`).

### 1.2 no_std Rust with raw imports and NVE's architecture (measured)

[`architecture/floor-rs/src/lib.rs`](architecture/floor-rs/src/lib.rs) is a standalone crate (`#![no_std]`, no wasm-bindgen, no allocator) that mirrors NVE's design at its minimum:

- **CommandBuffer:** one flat binary buffer per frame, handed to JS with a single import call (`f(ptr,len)`). It has three opcodes: `DefineTemplate`, `MountTemplate` and `SetText`.
- **Templates:** the static UI is a compile-time HTML template, instantiated with `<template>` plus `cloneNode`, which is what Solid, Svelte and Leptos do.
- **Events:** they arrive as plain `i32` parameters `(slot, kind)`.

Result:

- rustc output: 1,405 raw / **684 br**;
- with `wasm-opt -Oz`: 1,255 raw / 752 gz / **629 br** (code 401 B, data 762 B);
- the generic opcode-interpreting JS loader is 775 raw / **396 br**;
- **total 1,025 B br**, verified in headless Chrome as above.

Rust costs about 200 B br over hand-written WAT for the same design. That is the language overhead.

### 1.3 Irreducible engine pieces: minimum versus today

Per-crate attribution (twiggy, symbol build, `crate-attrib.py`) of the **counter-only** minimal shell. Generic monomorphisations are counted under `core`, `alloc` and `hashbrown`. Bytes are raw code+data.

| Piece | Today (counter-only, std) | Today after build-std + JS apply | Minimal design | Minimal size *(est.; floor-rs measured)* |
|---|---:|---:|---|---:|
| State (signals) | dioxus_signals + generational_box + parking_lot ≈ 6 K | ≈ 3 K | One `Cell` per signal plus a subscriber list (fine-grained) | 0.3–2 K |
| Reconcile/patch | dioxus_core 51.5 K, plus most of core 70 K / alloc 30 K / hashbrown 16 K | dioxus_core 47.2 K, core 38 K, alloc 21 K, hashbrown 11 K | No VDOM: static parts are templates; dynamic slots are effects that emit ops directly; only keyed lists need an LIS diff | 1–3 K |
| CommandBuffer encoder | nve_dioxus 24.7 K (writer, typed-attr string parser, hot patch, test ids) | 21.4 K | Byte pushes from the compiled templates and slots; styles already typed at compile time | 0.3–1 K (floor-rs: ~200 B) |
| Style parsing | nve_components 9.5 K + `f32::from_str` 3.8 K + flt2dec ~9 K | 4.2 K + … | None at runtime (compile-time `StyleProps`) | 0 |
| Event decode | part of nve_dioxus/nve_abi | — | `(slot, kind, payload)` parameters or a fixed-layout ring | 0.1–0.5 K |
| DOM apply | nve_backend_web 25.3 K + wasm_bindgen/web_sys 5 K + 80 imports 4.6 K, **plus 23.7 KB of JS glue** | moved to JS: 8.5 KB of glue left | JS interpreter of the CommandBuffer (like Dioxus's sledgehammer interpreter) | 2–4 K of JS (~1–1.5 KB br) |
| Allocator | dlmalloc 7.1 K | 5.0 K | Bump/arena per frame plus a small free-list (runtime track) | 0.2–2 K |
| Panics/fmt/backtrace | rustc_demangle 18.5 K, anyhow 7.2 K, std 7 K, panic strings and paths ~13.5 K of rodata | demangle **still 14.8 K**, anyhow 5.5 K | immediate-abort, no `fmt` in the hot path, no anyhow | ~0 |
| App (CounterApp) | 1.9 K | 1.6 K | template data plus 2 handlers | ~0.3 K |

**Engine floor: about 5–10 KB raw, about 2–4 KB br, plus the app.** Today the counter needs 349 KB raw of code+data.

## 2. Static versus dynamic content

**Where the 83 KB `StudioWorkspaceApp` comes from.** I wrote 60-copy probe components through `hosts/web-min` (feature `probe`) and measured the marginal bytes per construct on today's stack. Raw bytes are the honest number here: the probes are repetitive and compress unrealistically well.

| rsx construct (one instance) | raw B | br B |
|---|---:|---:|
| Static node pair (view and text with static styles): template **data** | 204 | 18 |
| Dynamic text `"v {n}"` | 180 | 18 |
| Interpolated style `"width: 1px; background-color: {c};"` | 251 | 37 |
| `if cond { view { … } }` | 314 | 36 |
| Event handler `onpress: move \|_\| …` | 500 | 41 |
| Keyed `for` loop | 526 | 42 |
| `#[component]` with a prop | 744 | 100 |

The studio sources contain 71 handlers, 80 `for` loops, about 225 `if` blocks and **244 interpolated `style:` strings against 76 static ones (76% dynamic)**. Even `{TEXT_PRIMARY}`, a `const`, turns a whole style attribute dynamic. That means a `format!` String allocation at render, a runtime `;`/`:` split plus `f32::from_str` in `apply_typed_attr_with_raw`, and string-diffing. Dioxus already compiles the static structure to `Template` data, so **the code is the dynamic slots and the control flow**, not the static markup.

**Upper-bound experiment.** I rewrote all 244 interpolated style literals to static ones (throwaway, reverted). Studio-only went from 201,609 to **193,560 B br (−8.0 KB, −4%)**. This also drops the value computations, so the realistic saving from compile-time styles is about 4–8 KB br. Removing the runtime parser saves a few KB more (`nve_components` typed-attr parsing, `f32::from_str`, and flt2dec once CSS writing also leaves wasm).

**Studio `.rodata` (115 KB).**

- CSS/style string fragments: 20.9 KB. These are the `format!` pieces like `"; font-size: 9px;"`.
- UI copy, keys and names: 18.5 KB.
- Panic/fmt messages: 8.3 KB.
- Source paths: 8.3 KB.
- Binary data: 59.4 KB (vtables, Template structs, tables).

In the counter-only build the rodata is 47.8 KB, of which 25 KB is strings and only `"Count: "` belongs to the app.

**Compile-time design that removes this:**

1. Make an `rsx` style a typed expression. The static declarations are parsed **at macro-expansion time** into pre-encoded `LayoutProps`/`StyleProps` bytes, or into one shared CSS class string on web. Interpolations become typed dynamic props (`background: ColorRgba`, `width: f32`) that the writer encodes straight into `SetStyle` without parsing. `const` interpolations fold into the static part.
2. Pre-encode each Dioxus `Template` once into a CommandBuffer fragment, an HTML `<template>` on web, which JS clones. Today `instantiate_template_node` walks the template and re-parses static style strings on every instantiation.
3. First paint uses frame-0 SSR HTML in the page (section 4). Studio's frame-0 HTML is 70,485 B raw but **2,903 B br**. Its CommandBuffer is 28,273 raw / 5,095 br. The backend also writes every default style property per node, and emitting only non-defaults would shrink both.

## 3. Code splitting and lazy loading (measured with `hosts/web-min`)

| Bundle | wasm br | JS br | total br |
|---|---:|---:|---:|
| Full shell (counter, studio, hot reload WS, pairing, multi-window, popup, interop, scene table, preinit) | 251,485 | 8,630 | 260,115 |
| Minimal shell, counter **and** studio | 204,069 | 4,397 | 208,466 |
| Minimal shell, studio only | 201,609 | 4,366 | 205,975 |
| Minimal shell, counter only | 122,829 | 4,378 | 127,207 |

What these show:

- **Dev and infra features cost 51.6 KB br (20%).** Hot reload (serde_json `HotPatchMsg`, 11 K raw for the deserializer alone), pairing, multi-window and popup, nve-interop, the scene table and the preinit cache all ship to production. They belong behind cargo features (`dev`, `pair`, `multiwindow`): a production build has no hot-reload socket at all. Lazy-loading them is the wrong tool.
- **Linking the counter next to studio costs only 2.5 KB br,** because the runtime is shared. Splitting one app per bundle matters for the counter (127 vs 208 KB) and hardly for studio.
- **The paired thin client needs no app code at all.** It only renders remote CommandBuffers. It should be its own bundle, about 30–50 KB br *est.*: backend, socket and std, without Dioxus.
- **Studio-specific code is 78.8 KB br.** Applying the construct cost model per file (`ui_*.rs`), the switchable panels (docs, studio, workbench, palette, board, metrics, islands, channels, stream) are about **54%** of the studio UI code. The CRDT sync and codec modules are about 13% more and are only needed when syncing. Splitting by panel with `#[wasm_split]` would defer roughly **30–40 KB br** of the initial studio download *(est.)*.
  - Caveats, all cited: the Dioxus 0.7 wasm-split is experimental and tied to `dx` and the router; fat LTO + `codegen-units=1` crashes `wasm-split-cli` (open PR [#5795](https://github.com/DioxusLabs/dioxus/pull/5795)); split routes render twice under SSR hydration ([discussion #5287](https://github.com/DioxusLabs/dioxus/discussions/5287)).
  - The shared runtime (dioxus-core, std) always stays in the main module, so splitting helps big apps and never the counter.
- **SSR and islands:** studio is interactive almost everywhere, so islands help little there. For `examples/nve-docs-site` they are the main lever. Cited: Leptos's islands demo drops from 274 KB to 24 KB uncompressed ([book.leptos.dev/islands.html](https://book.leptos.dev/islands.html)). For NVE: `WebBackend::render_html` already produces frame-0 HTML; add `data-nve-id` hydration that adopts the existing nodes instead of creating them.

## 4. Wizer (measured)

Wizer 10.0.0 was installed into a private root, so `build.sh` was unaffected.

- **It fails on the shipped pipeline:** running it on the wasm-bindgen output gives `Error: unsupported ref.null instruction`. `build.sh` prints a warning and silently ships the module without pre-initialisation. So the current "Frame-0 VDOM pre-init" never happens.
- **Run before wasm-bindgen** (on the raw rustc output), it succeeds:
  - before: 842,689 raw / 251,485 br;
  - after: **1,093,165 raw / 313,234 br (+61.7 KB br, +24.5%)**;
  - code is unchanged (2,404 functions); the data section grows to 379 KB in 9,102 segments.
- **Why it can't help:** Wizer can only drop code that is reachable from the initialiser *alone*. Here the render, reconcile and encode code is needed for every later frame anyway. What gets snapshotted is the live heap after frame 0:
  - counter: 39.6 KB of heap (native upper bound, 8-byte pointers);
  - studio: 337 KB;
  - plus the cached CommandBuffers of both apps.
- **Verdict:** remove Wizer, `wizer_initialize` and the preinit cache (they are part of the 51.6 KB of infra in section 3). For first paint, inline the frame-0 SSR HTML (2.9 KB br for studio) and hydrate. It gives faster first paint, needs no wasm at all for frame 0, and is smaller.

## 5. Change compiler or language?

| Language/toolchain | Data point | Source |
|---|---|---|
| Hand-written WAT | DOM counter **652 B br total** (measured); hello WASI 143 B | this repo; [RIoT Secure](https://riotsecure.se/blog/wasm_binary_size_in_high_Level_languages) |
| Rust no_std, raw imports | DOM counter **1,025 B br total** (measured) | this repo |
| Rust plus a framework | Leptos 0.8 counter 33.7 KB br; dioxus-web 0.7 108.5 KB br; NVE today 127 KB br (measured) | this repo |
| C (Emscripten standalone / WASI SDK) | fib 577 B / 269 B; hello 10.6 KB / 14 KB (libc printf) | [RIoT Secure](https://riotsecure.se/blog/wasm_binary_size_in_high_Level_languages) |
| Zig (freestanding, ReleaseSmall) | 432 B cart; the default panic handler pulls in std.fmt (4.3 → 14.5 KB); zigdom DOM demo ~24 KB | [wasmcart-zig](https://github.com/wasmcart/wasmcart-zig), [zigdom](https://github.com/ewaldhorn/zigdom) |
| AssemblyScript | `stub` runtime = bump allocator; `incremental` = TLSF + GC (no byte figure in the docs) | [AS runtime](https://www.assemblyscript.org/runtime.html) |
| MoonBit | minimal program 107 B; HTTP hello component 27 KB (vs ~100 KB Rust) | [zenn/mizchi](https://zenn.dev/mizchi/articles/introduce-moonbit?locale=en), [MoonBit blog](https://www.moonbitlang.com/blog/component-model) |
| TinyGo / Go | hello 108 KB / 2.23 MB; fib 20 KB / 1.85 MB | [RIoT Secure](https://riotsecure.se/blog/wasm_binary_size_in_high_Level_languages) |
| Kotlin/Wasm (WasmGC) | hello with DOM ~3–5 KB; Compose for Web several MB (skiko.wasm ~8 MB) | [seb.deleuze.fr](https://seb.deleuze.fr/introducing-kotlin-wasm/), [Kotlin Slack](https://slack-chats.kotlinlang.org/t/16406831/question-wasmjs-compose-what-is-the-typical-size-of-wasm-fil) |
| Dart (dart2wasm, WasmGC) | small sample 10 KB at -O2; Flutter web is MB-scale | [zetcode](https://zetcode.com/dart/webassembly/) |
| C# Blazor WASM (.NET 10) | ~2.1–2.25 MB br first load | [csharp-coder](https://www.csharp-coder.com/2026/08/blazor-webassembly-tutorial-2026-build.html) |
| Grain | no reliable published DOM-app numbers found | — |

**The WasmGC angle.** WasmGC removes the linear-memory allocator and refcounting for GC languages: Kotlin, Dart and MoonBit get a small hello world partly because of it. For NVE it does not fit:

1. rustc has no WasmGC codegen and none on the 2026 roadmap ([Rust project goals 2026](https://goals.rust-lang.org/2026/wasm-components.html)).
2. The `CommandBuffer` is a byte buffer in linear memory by design (zero-copy for every host). GC structs would need copying at the boundary.
3. An allocator is only ~5–7 KB raw (dlmalloc) and can shrink to under 1 KB with a frame arena.

**Verdict: stay with Rust.**

- The measured language overhead is ~400 B br over hand-written WAT.
- The NVE engine, ABI and codec are shared with the iOS, macOS, Android, Skia and Fabric hosts. A second language would duplicate the encoder and decoder and break the "single binary IR" invariant.
- Every size win available elsewhere (no GC runtime, no fmt, templates, fine-grained reactivity) is reachable in Rust with no_std/`build-std`, immediate-abort panics, raw `#[link(wasm_import_module)]` imports and a compiled view macro.
- C and Zig would give the same floor with less safety. MoonBit and AssemblyScript would give a similar floor and lose the shared engine. Kotlin, Dart, C# and Go are strictly larger.

## 6. Ranked changes

Savings are total br (wasm + JS) from the stage before. Measured with `hosts/web-min` unless marked *est.*

| # | Change | Counter saving | Studio saving | Effort | Risk | What breaks |
|---|---|---:|---:|---|---|---|
| 1 | One app per bundle; hot reload, pairing, multi-window, popup, interop, scene table and preinit/Wizer behind cargo features (prod has none) | **−132.9 KB** (260.1 → 127.2) | **−54.1 KB** (→ 206.0) | S–M | low | The `?app=` switch becomes separate bundles; dev features need a `--dev` build; the paired thin client becomes its own bundle |
| 2 | Remove Wizer and the preinit cache, use SSR frame-0 HTML | avoids +61.7 KB if Wizer is ever made to work; included in #1 | same | S | low | `is_preinitialized`/`take_preinit_*` API |
| 3 | `build-std` + `-Cpanic=immediate-abort` (toolchain track) | **−17.7 KB** (→ 109.5) | **−21.0 KB** (→ 184.9) | S | medium (nightly) | panic messages in the console |
| 4 | JS-side CommandBuffer interpreter: one import per frame; events come back as one encoded batch; nve-backend-web/web-sys out of wasm | **−20.4 KB** (→ 89.1), +~1.5–3 KB interpreter *(est.)* | **−20.0 KB** (→ 164.9), same + | M | medium | in-wasm `WebBackend` (keep it as the native test mirror); the canvas and IntersectionObserver paths move to JS |
| 5 | Drop anyhow/`std::backtrace` from dioxus-core (patch or fork), which kills rustc_demangle (14.8 K raw, survives immediate-abort through a vtable in the table) | ~−6–8 KB *(est.)* | same | S–M | low | Dioxus error-boundary formatting |
| 6 | Compile-time styles (typed `StyleProps`/`LayoutProps` from rsx, `const` folding, no runtime CSS parser, no flt2dec when JS writes CSS) | ~−3–5 KB *(est.)* | −4 to −8 KB (upper bound −8.0 measured) | M–L | low | string `style:` syntax becomes a typed macro (all examples) |
| 7 | Pre-encoded templates: CommandBuffer fragment / `<template>` + `cloneNode`; SSR hydration by `data-nve-id` | ~−2 KB *(est.)* | ~−3–6 KB *(est.)*; much faster frame 0 | M | medium | the writer's template walk; hot-patch of static attrs |
| 8 | Per-panel `#[wasm_split]` / own split tool for studio panels and CRDT sync | 0 | **−30–40 KB initial** *(est.)* | L | high (upstream bugs above) | needs wasm-split integration in `build.sh`; async panel load |
| 9 | Replace the Dioxus VDOM with a compiled fine-grained renderer (`nve-view!`: static parts are templates, slots are signal effects emitting CommandBuffer ops) | 89 → ~10–15 KB *(est.; Leptos-class 33.7 KB with std, measured)* | ~165 → ~70–90 KB *(est.)* | XL | high | the whole Dioxus API (rsx/hooks), subsecond/RSX hot reload, every example; native hosts gain the same win |
| 10 | no_std app/runtime layer with raw imports (no wasm-bindgen), frame-arena allocator, no `core::fmt` | ~−5–10 KB *(est.)*; floor 1.0 KB measured | ~−10–20 KB *(est.)* | XL | high | std collections and formatting in app code; wasm-bindgen ergonomics |

## 7. Staged roadmap

| Stage | Architecture change | Counter total br | Studio total br | Effort |
|---|---|---:|---:|---|
| 0 | today: one shell with both apps and dev features | 260.1 KB | 260.1 KB | — |
| 1 | #1 + #2: one app per prod bundle, dev features gated, no Wizer | **127.2 KB** (measured) | **206.0 KB** (measured) | S–M |
| 2 | #3: build-std + immediate-abort | **109.5 KB** (measured) | **184.9 KB** (measured) | S |
| 3 | #4: JS CommandBuffer interpreter | **89.1 KB** + ~2 KB (measured + est.) | **164.9 KB** + ~2 KB (measured + est.) | M |
| 4 | #5–#7: no anyhow/backtrace, compile-time styles, pre-encoded templates, SSR frame 0 | ~75–80 KB *(est.)* | ~145–150 KB *(est.)* | M–L |
| 5 | #8: per-panel split (studio) | ~75–80 KB | ~105–120 KB initial *(est.)* | L |
| 6 | #9: compiled fine-grained renderer instead of the VDOM | ~10–15 KB *(est.)* | ~50–70 KB total, ~35–45 KB initial *(est.)* | XL |
| 7 | #10: no_std runtime, raw imports, arena allocator | **~3–6 KB** *(est.; floor 1.0 KB measured)* | **~40–60 KB** total, **~25–40 KB** initial *(est.)* | XL |

**Feasibility.**

- **Stages 1–3 are cheap and measured:** −171 KB br for the counter (−66%) and −95 KB for studio (−37%). They need no engine redesign.
- **Stages 4–5 are incremental.**
- **Stage 6 is the real architectural decision.** The VDOM plus its monomorphisations (dioxus_core 47 K + most of core 38 K / alloc 21 K / hashbrown 11 K) is about 60% of what remains after stage 3. The comparison with Leptos (fine-grained, 28.6 KB br wasm) versus dioxus-web (VDOM, 101 KB) at identical UI and profile confirms it. NVE's binary CommandBuffer already *is* the retained-mode op stream a fine-grained renderer emits, so dropping the VDOM fits the architecture and benefits the native hosts too.
- **The hand-written-WAT size is reachable only for trivial apps.** For studio, the floor is set by the app's own logic: workspace state, CRDT codec, virtual list, palette search.

## Upstream bugs and quirks found

All are also recorded in `docs/UPSTREAM.md`.

- **Wizer 10.0.0 × wasm-bindgen 0.2.130.**
  - Symptom: `Error: unsupported ref.null instruction` on wasm-bindgen output (reference-types table init). `hosts/web-shell/build.sh` falls back silently, so pre-init never runs.
  - Mitigation: remove the Wizer step. If it is ever wanted, it must run on the raw rustc output *before* wasm-bindgen, and it costs +61.7 KB br.
- **rustc ≥ 1.96 on wasm32-unknown-unknown no longer passes `--allow-undefined`.**
  - Symptom: `rust-lld: undefined symbol: f` for a raw `extern "C"` import.
  - Mitigation: `#[link(wasm_import_module = "env")]` ([Rust blog](https://blog.rust-lang.org/2026/04/04/changes-to-webassembly-targets-and-handling-undefined-symbols/), [PR #149868](https://github.com/rust-lang/rust/pull/149868)).
- **Cargo nightly 1.101 (2026-10-12): `panic = "immediate-abort"` in a profile needs `-Zpanic-immediate-abort`** in addition to `-Zunstable-options -Cpanic=immediate-abort` and `-Zbuild-std`.
- **dioxus-core 0.7.10 → anyhow → std backtrace → rustc_demangle (~15–18 KB raw).** It is kept alive through an indirect-call table entry (a `Display` vtable), so it survives LTO **and** immediate-abort panics. Mitigation: patch it out (handoff to the runtime/toolchain tracks).
- **Dioxus wasm-split (0.7.x / 0.8.0-alpha.1)** crashes under fat LTO + `codegen-units=1` (walrus GC assertion; fix in open [PR #5795](https://github.com/DioxusLabs/dioxus/pull/5795)), and split routes render twice under SSR hydration ([#5287](https://github.com/DioxusLabs/dioxus/discussions/5287)).

## Handoffs to other tracks

- **toolchain:** build-std + immediate-abort measured −17.7 KB (counter) / −21.0 KB (studio) on the minimal shell. rustc_demangle survives it (see above). `wasm-opt -Oz` makes gzip *worse* on the full shell (310,944 → 315,548), but brotli only slightly so.
- **boundary:** a JS CommandBuffer interpreter (stage 3) measured −20 KB br. It replaces 80 web-sys imports and the per-listener closures with one import per frame and one export per event batch. The JS glue drops from 23.7 KB to 8.5 KB raw, most of it wasm-bindgen's own boilerplate, which raw imports would remove.
- **runtime:** dlmalloc (5–7 K) could become a frame arena. The CommandBuffer could live in a static scratch area (floor-rs needs no allocator at all). `hashbrown` (11–20 K) comes from dioxus-core and NVE node maps; dense `Vec` slabs keyed by `NodeId` would remove it.

## Prototypes in this branch

- `hosts/web-min/`: the minimal shell. Features:
  - `counter` / `studio`: one app per bundle;
  - `js-apply`: no wasm DOM backend; size probe only, the JS interpreter is not written;
  - `probe`: cost model; mounts the component file named by env `NVE_PROBE`.
- Tests: `tests/frame0_dump.rs` writes the frame-0 SSR HTML and CommandBuffer; `tests/frame0_heap.rs` measures the live heap after frame 0 (what Wizer would snapshot).
  - Run them with `cargo test -p nve-web-min --features counter,studio`.
  - Build with `docs/research/wasm-size/architecture/build-crate.sh nve-web-min nve_web_min <label> --features counter`.
- `docs/research/wasm-size/architecture/floor-wat/`: hand-written WAT counter, loader and a Node check.
  - Build: `wat2wasm counter.wat -o counter.wasm && wasm-opt -Oz counter.wasm -o counter.opt.wasm && node verify.mjs`.
- `docs/research/wasm-size/architecture/floor-rs/`: no_std Rust counter (standalone crate) and its opcode-interpreting loader.
  - Build: `cargo build --release --target wasm32-unknown-unknown && wasm-opt -Oz --enable-bulk-memory …`.
- `browser-check.html`: headless-Chrome click test for both floors. Serve the dir and copy each wasm to `<dir>/serve/` under the name its loader fetches.

## Sources

- RIoT Secure, WASM binary size across languages (2025-10): https://riotsecure.se/blog/wasm_binary_size_in_high_Level_languages
- Leptos islands sizes: https://book.leptos.dev/islands.html · binary size guide: https://book.leptos.dev/deployment/binary_size.html
- Dioxus 0.7 release (wasm-split): https://dioxuslabs.com/blog/release-070/ · wasm-split LTO bug: https://github.com/DioxusLabs/dioxus/pull/5795 · SSR double render: https://github.com/DioxusLabs/dioxus/discussions/5287
- sledgehammer_bindgen (batched JS interpreter used by dioxus-web): https://docs.rs/sledgehammer_bindgen · HN note on the single binary batch: https://news.ycombinator.com/item?id=42388665
- Rust wasm `--allow-undefined` removal: https://blog.rust-lang.org/2026/04/04/changes-to-webassembly-targets-and-handling-undefined-symbols/ · https://github.com/rust-lang/rust/pull/149868
- Rust 2026 wasm goals (no WasmGC): https://goals.rust-lang.org/2026/wasm-components.html
- AssemblyScript runtime variants: https://www.assemblyscript.org/runtime.html
- MoonBit sizes: https://www.moonbitlang.com/blog/component-model · https://zenn.dev/mizchi/articles/introduce-moonbit?locale=en
- Zig: https://github.com/wasmcart/wasmcart-zig · https://github.com/ewaldhorn/zigdom
- Kotlin/Wasm: https://seb.deleuze.fr/introducing-kotlin-wasm/ · https://slack-chats.kotlinlang.org/t/16406831/question-wasmjs-compose-what-is-the-typical-size-of-wasm-fil
- Dart: https://zetcode.com/dart/webassembly/ · https://dart.dev/web/wasm
- Blazor: https://www.csharp-coder.com/2026/08/blazor-webassembly-tutorial-2026-build.html
