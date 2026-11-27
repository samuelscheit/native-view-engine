# NVE Qt 6 Host Shell (`hosts/qt-shell`)

Native **Qt 6** desktop host shell for Native View Engine, offering both a **GPU-composited Qt Quick (`QQuickItem` / Scene Graph `QSGNode`)** adapter and a retained **`QWidget`** adapter built directly on the shared C++20 runtime (`cxx/nve-fabric-bridge/include/nve_host_runtime.hpp`).

## Architecture

- **`NveQtQuickItem`** (`NveQtQuickItem.hpp` / `NveQtQuickItem.cpp`, enabled automatically when `Qt6::Quick` is found):
  - `QQuickItem` (`QML_ELEMENT` / `NveSurface` in QML module `Nve 1.0`) rendering NVE's flat 64-byte `NveSceneView` (`nve::SceneTableBuilder`) directly onto Qt 6's hardware-accelerated RHI Scene Graph (Metal on macOS, Vulkan/OpenGL on Linux, D3D11/12 on Windows, Software for headless CI) with zero widget overhead.
  - Retained `QSGNode` tree with `QSGTransformNode`, `QSGOpacityNode`, `QSGClipNode` (hardware scissor clip for rectangular bounds, stencil geometry clip for rounded rects), portable `DrawTriangles` rounded-rect & border `QSGGeometryNode` / `QSGSimpleRectNode`, and generation-tracked per-node HiDPI `QSGTexture` text caching (`QTextLayout` + `QPainter`) so unchanged text nodes never re-rasterize.
  - Zero-copy `NveDirectEvent` dispatch for press, wheel scroll, focus/blur, and text input, plus live `geometryChange` relayout and atomic state-preserving `hotReload`.
- **`NveQtWidget`** (`NveQtWidget.hpp` / `NveQtWidget.cpp`):
  - Embeddable `QWidget` composing `nve::NativeHostSession` and `nve::RetainedViewReconciler<QWidget*, QtViewAdapter>`.
  - Custom antialiased `NveQtBoxWidget`, `QLabel`, `QScrollArea`, and `QLineEdit`.
- **`NveQtCommon.hpp`**: Shared `QColor`, `QFont`, and `QFontMetricsF` text measurement helpers used by both adapters.
- **`nve_qt_shell`** (`main.cpp`): Standalone preview window (`--dylib <path>`, or `--quick <dylib>` for a live GPU-accelerated `QQuickView`).

## Build, Run & Test

```bash
# 1. Build the example Kitchen Sink cdylib
cargo build -p nve-example-kitchen-sink

# 2. Configure & build Qt 6 host shell (optionally pass -DRN_REACT_COMMON_DIR=... for Yoga + JSI)
cmake -S hosts/qt-shell -B hosts/qt-shell/build -G Ninja \
  -DRN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon
cmake --build hosts/qt-shell/build

# 3. Run CTest (runs both QWidget and Qt Quick Scene Graph suites offscreen)
ctest --test-dir hosts/qt-shell/build --output-on-failure

# 4. Launch GPU-accelerated Qt Quick preview window (or omit --quick for QWidget mode)
./hosts/qt-shell/build/nve_qt_shell --quick target/debug/libnve_example_kitchen_sink.dylib
```
