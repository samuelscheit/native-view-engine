# `nve_flutter` — Native View Engine Host Plugin for Flutter

Embed any Rust **Native View Engine (NVE)** component inside a Flutter app with **zero-copy 120fps Skia texture compositing** (`NveTextureHostWidget`) or **native Fabric platform views** (`NvePlatformViewWidget`), plus direct `dart:ffi` access to modular `nve_get_interface` and shared-state streaming via `NveChannelClient`.

## Architecture

```
Flutter Widget Tree
  ├── NveTextureHostWidget  ──► FlutterTextureRegistry (IOSurface / AHardwareBuffer / DXGI / DMA-BUF)
  │                             └── 120fps zero-copy Skia compositing on iOS, Android, macOS, Windows, Linux
  ├── NvePlatformViewWidget ──► UiKitView (iOS) / AndroidView (Android) ──► NVE Fabric Bridge
  ├── NveAppInstance        ──► Direct dart:ffi bindings to modular nve_get_interface (`NveSurfaceApi`, `NveLayoutApi`, `NveComponentApi`, `NveDevApi`)
  └── NveChannelClient      ──► Dart Stream<NveChannelMessage> ◄──► NveChannelBus (theme, locale, auth, nav)
```

### Modular C ABI (`nve_get_interface`)

- **`NveSurfaceApi` (`1`)**: `create`, `destroy`, `render`, `dispatch_events`.
- **`NveLayoutApi` (`2`)**: zero-serialization `measure(handle, NveSizeConstraints) -> NveIntrinsicSize` and `hit_test(handle, x, y, pointer_phase) -> NveGestureDisposition`.
- **`NveComponentApi` (`3`)**: `set_props`, `drain_emitted_events`.
- **`NveDevApi` (`4`)**: `apply_hot_patch`, `snapshot_state`, `restore_state` (optional; may be NULL in release builds).

## Usage Example (`NveCounterCard` inside Flutter)

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

## Codegen Integration

Run `nve-codegen` (`export_flutter`) on any `NveExportSpec` to generate a strongly typed Dart `StatefulWidget` wrapping `NveTextureHostWidget` (for Skia) or `NvePlatformViewWidget` (for Fabric) with typed prop serialization, typed event callbacks, and typed controller commands.
