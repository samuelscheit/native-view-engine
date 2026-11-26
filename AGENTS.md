# AGENTS.md — `native-view-engine`

High-signal invariants, engineering principles, and workflow rules. Keep this file lean; put detailed specs and command references in `docs/`.

## Documentation Map

- [docs/architecture.md](docs/architecture.md) — Multi-backend architecture, binary IR/ABI model, and crate/host layout.
- [docs/quality-audit.md](docs/quality-audit.md) — Honest L0–L4 quality & completeness inventory across crates, backends, modules, and hosts.
- [docs/ui-maturity-audit.md](docs/ui-maturity-audit.md) — Clean-slate Tier 1–3 UI maturity inventory & gap scorecard.
- [docs/BACKLOG.md](docs/BACKLOG.md) — Open prioritized roadmap (keep open-only).
- [docs/CHANGELOG.md](docs/CHANGELOG.md) — Reverse-chronological log of shipped milestones (with commit hashes & metrics).
- [docs/UPSTREAM.md](docs/UPSTREAM.md) — Known upstream bugs, compiler/toolchain/OS quirks, and mitigations.
- [docs/iteration-speed.md](docs/iteration-speed.md) — Hot-reload budgets and full build/test/benchmark commands for all platforms.
- [scripts/check-all.sh](scripts/check-all.sh) — Unified cross-platform verification script (`--quick` or full).

## Non-Negotiable Invariants

- **Single binary IR & modular C ABI**: Keep `packages/core/engine` framework-agnostic (zero `dioxus-core` dependency). Cross host/FFI boundaries via one flat binary frame/scene ABI (`NveSceneView`, `EventRing`, `nve_get_interface`) locked with compile-time `offset_of!` / C++ `static_assert`. See [docs/architecture.md](docs/architecture.md).
- **Generated files are never committed**: Non-Java generated files MUST include `.generated.` (`*.generated.{h,hpp,dart,ts,rs,swift,kt,cpp}`); generated Java lives in `dev.nve.android.generated`. All generated ABI/wire files are gitignored and built via `cargo run --locked -p nve-cli -- codegen abi` (`--check` for full parity); never add un-suffixed shims.
- **Clean git & docs hygiene**: Never commit build/trace/screenshot/binary artifacts (`*.trace.json`, `cxx/build*`, `hosts/*/build`, etc.). Keep `BACKLOG.md` open-only, `quality-audit.md` accurate, and `CHANGELOG.md` updated. Inspect `git diff`, preserve unrelated local edits, and commit only owned paths via `git commit --only <paths> -F - <<'EOF'`.

## Engineering Principles

- **Data-Oriented Design, branchless hot paths & zero-cost ergonomics**: Default to **Tier 1 direct typed memory** (SoA / dense word tables, flat arenas/slabs, zero per-node `Box`/`Vec` heap allocations, cache-line alignment, branch-minimal batch loops, zero per-node downcasts/hash lookups); use **Tier 2 compact binary codecs** only across process/socket/disk boundaries (never JSON on runtime hot paths). Encode domain invariants in types (distinct `#[repr(transparent)]` handles / `IndexVec` — never mixed bare primitives; seal validation invariants at ingress so hot reads never re-validate) and wrap DOD internals in zero-cost read & write ergonomics (e.g. `#[derive(NveTable)]`, validated `NodeRef`/`NodeMut` views, codegen foreign getters).
- **Compile-time identity via `xxh64`**: Standardize on `const fn xxh64` (`nve-hash` / `nve-abi`) across Rust and foreign hosts (zero FNV-1a). Bake static/table/schema IDs at compile time and resolve to dense runtime slots once.
- **One standalone module per capability**: Every capability lives in `packages/modules/<cap>/` with a pure-spec root crate and thin leaf backend sub-crates (`web/`, `skia/`, `apple/`, `android/`, …). Core crates and backends MUST have zero dependencies on optional modules, and `hosts/` MUST NOT contain library code.
- **Platform-native APIs first & dogfood NVE**: Use OS/browser APIs first (`Intl.*`, DOM/CSS, Apple/Android/WinRT APIs) with opt-outable pure-Rust fallbacks only for SSR/headless/missing APIs. Dogfood NVE for our docs, site, Studio, charts, and codegen — when dogfooding exposes a missing engine primitive, build the primitive instead of patching app code.
- **Anti-slop, self-documenting names, DRY (`< 650` lines), zero backwards compatibility**: Hold the honest L0–L4 bar ([docs/quality-audit.md](docs/quality-audit.md)) — delete dead stubs, legacy shims, and obsolete exports immediately. Name formats/APIs clearly (no cryptic acronym soup or marketing buzzwords), keep heavy third-party runtimes behind optional adapters, and log toolchain/OS footguns in [docs/UPSTREAM.md](docs/UPSTREAM.md).

