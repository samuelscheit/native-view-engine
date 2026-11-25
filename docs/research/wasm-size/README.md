# WASM bundle size: how close can NVE get to hand-written WAT?

Synthesis of four parallel research tracks (2026-11-25, baseline HEAD `4e046ca`, stable rustc 1.98.1,
wasm-bindgen 0.2.130, binaryen 133). Every number is **brotli q11 of the shipped files, wasm + JS
glue**, unless marked otherwise. *m* = measured, *est.* = estimated from symbol sizes or overlapping
measurements.

| Track | Question | Report | Prototype |
|---|---|---|---|
| toolchain | compiler, std, flags, post-link, compression | [toolchain.md](toolchain.md) | **landed** on `main` (`2106b57`) |
| boundary | wasm-bindgen / web-sys vs a raw ABI + JS interpreter | [boundary.md](boundary.md) | branch `t3/reduce-wasm-js-boundary` @ `149f654` (`hosts/web-raw`, `nve_dom.js`, subsecond patch) |
| runtime | Rust code and crates inside the wasm | [runtime.md](runtime.md) | branch `t3/wasm-runtime-size-research` @ `3c91e58` (web-shell features, `nve_abi::{num,hash}`, dioxus patches, talc) |
| architecture | theoretical floor, templates, splitting, other languages | [architecture.md](architecture.md) | branch `t3/wasm-size-architecture` @ `27d5cc3` (`hosts/web-min`); floors in [architecture/](architecture/) |

## Answer

**The compiler is not the problem; the architecture is.** For the same counter UI:

| Implementation | total br |
|---|---:|
| hand-written WAT ([floor-wat](architecture/floor-wat/counter.wat)) | **652 B** *m* |
| `no_std` Rust, raw imports, CommandBuffer handed to a JS interpreter ([floor-rs](architecture/floor-rs/src/lib.rs)) | **1,025 B** *m* |
| Leptos 0.8 CSR (fine-grained) | 33.7 KB *m* |
| dioxus-web 0.7 (VDOM) | 108.5 KB *m* |
| NVE today (`nve-web-shell`, both apps + dev infra) | **260.1 KB** *m* |

Rust costs about 400 B over hand-written WAT for the same design. The 250× gap comes from what we
ship, not from how it is compiled:

1. **Everything is linked into every bundle:** both demo apps, hot reload (serde_json), pairing,
   popups/multi-window, the scene-table mirror and the Wizer preinit cache. That is 52 KB for studio
   and 133 KB for the counter.
