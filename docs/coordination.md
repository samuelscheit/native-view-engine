# Parallel Subagent Coordination Contract

All four vertical-slice agents work concurrently in the same worktree (`/Users/user/Developer/Developer/respond/native-view-engine`). Follow these rules strictly so zero file or lock conflicts occur.

## 1. Strict File Ownership Matrix

| Agent | Exclusive Owned Paths | `CARGO_TARGET_DIR` / Build Dir |
| :--- | :--- | :--- |
| **Agent 1 — Skia Window (`skia`)** | `crates/nve-backend-skia/**` | `CARGO_TARGET_DIR=target/agent-skia` |
| **Agent 2 — Fabric C++ & iOS (`fabric`)** | `cxx/nve-fabric-bridge/**`, `hosts/ios-shell/**` | `cxx/build-fabric`, `hosts/ios-shell/build` |
| **Agent 3 — Bi-Directional Codegen (`codegen`)** | `crates/nve-codegen/**` | `CARGO_TARGET_DIR=target/agent-codegen` |
| **Agent 4 — Web WASM (`web`)** | `crates/nve-backend-web/**`, `hosts/web-shell/**` | `CARGO_TARGET_DIR=target/agent-web` |

## 2. Frozen Shared Files (DO NOT EDIT)

The coordinator owns and integrates these after workers finish:
- Root `Cargo.toml` and `Cargo.lock` (all needed crates/members are already pre-registered; pass `--offline` or avoid touching `Cargo.lock` if possible; if you MUST add a crate to your own `Cargo.toml`, coordinate with the coordinator first!)
- `crates/nve-abi/**` and `cxx/nve-fabric-bridge/include/nve_abi.h` (ABI v1 with `set_props` & `drain_emitted_events` is frozen)
- `crates/nve-engine/**`, `crates/nve-components/**`, `crates/nve-dioxus/**`, `crates/nve-cli/**`, `examples/counter/**`

## 3. Shared Interface Contracts

- **NVE -> RN Embedded Component (`NveAppVTable`):**
  - `set_props(handle, slice)` accepts UTF-8 JSON prop object (or future binary prop map) and returns `1` if re-render is needed.
  - `drain_emitted_events(handle)` returns an `EventRing` (`nve_abi::encode_events`) byte slice for forwarding into RN `EventEmitter`.
  - **Codegen <-> Fabric alignment:** Agent 3 (`nve-codegen`) and Agent 2 (`fabric`) should coordinate via `t3_thread_send` / `t3_thread_read` so generated C++ Fabric prop/event wrappers match `cxx/nve-fabric-bridge`'s `NveFabricHost` / `RCTNveComponentView` symbols exactly!
- **Hot Reload WebSocket (`HotPatchMsg` JSON):**
  - `nve dev` serves JSON-serialized `nve_engine::HotPatchMsg` on `ws://127.0.0.1:8089`.
  - Both Agent 1 (`skia`) and Agent 4 (`web`) should consume the exact `HotPatchMsg` JSON schema (`template_patch`, `dylib_reload`, `dev_status`).

## 4. Verification & Commits

- Each agent verifies ONLY its own crate/C++ target (using its own `CARGO_TARGET_DIR` or CMake build dir). Do NOT run `cargo test --workspace` or `git commit` — the coordinator runs unified verification and commits cleanly once all four land.
