# WASM size research: toolchain track

Scope: everything that shrinks the shipped `hosts/web-shell` bundle **without changing app or library source**:
compiler, std, flags, target features, linker, wasm-bindgen, binaryen, post-link rewriting, compression.

## TL;DR

- The toolchain-only best pipeline ships **212,798 B wasm + 5,930 B JS = 218,728 B brotli** (q11).
  The baseline (HEAD `4e046ca`, `build.sh` as it was) shipped 251,541 + 8,630 = **260,171 B**.
  That is **−41.4 KB (−15.9 %)**. All 6 headless-Chrome e2e suites pass on the new bundle
  (`node hosts/web-shell/e2e/run_e2e.mjs`: 0 console errors, 0 wasm panics), and the screenshot was checked.
- Two more toolchain-side levers are larger than the whole flag work, but they are deployment changes:
  - **Serve the precompressed q11 file.** CDNs usually compress on the fly at brotli q4–q5, which gives
    240–255 KB for the same wasm (+27…+42 KB).
  - **Compression Dictionary Transport** (previous build as the dictionary). The update download drops
    to 21.7 KB after one commit and 57.4 KB after 11 commits / +10.7 k source lines.
- The baseline had a silent bug: `strip = "symbols"` deletes the `target_features` section, so `wasm-opt -Oz`
  **failed validation** and `build.sh` shipped the un-optimized wasm. Fixing it naïvely would have made
  things worse: binaryen `-Oz` *grows* brotli by 2.7 KB on LLVM `-Oz` output. The reason is function
  reordering plus inlining, local-cse and memory-packing. This also explains why gzip got worse in the
  original measurement.
- Verdict: the toolchain floor is about 210 KB brotli. What remains is code that is genuinely reachable:
  the studio app is 143 KB raw, `.rodata` 96 KB, dioxus-core 77 KB, alloc/core generics 112 KB,
  serde_json 23 KB, hashbrown 24 KB. No compiler, backend or flag gets near a "hand-written WAT" size.
  The next order of magnitude can only come from the source/library tracks (see Handoffs).
  A different Rust backend is not an option: Cranelift and GCC do not emit wasm, so LLVM is the only path.

## Method

- Harness: [`toolchain/measure.sh`](toolchain/measure.sh). It does `build` (toolchain + rustflags, own target dir),
  `post` (wasm-bindgen + wasm-opt flags), and `final` (the exact `build.sh` post-pipeline). Every row reports the
  shipped files: wasm raw / `gzip -9` / `brotli -q 11`, the JS glue raw / gzip / brotli, and **total brotli**.
- Tools: rustc stable 1.98.1, nightly `1.101.0-nightly (c1070d693 2026-09-28)` and `nightly-2026-10-20`,
  wasm-bindgen 0.2.130, binaryen `wasm-opt` 133, wasm-tools, twiggy, wizer 10.0.0, wasm-snip 0.4.0, terser 5, esbuild 0.25.
- Builds take 31–60 s each (`-Zbuild-std` included), so experiments ran four at a time.

## Cumulative pipeline (each row adds to the previous one)

| # | Step | wasm br | JS br | total br | Δ total |
|---|------|--------:|------:|---------:|--------:|
| 0 | Baseline as shipped (stable, `strip="symbols"`, wasm-opt silently failed) | 251,541 | 8,630 | 260,171 | — |
| 0b | …if wasm-opt `-Oz` had worked (unstripped input) | 254,383 | 8,520 | 262,903 | **+2,732** |
| 1 | `strip="debuginfo"` + `--remove-name-section` (keeps `target_features` → externref glue, JS raw 64.4→57.9 KB) | 251,648 | 8,520 | 260,168 | −3 |
| 2 | nightly, prebuilt std (no change on its own) | 251,373 | 8,527 | 259,900 | −268 |
| 3 | `-Zbuild-std=std,panic_abort` + `-Cpanic=immediate-abort` | 228,219 | 8,537 | 236,756 | **−23,144** |
| 4 | `-Zbuild-std-features=optimize_for_size` | 224,050 | 8,535 | 232,585 | **−4,171** |
| 5 | `-Zfmt-debug=shallow -Zlocation-detail=none` | 223,222 | 8,535 | 231,757 | −828 |
| 6 | snip `rustc_demangle` + `std::backtrace` (dead on this target) | 217,380 | 8,535 | 225,915 | **−5,842** |
| 7 | compression-tuned wasm-opt pass list (not `-Oz`) | 214,225 | 8,535 | 222,760 | −3,155 |
| 8 | sort functions by body size (before wasm-opt) | 213,161 | 8,535 | 221,696 | −1,064 |
| 9 | terser `--module -c passes=3 -m --toplevel` on the JS glue (+snippets) | 213,161 | 5,930 | 219,091 | **−2,605** |
| — | **`build.sh` end-to-end** (fresh build, same steps) | **212,798** | **5,930** | **218,728** | **−41,443** |
| opt | `-Zfmt-debug=none` instead of `shallow` (needs 3 source fixes, see Risks) | −2,284 | −273 | ≈216.2 K | −2,557 |