2. **std panic/fmt/backtrace machinery:** about 23 KB is removed by `-Cpanic=immediate-abort`
   (shipped in stage 1, then reverted by decision so that production panics keep their message and
   location, see [toolchain.md](toolchain.md#decision-panic-messages-in-production-over-23-kb)), plus
   12 KB of `rustc-demangle` that `anyhow`'s `std` feature pulls in through dioxus-core.
3. **A string round trip for every style value:** RSX `"width: 12px"` is parsed with `f32::from_str`
   and re-formatted with Grisu/Dragon4 into CSS. That is about 30 KB raw, or 12.7 KB br.
4. **The DOM is applied inside wasm through web-sys:** one import call per DOM op, one Closure per
   listener, and 64 KB of JS glue. Together about 17–19 KB br.
5. **The VDOM runtime:** dioxus-core costs about 46 KB br on its own (*m*, isolated probe). A
   fine-grained signals core costs 6.5–9.6 KB (sycamore-reactive / reactive_graph, *m*). After
   stages 1–4 below, the VDOM and its generic monomorphisations are about 60 % of what is left.

**Changing compiler or language would not help.** LLVM is the only wasm backend for Rust:
Cranelift and GCC emit no wasm, and there is no WasmGC codegen on the 2026 roadmap. C and Zig give
the same floor with less safety. MoonBit and AssemblyScript give a similar floor but lose the engine
and ABI that NVE shares with every native host. Go, Kotlin, Dart and C# are strictly larger. Every
size lever those languages have (no runtime, no fmt, templates, fine-grained updates) is also
available in Rust.

**What libraries we use, and how:**

| Library | Today | Needed for a minimal bundle |
|---|---|---|
| wasm-bindgen + web-sys | 120 imports, a closure per listener, 64 KB glue | none in production: 0 imports and 5 raw exports, plus a 3.5 KB br `nve_dom.js` (*m*, byte-identical DOM) |
| dioxus-core / -signals | VDOM, `anyhow` std, SipHash sets | short term: patch (`anyhow` without std, `FxHashSet`, subsecond's wasm deps opt-in). Long term: replace it with a compiled fine-grained runtime |
| serde_json | hot-reload fallback, island/channel payloads | removed from wasm; binary NVP1 only, which the binary-wire invariant already requires |
| std | panic/fmt/backtrace | keep std, rebuilt with `-Zbuild-std` (`optimize_for_size`); immediate-abort was reverted, so panic messages and locations ship (~23 KB) |
| dlmalloc | 7 KB | talc (−2.5 KB), later a per-frame arena |
| HashMap / BTreeMap | 23 KB hashbrown + ~37 KB of BTree instantiations (studio) | dense `NodeId` slabs plus one shared sorted `VecMap` |
| Wizer | ran on nothing (it fails on bindgen output); +62 KB if it works | removed; frame 0 comes from SSR HTML (2.9 KB br for studio) |

## Staged roadmap

| Stage | Change | Counter | Studio | Effort | Risk |
|---|---|---:|---:|---|---|
| 0 | today | 260.1 *m* | 260.1 *m* | — | — |
| 1 | **toolchain pipeline**: nightly `-Zbuild-std` + immediate-abort + `optimize_for_size`, fmt/location flags, compression-tuned wasm-opt list, size-sorted functions, terser, `strip="debuginfo"`, no Wizer | 218.9 *m* (one shared bundle) | 218.9 *m* | done | low |
| 2 | **one app per production bundle**; `dev`/`multi-window`/`scene-view` features; `anyhow` without std; `nve_abi::num` fixed-point codec; NVP1-only hot reload; talc; FxHash | **105.1** *m* | **277.0** *m* (app grew ~105 KB on `main` meanwhile) | done | low |
| 3 | **raw boundary**: 0 imports, `nve_dom.js` interprets the CommandBuffer from linear memory, delegated events written into the EventRing; subsecond patch | ~66–70 *est.* (−19.1 *m* on a stable counter) | ~140 *est.* (−16.5 *m*) | L | medium |
| 4 | no JSON in production (channel bus, island props); dense slabs; `VecMap`; one sort instantiation; compile-time typed styles and pre-encoded templates | ~55 *est.* | ~120–125 *est.* | M–L | low–medium |
| 5 | per-panel lazy split of studio (needs a working split tool) | ~55 | ~85–95 initial *est.* | L | high |
| 6 | **NVE-native compiled fine-grained runtime** instead of dioxus-core: static parts are template bytes, slots are signal effects that emit CommandBuffer ops, LIS only for keyed lists | ~15–25 *est.* | ~70–80 total, ~40–50 initial *est.* | XL | high |
| 7 | `no_std` runtime layer, frame arena, no `core::fmt` | ~3–6 *est.* (floor 1.0 *m*) | ~40–60 total, ~25–40 initial *est.* | XL | high |

How to read it:

- **Stages 1–2 are done.** They take the counter from 260 to 105.1 KB. Studio is 277.0 KB because the
  app itself grew by ~105 KB on `main` meanwhile (−23 % vs the shared bundle at `64024e5`).
- **Stage 3 is a clean subtraction** that fits the "one CommandBuffer per frame" invariant.
  `WebDomMirror` stays as the semantic spec, and a Node parity test pins the JS interpreter to it.
- **Stage 6 is the real decision.** It is the only step that gets within an order of magnitude of
  hand-written WAT for non-trivial apps. Native hosts benefit too, because they consume the same
  CommandBuffer. The cost is the Dioxus API: rsx, hooks, router, subsecond, and every example.
  Tier-1 NVP1 patching maps directly onto a template table, and Tier-2 state onto the existing
  `signal_archive`.
- **Hand-written-WAT size is reachable only for trivial apps.** For studio the floor is the app's
  own logic: workspace state, CRDT codec, virtual list and palette search.

## Delivery: independent of the bundle and just as large

- **Serve the precompressed q11 `.br`** (now written by `build.sh`). CDNs that compress on the fly
  use q4–q5, which costs +27–42 KB for the same wasm.
- **Compression Dictionary Transport** (`Use-As-Dictionary`, `dcb`/`dcz`; Chromium only, other
  browsers fall back to plain br). An update download drops to 21.7 KB one commit after the
  previous build and 57.4 KB eleven commits after.

## Already landed

`main` `2106b57` (stage 1, toolchain): 260,171 → 218,880 B total br, all 6 e2e suites green. The
SSR/hydration merges that followed grew the same shared bundle to 240,304 B before stage 2.

Stage 2 (branch `t3/wasm-bundle-size-stage-two`): `hosts/web-shell/build.sh` now builds one
production bundle per app plus a dev bundle. All 7 e2e suites pass on the release bundles; the
production suites also assert that the other app, the dev probe and the scene view are absent.

Main kept growing while stage 2 was in flight (CRDT peer transport, the nve-ui studio chrome,
TypedStyle props). The same shared bundle went from 240,304 B (`83d2f82`) to 290,960 B (`64024e5`).
The stage-2 rows below are measured after merging `17be096` (which adds the web NativeView mount
adapters and the studio router/sdk/traits work on top of `64024e5`).

| Bundle (brotli q11, bytes) | wasm raw | wasm gzip | wasm br | JS br | **total br** | budget |
|---|---:|---:|---:|---:|---:|---:|
| one shared bundle, `main` `83d2f82` | 756,044 | 286,736 | 232,347 | 7,957 | 240,304 | 245,760 |
| one shared bundle, `main` `64024e5` | 919,932 | 353,452 | 283,003 | 7,957 | 290,960 | 245,760 (over) |
| `pkg/studio` (`app-studio`) | 933,267 | 340,186 | 270,262 | 6,751 | **277,013** | 285,696 |
| `pkg/counter` (`app-counter`) | 292,390 | 117,753 | 98,441 | 6,689 | **105,130** | 108,544 |
| `pkg/dev` (default features: both apps, dev, multi-window, scene-view) | 1,043,585 | 381,496 | 301,311 | 8,324 | 309,635 | 318,464 |

On `64024e5` the same bundles were studio 222,850 (−23.4 % vs that shared bundle), counter 99,010
(−66.0 %) and dev 255,949 B. Merging `17be096` adds ~54 KB of studio app code (router, SDK
services, traits, NativeView adapters). One SDK error message that formatted an `f32` relinked
`core::fmt` float printing (+4.9 KB br); it now uses a static message.

Before main moved (same pipeline, on `83d2f82`) the bundles were studio 172,047, counter 96,822
and dev 205,633 B. The dioxus fork plus the removed snip step alone was −2.9 KB on the shared
bundle (240,304 → 237,443). Production bundles also drop the other app, `dev`, `multi-window` and
`scene-view`. The counter misses the 87.1 KB research figure by ~12 KB. That figure predates the
SSR/hydration merges (~21 KB on every bundle); the counter bundle still ships
`render_counter_ssr_bundle` and `ProgressiveIslandHost`.

The name section of the dev build contains no `core::fmt::float`, `flt2dec`, `dec2flt`, SipHash,
`RandomState`, dlmalloc or backtrace code. One merged `<[u8]>::starts_with` instance carries a
`rustc_demangle` name but no demangler code (see docs/UPSTREAM.md). `serde_json` (studio island and
channel payloads, the counter's JSON props and snapshots) and `anyhow`'s error chain remain; the
JSON is stage 4.

The snip step is gone. dioxus-core's `anyhow` lost `std` in the fork
`samuelscheit/dioxus@nve/v0.7.10-wasm-size`, which is the root fix, so nothing has to be trapped.

Panic messages and caching (branch `t3/web-host-fixes`, on `main` `851eb4b`): release bundles keep
panic messages and `file:line` again. `-Cpanic=immediate-abort -Zfmt-debug=shallow
-Zlocation-detail=none` are dropped; the pinned nightly stays for `-Zbuild-std` with
`optimize_for_size`. A start-time hook logs `[NVE Web Panic] ...` in every bundle, and the e2e proves
it with a real panic in the release dev bundle. Every bundle is also published content-hashed
(`pkg/<bundle>/<hash>/` + `manifest.json`, see `hosts/web-shell/README.md` "Caching").

| Bundle (brotli q11, bytes) | wasm raw | wasm gzip | wasm br | JS br | **total br** | with the 3 flags | budget |
|---|---:|---:|---:|---:|---:|---:|---:|
| `pkg/studio` | 1,027,257 | 376,696 | 299,020 | 7,697 | **306,717** | 285,929 (+20,788) | 316,416 |
| `pkg/counter` | 363,641 | 147,144 | 123,263 | 7,617 | **130,880** | 114,514 (+16,366) | 135,168 |
| `pkg/dev` | 1,152,451 | 421,842 | 333,431 | 9,280 | **342,711** | 319,827 (+22,884) | 353,280 |

The rest of the growth over the stage-2 rows above comes from `main` itself (camera adapters, nve-ui,
TV/spatial focus) between `17be096` and `851eb4b`.

The upstream bugs and quirks found by all tracks are consolidated in
[docs/UPSTREAM.md](../../UPSTREAM.md) under *Dioxus*, *WebAssembly size toolchain* and *Browsers*.
