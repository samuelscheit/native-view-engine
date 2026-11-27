# `nve-codegen` (`packages/core/codegen`)

Binary, compile-time-verified multi-target code generator for `native-view-engine` (NVE).

## Compiler Architecture (`src/`)

`packages/core/codegen/src/` is structured as a three-stage compiler pipeline: **Schemas (`schema/`)** + **Frontends (`frontend/`)** -> **Target Emitters (`targets/<lang>/`)**, backed by shared crate-root utilities (`lib.rs`, `error.rs`, `naming.rs`).

| Layer / Module | Purpose |
| :--- | :--- |
| `lib.rs`, `error.rs`, `naming.rs` | Crate root public re-exports (`nve_codegen::*`), unified `import_rn_*` / `export_*` entrypoints, structured `CodegenError`, and cross-language identifier/casing helpers. |
| `schema/` | All IRs, `#[repr(C)]` layout computation, semantic validation, deterministic XXH64 schema hashing, and colocated `AbiFileDescriptor` bridges: |
| `schema/export.rs` | Shared export & RN IR (`GeneratedFile`, `NveExportSpec`, `ParsedRnSpec`, `RnComponentSpec`, `TurboModuleSpec`, `ExportTarget`, output bundles). |
| `schema/hybrid/` | Hybrid FFI IR (`mod.rs`, `file_spec.rs`, `validate.rs`), colocated `#[derive(NveAbi)]` descriptor bridge (`schema_bridge.rs`), and Core/Scene ABI specs & hashes (`core_abi.rs`). |
| `frontend/` | All input specification lexers and recursive-descent parsers: |
| `frontend/rn_spec/` | React Native Fabric `*NativeComponent.{ts,js}` & TurboModule `Native*.{ts,js}` lexer and parser (`mod.rs`, `lexer.rs`, `syntax.rs`, `calls.rs`, `lowering.rs`). |
| `frontend/hybrid_rust/` | Pure-Rust declarative Hybrid FFI `.rs` lexer and parser (`mod.rs`, `lexer.rs`, `items.rs`, `traits.rs`). |
| `targets/` | One cohesive directory per output language, plus unified multi-target dispatchers in `targets/mod.rs` (`generate_all_abi_files`, `generate_hybrid_target`/`generate_hybrid_all`, `generate_import_rn`, `generate_export_rn`): |
| `targets/c_cpp/` | C11 / C++20 / ObjC++ emitters: Core & Scene ABI (`core_abi.rs`), RN import adapter (`import_rn.rs`), RN Fabric C++/iOS/Windows exports (`export_rn_cpp.rs`, `export_rn_ios.rs`, `export_rn_windows.rs`), and Hybrid FFI (`hybrid/`). |
| `targets/rust/` | Rust emitters: RN import `nve-dioxus` bindings (`import_rn/`), and Hybrid FFI types/vtables/codecs (`hybrid/`). |
| `targets/swift/` | Swift emitters: SwiftUI `View` export (`swiftui.rs`), and Hybrid FFI C-interop (`hybrid/`). |
| `targets/kotlin_java/` | Kotlin & Java emitters: Java Core & Scene ABI (`java_core_abi.rs`), Jetpack Compose export (`compose.rs`), Android RN Fabric `ViewManager` export (`export_rn_android.rs`), and Kotlin/JNI Hybrid FFI (`kotlin_hybrid/`). |
| `targets/dart/` | Dart / Flutter emitters: `dart:ffi` Core & Scene ABI (`core_abi.rs`), Flutter `StatefulWidget` export (`flutter.rs`), and Hybrid FFI (`hybrid/`). |
| `targets/typescript/` | TypeScript / Web emitters: Core & Scene ABI (`core_abi.rs`), Web Custom Element export (`web_component.rs`), RN Fabric TS spec export (`export_rn_ts.rs`), and Hybrid FFI WASM client (`hybrid/`). |

## Generated-File Naming Invariant

Every generated file emitted by `nve-codegen` MUST obey one of two strict naming conventions:

- **Non-Java targets**: filename MUST include `.generated.` — `*.generated.{h,hpp,dart,ts,rs,swift,kt,cpp}` (e.g. `greeter_hybrid.generated.rs`, `GreeterHybridSpec.generated.hpp`, `nve_abi.generated.h`).
- **Java targets**: MUST live in a `generated/` package (`package dev.nve.android.generated;`, e.g. `generated/NveCoreAbi.java`, `generated/NveSceneBuffer.java`).

Never add un-suffixed shim headers or symlinks alongside generated artifacts.

All generated Core ABI, Scene ABI, and XXH64 mirror files (`*.generated.*` and `hosts/android-shell/android-app/src/dev/nve/android/generated/`) are **gitignored** and materialized automatically by host build hooks (CMake `cxx/cmake/NveCodegen.cmake`, Gradle `generateNveAbi`, `hosts/web-shell/build.sh`, `scripts/check-all.sh`, and CI) or on demand via `cargo run --locked -p nve-cli -- codegen abi`. Rust tests (`cargo test --workspace`) run cleanly on a fresh checkout with zero generated files on disk.

## Defining Schemas (Colocated `#[derive(NveAbi)]` in Owning Packages)

Centralized `.nve.rs` spec files (`packages/core/codegen/specs/`) have been **eliminated**. Every owning crate derives `#[derive(nve_abi::NveAbi)]` (implemented with zero external dependencies in `packages/core/abi-macros`) directly on its real compiled `#[repr(C)]` structs, unions, and `#[repr(u8|u16|u32|i32)]` enums:

- **Compile-time single source of truth (`packages/core/abi-macros` + `packages/core/abi/src/schema.rs`)**:
  - `#[derive(NveAbi)]` emits `impl NveStructSchema` / `NveUnionSchema` / `NveEnumSchema` + `NveSchema` with `const SCHEMA` carrying `rustc`-computed `core::mem::size_of::<Self>()`, `core::mem::align_of::<Self>()`, `core::mem::offset_of!(Self, field)`, and enum discriminants.
  - Optional layout contracts (`#[nve_abi(size = N, size_64 = N, align = N)]` and field `#[nve_abi(offset = N, offset64 = N, c_type = "...")]`) emit compile-time `const` assertions automatically.
  - `nve-codegen` converts colocated `nve_abi::schema::AbiFileDescriptor` values (`core_abi_descriptor()`, `scene_abi_descriptor()`) directly into `HybridFileSpec` via `from_abi_descriptor` — zero duplicated spec files and zero `wasm32` runtime bloat (dead-code-eliminated `const` metadata).
- **Direct in-process FFI (static `extern "C"` by default; optional vtables for `dlopen`/`cdylib`)**: derive `#[derive(NveAbi)]` on real `#[repr(C)]` types or construct a `nve_codegen::HybridFileSpec` (`HybridStructSpec`, `HybridEnumSpec`, `c_functions`, and optionally `HybridModuleSpec`/`HybridViewSpec`) directly in Rust and call `generate_hybrid_target(&spec, HybridTarget::Cpp)` (or `generate_hybrid_all`).
  - **Default for statically linked builds**: emit **direct static `extern "C"` C functions** via `HybridFileSpec::c_functions` (zero vtable, zero indirect call overhead, LTO-inlinable — e.g. `nve_clipboard_write_text(NveStrSlice text) -> int32_t`, `nve_system_env_get_theme(NveHybridNveThemeSnapshot* out_theme) -> int32_t`).
  - **Strictly optional for dynamic plugins**: emit `#[repr(C)]` function-pointer vtable structs (`HybridModuleSpec`) **only** for `dlopen` / `LoadLibrary` `cdylib` hot-reload boundaries where symbols cannot be statically linked.

## Verification & ABI Drift Check

```bash
# Run all unit, validation, adversarial security, and real-compiler execution tests
cargo test --locked -p nve-codegen

# Enforce zero Clippy warnings across library and tests
cargo clippy --locked -p nve-codegen --all-targets -- -D warnings

# Generate all 16 Core ABI (5) + Scene ABI (5) + XXH64 mirror (6) artifacts on disk and verify 16/16 parity
cargo run --locked -p nve-cli -- codegen abi && cargo run --locked -p nve-cli -- codegen abi --check
```
