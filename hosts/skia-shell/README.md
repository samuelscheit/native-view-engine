# Native View Engine — Skia Desktop Shell (`hosts/skia-shell`)

Cross-platform desktop host (`nve-skia-shell`) that loads any compiled NVE `cdylib` (`nve_get_interface`), drives `nve-backend-skia` (`DisplayList` + Flexbox + `CosmicTextEngine`), and presents into a live `winit` window with hardware GPU swapchain support (`skia-metal`, `skia-vulkan`, `skia-d3d`) and automatic zero-C++ `softbuffer` fallback.

## Features

- **Pure Library / Host Split**: Mirrors `packages/backends/web` + `hosts/web-shell` — all `winit`, `softbuffer`, and WebSocket hot-reload host logic lives here while `packages/backends/skia` remains a pure rendering backend library.
- **2-Tier Hot Reload (`nve dev`)**: Connects to `ws://127.0.0.1:8089` (configurable via `--ws <url>` or disabled via `--no-ws`) to apply Tier-1 template patches and Tier-2 `cdylib` reloads with state preservation.
- **Instant-Boot Frame-0 `MmapSnapshot`**: Pass `--snapshot <path.nvesnap> [--schema <ver>]` to replay a freeze-dried Frame-0 `CommandBuffer` directly from memory-mapped storage before `dlopen`, restore app state on boot, and atomically persist the latest snapshot on exit.
- **Headless Screenshot Capture**: Pass `--screenshot <out.ppm> [--width W] [--height H] [--scale S]` to render headlessly without a display server and write a binary `P6` PPM image.
- **Hardware GPU Swapchains**: Forward `--features skia`, `--features skia-metal`, `--features skia-vulkan`, or `--features skia-d3d` directly to `nve-backend-skia`.

## Build & Run

```bash
# Build an example app cdylib
cargo build -p nve-example-kitchen-sink

# Launch the Skia desktop shell (zero-C++ softbuffer default)
cargo run -p nve-skia-shell -- \
  target/debug/libnve_example_kitchen_sink.dylib --no-ws

# Launch with Frame-0 MmapSnapshot instant boot + atomic save on exit
cargo run -p nve-skia-shell -- \
  target/debug/libnve_example_kitchen_sink.dylib \
  --no-ws --snapshot /tmp/kitchen-sink.nvesnap --schema 1

# Headless PPM screenshot capture
cargo run -p nve-skia-shell -- \
  target/debug/libnve_example_kitchen_sink.dylib \
  --no-ws --width 1280 --height 800 --scale 2.0 --screenshot /tmp/kitchen-sink.ppm

# Build with Apple Metal hardware GPU swapchain (macOS)
FORCE_SKIA_BINARIES_DOWNLOAD=1 cargo check -p nve-skia-shell --features skia-metal

# Run unit tests
cargo test -p nve-backend-skia -p nve-skia-shell
```
