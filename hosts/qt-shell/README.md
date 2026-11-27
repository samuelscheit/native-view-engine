# NVE Qt 6 Host Shell (`hosts/qt-shell`)

Native **Qt 6** desktop host for Native View Engine with two front ends on one host core: a retained **`QWidget`** tree and a **Qt Quick** item rendering a retained scene graph. Both sit on the shared C++ host stack (`packages/core/host-cxx`: `HostSession`, `RetainedTree`, `SceneTables` / `read_node`, `EventFactory`, `ControlledText`, `app_state`) over `libnve_runtime` and `nve::canvas_cxx`, linked through `cxx/cmake/NveHostStack.cmake`.

## Architecture

- **`QtHost`** (`NveQtHost.*`): the shared host core: session, app modules (load / hot reload), paired thin clients, state files, queued UI-thread wakes guarded by an alive token, the `QTextLayout` text measurer, event dispatch (press / links / pointer / focus / text / scroll), and content resolution (text, cached images, canvas draw lists). Each front end implements only `sync_views`, `clear_views` and `after_sync`.
- Shared modules: `NveQtPaint` (boxes, borders, radii, shadows, images, text), `NveQtText` (typography, sanitized scene strings), `NveQtDrawList` (`NVE_DRAW1` via `QPainter`), `NveQtImageCache` (content-keyed LRU), `NveQtSemantics` (accessibility roles / states / value / attributes).
- **`NveQtWidget`** (`NveQtWidget*`, `NveQtCustomWidgets.cpp`, `NveQtWidgetReconciler.cpp`): painted node widgets, native `QLineEdit` / `QPlainTextEdit` inputs, `QScrollArea` scroll views, accessible widgets.
- **`NveQtQuickItem`** (`NveQtQuick*`, `NveQtSGBuilder.cpp`; built when `Qt6::Quick` is found): QML `NveSurface` in `import Nve 1.0`. GUI-thread node records feed a retained `QSGNode` tree updated O(changed) in `updatePaintNode` (transform / opacity / clip / scroll nodes, solid rects, per-node HiDPI textures re-rasterized only on paint changes). Host-side picking in paint order, wheel scrolling with chaining, keyboard focus traversal, controlled text editing with IME, and a per-node `QAccessibleInterface` tree.
- **`NveQtWindowManager`**: multi-window surfaces.
- **`nve_qt_shell`** (`main.cpp`): standalone window; `--quick` selects the Qt Quick front end.

## Build, Run & Test

```bash
cargo build -p nve-example-kitchen-sink
cmake -S hosts/qt-shell -B hosts/qt-shell/build -G Ninja
cmake --build hosts/qt-shell/build
# nve_qt_shell_test (Widgets) + nve_qt_quick_test (Quick, software scene graph), ASan + UBSan.
NVE_TEST_KITCHEN_SINK_DYLIB=$PWD/target/debug/libnve_example_kitchen_sink.dylib \
  ctest --test-dir hosts/qt-shell/build --output-on-failure
./hosts/qt-shell/build/nve_qt_shell --dylib target/debug/libnve_example_kitchen_sink.dylib [--quick]
```

Options: `--dylib <path>`, `--pair <uri> [--pair-token <token>]`, `--state <file>`, `--no-state`, `--reset-state`, `--schema <ver>`, `--width` / `--height`. Environment: `NVE_STATE_DISABLE=1`, `NVE_STATE_PATH=<file>`, `NVE_RESET_STATE=1`. State is saved on window close, SIGINT / SIGTERM and shutdown; falling back to the built-in demo app disables persistence.