## How We Design, Orchestrate & Ship

- **Adversarial first-principles design before coding**:
  1. *Question premises first*: Challenge existing ADRs, prior decisions, and your own assumptions — ask whether a layer, enum, codec, or abstraction should exist at all before optimizing it.
  2. *Parallel independent research & multi-round critique*: Delegate read-only subagents to audit live code, search the web for existing crates/standards, and run adversarial self-critique rounds until zero cracks remain. Resume warm subagents (`t3_thread_send`) when iterating on feedback rather than spawning cold duplicates.
  3. *Cross-examine, prototype & align*: Compare options with concrete byte/allocation/branch counts, prototype/benchmark non-obvious behavior, and surface genuine irreducible tradeoffs before implementing.
- **Cohesive full-depth execution with mandatory live deviation**: Pick one vertical slice at a time and carry it to end-to-end depth. Optimize for **long-term architectural correctness**, not intermediate-slice compilability or throwaway bridges. Treat plans as hypotheses: verify every assumption against live code and **proactively deviate** whenever you find a better design, missing foundation, or hidden hole.
- **Zero-priming feature discovery** *(for roadmap & maturity audits)*: Delegate a fresh T3 `research` agent (`isolation: "shared"`) with **zero repo reads** and **zero priming** (state only the general category, require web search) to output edge-case-aware Tier 1/2/3 capabilities, then diff against the repo (**Implemented / Partial / Missing**).
- **Self-correcting verification, pervasive benchmarks & 4-lens dual review (any subsystem)**:
  1. *Contract-first invariants & dogfood ergonomics*: Before coding, define falsifiable domain invariants and enforce them automatically (0-alloc `CountingAlloc` hot-path tests, layout `const assert`, fuzz/property tests, real live browser/window `:hover`/`:active`/drag/`getBoundingClientRect()`/keyboard/IME checks + desktop/mobile screenshot critique, native + `wasm32` Clippy `-D warnings`, ABI parity, native host suites; [docs/iteration-speed.md](docs/iteration-speed.md)). Write realistic consumer call sites to catch awkward APIs early.
  2. *Pervasive micro & macro benchmarks*: Ship benchmarks measuring **both latency and allocations/sizes** across realistic and stress inputs for every kind of work, plus release ASM checks on hot primitives — never trust unmeasured performance claims.
  3. *Mandatory fresh-eyes 4-lens dual review before handback*: On every non-trivial change (any domain), run **both** `gemini-4-neon` and `anthropic/claude-opus-5-5` in parallel to read the full diff and touched files line-by-line across **(1) Performance & Resources**, **(2) Types, Soundness & Edge Cases**, **(3) Ergonomics & Minimalism**, and **(4) Verification Honesty**. Fix real findings before reporting done, and promote every newly discovered bug class into a permanent automated guard (test/bench/lint/type).
  4. *Treat invariant & lint failures as real engineering flaws*: Never silence, rename around, or mechanically patch an invariant failure. Treat every violation as a signal of a genuine architectural flaw — investigate the root cause through parallel research, evaluate competing designs with tradeoffs, align with the user on non-obvious choices, and only then implement the best long-term fix.
- **Delegation & landing**: Always delegate via T3 `delegate_task` (never native `invoke_subagent`). Use `isolation: "shared"` directly on the main checkout when only one editing subagent runs (reuses warm build caches and avoids worktree churn); use `isolation: "worktree"` only when parallel editing delegates run concurrently, and remote Windows `samuel-pc` for Win32/MSVC/D3D12/RNW. **Model policy**: default to `gemini-4-neon` (`antigravity`) for implementation; run **both** `gemini-4-neon` and `anthropic/claude-opus-5-5` in parallel for non-trivial research, design, and adversarial reviews; **never use `gateway/gpt-6-luna` or `gateway/gpt-5.6`**. **Failure recovery**: if a subagent errors out, try resuming it (`t3_thread_send`) first; if resuming isn't possible, start a replacement on the alternate allowed model with a complete handoff (branch/commits, worktree state, findings, remaining work) so it continues without redoing finished work. Never forbid delegates from fixing shared foundations when root-cause architecture requires it. Steer active delegates (`t3_thread_send`) when requirements shift; each delegate verifies and lands its own finished work onto `main`.

## Quick Verification

```bash
./scripts/check-all.sh --quick
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run --locked -p nve-cli -- codegen abi && cargo run --locked -p nve-cli -- codegen abi --check
```
