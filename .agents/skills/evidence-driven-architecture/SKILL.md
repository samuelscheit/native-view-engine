---
name: evidence-driven-architecture
description: Research, evaluate tradeoffs, prototype/benchmark edge cases, and run adversarial review before shipping non-trivial architecture or UI features.
---

1. **Question premises first (never optimize inside a stale box)**: Challenge existing ADRs, prior decisions, the user's framing, and your own assumptions — ask whether a layer, buffer, codec, or abstraction should exist at all before optimizing it.
2. **Research first (subagent)**: Delegate a read-only research subagent to survey current standards, real-world browser/OS quirks, and existing repo primitives before designing.
3. **Evaluate tradeoffs**: Compare concrete options in a compact matrix (latency, bundle/memory cost, platform fidelity, maintenance, security, edge cases) and pick the cleanest root-cause primitive—never a hack or shim.
4. **Prototype & measure**: When performance, layout, or wire/FFI behavior is non-obvious, run a focused prototype test or Criterion benchmark before committing to the abstraction.
5. **Implement cleanly**: Platform-native OS/browser APIs first (opt-outable pure-Rust fallback only when absent), zero-serialization in-process, self-documenting names (no cryptic acronym soup), files `< 650` lines, zero backwards-compatibility leftovers.
6. **Verify deeply & review adversarially**: Edge-case/fuzz/E2E tests, native + `wasm32` Clippy (`-D warnings`), Desktop/Mobile visual screenshot inspection for UI, and an adversarial subagent review on high-impact changes.
