# Windows Native Desktop & `react-native-windows` Fabric Host Shell (`hosts/windows-shell`)

Native Windows Win32 + Direct2D/DirectWrite desktop host shell and embeddable `react-native-windows` 0.83+/0.88+ Composition Fabric component host built on `cxx/nve-fabric-bridge` (`nve_host_runtime.hpp`).

## Components

1. **DirectWrite Text Measurer (`NveWinTextMeasurer.hpp`)**:
   RAII `nve::win::DirectWriteTextMeasurer` backed by `IDWriteFactory`, cached Segoe UI `IDWriteTextFormat`, and `IDWriteTextLayout::GetMetrics`, plugged directly into `nve::NativeHostSession::set_text_measurer(...)`.
2. **Retained Visual Tree & RNW Fabric Host (`NveRnwComponentView.hpp` / `NveRnwComponentView.cpp`)**:
   - `nve::win::NveWinVisualTree`: reconciles `nve::RenderedNodeSnapshot` frames using `nve::RetainedViewReconciler`, computes z-ordered paint lists, and performs reverse z/tree interactive hit-testing.
   - `nve::win::NveRnwComponentHost`: portable C++20 controller wrapping `nve::NativeHostSession` for props JSON, layout resize, press dispatch, outbound event forwarding, and state-preserving Tier-2 `.dll` hot-reload.
   - Under `-DNVE_WITH_RNW_FABRIC=ON`: `register_nve_rnw_component(...)` binds directly to modern `react-native-windows` Composition Fabric (`winrt/Microsoft.ReactNative.Composition.h` — `IReactPackageBuilderFabric`, `IReactCompositionViewComponentBuilder`, `ViewComponentView`, `ContentIslandComponentView`, `LayoutMetrics`).
3. **Standalone Win32 + Direct2D/DirectWrite Host (`NveWinHostWindow.hpp` / `NveWinHostWindow.cpp` / `main.cpp`)**:
   DPI-aware (`GetDpiForWindow` / `WM_DPICHANGED`) Win32 window rendering rounded rectangles, borders, opacity layers (`ID2D1Layer`), z-order, and DirectWrite text, with `WM_LBUTTONUP` click dispatch and headless `--screenshot <path.bmp>` export via `ID2D1DCRenderTarget`.

## Build & Test (Windows MSVC + Ninja)

```powershell
# Standalone build (requires only VS/MSVC + Windows SDK; zero RNW install needed):
cmake -S hosts/windows-shell -B hosts/windows-shell/build -G Ninja
cmake --build hosts/windows-shell/build
ctest --test-dir hosts/windows-shell/build --output-on-failure

# Build with React Native Yoga + JSI from a local react-native / rnw checkout:
cmake -S hosts/windows-shell -B hosts/windows-shell/build -G Ninja `
  -DRN_REACT_COMMON_DIR="C:\path\to\react-native\ReactCommon"
cmake --build hosts/windows-shell/build

# Run live Rust counter DLL test (builds nve_example_counter.dll first):
cargo build -p nve-example-counter
$env:NVE_TEST_COUNTER_DYLIB = (Resolve-Path target\debug\nve_example_counter.dll).Path
ctest --test-dir hosts/windows-shell/build --output-on-failure

# Launch standalone Win32/Direct2D preview window or render headless BMP screenshot:
.\hosts\windows-shell\build\nve_windows_shell.exe --dylib target\debug\nve_example_counter.dll
.\hosts\windows-shell\build\nve_windows_shell.exe --dylib target\debug\nve_example_counter.dll --screenshot counter.bmp
```

## Embedding in `react-native-windows` 0.83+/0.88+ Composition Fabric

Enable `-DNVE_WITH_RNW_FABRIC=ON` and point `-DRNW_GENERATED_DIR` at your RNW app's generated C++/WinRT headers directory (containing `winrt/Microsoft.ReactNative.Composition.h`):

```powershell
cmake -S hosts/windows-shell -B hosts/windows-shell/build-rnw -G Ninja `
  -DNVE_WITH_RNW_FABRIC=ON `
  -DRNW_GENERATED_DIR="C:\path\to\rnw-app\Generated Files" `
  -DRN_REACT_COMMON_DIR="C:\path\to\react-native\ReactCommon"
cmake --build hosts/windows-shell/build-rnw
```

Then register your NVE Rust component inside your RNW `IReactPackageProvider::CreatePackage`:

```cpp
nve::win::NveRnwComponentRegistrationConfig cfg{};
cfg.component_name = L"NveCounterView";
cfg.default_dylib_path = "nve_example_counter.dll";
nve::win::register_nve_rnw_component(packageBuilder, std::move(cfg));
```
