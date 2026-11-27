# `nve_flutter` — Native View Engine host for Flutter

Runs an NVE app on `libnve_runtime` and renders it on Flutter's own `Canvas` (zero PlatformViews)
through `package:nve_host` (`packages/core/host-dart`).

```
Flutter widget tree
  ├── NveCanvasView        RenderBox: layout at its size, paint, input, semantics
  │     └── NveCanvasController ──► RuntimeSession (dart:ffi) ──► libnve_runtime
  │                                    ├── attached app (nve_get_interface)
  │                                    ├── live pairing (nve://pair)
  │                                    └── NVE2 replay
  └── NvePairedSurface     pairing controls + NveCanvasView
```

- **Painting** reads the published scene tables directly (`NveScenePainter`): fills, borders,
  radius, opacity, visibility, overflow clip, `z_index`, transforms, inherited typography, text,
  text inputs, images, and scroll shifts. Module tables paint through `NveTablePainter`s
  (`NveCanvasTablePainter` for `NVE_DRAW1` draw lists).
- **Input**: pointer down/move/up and press go to the runtime's hit-test target; wheel and drag
  (past touch slop) scroll the deepest scroll container via `scroll_by`.
- **Semantics**: `Accessibility` roles/states/labels, text, inputs (masked when secure), images,
  and press targets, with taps routed back as presses.
- **Commands** (`NveCanvasController.sendCommand`) are root `Custom` events.

- **Runtime hooks**: `setProps` forwards host props (`set_props`), `onAppEvent` receives events
  the app emitted (`drain_emitted_events`, drained after every tick), and painters resolve image
  sources and module draw lists through `resources` (`resource_payload`, borrowed per scene read).

## Quickstart

```dart
final controller = NveCanvasController.open(
  app: ffi.DynamicLibrary.open('libnve_example_kitchen_sink.dylib'),
);
// ...
NveCanvasView(controller: controller);
```

See [example/main.dart](example/main.dart).

## Verification

```bash
cargo build -p nve-runtime -p nve-example-kitchen-sink
(cd hosts/flutter-plugin && flutter analyze && flutter test)
```

`scripts/check-all.sh` also analyzes a widget generated from
[test/fixtures/export_spec.json](test/fixtures/export_spec.json) against the plugin API.
