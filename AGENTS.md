# AGENTS.md — `native-view-engine`

High-signal invariants, engineering practices, and workflow rules for agents working in this repository.

## Documentation Map (`docs/`)

- [docs/architecture.md](docs/architecture.md) — Multi-backend architecture, binary IR/ABI model, and crate/host layout.
- [docs/quality-audit.md](docs/quality-audit.md) — Code-truth L0–L4 (Gold Standard) quality & completeness inventory across all crates, backends, modules, bridges, and hosts.
- [docs/ui-maturity-audit.md](docs/ui-maturity-audit.md) — Clean-slate Tier 1–3 UI maturity inventory & NVE gap scorecard (cross-linked with Backlog & Changelog).
- [docs/BACKLOG.md](docs/BACKLOG.md) — Open prioritized roadmap (keep open-only).
- [docs/CHANGELOG.md](docs/CHANGELOG.md) — Complete reverse-chronological log of shipped milestones.
- [docs/UPSTREAM.md](docs/UPSTREAM.md) — Known upstream bugs, toolchain/OS quirks, and mitigations.
- [docs/iteration-speed.md](docs/iteration-speed.md) — 4-tier hot-reload budgets and CMake/Ninja, iOS/macOS, Windows RNW, Android, SwiftUI, Flutter, Skia, Web WASM, and build/test/benchmark commands.
- [scripts/check-all.sh](scripts/check-all.sh) — Unified cross-platform verification script (`--quick` or full).

## Non-Negotiable Invariants

- **Single binary IR & modular C ABI**: Every frame crosses the Rust/host boundary as one flat batched `CommandBuffer`, `EventRing`, or zero-decode `NveSceneView`. Expose modules solely via `nve_get_interface(1..=5)` (`NveApp` + `export_nve_app!`). Keep `packages/core/engine` framework-agnostic (zero dependency on `dioxus-core`).
- **Generated-file naming & build-time generation**: Every generated non-Java file MUST include `.generated.` (`*.generated.{h,hpp,dart,ts,rs,swift,kt,cpp}`). Every generated Java file MUST live in a `generated/` package (`package dev.nve.android.generated;`). All generated ABI/Wire files (`*.generated.*`, `hosts/android-shell/android-app/src/dev/nve/android/generated/`) are **gitignored** and generated automatically during CMake/Gradle/Web builds or on demand via `cargo run --locked -p nve-cli -- codegen abi` (verified via `--check` for `15/15` Core + Scene + Wire parity); never commit generated bindings or add un-suffixed shim headers/symlinks.
- **Keep build/trace artifacts out of git**: Never commit `*.trace.json`, `*.perfetto.json`, `cxx/build*`, `hosts/*/build`, `hosts/swiftui-kit/.build`, `hosts/flutter-plugin/.dart_tool`, screenshots, or compiled binaries.

## How We Work & Core Focus Areas

