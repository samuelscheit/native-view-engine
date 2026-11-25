# `nve-codegen` (`packages/core/codegen`)

Zero-JSON, compile-time-verified multi-target code generator for `native-view-engine` (NVE).

## Architecture & Submodules (`src/`)

| Module | Purpose |
| :--- | :--- |
| `reloc/` | Zero-copy aligned `RelocSlice<T>` / `RelocStr` & 3-tier `VerifyMode::{Full, HeaderOnly, Unchecked}` code generator (`ir.rs`, `validate.rs`, `emit_rust.rs`, `rust_runtime.rs`, `emit_cpp.rs`, `cpp_runtime.rs`). Emits standalone Rust 2024 modules and C++20 headers with 32-byte `RelocHeader`, deterministic FNV-1a64 `schema_hash`, zero-padding POD `as_bytes()`, and compile-time `size_of`/`offset_of` checks. |
| `hybrid/` | Nitro-inspired typed Hybrid FFI generator (`ir/`, `parser/`, `core_abi/`, `emit_{rust,cpp,swift,kotlin,dart,ts}/`). Emits direct static `extern "C"` functions, `#[repr(C)]` POD structs/enums, and optional dynamic `dlopen`/`cdylib` vtables across C11/C++20/Swift/Kotlin/Dart/TypeScript/Rust, plus NVE's checked-in Core ABI (`nve_abi.generated.*`) and 64-byte Scene ABI (`nve_scene_abi.generated.*`). |
| `wire_codecs/` | Single-source emitter (`spec.rs`, `emit_{cpp,dart,java,swift,ts}.rs`) for shared binary wire helpers (`FNV-1a64`, `EventRing`, `NK1` keydown, `NVE_DRAW1`) across C++20, Dart, Java, Swift, and TypeScript. |
| `import_rn/` | Parses React Native Fabric `*NativeComponent.{ts,js}` and TurboModule `Native*.{ts,js}` specs (via `lexer.rs` + `parser/`) and emits zero-JS `nve-dioxus` Rust components + C++20 Fabric adapters. |
| `export_rn/` | Exports an `NveExportSpec` to a drop-in React Native Fabric NativeComponent package (TS, C++20, Obj-C++, Kotlin, Windows). |
| `export_flutter.rs` | Exports an `NveExportSpec` to a typed Flutter `StatefulWidget` + `dart:ffi` binding. |
| `export_swiftui.rs` | Exports an `NveExportSpec` to a typed SwiftUI `View` wrapping `NveSwiftUI`. |
| `export_compose.rs` | Exports an `NveExportSpec` to a typed Jetpack Compose `@Composable` wrapping `NveSurface`. |
| `export_web.rs` | Exports an `NveExportSpec` to a typed Web Custom Element `<nve-...>` `.js` + `.d.ts`. |
| `ir.rs`, `naming.rs`, `error.rs` | Shared export IR (`GeneratedFile`, `NveExportSpec`), cross-language identifier/casing helpers, and structured `CodegenError`. |

## Generated-File Naming Invariant

Every generated file emitted by `nve-codegen` MUST obey one of two strict naming conventions:

- **Non-Java targets**: filename MUST include `.generated.` — `*.generated.{h,hpp,dart,ts,rs,swift,kt,cpp}` (e.g. `asset_pack_reloc.generated.rs`, `AssetPackReloc.generated.hpp`, `nve_abi.generated.h`).
- **Java targets**: MUST live in a `generated/` package (`package dev.nve.android.generated;`, e.g. `generated/NveCoreAbi.java`, `generated/NveSceneBuffer.java`).

Never add un-suffixed shim headers or symlinks alongside generated artifacts.

## Defining Schemas (Colocated in Owning Packages)

Schemas MUST NOT be centralized in `packages/core/codegen/specs/` — each owning crate/module owns its own schema directly in Rust using typed IR structs (`RelocSchema` or `HybridFileSpec`), ready for `trait NveSchema` / `#[derive(NveAbi)]` integration without hand-parsed `.nve.rs` files:

- **Zero-copy binary slabs (`mmap`, disk caches, network/IPC frames)**: construct a `nve_codegen::RelocSchema` (`RelocStruct`, `RelocField`, `RelocEnum`, `RelocType::{RelocSlice, RelocStr, ...}`) in the owning crate and call `generate_reloc_rust(&schema)` / `generate_reloc_cpp(&schema)` (or `generate_reloc_all(&schema)`).
  - Use `VerifyMode::Full` for untrusted network sockets and external files (verifies alignment, magic, FNV-1a64 `schema_hash`, bounds, `u32` overflow, UTF-8, `bool`, enum discriminants, and recursion depth before creating any reference).
  - Use `VerifyMode::HeaderOnly` for trusted local `mmap` caches and compiled-in static assets (O(1) header + root check).
  - Use `unsafe { view_unchecked(buf) }` for 0-check raw pointer casts when invariants are already proven.
- **Direct in-process FFI (static `extern "C"` by default; optional vtables for `dlopen`/`cdylib`)**: construct a `nve_codegen::HybridFileSpec` (`HybridStructSpec`, `HybridEnumSpec`, `c_functions`, and optionally `HybridModuleSpec`/`HybridViewSpec`) directly in Rust and call `generate_hybrid_target(&spec, HybridTarget::Cpp)` (or `generate_hybrid_all`).
  - **Default for statically linked builds**: emit **direct static `extern "C"` C functions** via `HybridFileSpec::c_functions` (zero vtable, zero indirect call overhead, LTO-inlinable — e.g. `nve_clipboard_write_text(NveStrSlice text) -> int32_t`, `nve_system_env_get_theme(NveHybridNveThemeSnapshot* out_theme) -> int32_t`).
  - **Strictly optional for dynamic plugins**: emit `#[repr(C)]` function-pointer vtable structs (`HybridModuleSpec`) **only** for `dlopen` / `LoadLibrary` `cdylib` hot-reload boundaries where symbols cannot be statically linked.

*(Note: `specs/nve_core_abi.nve.rs` and `specs/nve_scene_abi.nve.rs` remain only for the existing 10 Core/Scene ABI artifacts until migrated; do not add new `.nve.rs` files under `specs/`.)*

## Verification & ABI Drift Check

```bash
# Run all unit, validation, adversarial security, and real-compiler execution tests
cargo test --locked -p nve-codegen

# Enforce zero Clippy warnings across library and tests
cargo clippy --locked -p nve-codegen --all-targets -- -D warnings

# Verify all 15 checked-in Core ABI (5) + Scene ABI (5) + Wire Codec (5) artifacts have zero drift
cargo run --locked -p nve-cli -- codegen abi --check
```
