# GTK4 Native Host Shell (`hosts/gtk-shell`)

Native **GTK4** (Linux Wayland/X11 and cross-platform GTK4) host shell built on top of the shared C++20 runtime (`cxx/nve-fabric-bridge/include/nve_host_runtime.hpp`).

## Architecture

- **`NveGtkHost` (`NveGtkHost.hpp` / `NveGtkHost.cpp`)**:
  - Reuses `nve::NativeHostSession` and `nve::RetainedViewReconciler<GtkWidget*, GtkViewAdapter>` directly.
  - Registers a native **Pango** (`PangoLayout`) text measurer via `session.set_text_measurer(...)` for exact pixel font sizing and `PANGO_WRAP_WORD_CHAR` wrapping.
  - Maps NVE shadow nodes to native GTK4 widgets:
    - `NVE_COMP_VIEW` / `NVE_COMP_PRESSABLE` / wrapper `NVE_COMP_TEXT` → `GtkFixed` (`GTK_OVERFLOW_HIDDEN` when `border_radius > 0`)
    - Leaf `NVE_COMP_TEXT` → `GtkLabel` (`wrap = TRUE`, top-left aligned, `can_target = FALSE`)
    - `NVE_COMP_SCROLL_VIEW` → `GtkScrolledWindow` wrapping an inner `GtkFixed` content container
    - `NVE_COMP_TEXT_INPUT` → `GtkEntry`
  - Applies host-scoped CSS classes (`.nve-host-<id>-node-<node_id>`) through a single `GtkCssProvider` attached via `gtk_style_context_add_provider_for_display`, with a `GTK_CHECK_VERSION(4, 12, 0)` guard between `gtk_css_provider_load_from_string` and `gtk_css_provider_load_from_data` for GTK 4.6–4.16+ compatibility.
  - Dispatches native clicks via `GtkGestureClick` (`released`) and tracks container/window resizes automatically.
- **Standalone Preview App (`main.cpp`)**:
  - `nve_gtk_shell [--dylib <path/to/libapp.so>] [--width W] [--height H]` launches a `GtkApplication` / `GtkApplicationWindow` hosting `NveGtkHost`.

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

## Build & Test

```bash
# Configure and build (add -DRN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon for Yoga+JSI):
cmake -S hosts/gtk-shell -B hosts/gtk-shell/build -G Ninja
cmake --build hosts/gtk-shell/build

# Run CTest on a desktop session:
ctest --test-dir hosts/gtk-shell/build --output-on-failure

# Run CTest in headless Linux CI with xvfb-run:
xvfb-run -a ctest --test-dir hosts/gtk-shell/build --output-on-failure

# Run CTest with the live Rust Kitchen Sink cdylib:
cargo build -p nve-example-kitchen-sink
NVE_TEST_KITCHEN_SINK_DYLIB="$(pwd)/target/debug/libnve_example_kitchen_sink.so" \
  xvfb-run -a ctest --test-dir hosts/gtk-shell/build --output-on-failure

# Launch the interactive GTK4 preview window:
./hosts/gtk-shell/build/nve_gtk_shell
./hosts/gtk-shell/build/nve_gtk_shell --dylib target/debug/libnve_example_kitchen_sink.so
```
