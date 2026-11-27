# Upstream Issues (`docs/UPSTREAM.md`)

Known bugs, quirks, and footguns discovered in upstream dependencies or platform toolkits, along with our local mitigations. Whenever an agent or delegate hits a real upstream issue, append a concise entry here with exact version/file/workaround details.

---

### React Native Yoga (`yogacore`)

- **`StyleValuePool` float-to-int overflow UB on large scalars**
  - **Observed**: `StyleValuePool::isIntegerPackable` does `static_cast<int32_t>(f)` before its range check, so any setter value with `|v| >= 2^31` (or `±inf` on number setters such as `flexGrow`) is float-cast-overflow UB (`StyleValuePool.h:182` under UBSan).
  - **Upstream**: fixed by react/yoga#2075 (`3a201ff4`, 2026-11-26), not yet in a release (v3.2.2) nor in React Native's vendored copy.
  - **Mitigation**: `cxx/nve-fabric-bridge/src/command_decoder.hpp` (`is_valid_scalar`) rejects non-finite values and scalars with `|v| > 1e7f` (conservative) before they reach Yoga.
- **Detached `YGNodeRef` returns `NaN` layout metrics**
  - **Observed**: Reading `YGNodeLayoutGet{Left,Top,Width,Height}` on a node that was never laid out or was detached returns `NaN`, which triggers AppKit's `_NSViewValidateGeometry` `SIGTRAP` if forwarded into `NSView.frame`.
  - **Mitigation**: `cxx/nve-fabric-bridge/src/fabric_adapter.cpp` sanitizes non-finite layout outputs to `0.0f`, and `hosts/apple-common/NveAppleComponentHost.mm` guards `frame` geometry.

### Dioxus (`dioxus-core 0.7.10`)

- **Nested `Text` wrapper/leaf split for inline literals**
  - **Observed**: `text { style: "...", "literal" }` emits a styled parent `Text` element wrapping an unstyled child `Text` leaf node, so host shells that don't inherit text styles render invisible or unstyled leaves.
  - **Mitigation**: `packages/core/dioxus/src/lib.rs` and `cxx/nve-fabric-bridge/src/fabric_adapter.cpp` propagate `text_color` and `font_size` from styled parent `Text` nodes down to unstyled child `Text` leaves; native hosts mount wrapper `Text` nodes with children as containers and leaf `Text` nodes as native text labels.