- **Dogfood NVE & build missing foundations**: Build our own landing page, docs framework (`nve-docs`, `nve-mdx`), benchmark charts (`NVE_DRAW1`), flagship Studio app, and ABI/wire codegen (`nve-codegen` generating NVE's own ABIs) directly on NVE — never reach for external JS frameworks (Next.js/React/Vite/Chart.js/Node) or ad-hoc shims. When dogfooding exposes a missing engine capability (SSR, hydration, `flex-wrap`, weighted text measurement, `nve-ui`, `nve-router`, `nve-sdk`), build the real engine primitive instead of working around it in app code.
- **Platform-native APIs first, fallback only when absent**: Always leverage built-in OS and browser APIs first (`Intl.*`, Web DOM/CSS, Apple `Foundation`/AVFoundation/AppKit/UIKit, Android `android.icu`/CameraX, Win32/WinRT/`icu.dll`) for zero bundle bloat and full platform fidelity; provide pure-Rust fallbacks strictly for SSR, headless testing, or platforms lacking the API (and keep fallbacks opt-outable via feature flags when unused).
- **Direct typed memory first, minimal dependencies, no buzzword slogans**: Maximum performance and clean API design mean **Tier 1 (default for all in-process, shared-memory, and FFI boundaries)**: direct typed Rust structs, `#[repr(C)]` slabs, and borrowed slices (`&[T]`, `NveSceneView`, `NveFrameCommandBatch`, `NvePropSlice`, `NveEventBatch`, `DirectByteBuffer`) with no serialization, no parsing, and no out-of-band side-channels. **Tier 2 (ONLY when bytes cross a network socket, IPC pipe, or disk file)**: compact binary formats (`packages/core/wire`, `RelocContainer`, `MmapOffsetArena`) — no JSON on runtime hot paths, no unnecessary `serde_json` in core/app crates. Lock `#[repr(C)]` layouts with compile-time `offset_of!` and C11/C++20 `NVE_STATIC_ASSERT(offsetof(...))`. Keep heavy third-party runtimes (RN Fabric/Yoga, VisionCamera v5) strictly behind optional pluggable adapters. Never slap marketing buzzwords ("Zero-JSON", etc.) onto manifests, comments, or docs; state literal facts and honest limitations.
- **Self-documenting naming (no cryptic acronym soup)**: Never invent opaque 3–4 letter codes (`NVH1`, `NVW1`, `NVT1`, `NVD1`, `NVC1`, `NVWS1`, `NVB1`, `NVERELOC`…) in types, modules, docs, or format headers. Name every format, module, magic identifier, and API so an outsider immediately understands its purpose (e.g., `NveTransportPacket` / `NVE_TRANSPORT_V1`, `NveFrameDelta`, `NveCrdtDelta`, `NveRelocScene`, `NveWebSnapshot`, `NveWebState`).
- **Anti-slop, honest L0–L4 bar & evidence-driven design**: Never ship stub crates, mock-only facades, or hacky workarounds pretending to be complete modules — delete dead/incomplete stubs and record them honestly in [docs/BACKLOG.md](docs/BACKLOG.md) and [docs/quality-audit.md](docs/quality-audit.md) (L0 Stub → L1 Prototype → L2 Working Core → L3 Production-Grade → L4 Gold Standard, the VisionCamera bar). For non-trivial architecture or UI work, follow [`.agents/skills/evidence-driven-architecture/SKILL.md`](.agents/skills/evidence-driven-architecture/SKILL.md). If implementation exposes a missing engine foundation or better direction, **stop, replan proactively, and build the real foundation first** instead of patching around it.
- **Strict DRY & modular code (`< 650` lines)**: Share platform and wire logic instead of copy-pasting (`hosts/apple-common/`, `cxx/nve-fabric-bridge/`, single-source `nve-codegen` for ABI & wire codecs). Split oversized files along cohesive subsystem boundaries.
- **Zero backwards compatibility**: Refactor cleanly and delete dead/legacy code, obsolete exports, and fallback shims immediately instead of keeping compatibility layers.
- **Root causes & upstream memory**: Fix underlying invariants rather than patching symptoms. Whenever you hit an upstream bug, compiler/toolchain quirk, or OS footgun, document version + symptom + mitigation in [docs/UPSTREAM.md](docs/UPSTREAM.md).
- **Deep behavioral verification (never mock-only theater)**: Unit tests against pure state structs or `MirrorElement` string assertions NEVER prove a component works. For UI components and hosts, verify **real end-to-end interaction behavior** in a live browser/window: move a real pointer and assert computed `:hover`/`:active` styles, drag real handles and assert layout geometry changes, verify floating overlay coordinates via `getBoundingClientRect()` without shifting page flow, test real keyboard/IME/wheel input, and keep workspace + `wasm32` Clippy (`-D warnings`), native host suites, and benchmarks green.
- **Docs & commit hygiene**: Keep [docs/BACKLOG.md](docs/BACKLOG.md) open-only, keep [docs/quality-audit.md](docs/quality-audit.md) accurate as components mature, and log every landed milestone with commit hashes and verification metrics in [docs/CHANGELOG.md](docs/CHANGELOG.md). Inspect `git diff`, preserve unrelated local edits, and commit only owned paths with a conventional commit message on stdin (`git commit --only <paths> -F - <<'EOF'`).

## Orchestrator

- **One vertical slice at a time at full depth**: Do NOT fan out 3–4 parallel feature waves at once. Pick one architectural vertical slice (API design, performance, or one subsystem), carry it to real end-to-end correctness and depth, verify it thoroughly, and land it cleanly before starting the next. Delegate substantial work (and all subagent reviews) via T3 `delegate_task` — NEVER use Antigravity's native `invoke_subagent` — using `isolation: "worktree"` for local edits and remote Windows `samuel-pc` for Win32/MSVC/D3D12/RNW verification. Never forbid a delegate from fixing shared foundations (`packages/core/abi`, `Cargo.toml`, `hosts/`) when the clean root-cause architecture requires it.
- **Fast landing & live steering**: Merge, verify, and push finished branches onto `main` cleanly. Steer active delegates mid-turn (`t3_thread_send`) when requirements evolve rather than letting work go stale.

## Quick Verification Entry Points

```bash
./scripts/check-all.sh --quick                                        # Fast local suite
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run --locked -p nve-cli -- codegen abi && cargo run --locked -p nve-cli -- codegen abi --check  # Generate + 15/15 parity check
```

## Learned rules

- **Delegates land their own work**: Each delegate merges, verifies and pushes its own finished branch onto `main` as soon as it is done, rebasing onto the latest `main` and resolving conflicts itself. Do not collect finished branches for the coordinator to merge in a batch, which makes everyone wait on the slowest agent.
