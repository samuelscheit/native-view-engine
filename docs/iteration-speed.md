# Iteration Speed & Tooling Playbook

Iteration speed is a hard architectural invariant of `native-view-engine`, not an afterthought. Every workflow has an explicit latency budget.

## 4-Tier Iteration Loop

| Tier | Trigger | Target Budget | Mechanism |
| :--- | :--- | :--- | :--- |
| **Tier 0: Headless Verification** | `cargo test` / `cargo nve inspect` | **< 300 ms** | Pure-Rust `nve-backend-headless` (`taffy` Flexbox + mutation log) + standalone C++20 `nve-fabric-bridge` tests via Ninja + `kache`/`ccache`. Zero Simulator/Xcode/Gradle. |
| **Tier 1: RSX / Style Hot Patch** | Edit text, style, or layout props while `nve dev` runs | **< 50 ms** | WebSocket `HotPatchMsg::TemplatePatch` applied in-place to the running `nve-engine` / `nve-dioxus` instance. Works on Desktop, Simulators, Emulators, and **physical iOS/Android devices**. |
| **Tier 2: Rust Logic Hot Swap** | Edit Rust component logic / state / handlers while `nve dev` runs | **150–400 ms** | Native Host Shell (iOS Simulator / Android Emulator / macOS Skia) is compiled **once**. `nve dev` rebuilds only the thin app `cdylib` (`examples/<app>`) with `kache` + `split-debuginfo=unpacked`, stamps a versioned `.dylib`/`.so`, and pushes `HotPatchMsg::DylibReload`. Host snapshots app state, `dlopen`s the new generation via `nve_get_interface` (`NveSurfaceApi` + `NveDevApi`), and restores state. (Compatible with `subsecond` in-place jump-table patching.) |
| **Tier 3: C++ Bridge / Native Views** | Edit `cxx/nve-fabric-bridge` | **1–3 s** | Heavy RN dependencies (`ReactCommon`, `Yoga`, `Folly`) are prebuilt once per RN version and cached; Ninja + `kache`/`ccache` compiles only our ~1–2k LOC bridge. |

## Local Acceleration Configuration

1. **Compilation Caching:** Inherits global `rustc-wrapper = "kache"` and `HOST_CC/HOST_CXX = "kache cc/c++"`, plus `ccache` for CMake/Ninja.
2. **Fast macOS Linking:** `.cargo/config.toml` sets `-C split-debuginfo=unpacked` to skip `dsymutil` on every dev rebuild.
3. **Workspace Dependency Profiles:** `Cargo.toml` compiles heavy dependencies (e.g. `taffy`, `serde`) with `opt-level = 2` while keeping workspace crates at `opt-level = 0` so runtime tests and benchmarks stay fast without slowing down incremental workspace compiles.
4. **Stage-by-Stage Telemetry:** Every committed frame records `FrameTiming` (`reconcile_us`, `encode_us`, `apply_us`, `layout_us`, `diff_us`, `mount_us`) and exports Chrome/Perfetto JSON traces (`cargo nve bench --trace bench.trace.json`).
