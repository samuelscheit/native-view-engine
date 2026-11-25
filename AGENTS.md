# AGENTS.md — `native-view-engine`

High-signal invariants, engineering focus areas, and workflow rules for agents working in this repository.

## Documentation Map (`docs/`)

- [docs/architecture.md](docs/architecture.md) — Multi-backend architecture, binary IR/ABI model, and crate/host layout.
- [docs/BACKLOG.md](docs/BACKLOG.md) — Open prioritized roadmap (keep open-only).
- [docs/CHANGELOG.md](docs/CHANGELOG.md) — Complete reverse-chronological log of shipped milestones.
- [docs/UPSTREAM.md](docs/UPSTREAM.md) — Known upstream bugs, toolchain/OS quirks, and mitigations.
- [docs/iteration-speed.md](docs/iteration-speed.md) — 4-tier hot-reload budgets and canonical per-host build/test/benchmark commands.
- [scripts/check-all.sh](scripts/check-all.sh) — Unified cross-platform verification script (`--quick` or full).

## Non-Negotiable Invariants

- **Single binary IR & modular C ABI**: Every frame crosses the Rust/host boundary as one flat batched `CommandBuffer`, `EventRing`, or zero-decode `NveSceneView`. Expose modules solely via `nve_get_interface(1..=5)` (`NveApp` + `export_nve_app!`). Keep `crates/nve-engine` framework-agnostic (zero dependency on `dioxus-core`).
- **Generated-file naming**: Every generated non-Java file MUST include `.generated.` (`*.generated.{h,dart,ts,rs,swift,kt,cpp}`). Every generated Java file MUST live in a `generated/` package (`package dev.nve.android.generated;`). Keep `cargo run --locked -p nve-cli -- codegen abi --check` green; never add un-suffixed shim headers or symlinks.
- **Keep build/trace artifacts out of git**: Never commit `*.trace.json`, `*.perfetto.json`, `cxx/build*`, `hosts/*/build`, `hosts/swiftui-kit/.build`, `hosts/flutter-plugin/.dart_tool`, or compiled binaries.

## How We Work & Core Focus Areas

- **Zero-overhead, zero-JSON boundaries**: For every boundary (FFI, events, interop, host bridges, sync/CRDT), research proven designs and choose the zero-cost binary / direct-FFI / zero-copy path even when harder. Never ship JSON serialization, wrapper layers, or extra per-frame copies; lock `#[repr(C)]` layouts with compile-time `offset_of!` and C11 `_Static_assert(offsetof(...))`.
- **Strict DRY & modular code (`< 650` lines)**: Share platform and wire logic instead of copy-pasting (e.g., `hosts/apple-common/` for iOS + macOS, `cxx/nve-fabric-bridge/` for C++ hosts, single-source `nve-codegen` for ABI/codecs). Split oversized files along cohesive subsystem boundaries.
- **Zero backwards compatibility**: Refactor cleanly and delete dead/legacy code, obsolete exports, and fallback shims immediately instead of keeping compatibility layers.
- **Root causes & upstream memory**: Fix underlying invariants rather than patching symptoms. Whenever you hit an upstream bug, quirk, or footgun in a dependency, compiler, or platform toolkit, document version + symptom + mitigation in [docs/UPSTREAM.md](docs/UPSTREAM.md).
- **Deep verification, live E2E & visual review**: Write edge-case, property, fuzz, and live E2E tests (corrupt/truncated buffers, focus/IME/a11y, hot-swap state preservation, ABI error paths) rather than smoke tests. Keep workspace + `wasm32` Clippy (`-D warnings`), host CTest/Swift/Flutter suites, and statistical benchmarks (`cargo bench`, `nve_fabric_bridge_bench`) green. For visual/UI changes, capture and inspect rendered screenshots (plus an adversarial review) before calling work done.
- **Parallel execution & fast landing**: Do small tasks directly; delegate substantial work via T3 `delegate_task` (never native subagents) with strictly disjoint file ownership (`isolation: "worktree"` for parallel local work, remote Windows `samuel-pc` for Win32/MSVC/D3D12/RNW verification). Merge and verify finished branches onto `main` immediately—do not batch finished branches while waiting on others.
- **Clean commits**: Inspect `git diff` and commit only your own paths with a conventional commit message on stdin (`git commit --only <paths> -F - <<'EOF'`).

## Quick Verification Entry Points

```bash
./scripts/check-all.sh --quick                                        # Fast local suite
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run --locked -p nve-cli -- codegen abi --check                  # Single-source ABI drift check
```

See [docs/iteration-speed.md](docs/iteration-speed.md) for CMake/Ninja, iOS/macOS, Windows RNW, Android, SwiftUI, Flutter, Skia, Web WASM, and benchmark commands.
