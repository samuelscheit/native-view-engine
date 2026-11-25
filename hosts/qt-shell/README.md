# NVE Qt 6 Host Shell (`hosts/qt-shell`)

Native **Qt 6 (`QWidget`)** desktop host shell for Native View Engine, built directly on the shared C++20 runtime (`cxx/nve-fabric-bridge/include/nve_host_runtime.hpp`).

## Architecture

- **`NveQtWidget`** (`NveQtWidget.hpp` / `NveQtWidget.cpp`): Embeddable `QWidget` composing `nve::NativeHostSession` and `nve::RetainedViewReconciler<QWidget*, QtViewAdapter>`.
  - Native `QFontMetricsF` text measurement matching `QLabel` logical pixel sizing 1:1.
  - Custom antialiased `NveQtBoxWidget` (`QPainter::drawRoundedRect`) for `NVE_COMP_VIEW`, `NVE_COMP_PRESSABLE`, and wrapper `NVE_COMP_TEXT` containers without QSS cascade side effects.
  - Native `QLabel` (leaf `NVE_COMP_TEXT` with `WA_TransparentForMouseEvents`), `QScrollArea` (`NVE_COMP_SCROLL_VIEW`), and `QLineEdit` (`NVE_COMP_TEXT_INPUT`).
  - Live resize relayout, mouse press routing, outbound `nve::DecodedEvent` callbacks, and atomic state-preserving `reloadDylib`.
- **`nve_qt_shell`** (`main.cpp`): Standalone preview window (`--dylib <path>`).

## Build, Run & Test

```bash
# 1. Build the example counter cdylib
cargo build -p nve-example-counter

# 2. Configure & build Qt 6 host shell (optionally pass -DRN_REACT_COMMON_DIR=... for Yoga + JSI)
cmake -S hosts/qt-shell -B hosts/qt-shell/build -G Ninja \
  -DRN_REACT_COMMON_DIR=/path/to/react-native/packages/react-native/ReactCommon
cmake --build hosts/qt-shell/build

# 3. Run CTest (uses QT_QPA_PLATFORM=offscreen automatically)
ctest --test-dir hosts/qt-shell/build --output-on-failure

# 4. Run full end-to-end test against the Rust counter cdylib (with optional PNG export)
QT_QPA_PLATFORM=offscreen \
  NVE_TEST_COUNTER_DYLIB="$PWD/target/debug/libnve_example_counter.dylib" \
  NVE_TEST_SCREENSHOT_PATH="/tmp/nve_qt6_counter_verify.png" \
  ./hosts/qt-shell/build/nve_qt_shell_test

# 5. Launch interactive desktop preview window
./hosts/qt-shell/build/nve_qt_shell --dylib target/debug/libnve_example_counter.dylib
```
