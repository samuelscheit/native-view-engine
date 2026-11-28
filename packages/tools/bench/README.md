# `nve-bench`

Deterministic fixture library and Criterion benchmark harness (`packages/tools/bench`, **Tools layer**) measuring ABI, backend, Dioxus, interop, render-sink handoff, and WASM bundle size metrics.

## Key Public API

- `DEFAULT_VIEWPORT`, `build_row_tree_cmd`, `build_node_tree_cmd`, `build_card_scene_cmd`, `build_event_batch`
- `BenchCounterApp`, `create_bench_dioxus_surface`, `BenchFfiApp`
- `wasm_bundle_size`

```rust
use nve_abi::FrameId;
use nve_bench::build_row_tree_cmd;

let cmd_bytes = build_row_tree_cmd(100, FrameId(1));
assert!(!cmd_bytes.is_empty());
```

## Dependency Invariants

- **Depends on**: `nve-abi`, `nve-dioxus`, `nve-engine` (plus dev/bench backends and `criterion`).
- **Must NOT be depended on by**: any library crate under `packages/{core,backends,framework,modules}/`.

## Verification

```bash
cargo test -p nve-bench
cargo bench -p nve-bench --no-run
```