- **`anyhow` `std` feature drags backtrace printing + `rustc-demangle` into wasm**
  - **Observed**: dioxus-core depends on `anyhow` (1.0.105) with default features; `std` makes every error capture a `std::backtrace::Backtrace` whose `Debug`/`Display` printer links `rustc-demangle` (~19 KB raw, −12 KB brotli on the counter wasm), reachable through a vtable so it survives fat LTO and `-Cpanic=immediate-abort`, although wasm32-unknown-unknown never captures frames.
  - **Mitigation**: fork `samuelscheit/dioxus@nve/v0.7.10-wasm-size` (commit `6f7e0d1`, pinned by rev `e617997` in the root `[patch.crates-io]`): the dioxus workspace dep is `anyhow = { default-features = false }` and every member that needs `std` opts back in (Cargo ignores a member's `default-features = false` when the workspace dep enables defaults). no_std anyhow uses `core::error::Error`, so no backtrace or demangler is linked and `build.sh` no longer snips anything. Drop the patch once upstream takes it.
- **`HashSet<ReactiveContext>` uses std `RandomState`** (dioxus-core + dioxus-signals): links SipHash-1-3 and random-key init into wasm although `rustc-hash` is already a dependency. Mitigation: `FxHashSet` in the same fork (`91dd6d5`, −0.6 KB br), with a hasher-generic `SubscriberList` impl so external std `HashSet` subscriber lists keep compiling.
- **`subsecond 0.7.10` has an unconditional wasm32 dependency on wasm-bindgen/js-sys/web-sys/wasm-bindgen-futures** (its browser patch loader). A wasm linked *without* the wasm-bindgen CLI keeps every js-sys `__wbindgen_describe_*` export: counter 299 KB → 521 KB raw (+38 % brotli). Mitigation: the fork (`e617997`) puts the loader behind an opt-in `wasm-patching` feature (enabled by dioxus-devtools); without it `apply_patch` returns an explicit `PatchError`.
- **Symbol names can blame a crate that is not linked**: after fat LTO, identical monomorphisations are merged and keep the first instantiating crate's mangled name. The std built by `-Zbuild-std` makes `<[u8]>::starts_with` show up as `…14rustc_demangle` in the name section although its callers are nve-dioxus/nve-abi and no demangler code ships. Check callers before chasing such a symbol.
- **`subsecond` / `tracing` look heavy in twiggy but cost 0 bytes in release**: `HotFn::try_call` short-circuits on `!cfg!(debug_assertions)` and twiggy names component bodies `<C as subsecond::HotFunction>::call_it`; tracing callsites fold away without a subscriber. No action needed.
- **wasm-split (`0.7.x` / `0.8.0-alpha.1`) crashes under fat LTO + `codegen-units = 1`** (walrus GC assertion `!self.dead.contains(&id)`, open PR DioxusLabs/dioxus#5795), and split routes render twice under SSR hydration (discussion #5287). Mitigation: none adopted; per-panel splitting needs a pinned fixed `wasm-split-cli` or an own split tool.
- **`Runtime::spawn(scope, fut)` skips `scope.spawned_tasks` registration**
  - **Observed**: Calling `Runtime::current().spawn(scope, fut)` directly inserts a `LocalTask` into `Runtime.tasks` and enqueues it onto `dirty_tasks`, but does NOT register the task ID in `scope.spawned_tasks`. If `scope` unmounts before the task is polled, `remove_scope` fails to cancel the task and `VirtualDom::pop_task()` panics on `debug_assert!(self.scopes.contains(tasks.order.id.0))` (`scheduler.rs:155`); in release the future is polled after unmount and leaks until the `VirtualDom` drops. Still on main (`dioxus-vdom`). Reported: DioxusLabs/dioxus#6156 (repro: `docs/research/upstream-repros/dioxus-runtime-spawn-scope`).
  - **Mitigation**: Always spawn onto an ancestor scope via `Runtime::current().in_scope(scope, || dioxus_core::spawn(fut))` (used in `packages/framework/query-dioxus/src/bridge.rs`), which registers the task in `scope.spawned_tasks` so unmount cancels and drops it cleanly before `pop_work` runs.

### rust-analyzer (`0.4.3133`): string-parsed proc-macro output gets out-of-file spans

- **Observed**: `codeLens/resolve` (impls lens) panics `line-index … invalid offset` on every `#[derive(NveAbi)]` type whose expansion is long. rust-analyzer's proc-macro server `TokenStream::from_str` (`crates/proc-macro-api/src/token_stream.rs`, `SpanLike::derive_ranged`) keeps the call-site anchor but sets each token's range to its byte offset inside the parsed string, so ranges point past the derive and often past EOF. rustc gives such tokens `Span::call_site()`.
- **Upstream**: rust-lang/rust-analyzer#22482 (open); still on master `efc6ca7`. Contradicts `impl FromStr for TokenStream` ("All tokens in the parsed stream get `Span::call_site()` spans").
- **Mitigation**: `packages/core/abi-macros/src/lib.rs` (`parse_at_call_site`) re-spans every parsed token to `Span::call_site()`. Any new proc-macro that emits via `str::parse::<TokenStream>()` must do the same.

### Rust `std` on `wasm32-unknown-unknown`

- **`std::time::Instant::now()` panics at runtime**
  - **Observed**: Calling `std::time::Instant::now()` inside `DioxusSurface::render_frame` causes an immediate runtime panic when booted in browser WASM (`hosts/web-shell`).
  - **Mitigation**: Target-gated `FrameTimer` in `packages/core/dioxus/src/lib.rs` (real `Instant` on native, no-op on `wasm32`).

### Xcode 27 / CoreSimulator / Swift 6.4 (`swiftlang-6.4.0.34.1`)

- **`Simulator.app` replaced by `DeviceHub.app`**
  - **Observed**: In Xcode 27, `/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app` is absent (`open -a Simulator` fails with `Unable to find application named 'Simulator'`); the Simulator GUI lives at `/Applications/Xcode.app/Contents/Applications/DeviceHub.app`.
  - **Mitigation**: `hosts/ios-shell/run-simulator.sh` checks `$DEV_DIR/Applications/Simulator.app` first, falls back to `$DEV_DIR/../Applications/DeviceHub.app`, and keeps GUI launch non-fatal so `xcrun simctl install` and `launch` always succeed.
- **`swift-frontend -O` leaks ~32 GB RAM compiling `Collection` with compile-time `2^32` `endIndex`**
  - **Observed**: Under Apple Swift 6.4 (`swiftlang-6.4.0.34.1`, macOS 27.2.0), compiling a zero-sized `Collection`/`RandomAccessCollection` whose `endIndex` is a computed constant `Int(UInt32.max) + 1` under `-O` or `-Osize` (with or without `-wmo`, even if the type is unreferenced) causes `swift-frontend` to spin at 100% CPU and consume ~32 GB RSS (8 B per element; scales linearly). Cause: SIL `ObjectOutliner.getInitialization` eagerly allocates one slot per tail element of the `alloc_ref [tail_elems $UInt8 * 4294967296]` in the specialized `_copyCollectionToContiguousArray`. Reported: swiftlang/swift#94725 (repro: `docs/research/upstream-repros/swift-collection-endindex-oom`).
  - **Mitigation**: Keep `endIndex` as a stored `let endIndex: Int` property (`packages/core/host-swift/Tests/NveHostTests/BinCodecTests.swift`, `HugeBytes(endIndex: Int(UInt32.max) + 1)`), which drops peak `swift-frontend` RSS from ~32 GB to ~155 MB and compile time to ~1.2 s.

### GTK 4 (`gtk4 4.24.2`)

- **`gtk_fixed_get_child_position` returns the last *allocated* offset, not the put/move position**
  - **Observed**: In GTK 4.24.2 (unchanged on main), `gtk_fixed_get_child_position` uses `gtk_widget_compute_point`, i.e. the child's allocation: it returns `(0, 0)` until the first size-allocate (e.g. headless CTest fixtures) and the old position after `gtk_fixed_move` until the next allocation. Mapping/rooting is irrelevant. The docs promise the put/move translation. Reported: GNOME/gtk#8579 (repro: `docs/research/upstream-repros/gtk-fixed-child-position-unmapped`).
  - **Mitigation**: Read child translation from `gtk_fixed_get_child_transform` + `gsk_transform_to_translate` (see `hosts/gtk-shell/tests/gtk_shell_test.cpp`).

### Eclipse OpenJ9 (`openj9 0.59.0` / OpenJDK 25.0.3) + Android SDK `d8` (`build-tools 36.0.0`)

- **GC stack-slot assertion (`ScavengerRootScanner.hpp:105`) during `d8` dexing**
  - **Observed**: When Eclipse OpenJ9 0.59.0 is the default `java` on `PATH` on macOS arm64, Android `build-tools/36.0.0/d8` crashes inside `ForkJoinPool` (`com/android/tools/r8/internal/n30.<init>(I)V`) with `Misaligned object` / `j9mm.479 ** ASSERTION FAILED ** at .../openj9/runtime/gc_glue_java/ScavengerRootScanner.hpp:105` and hangs for ~70s writing multi-GB core/javacore/snap dumps.
  - **Cause**: `j9localmap_LocalBitsForPC` (`runtime/stackmap/localmap.c`) always marks slot 0 of every `<init>` as an object, even after R8's `n30.<init>(I)V` legally `istore_0`s an int there; any GC in that frame scans the int. Every GC policy and `-Xint` fail; 0.59.0–0.63.0 and master affected. Reported: eclipse-openj9/openj9#25271 (repro: `docs/research/upstream-repros/openj9-d8-scavenger-assert`).
  - **Mitigation**: `hosts/android-shell/run-emulator.sh` detects OpenJ9 and switches `JAVA_HOME`/`PATH` to an installed HotSpot/JBR JDK (e.g., Android Studio JBR or Adoptium Temurin) before invoking `javac`/`d8`/`apksigner`, and fails with a clear error when none is installed (no OpenJ9 option avoids the crash).

### iOS 27 Simulator SDK (`iPhoneSimulator27.0.sdk`)

- **Fatal `SIGTRAP` at launch without `UIScene` lifecycle adoption**
  - **Observed**: Apps linked against `iPhoneSimulator27.0.sdk` that still create a `UIWindow` in `UIApplicationDelegate` without `UIApplicationSceneManifest` / `UIWindowSceneDelegate` trap immediately on launch inside `UIKitCore`'s `___UIApplicationEvaluateRuntimeIssueForNoSceneLifecycleAdoption_block_invoke` (`EXC_BREAKPOINT` / `SIGTRAP`).
  - **Mitigation**: `hosts/ios-shell/Info.plist` declares `UIApplicationSceneManifest` (`NveIosSceneDelegate`) and `hosts/ios-shell/main.mm` creates the `UIWindow` via `initWithWindowScene:` inside `UIWindowSceneDelegate`'s `scene:willConnectToSession:options:`.

### Windows 11 (`afd.sys`) + MSVC `19.51` + Cargo `1.100.0`

- **`afd.sys` `SO_RCVTIMEO` UDP datagram drops & `WSAECONNRESET` bounces**
  - **Observed**: On Windows 11 (`10.0.26200`), calling blocking `recv_from` on a `UdpSocket` with a short `SO_RCVTIMEO` (e.g., 5 ms) cancels pending `IOCTL_AFD_RECV_DATAGRAM` IRPs on each 15.625 ms timer tick, silently dropping incoming loopback datagrams (~75% test flake). Unreachable UDP replies also surface `WSAECONNRESET` (`ErrorKind::ConnectionReset`) on subsequent `recv_from` calls.
  - **Mitigation**: `packages/core/interop/src/sync_transport/udp.rs` uses `set_nonblocking(true)` with a short sleep on `WouldBlock` and ignores `ErrorKind::ConnectionReset` on `recv_from`.
- **Cargo `1.100.0` per-unit `build-dir` layout + Windows 32 KiB `CreateProcessW` limit (`OS error 206`)**
  - **Observed**: Cargo `1.100.0` places `.rlib`s under `target/<profile>/build/<pkg>/<hash>/out/` instead of flat `target/<profile>/deps/`. Passing every `-L dependency=...` directory on the `rustc` command line exceeds Windows' 32,767-char `CreateProcessW` limit (`ERROR_FILENAME_EXCED_RANGE`).
  - **Mitigation**: `packages/core/codegen/tests/hybrid_codegen_tests.rs` scans both layouts and passes dependency flags via a `rustc @response_file`.
- **MSVC `link.exe` `LNK2005` duplicate `nve_get_interface` under workspace feature unification**
  - **Observed**: In `cargo test --workspace`, Cargo unifies `default = ["cdylib-export"]` across `examples/counter` and `examples/studio-workspace`. When `nve-web-shell` links both `rlib`s, MSVC `link.exe` rejects duplicate `nve_get_interface` definitions if the export shares the root-module CGU with normal library functions.
  - **Mitigation**: Wrap `export_nve_app!` inside a private `mod cdylib_export` so `rustc` places `nve_get_interface` in its own CGU (`cdylib` DLLs still export it cleanly while multi-app `rlib` test binaries avoid pulling duplicate symbols).
- **MSVC `mspdbsrv.exe` `C1090` RPC failure on concurrent Ninja `/Zi /FS` builds**
  - **Observed**: Running two independent Ninja MSVC builds concurrently in the same session can fail with `C1090` (`0x0000071A` / `0x000006BA`) when one build exits while another writes PDBs.
  - **Mitigation**: Serialize MSVC Ninja builds or use `/Z7` / `Release` without shared PDBs.
- **Windows `core.autocrlf = true` vs LF-only generated ABI drift checks**
  - **Observed**: Checking out generated `.h`/`.dart`/`.java`/`.ts` ABI bindings with CRLF on Windows trips byte-for-byte drift checks against LF generator output.
  - **Mitigation**: `packages/core/codegen/tests/{codegen_tests,hybrid_codegen_tests}.rs` and `packages/tools/cli/src/codegen.rs` normalize `\r\n` to `\n` before comparison.

### WebAssembly size toolchain (rustc nightly 2026-09-28/10-20, wasm-bindgen 0.2.130, binaryen 133, wizer 10, wasm-snip 0.4)

- **`strip = "symbols"` drops `target_features`; wasm-opt then rejects the module and build scripts silently ship unoptimized wasm**
  - **Observed**: wasm-ld `--strip-all` removes the `target_features` custom section. Since Rust 1.87 (LLVM 20) wasm32-unknown-unknown emits bulk-memory and nontrapping-fptoint by default, so `wasm-opt -Oz` fails validation (`memory.copy operations require bulk memory`, rust-lang/rust#141080); our old `build.sh` caught the failure and shipped the unoptimized file. wasm-bindgen 0.2.130 additionally warns and falls back to the non-externref JS glue (+6.5 KB raw).
  - **Mitigation**: `[profile.wasm-release] strip = "debuginfo"`; names are removed by `wasm-opt --strip-debug` after wasm-bindgen (wasm-bindgen#5205/#5382 track detection without the section).
- **binaryen `-Oz`/`-Os`/`-O2+` make brotli/gzip output larger on LLVM `-Oz` input**
  - **Observed**: raw −7 % but brotli +1.1 %, gzip +1.6 %. Culprits measured pass-by-pass: inlining-optimizing, local-cse, memory-packing, coalesce/simplify-locals, const-hoisting, and function reordering (random function order costs +4 KB brotli / +22 KB gzip). binaryen#4322 is open since 2021.
  - **Mitigation**: `hosts/web-shell/build.sh` runs an explicit compression-neutral pass list and sorts functions by size before it.
- **binaryen drops the `name` section unless `-g` is passed** — later name-based tools (function sorting, twiggy) see `$N` indices only. Run name-based rewrites before wasm-opt.
- **wasm-snip 0.4.0 cannot parse externref modules** (`failed to parse code section: found type anyref, expected i32`; its walrus is pre-reference-types). No snipping is needed any more; the remaining name-based rewrite (size-sorted function order) is `hosts/web-shell/tools/wasm-sort-funcs.py` (wasm-tools print/parse round trip).
- **Wizer 10 cannot pre-initialize wasm-bindgen output** (`unsupported ref.null instruction`, reference types unsupported). On the raw pre-bindgen module it runs, but the heap snapshot grows the data section 96 KB → 348 KB (+61 KB brotli) and cannot remove the render code (still needed at runtime). Mitigation: Wizer step removed from `build.sh`.
- **`-Zvirtual-function-elimination` ICEs with `-Zbuild-std`** (`llvm.type.checked.load intrinsic must be lowered by the LowerTypeTests pass`, rust-lang/rust#164267, fix PR #164290). Passing it only to the final crate builds but saves just 0.3 KB brotli; not used.
- **`panic_immediate_abort` build-std feature is gone** (nightly ≥ 2025-09-24): use `-Zunstable-options -Cpanic=immediate-abort` with `-Zbuild-std` (rust-lang/rust#146317). A Cargo profile `panic = "immediate-abort"` additionally needs `-Zpanic-immediate-abort` (cargo 1.101.0-nightly 2026-10-12); `build.sh` passes the rustflag instead so stable Cargo still parses the workspace.
- **rustc ≥ 1.96 no longer passes `--allow-undefined` on wasm32-unknown-unknown**: a raw `unsafe extern "C" { fn f(..); }` import fails with `rust-lld: undefined symbol: f`. Mitigation: `#[link(wasm_import_module = "env")]` on import blocks (rust-lang/rust#149868).
- **wasm-ld keeps padded 5-byte LEB128 relocations in the final binary**: wasm-bindgen (walrus) and wasm-opt re-encode silently, so a pipeline without them is 7.4 % larger raw (+1.25 KB brotli on the counter). Mitigation: `-C link-arg=--compress-relocations` (no debuginfo) or any binaryen round trip.
- **`core::fmt` float printing and `str::parse::<f32>` cost ~30 KB of wasm** (Grisu + Dragon4, Eisel-Lemire + 10 KB pow5 table), pulled in by the style string → f32 → CSS string round trip. Mitigation: never `{}`/`{:.N}` floats or `.parse::<f32>()` in wasm-linked code; a fixed-point codec (prototype `nve_abi::num`, −12.7 KB br) or moving CSS formatting to JS removes both.
- **wasm-bindgen 0.2.130 leaves closure descriptor `Debug` strings in `.rodata`** of the final module (~2 KB, e.g. `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("MouseEvent")] … })`). Disappears with a raw-export boundary.
- **twiggy 0.7 `dominators`/`paths` stop at `table[0]`** for every vtable / fn-pointer reached function, hiding why fmt impls and closures are retained. Mitigation: `docs/research/wasm-size/runtime/why.py` follows `i32.const <slot>` and rodata vtable edges.

### Browsers (Chrome 13x): synchronous events inside DOM mutations

- **Removing a focused `<input>` fires `blur` synchronously, mid-mutation** (spec violation since whatwg/html#8392; Gecko/WebKit comply; tracked as crbug 41484175, fix behind the not-yet-default `OmitBlurEventOnElementRemoval` flag)
  - **Observed**: a host that dispatches DOM events into Rust immediately gets re-entered while a frame is still being applied; with `RefCell` this is a `BorrowMutError` panic (`unreachable` under immediate-abort).
  - **Mitigation**: `hosts/web-shell` uses `try_borrow_mut` so the event waits for the next pump; a JS-side CommandBuffer interpreter must queue events raised during `apply` and flush them afterwards (prototype `nve_dom.js`, branch `t3/reduce-wasm-js-boundary`).

### Taffy (`taffy 0.16.0`)

- **`compute_leaf_layout` drops `style.aspect_ratio` under `SizingMode::ContentSize`**
  - **Observed**: When Flexbox measures a leaf item's hypothetical cross size (`compute/flexbox.rs` passes `SizingMode::ContentSize` with `known_dimensions` populated from `flex-basis`), `compute/leaf.rs` resets `aspect_ratio` to `None`, so an empty leaf with `flex-basis` + `aspect-ratio` (and `width`/`height: auto`) collapses to `0` on the cross axis.
  - **Upstream**: DioxusLabs/taffy#804 (open; still on main `497de49`; candidate fixes #1081, #1213, #1215). Also hits `flex-grow` + `aspect-ratio`.
  - **Mitigation**: none. The former measure-closure workaround (`taffy_engine.rs`) was lost in the DOD NodeStore rewrite (`cc116031`); `taffy_store/style_view.rs` only forwards `aspect_ratio`.
- **`compute_root_layout` re-runs the root positioning pass for unclaimed `position: fixed` boxes on a full cache hit**
  - **Observed**: `compute/mod.rs` clones the cached root output's `OofCandidates` (`Box<Vec>`) and re-lays out every unclaimed fixed box into fresh `hoisted`/`unclaimed` `Vec`s on every call, even when nothing changed (16 heap allocations/frame for 100 fixed toasts; scales with fixed-box count).
  - **Mitigation**: `packages/backends/headless/src/taffy_store/mod.rs` tracks `layout_dirty` (set by every cache invalidation, viewport/root change, measurer swap, reset) and skips `compute_root_layout` + the rounding walk on clean frames; guarded by `tests/taffy_hoisted_zero_alloc.rs`. Dirty passes still pay Taffy's per-pass scratch (~241 allocations/frame on the 1k-node `taffy_relayout_1k_hoisted` fixture).
