# AGENTS.md — `native-view-engine`

High-signal invariants, engineering practices, and workflow rules for agents working in this repository.

## Documentation Map (`docs/`)

- [docs/architecture.md](docs/architecture.md) — Multi-backend architecture, binary IR/ABI model, and crate/host layout.
- [docs/BACKLOG.md](docs/BACKLOG.md) — Open prioritized roadmap (keep open-only).
- [docs/CHANGELOG.md](docs/CHANGELOG.md) — Complete reverse-chronological log of shipped milestones.
- [docs/UPSTREAM.md](docs/UPSTREAM.md) — Known upstream bugs, toolchain/OS quirks, and mitigations.
- [docs/iteration-speed.md](docs/iteration-speed.md) — 4-tier hot-reload budgets and CMake/Ninja, iOS/macOS, Windows RNW, Android, SwiftUI, Flutter, Skia, Web WASM, and build/test/benchmark commands.
- [scripts/check-all.sh](scripts/check-all.sh) — Unified cross-platform verification script (`--quick` or full).

## Non-Negotiable Invariants

- **Single binary IR & modular C ABI**: Every frame crosses the Rust/host boundary as one flat batched `CommandBuffer`, `EventRing`, or zero-decode `NveSceneView`. Expose modules solely via `nve_get_interface(1..=5)` (`NveApp` + `export_nve_app!`). Keep `crates/nve-engine` framework-agnostic (zero dependency on `dioxus-core`).
- **Generated-file naming**: Every generated non-Java file MUST include `.generated.` (`*.generated.{h,hpp,dart,ts,rs,swift,kt,cpp}`). Every generated Java file MUST live in a `generated/` package (`package dev.nve.android.generated;`). Keep `cargo run --locked -p nve-cli -- codegen abi --check` green (`15/15` Core + Scene + Wire artifacts); never add un-suffixed shim headers or symlinks.
- **Keep build/trace artifacts out of git**: Never commit `*.trace.json`, `*.perfetto.json`, `cxx/build*`, `hosts/*/build`, `hosts/swiftui-kit/.build`, `hosts/flutter-plugin/.dart_tool`, screenshots, or compiled binaries.

## How We Work & Core Focus Areas

- **Dogfood NVE & build missing foundations**: Build our own landing page, docs framework (`nve-docs`, `nve-mdx`), benchmark charts (`NVE_DRAW1`), flagship Studio app, and ABI/wire codegen (`nve-codegen` generating NVE's own ABIs) directly on NVE — never reach for external JS frameworks (Next.js/React/Vite/Chart.js/Node) or ad-hoc shims. When dogfooding exposes a missing engine capability (SSR, hydration, `flex-wrap`, weighted text measurement, `nve-ui`, `nve-router`, `nve-sdk`), build the real engine primitive instead of working around it in app code.
- **Zero-overhead, zero-JSON, minimal dependencies**: Research proven designs first and choose the zero-cost binary / direct-FFI / zero-copy path even when harder. Never ship JSON serialization, wrapper layers, CDN scripts, or extra per-frame copies on hot paths; lock `#[repr(C)]` layouts with compile-time `offset_of!` and C11/C++20 `NVE_STATIC_ASSERT(offsetof(...))`. Prefer zero-dependency Rust/native-OS implementations; keep heavy third-party runtimes (RN Fabric/Yoga, VisionCamera v5) strictly behind optional pluggable adapters.
- **Propose tradeoffs & adversarial review**: For larger features or UI/UX work, research strong references first, present concrete architectural options with tradeoffs before implementing, and run adversarial reviews on high-impact designs and rendered UI before calling work done.
- **Strict DRY & modular code (`< 650` lines)**: Share platform and wire logic instead of copy-pasting (`hosts/apple-common/`, `cxx/nve-fabric-bridge/`, single-source `nve-codegen` for ABI & wire codecs). Split oversized files along cohesive subsystem boundaries.
- **Zero backwards compatibility**: Refactor cleanly and delete dead/legacy code, obsolete exports, and fallback shims immediately instead of keeping compatibility layers.
- **Root causes & upstream memory**: Fix underlying invariants rather than patching symptoms. Whenever you hit an upstream bug, compiler/toolchain quirk, or OS footgun, document version + symptom + mitigation in [docs/UPSTREAM.md](docs/UPSTREAM.md).
- **Deep verification, cross-platform E2E & visual QA**: Write edge-case, property, fuzz, cross-language compiler/execution, and live browser/native E2E tests rather than smoke tests. Keep workspace + `wasm32` Clippy (`-D warnings`), host CTest/Swift/Flutter/Gradle suites, Windows 11 MSVC/RNW/D3D12 verification, and statistical benchmarks green. For visual/UI changes, capture and visually inspect screenshots across Desktop Dark/Light, Mobile, and Native Skia (fixing any layout/clipping/contrast defects) before shipping.
- **Docs & commit hygiene**: Keep [docs/BACKLOG.md](docs/BACKLOG.md) open-only and log every landed milestone with commit hashes and verification metrics in [docs/CHANGELOG.md](docs/CHANGELOG.md). Inspect `git diff`, preserve unrelated local edits, and commit only owned paths with a conventional commit message on stdin (`git commit --only <paths> -F - <<'EOF'`).

## Orchestrator

- **Parallel execution without bottlenecking**: Do tiny tasks directly; delegate substantial work (and all subagent reviews) exclusively via T3 `delegate_task` — NEVER use Antigravity's native `invoke_subagent` — with strictly disjoint file ownership (`isolation: "worktree"` for parallel local work, remote Windows `samuel-pc` for Win32/MSVC/D3D12/RNW verification). Check and coordinate with peer threads touching adjacent areas before editing shared surfaces.
- **Fast landing & live steering**: Merge, verify, and push finished branches onto `main` immediately as each completes—never batch finished branches while waiting on others. Steer active delegates mid-turn (`t3_thread_send`) when requirements evolve rather than letting work go stale.

## Quick Verification Entry Points

```bash
./scripts/check-all.sh --quick                                        # Fast local suite
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run --locked -p nve-cli -- codegen abi --check                  # 15/15 ABI + Wire drift check
```
