# `nve_flutter` — Native View Engine Host Plugin for Flutter

Embed any compiled Rust **Native View Engine (NVE)** app or island inside Flutter on macOS, Windows, Linux, iOS, and Android:

- **Path 1 — Direct `RenderBox` + Impeller/Skia `Canvas` (`NveCanvasView`)**: renders NVE's binary `CommandBuffer` straight onto Flutter's own `Canvas` via `dart:ffi` with **zero PlatformViews** and **zero native plugin boilerplate**.
- **Path 2 — GPU Texture Compositing (`NveTextureHostWidget`)**: zero-copy 120fps Skia compositing via `FlutterTextureRegistry`.
- **Path 3 — Native Fabric Platform Views (`NvePlatformViewWidget`)**: `UiKitView` / `AndroidView` embedding.
- **Direct FFI & Channels**: `NveAppInstance` (modular `nve_get_interface`) and `NveChannelClient` (`NveChannelBus` Dart `Stream`s).

## Architecture

```
Flutter Widget Tree
  ├── NveCanvasView         ──► Custom RenderBox + Impeller/Skia Canvas (zero PlatformViews)
  │                             └── dart:ffi -> NveRetainedTree -> NveFlexLayoutSolver (TextPainter) -> Canvas
  ├── NveTextureHostWidget  ──► FlutterTextureRegistry (IOSurface / AHardwareBuffer / DXGI / DMA-BUF)
  ├── NvePlatformViewWidget ──► UiKitView (iOS) / AndroidView (Android) ──► NVE Fabric Bridge
  ├── NveAppInstance        ──► Direct dart:ffi bindings to modular nve_get_interface
  └── NveChannelClient      ──► Dart Stream<NveChannelMessage> ◄──► NveChannelBus
```

### Modular C ABI (`nve_get_interface`)

- **`NveSurfaceApi` (`1`)**: `create`, `destroy`, `render`, `dispatch_events`.
- **`NveLayoutApi` (`2`)**: zero-serialization `measure(handle, NveSizeConstraints) -> NveIntrinsicSize` and `hit_test(handle, x, y, pointer_phase) -> NveGestureDisposition`.
- **`NveComponentApi` (`3`)**: `set_props`, `drain_emitted_events`.
- **`NveDevApi` (`4`)**: `apply_hot_patch`, `snapshot_state`, `restore_state` (optional; may be NULL in release builds).

## Quickstart: Direct `NveCanvasView` (Zero PlatformViews)

Build any NVE Rust cdylib (e.g. `cargo build -p nve-example-kitchen-sink`), then mount it directly on Flutter's Canvas:

```dart
import 'package:flutter/material.dart';
import 'package:nve_flutter/nve_canvas.dart';

class DirectCounterScreen extends StatefulWidget {
  const DirectCounterScreen({super.key, required this.libraryPath});
  final String libraryPath;

  @override
  State<DirectCounterScreen> createState() => _DirectCounterScreenState();
}

class _DirectCounterScreenState extends State<DirectCounterScreen> {
  late final NveCanvasController _controller;

  @override
  void initState() {
    super.initState();
    _controller = NveCanvasController.open(
      surfaceId: 1,
      libraryPath: widget.libraryPath,
    );
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: NveCanvasView(controller: _controller),
      floatingActionButton: FloatingActionButton(
        tooltip: 'State-preserving Hot Reload',
        onPressed: () => _controller.reloadLibrary(widget.libraryPath),
        child: const Icon(Icons.refresh),
      ),
    );
  }
}
```

See [example/main.dart](example/main.dart) for a complete app with runtime library switching, hot-reload, and frame/node counters.

## Texture / PlatformView Usage (`NveCounterCard`)

```dart
import 'package:flutter/material.dart';
import 'package:nve_flutter/nve_flutter.dart';

class CounterScreen extends StatefulWidget {
  const CounterScreen({super.key});

  @override
  State<CounterScreen> createState() => _CounterScreenState();
}

class _CounterScreenState extends State<CounterScreen> {
  final _controller = NveSurfaceController();
  final _channels = NveChannelClient();
  int _count = 0;

  @override
  void dispose() {
    _channels.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('NVE in Flutter')),
      body: Center(
        child: NveTextureHostWidget(
          componentName: 'NveCounterCard',
          vtableSymbol: 'nve_get_interface',
          controller: _controller,
          channelClient: _channels,
          width: 320,
          height: 180,
          props: <String, dynamic>{
            'title': 'Rust Counter',
            'count': _count,
          },
          onEvent: (NveEvent event) {
            if (event.name == 'onIncrement') {
              setState(() => _count += 1);
            }
          },
        ),
      ),
      floatingActionButton: FloatingActionButton(
        onPressed: () => _controller.dispatchCommand('reset'),
        child: const Icon(Icons.refresh),
      ),
    );
  }
}
```

## Verification

```bash
cargo build -p nve-example-kitchen-sink
(cd hosts/flutter-plugin && flutter analyze && flutter test)
```

## Codegen Integration

Run `nve-codegen` (`export_flutter`) on any `NveExportSpec` to generate a strongly typed Dart `StatefulWidget` wrapping `NveCanvasView`, `NveTextureHostWidget`, or `NvePlatformViewWidget` with typed prop serialization, typed event callbacks, and typed controller commands.
