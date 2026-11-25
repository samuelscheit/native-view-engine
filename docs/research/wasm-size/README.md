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
2. **std panic/fmt/backtrace machinery:** about 23 KB is removed by `-Cpanic=immediate-abort`, plus
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
| serde_json | hot-reload fallback, island/channel payloads | removed from wasm; binary NVP1 only, which the zero-JSON invariant already requires |
| std | panic/fmt/backtrace | keep std, rebuilt with `-Zbuild-std` and immediate-abort; `no_std` gains < 1 KB after that |
| dlmalloc | 7 KB | talc (−2.5 KB), later a per-frame arena |
| HashMap / BTreeMap | 23 KB hashbrown + ~37 KB of BTree instantiations (studio) | dense `NodeId` slabs plus one shared sorted `VecMap` |
| Wizer | ran on nothing (it fails on bindgen output); +62 KB if it works | removed; frame 0 comes from SSR HTML (2.9 KB br for studio) |

## Staged roadmap

| Stage | Change | Counter | Studio | Effort | Risk |
|---|---|---:|---:|---|---|
| 0 | today | 260.1 *m* | 260.1 *m* | — | — |
| 1 | **toolchain pipeline**: nightly `-Zbuild-std` + immediate-abort + `optimize_for_size`, fmt/location flags, compression-tuned wasm-opt list, size-sorted functions, terser, `strip="debuginfo"`, no Wizer | 218.9 *m* (one shared bundle) | 218.9 *m* | done | low |
| 2 | **one app per production bundle**; `dev`/`multi-window`/`scene-view` features; `anyhow` without std; `nve_abi::num` fixed-point codec; NVP1-only hot reload; talc; FxHash | 100.2 *m* stable / 87.1 *m* + immediate-abort | 180.0 *m* stable / ~158 *est.* + stage 1 | S–M | low |
| 3 | **raw boundary**: 0 imports, `nve_dom.js` interprets the CommandBuffer from linear memory, delegated events written into the EventRing; subsecond patch | ~66–70 *est.* (−19.1 *m* on a stable counter) | ~140 *est.* (−16.5 *m*) | L | medium |
| 4 | no JSON in production (channel bus, island props); dense slabs; `VecMap`; one sort instantiation; compile-time typed styles and pre-encoded templates | ~55 *est.* | ~120–125 *est.* | M–L | low–medium |
| 5 | per-panel lazy split of studio (needs a working split tool) | ~55 | ~85–95 initial *est.* | L | high |
| 6 | **NVE-native compiled fine-grained runtime** instead of dioxus-core: static parts are template bytes, slots are signal effects that emit CommandBuffer ops, LIS only for keyed lists | ~15–25 *est.* | ~70–80 total, ~40–50 initial *est.* | XL | high |
| 7 | `no_std` runtime layer, frame arena, no `core::fmt` | ~3–6 *est.* (floor 1.0 *m*) | ~40–60 total, ~25–40 initial *est.* | XL | high |

How to read it:

- **Stages 1–2 are measured and need no design decision.** They take the counter from 260 to about
  87 KB and studio to about 160 KB.
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

`main` `2106b57` (toolchain): 260,171 → 218,880 B total br, all 6 e2e suites green.

The snip step was hardened in review. The first version also trapped `Backtrace::capture` and its
drop glue, which would have aborted the app on every `anyhow::Error`; `wat-snip.py --keep` now fails
the build if a pattern ever matches a live path. The snip itself goes away once dioxus-core's
`anyhow` loses `std` (stage 2), which is the root fix.

The upstream bugs and quirks found by all tracks are consolidated in
[docs/UPSTREAM.md](../../UPSTREAM.md) under *Dioxus*, *WebAssembly size toolchain* and *Browsers*.
