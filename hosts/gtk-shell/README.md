# GTK4 Native Host Shell (`hosts/gtk-shell`)

Native **GTK4** (Linux Wayland/X11, and macOS via Homebrew GTK) host for NVE apps. It runs apps on
`libnve_runtime` and mirrors the runtime's published scene into retained GTK widgets through the
shared C++ host stack (`packages/core/host-cxx`, linked via `cxx/cmake/NveHostStack.cmake`).

## Architecture

- **`NveGtkHost` (`NveGtkHost.*`)** owns a `nve::host::HostSession` (runtime, app attach, `cdylib`
  load / hot reload, pairing, app-state files) and a `nve::host::RetainedTree<Adapter>`:
  - Each sync holds one `RuntimeSceneGuard` and visits only the runtime's dirty nodes
    (`dirty_indices`); a source change (attach / load / reload / pair) runs one full pass.
  - Runtime wakes are always queued onto the GTK main loop (`g_idle_source_new`), never run inline.
  - A **Pango** measurer is installed as the runtime's text measurer.
- **Widget mapping (`NveGtkReconciler.cpp`)**: the view tag is `WidgetKind | accessible role << 8`,
  so a kind or role change recreates the widget.
  - views → `GtkFixed`; text → `GtkLabel`; images → `GtkPicture` (content-keyed texture LRU);
  - single-line inputs → `GtkEntry`, multiline → `GtkTextView` (controlled text via
    `nve::host::ControlledText`: app values win, user echoes are absorbed);
  - scroll nodes and `overflow: scroll` → `GtkScrolledWindow` + inner `GtkFixed`;
  - `nve.canvas` nodes → `GtkDrawingArea` painting the `NVE_DRAW1` draw list with Cairo (`NveGtkMedia.cpp`).
- **Style & accessibility (`NveGtkStyleA11y.cpp`)**: per-node CSS rules under host-scoped classes
  (`.nve-host-<id>-node-<node_id>`) in one `GtkCssProvider`; ARIA roles / states / properties map
  to `GtkAccessible`.
- **Input**: `GtkGestureClick` presses and link activation, text change / submit / focus / blur,
  and scroll offsets (kept in step with the runtime's `scroll_by`).
- **`NveGtkWindowManager`**: multi-window surfaces, one host per window.

## Prerequisites

**Ubuntu / Debian (22.04 / 24.04+):**

```bash
sudo apt-get update
sudo apt-get install -y build-essential cmake ninja-build pkg-config libgtk-4-dev xvfb
```

**Fedora:**

```bash
sudo dnf install -y gcc-c++ cmake ninja-build pkgconf-pkg-config gtk4-devel xorg-x11-server-Xvfb
```

**macOS (optional, via Homebrew):**

```bash
brew install gtk4 pkg-config cmake ninja
```

A Rust toolchain is required: the build compiles `libnve_runtime` through `NveRuntime.cmake`.

## Build & Test

Debug / untyped builds enable ASan + UBSan (`NVE_HOST_SANITIZE`, see `NveHostStack.cmake`).

```bash
cmake -S hosts/gtk-shell -B hosts/gtk-shell/build -G Ninja
cmake --build hosts/gtk-shell/build

# Desktop session:
ctest --test-dir hosts/gtk-shell/build --output-on-failure
# Headless Linux CI:
xvfb-run -a ctest --test-dir hosts/gtk-shell/build --output-on-failure

# Also run the live Rust kitchen-sink cdylib (render, measure, press, hot reload):
cargo build -p nve-example-kitchen-sink
NVE_TEST_KITCHEN_SINK_DYLIB="$(pwd)/target/debug/libnve_example_kitchen_sink.so" \
  xvfb-run -a ctest --test-dir hosts/gtk-shell/build --output-on-failure
```

## Preview app (`main.cpp`)

```bash
./hosts/gtk-shell/build/nve_gtk_shell                     # built-in counter demo
./hosts/gtk-shell/build/nve_gtk_shell --dylib target/debug/libnve_example_kitchen_sink.so
./hosts/gtk-shell/build/nve_gtk_shell --pair nve://host:port --pair-token <token>
```

Options: `--state <file>`, `--no-state`, `--reset-state`, `--schema <ver>`, `--width W`, `--height H`
(env: `NVE_STATE_DISABLE=1`, `NVE_STATE_PATH=<file>`, `NVE_RESET_STATE=1`). App state persists
across launches unless the shell fell back to the built-in demo after a failed load or pair.
