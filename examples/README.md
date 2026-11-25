# Examples

## Runnable

- **[`counter`](counter/)** (`nve-example-counter`) — Stateful Dioxus `rsx!` counter built as both `rlib` (headless/CLI/tests) and `cdylib` (`nve_get_interface` native hosts & Tier-2 hot-swap).
- **[`studio-workspace`](studio-workspace/)** (`nve-studio-workspace`) — Flagship **Aether Workspace** (local-first issue board, `# engineering` / `# design-sync` channels, live layout/frame stress metrics) paired with a docked, toggleable **NVE Studio** meta-workbench (live accent/radius/density self-patching, X-Ray Flexbox HUD, deterministic time-travel scrubber, and Tier-2 `cdylib` state preservation).

```bash
# Counter example
cargo build -p nve-example-counter
cargo test -p nve-example-counter
cargo nve dev --package nve-example-counter --watch examples/counter/src

# Studio Workspace flagship example (single cdylib across every native host)
cargo build -p nve-studio-workspace
cargo test -p nve-studio-workspace

# Launch on native macOS AppKit host
./hosts/macos-shell/build/nve_macos_shell --dylib target/debug/libnve_studio_workspace.dylib

# Launch on native Qt 6 host
./hosts/qt-shell/build/nve_qt_shell --dylib target/debug/libnve_studio_workspace.dylib --width 960 --height 640

# Launch on Skia Desktop preview (with live hot-reload support)
cargo run -p nve-backend-skia --features desktop --example preview -- target/debug/libnve_studio_workspace.dylib --no-ws
```

## Flagship Vision

### Local-First Workspace & Messaging Suite

Linear × Notion × Slack in one ~15 MB binary: `<50 ms` cold start, `<40 MB` RAM, 120fps 100k-item views, and real native OS text, IME, and accessibility across macOS, Windows, Linux, iOS, and Android — paired with instant zero-install Web WASM share links and native-speed `nve_get_interface` user plugins.

### NVE Studio (Meta Workbench)

The self-hosting workbench used to build it: side-by-side live native surfaces (AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, WASM), live Taffy/Yoga + X-Ray HUD toggle, Cmd-click Tier-1 (`<2 ms`) / Tier-2 (`<300 ms`) self-editing hot-reload, deterministic `CommandBuffer` time-travel replay, and ~10 KB/s binary delta live pairing.