Stable-only variant (no nightly): steps 1 and 6–9 on stable 1.98.1 give 241,061 + 5,811 = **246,872 B**
(−13.3 KB), measured before the sort step (about another −1 KB with it).

## Findings in detail

### 1. Compiler and std (nightly)

All rows below are relative to the step-5 base (n3s, 221,292 B wasm br after the safe pass list):

| Experiment | wasm br Δ | Verdict |
|---|---:|---|
| `-Zbuild-std` + `-Cpanic=immediate-abort` (vs prebuilt std) | **−23,154** | Take. Removes every panic message, location and formatting path, including std's. |
| `-Zbuild-std-features=optimize_for_size` | **−4,169** | Take. Size-oriented algorithms in core/alloc (sort, fmt, etc.). |
| `-Zfmt-debug=shallow` + `-Zlocation-detail=none` | −828 | Take. Behaviour-safe: enum `{:?}` still prints the variant name. |
| `-Zfmt-debug=none` | −2,284 (and JS −273) | Take after source fixes. `{:?}` prints nothing, which breaks visible text (see Risks). |
| `-Zshare-generics=y` | 0 | No effect with fat LTO + 1 CGU. |
| `-Zvirtual-function-elimination` (all crates) | ICE | LLVM error `llvm.type.checked.load … LowerTypeTests` (rust#164267). |
| `-Zvirtual-function-elimination` (final crate only, via `cargo rustc -- …`) | −327 | Not worth a known-buggy flag (also LLVM#240258). |
| `opt-level="s"` (vs `"z"`) | +15,695 | Reject. |
| `lto="thin"` (vs fat) | +16,313 | Reject. |
| `-Cllvm-args=-inline-threshold=0` | +1,681 | Reject. LLVM `-Oz` already inlines minimally; less inlining keeps more call/frame overhead. |
| `-Zmir-opt-level=4` | +5,811 | Reject. |
| `-Zinline-mir=no` | 0 | Default already. |
| `-Zcross-crate-inline-threshold=never` | +622 | Reject. |
| `-Zmerge-functions=disabled` / `trampolines` | +1,014 / 0 | Default (aliases) is best. |

`panic_immediate_abort` is no longer a build-std feature; on nightlies from 2025-09-24 it is the panic strategy
`-Zunstable-options -Cpanic=immediate-abort` ([rust#146317](https://github.com/rust-lang/rust/pull/146317),
[MCP 909](https://github.com/rust-lang/compiler-team/issues/909), [leptos book#279](https://github.com/leptos-rs/book/pull/279)).
It is passed through `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS`, so host build scripts and proc-macros
are unaffected, and the workspace's stable `Cargo.toml` stays parseable. Cargo's `panic = "immediate-abort"`
profile key would need `cargo-features` and therefore a nightly for every workspace build.

### 2. Target and features

| Variant | wasm br | JS br | total Δ vs generic |
|---|---:|---:|---:|
| `generic` (default: bulk-memory, multivalue, mutable-globals, nontrapping-fptoint, reference-types, sign-ext) | 221,292 | 8,535 | — |
| `-Ctarget-cpu=mvp` | 220,803 | 8,804 | −220 (wasm −489, but wasm-bindgen loses externref → heap-slab glue +10 KB raw JS) |
| `mvp` + `+reference-types,+multivalue` | 221,939 | 8,535 | +647 |
| generic `-bulk-memory,-bulk-memory-opt` | 221,524 | 8,535 | +232 |
| `bleeding-edge` (tail calls: 400 `return_call`) | 222,503 | 8,535 | +1,211 |

Feature choice changes the size by under ±1.2 KB. `generic` is the right default because it keeps externref glue
and needs no build-std just for the features. `wasm32v1-none` ships only `core`/`alloc` and no std
([rustc book](https://doc.rust-lang.org/nightly/rustc/platform-support/wasm32v1-none.html)). Using it would need
`no_std` dioxus-core, generational-box, futures, serde_json and so on, plus wasm-bindgen with
`default-features=false`. That is a source/library project, and its size upside over build-std + immediate-abort is
small: about 5.5 KB raw of std, 2 KB of backtrace and 5 KB of dlmalloc remain.

### 3. Linker and stripping

- `strip = "symbols"` makes wasm-ld `--strip-all`, which **also drops `target_features`**. That has two effects:
  - wasm-bindgen 0.2.130 warns and generates the non-externref heap-slab JS glue (64.4 KB vs 57.9 KB raw).
    [wasm-bindgen#5205](https://github.com/wasm-bindgen/wasm-bindgen/pull/5205) and
    [#5382](https://github.com/wasm-bindgen/wasm-bindgen/pull/5382) address the detection.
  - binaryen cannot detect features and rejects `memory.copy` / `i64.trunc_sat`. These are default since Rust 1.87
    ([rustc book](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html),
    [rust#141080](https://github.com/rust-lang/rust/issues/141080)).
- The correct place to drop names is **after wasm-bindgen**: either `--remove-name-section` or `wasm-opt --strip-debug`.
  In this pipeline names must survive until step 6/8 (snip/sort), so wasm-opt strips them. Profile: `strip = "debuginfo"`.
- `--gc-sections` is already the default for wasm-ld. `-zstack-size` does not affect file size.
  `--compress-relocations` only applies to relocatable objects.

### 4. wasm-bindgen and the JS glue

| Target / flags | entry JS raw | JS br |
|---|---:|---:|
| `--target web` (current) | 57,922 | 8,535 |
| `--target web --omit-default-module-path --encode-into always` | 57,525 | 8,447 |
| `--target bundler` (2 files) | 548 + 50,848 | 241 + 7,657 |
| `--target experimental-nodejs-module` / `deno` | 54,273 / 53,936 | 7,776 / 7,705 |
| `--target module` (source-phase `import source`, Chromium-only today) | 54,128 | 7,707 |
| `web` + esbuild `--minify` | 24,107 | 5,872 |
| `web` + terser (`--module -c passes=3 -m --toplevel`) | 24,253 | **5,746** |

Doc comments make up a large share of the unminified glue, and minification removes most of the difference between
targets. `--weak-refs` adds code (FinalizationRegistry). `--no-demangle` only affects names that are stripped anyway.
`--remove-producers-section` saves 60 B. wasm-bindgen 0.2.130 has no `no-modules` output for this crate (it fails).

### 5. binaryen wasm-opt: why `-Oz` loses, and what to run instead

Levels on the stable unstripped module (251,648 B br without wasm-opt):

| `-O1` | `-O2` | `-O3` | `-Os` | `-Oz` | `-Oz -tnh` (n2) |
|---:|---:|---:|---:|---:|---:|
| 251,266 | 252,605 | 254,200 | 254,053 | 254,406 | −641 vs `-Oz`, still worse than none |

Single passes on the same input (Δ brotli vs no pass, in bytes):

- **Hurt**:
  - `--inlining-optimizing` +3,233
  - `--local-cse` +3,337
  - `--reorder-functions` +1,980 (and **+12.6 KB gzip**)
  - `--memory-packing` +2,047
  - `--coalesce-locals` +1,754
  - `--simplify-locals` +1,336
  - `--const-hoisting` +1,642
  - `--dae-optimizing` +692
  - `--merge-similar-functions` +620
- **Help**:
  - `--reorder-locals` −970
  - `--duplicate-function-elimination` −755
  - `--remove-unused-module-elements` −408
  - `--converge` on the list −447
- **Neutral or slightly worse**: `--gufa` (+266), `-cw --gufa --signature-*` (+222), `--precompute-propagate` (0).
- `--minify-imports-and-exports` gives −215 but needs a matching JS rewrite.

Root cause: LLVM `-Oz` output is already small and very *regular*. Binaryen's raw-size passes (inlining, CSE,
local coalescing, segment splitting) trade that regularity for fewer bytes, and brotli loses more than it gains.
This is the same pattern as [binaryen#4322](https://github.com/WebAssembly/binaryen/issues/4322) (open since 2021,
which blames inlining) and the recent reports in [comprs#622](https://github.com/derodero24/comprs/pull/622) and
[sgl#895](https://github.com/stevepryde/sgl/issues/895).

**Function order** is the hidden variable. With the same code and only the order changed, using the wat-reorder experiment:

| order | gzip -9 | brotli | zstd -19 |
|---|---:|---:|---:|
| link order (LLVM) | 262,644 | 214,225 | 226,957 |
| sorted by name | **260,254** | 213,547 | 226,327 |
| sorted by body size | 262,224 | **213,126** | 226,326 |
| random shuffle | 284,690 | 218,273 | 232,159 |

gzip's 32 KB window is very sensitive to order: a shuffle costs +22 KB. This is why the baseline's "`-Oz` made gzip
worse" happened, since `-Oz` includes function reordering by call count. `build.sh` sorts by size in the same WAT
round trip as the snip, then runs a pass list without reordering:
`--strip-debug --strip-producers --strip-target-features --duplicate-function-elimination
--remove-unused-module-elements --vacuum --merge-blocks --dce --remove-unused-brs --optimize-instructions
--precompute --reorder-locals --converge`.

An automated greedy pass search (each round adds the best brotli pass) stalled at 216.3 KB, worse than this list,
because it took `-O1` first and got stuck in a local optimum. A real "similarity ordering" pass, the idea in
binaryen#4322 that was never finished, is the remaining upstream opportunity here. Order alone is worth 1–5 KB brotli.

### 6. Snipping dead-but-reachable code

On `wasm32-unknown-unknown`, std's backtrace is `unsupported`: `capture` never produces frames. However, the
`Debug` impl of `anyhow` (pulled in by dioxus-core's `CapturedPanic`) keeps `Backtrace: Display` → `rustc_demangle`
statically reachable: 15.0 KB + 2.1 KB raw. Replacing those 46 bodies with `unreachable` and letting wasm-opt remove
the callees saves **−5.8…−6.3 KB brotli** with zero behaviour change.

- `wasm-snip` 0.4.0 cannot parse externref modules (`found type anyref`), so
  [`hosts/web-shell/tools/wat-snip.py`](../../../hosts/web-shell/tools/wat-snip.py) does it with a
  `wasm-tools print`/`parse` round trip (1.3 s). The same pass does the size sort.
- `--snip-rust-fmt-code` / `--snip-rust-panicking-code` have no effect after immediate-abort: there is nothing left to snip.
- `wasm-metadce` needs a manual reachability graph and would only rediscover what `--remove-unused-module-elements`
  already does.

### 7. Wizer

`build.sh` ran wizer *after* wasm-bindgen:

- Wizer 10 rejects that module (`unsupported ref.null instruction`, reference types unsupported), so the step was dead
  whenever wizer was installed, and skipped otherwise.
- Wizer does run on the raw pre-bindgen module. The result: `.data` grows from 96 KB to 348 KB in 9,503 segments
  (the heap snapshot, including Dioxus arenas and allocator state), and total brotli grows to **282,628 (+61 KB)**.
- Pre-initialization cannot let wasm-opt DCE the init code. The `StudioWorkspaceModule::create` + `render` path that
  `wizer.initialize` exercises is the same path every later frame needs, so only the trivial wrapper disappears.

The step was removed. Removing the source-side `wizer.initialize` export and `PreinitCache` is a source-track item.

### 8. Compression and delivery

| Encoding of the final wasm (688,367 B) | bytes |
|---|---:|
| brotli q11 (precompressed; lgwin 24 and `--large_window` give identical results) | **212,798** |
| brotli q9 / q6 / q5 / q4 (typical CDN on-the-fly levels) | 235,151 / 237,835 / 239,603 / 254,719 |
| zstd -19 / `--ultra -22 --long=27` | 225,644 / ~226 K |
| gzip -9 / -6 | 261,468 / 263,182 |

- Brotli q11 beats zstd's best by about 6 %. Serve the precompressed `.br` (written by `build.sh`) with
  `Content-Type: application/wasm` and `Content-Encoding: br`. `WebAssembly.instantiateStreaming` decodes the
  content-encoding transparently, but it needs the exact `application/wasm` MIME type, which `index.html` already
  relies on through wasm-bindgen's `init`.
- **Compression Dictionary Transport** (RFC 9842 per MDN;
  [Chromium docs](https://chromium.googlesource.com/chromium/src.git/+/main/docs/experiments/compression-dictionary-transport.md),
  [MDN source](https://github.com/mdn/content/blob/main/files/en-us/web/http/guides/compression_dictionary_transport/index.md)):
  serve each build with `Use-As-Dictionary: match="/pkg/*"` and answer `Available-Dictionary` requests with `dcb`
  (brotli) or `dcz` (zstd) deltas.

  | previous build used as dictionary | wasm `dcb` | wasm `dcz` | JS `dcb` |
  |---|---:|---:|---:|
  | flag-only rebuild | 33,755 | 36,191 | — |
  | 1 commit back (`e691874~1`) | **21,664** | 23,131 | 231 |
  | 11 commits back, +10.7 k lines (`21d86ac~1`, before multi-window) | **57,367** | 61,760 | 1,715 |

  Support: Chromium only. Shared brotli/zstd have been there since M117/M118, with the `dcb`/`dcz` encodings since
  M127; secondary sources disagree on when it became default-on. Firefox has an open implementation bug and Safari
  has announced nothing. Both fall back to plain br. Cloudflare's shared-dictionary support is in beta
  ([docs](https://developers.cloudflare.com/speed/optimization/content/shared-dictionaries/)).

### 9. "Change the compiler?"

- **rustc_codegen_cranelift** emits native machine code only. Rust's wasm path is LLVM
  ([wasmtime#12598](https://github.com/bytecodealliance/wasmtime/issues/12598), bjorn3).
  [CogWasm](https://github.com/TeamNodeCraft/CogWasm) (Cranelift IR → wasm) is experimental and would be larger,
  since Cranelift does far less size optimization than LLVM `-Oz`.
- **rustc_codegen_gcc** has no wasm target.
- **LLVM versions**: different nightlies vary by ±0.5 % here. The pin is about reproducibility, not size.
- **Other languages**, for reference only (the languages track measures them): Zig/C without libc, AssemblyScript
  and MoonBit have tiny runtimes. My estimate, not measured here: a DOM-glue module in any of them, or a hand-written
  WAT decoder for our CommandBuffer, would be in the low-KB range. These numbers are small
  because the *program content* is small (no VDOM diff, no generic collections, no formatting), not because the
  compiler is better. On identical program content LLVM `-Oz` is close to hand-written, and the 15 % found here is
  mostly runtime/std overhead plus compressibility.
- Prior art: Dioxus claims about 60 KB brotli for hello-world ([FAQ](https://github.com/DioxusLabs/dioxus/blob/main/notes/FAQ.md)).
  Its [optimizing guide](http://dioxuslabs.com/learn/0.7/guides/tips/optimizing/) reports 310 → 234 KB from nightly
  build-std alone, which is the same lever as step 3 here.

## Ranked change list

| Rank | Change | Saving (brotli) | Effort | Risk / what breaks |
|---:|---|---:|:---:|---|
| 1 | Precompressed brotli q11 at the edge instead of on-the-fly q4–q5 | **27–42 KB** vs typical CDN | S | Needs host config (`.br` sidecars are now emitted) |
| 2 | Compression Dictionary Transport for returning visitors | −73 … −90 % per update | M | Chromium only; other browsers fall back; the server must keep old builds addressable |
| 3 | Nightly `-Zbuild-std` + `-Cpanic=immediate-abort` | **23.1 KB** (measured) | S (done) | Pinned nightly for release builds. Panics trap without a message (already `panic=abort`, so not catchable either); use `--dev` to debug panics |
| 4 | Snip `rustc_demangle` + `std::backtrace` | **5.8 KB** | S (done) | None at runtime (unreachable on this target). Build needs python3 + wasm-tools |
| 5 | `optimize_for_size` std | **4.2 KB** | S (done) | Slower sort/fmt algorithms (not hot here) |
| 6 | Tuned wasm-opt list instead of `-Oz` | 3.2 KB vs none; 5.9 KB vs working `-Oz` | S (done) | None |
| 7 | Minify the JS glue (terser) | **2.6 KB** | S (done) | Needs `npx`; skipped with a notice when absent |
| 8 | `-Zfmt-debug=none` (instead of shallow) | 2.6 KB | S + source | Breaks 3 visible strings until fixed: gesture "Reason" (`examples/studio-workspace/src/state/actions.rs:594`), pairing reject reason (`hosts/web-shell/src/pairing.rs:237`), and every `Display` implemented as `{self:?}` (`SceneError`, `DecodeError`, `BackendError`, `LoaderError`) |
| 9 | Size-sorted function order | 1.1 KB (gzip/zstd also improve) | S (done) | None |
| 10 | `fmt-debug=shallow` + `location-detail=none` | 0.8 KB | S (done) | `{:?}` of structs prints only type names |
| 11 | `strip="debuginfo"` | ~0.1 KB, but fixes the wasm-opt failure and externref glue (JS −6.5 KB raw) | S (done) | None |
| 12 | Remove the wizer step | avoids +61 KB when wizer is installed | S (done) | None (it could not work on bindgen output) |
| — | Rejected: opt-level s, thin LTO, mir-opt-level 4, inline-threshold, VFE, bleeding-edge/mvp CPU, zstd over br, any `-O`/`-Oz`, wasm32v1-none, alternative backends | — | — | — |

## Ready-to-use change set (committed on this branch)

- `Cargo.toml` `[profile.wasm-release]`: `lto = "fat"` (explicit) and `strip = "debuginfo"` (was `"symbols"`).
- `hosts/web-shell/build.sh`, release pipeline:
  1. Pinned nightly `nightly-2026-10-20` (override with `NVE_WASM_TOOLCHAIN`), auto-installed with `rust-src`.
     Build flags: `-Zbuild-std=std,panic_abort -Zbuild-std-features=optimize_for_size` and wasm32-only rustflags
     `-Zunstable-options -Cpanic=immediate-abort -Zfmt-debug=shallow -Zlocation-detail=none`.
  2. wasm-bindgen with names kept.
  3. `tools/wat-snip.py`: snip, then size sort.
  4. The tuned wasm-opt list.
  5. terser.
  6. `.br`/`.gz` sidecars, and a brotli (wasm+JS) budget of 240 KB.

  `--dev` is unchanged: stable, no post-processing. CI only runs `--dev`. The wizer step is gone.
- `hosts/web-shell/tools/wat-snip.py`: snip + order tool, a wasm-tools round trip that is safe for externref.
- `docs/research/wasm-size/toolchain/measure.sh` (harness) and `wat-reorder.py` (ordering experiment).
- `docs/UPSTREAM.md`: new "WebAssembly size toolchain" section.
- Verified: `./hosts/web-shell/build.sh` runs in about 50 s (warm deps), and `node hosts/web-shell/e2e/run_e2e.mjs`
  passes all 6 suites: Studio desktop, mobile resize, Counter, Docs, Workbench, and live `nve pair` thin client.

## Upstream bugs and quirks found

| Component (version) | Symptom | Mitigation |
|---|---|---|
| wasm-ld + rustc `strip="symbols"` (1.98) | `target_features` section removed | `strip="debuginfo"`, strip names after bindgen |
| binaryen 133 | without `target_features` it rejects default Rust 1.87+ output (bulk-memory, sat-trunc); `build.sh` silently shipped unoptimized wasm | keep the section, or pass `--enable-*` |
| binaryen 133 `-Oz`/`-Os`/`-O2+` | raw −7 % but brotli +1.1 % / gzip +1.6 % on LLVM `-Oz` input; reordering, inlining, local-cse and memory-packing cost compressibility ([#4322](https://github.com/WebAssembly/binaryen/issues/4322)) | explicit pass list + size-sorted order |
| binaryen 133 | the `name` section is dropped unless `-g` is passed | run name-based tools before wasm-opt |
| wasm-bindgen 0.2.130 | without `target_features` it falls back to heap-slab JS glue (+6.5 KB raw) | keep the section ([#5382](https://github.com/wasm-bindgen/wasm-bindgen/pull/5382)) |
| wasm-snip 0.4.0 | cannot parse externref (`found type anyref`) | `tools/wat-snip.py` |
| wizer 10.0.0 | `unsupported ref.null` on bindgen output; on raw input +61 KB brotli | removed |
| rustc nightly 2026-09-28 `-Zvirtual-function-elimination` | LLVM ICE with build-std ([rust#164267](https://github.com/rust-lang/rust/issues/164267), fix [#164290](https://github.com/rust-lang/rust/pull/164290)) | not used |
| rustc nightly ≥ 2025-09-24 | `-Zbuild-std-features=panic_immediate_abort` is an error | `-Zunstable-options -Cpanic=immediate-abort` |
| rustup `nightly-2026-10-20` | the toolchain has no prebuilt wasm32 std installed | fine with `-Zbuild-std` (rust-src only) |

## Handoffs (other tracks; not pursued here)

- **Libraries/source**:
  - `serde_json` HotPatchMsg (11 KB fn + 23 KB serde_json) and `subsecond` (12.6 KB) are dev-only hot-reload code
    in the release wasm. Gate them behind a feature, or use the existing binary codec (the zero-JSON rule).
  - `anyhow` (via dioxus-core) keeps backtrace/demangle alive. The snip fixes it in the toolchain, but the root
    cause is anyhow's std backtrace.
  - Replace `dlmalloc` (5 KB) with a smaller allocator.
  - `hashbrown` monomorphizations (24 KB).
  - The `.rodata` demo content and base64 PNG (about 53 KB of strings).
  - Fix the `Display = {:?}` impls to unlock `-Zfmt-debug=none`.
  - Delete `wizer.initialize` / `PreinitCache`.
- **Architecture**:
  - Split studio and counter into separate wasm modules, or lazy-load the studio panels. The studio app alone is
    143 KB raw.
  - Per-DOM-op imports could become a single batched JS decoder of the CommandBuffer, which would make the wasm
    import surface (120 imports, 7 KB) and much of the web-sys glue disappear.
  - `wasm-opt --minify-imports-and-exports` together with a JS-side rename map (−0.2 KB).
- **Languages track**: hand-written WAT or Zig numbers for the CommandBuffer→DOM decoder alone.

## Sources

- rustc `-Cpanic=immediate-abort`: https://github.com/rust-lang/rust/pull/146317, https://github.com/rust-lang/compiler-team/issues/909, https://github.com/leptos-rs/book/pull/279
- wasm32 default features: https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html, https://github.com/rust-lang/rust/issues/141080
- wasm32v1-none: https://doc.rust-lang.org/nightly/rustc/platform-support/wasm32v1-none.html, https://internals.rust-lang.org/t/can-we-have-either-a-new-target-wasm32v1-unknown-unknown-or-a-set-of-new-targets-wasm32-v1-web-bindgen/22041
- wasm-bindgen + strip: https://github.com/wasm-bindgen/wasm-bindgen/pull/5205, https://github.com/wasm-bindgen/wasm-bindgen/pull/5382
- binaryen compressed size: https://github.com/WebAssembly/binaryen/issues/4322, https://github.com/derodero24/comprs/pull/622, https://github.com/stevepryde/sgl/issues/895, https://github.com/opentidehq/language-server/issues/223
- VFE ICE: https://github.com/rust-lang/rust/issues/164267, https://github.com/rust-lang/rust/pull/164290, https://github.com/llvm/llvm-project/issues/240258
- Cranelift / GCC backends: https://github.com/bytecodealliance/wasmtime/issues/12598, https://github.com/TeamNodeCraft/CogWasm, https://users.rust-lang.org/t/testing-cranelift-for-wasm32-unknown-unknown/82977
- Compression Dictionary Transport: https://chromium.googlesource.com/chromium/src.git/+/main/docs/experiments/compression-dictionary-transport.md, https://developers.cloudflare.com/speed/optimization/content/shared-dictionaries/, https://github.com/mdn/content/blob/main/files/en-us/web/http/guides/compression_dictionary_transport/index.md
- Dioxus sizes: https://github.com/DioxusLabs/dioxus/blob/main/notes/FAQ.md, http://dioxuslabs.com/learn/0.7/guides/tips/optimizing/, https://github.com/DioxusLabs/dioxus/issues/732
