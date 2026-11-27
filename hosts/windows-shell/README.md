# Windows host shell (`hosts/windows-shell`)

Native Win32 + Direct2D / DirectWrite host for NVE apps, built on the shared C++ host stack
(`cxx/cmake/NveHostStack.cmake`: `nve::host_cxx` + `libnve_runtime` + `nve::canvas_cxx`). It reads
the runtime's zero-copy `NveRuntimeSceneView` through `RetainedTree` / `SceneTables`.

## Layers

| Files | Platform | Role |
|---|---|---|
| `NveWinVisualTree.{hpp,cpp}` | every OS | Retained model. `RetainedTree` mirrors the scene O(changed) into owned `WinNodeVisual`s: visual style, box shadows, inherited typography (`TextPaint`), text, images (URL / bytes), `nve.canvas` draw lists, text-input rows, scroll offsets, interaction, ARIA semantics, links. Also paint-order walk (z-index, scroll shift, clips, visibility, opacity), hit testing, image-fit and shadow geometry. |
| `NveWinHost.{hpp,cpp}` | every OS | One runtime surface (`HostSession`): in-process app, `cdylib` with state-preserving hot reload, or paired `nve dev`. Also app-state files, props / hot patches, and input: press / link at a point, wheel → `scroll_by` + `Scroll` (edge chaining), hover `Pointer*`, cursors, text / submit / focus. On Windows it installs the DirectWrite measurer. |
| `NveWinWindowManager.{hpp,cpp}` | every OS | Multi-window lifecycle: modal blockers, close veto, cascade close, quit on last close. Uses headless `NveWinHost`s off Windows. |
| `NveWinTextMeasurer.hpp` | Windows | DirectWrite measurer (`NveRuntimeMeasureTextFn`) and the `IDWriteTextLayout`s the painter draws: family, size, weight, style, wrapping, alignment, decoration, letter spacing, line height, max lines. |
| `NveWinD2DRenderer.cpp`, `NveWinD2DMedia.cpp` | Windows | Direct2D painter: shadows, rounded backgrounds and clips, borders, opacity layers, images, canvas `NVE_DRAW1` lists, text. Also the WIC image cache (`data:` / `file://` / paths / inline bytes) and headless BMP export. |
| `NveWinHostWindow.{hpp,cpp}`, `NveWinTextInputs.{hpp,cpp}` | Windows | Per-monitor-DPI window. Runtime wakes are posted (`WM_APP_WAKE`) and dropped once the window is destroyed. Native `EDIT` overlays for text inputs (`host::ControlledText`). Cursors, hover tracking, links open via `ShellExecuteW`. |
| `NveRnwComponentView.{hpp,cpp}` | Windows + RNW | Optional react-native-windows 0.83+ Composition Fabric component on top of `NveWinHost`. Built only as `nve_rnw_component` (`-DNVE_WITH_RNW_FABRIC=ON`). The host never depends on it. |

## Build & test

On every OS (macOS / Linux run the portable model, controller and window-manager suites):

```bash
cmake -S hosts/windows-shell -B build/windows-shell -G Ninja
cmake --build build/windows-shell
ctest --test-dir build/windows-shell --output-on-failure
```

On Windows (MSVC + Ninja), the same commands also build `nve_windows_shell.exe`. The test then adds
the Direct2D / DirectWrite / WIC / live-window suite. `NVE_TEST_KITCHEN_SINK_DYLIB=<path to
nve_example_kitchen_sink.dll>` runs the live Rust `cdylib` test. React-native-windows component:
add `-DNVE_WITH_RNW_FABRIC=ON -DRNW_GENERATED_DIR=<rnw app>\Generated Files`.

## Run

```powershell
nve_windows_shell.exe                                  # built-in counter demo
nve_windows_shell.exe --dylib target\debug\nve_example_kitchen_sink.dll
nve_windows_shell.exe --pair nve://127.0.0.1:7878 --pair-token <token>
nve_windows_shell.exe --dylib app.dll --screenshot out.bmp --scale 2   # headless
```

App state persists across launches: `--state <file>`, `--no-state`, `--reset-state`, `--schema <ver>`,
`NVE_STATE_DISABLE=1`, `NVE_STATE_PATH=<file>`, `NVE_RESET_STATE=1`. Paired apps keep their state on
the server. When pairing or the dylib load fails, the shell runs the built-in counter with
persistence disabled. The failed app's state is neither restored nor overwritten.
