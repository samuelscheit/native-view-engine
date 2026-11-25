# Examples

## Runnable

- **[`counter`](counter/)** (`nve-example-counter`) — Stateful Dioxus `rsx!` counter built as both `rlib` (headless/CLI/tests) and `cdylib` (`nve_get_interface` native hosts & Tier-2 hot-swap).

```bash
cargo build -p nve-example-counter
cargo test -p nve-example-counter
cargo nve dev --example counter
```

## Flagship Vision

### Local-First Workspace & Messaging Suite

Linear × Notion × Slack in one ~15 MB binary: `<50 ms` cold start, `<40 MB` RAM, 120fps 100k-item views, and real native OS text, IME, and accessibility across macOS, Windows, Linux, iOS, and Android — paired with instant zero-install Web WASM share links and native-speed `nve_get_interface` user plugins.

### NVE Studio (Meta Workbench)

The self-hosting workbench used to build it: side-by-side live native surfaces (AppKit, WinUI, Qt 6, GTK4, iOS/Android Fabric, WASM), live Taffy/Yoga + X-Ray HUD toggle, Cmd-click Tier-1 (`<2 ms`) / Tier-2 (`<300 ms`) self-editing hot-reload, deterministic `CommandBuffer` time-travel replay, and ~10 KB/s binary delta live pairing.
