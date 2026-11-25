import 'dart:io' show Platform;

import 'package:flutter/material.dart';
import 'package:nve_flutter/nve_canvas.dart';

/// Runnable Flutter demo embedding any compiled NVE Rust app (`lib*.dylib` / `*.so` / `*.dll`)
/// directly on Flutter's Impeller/Skia Canvas via [NveCanvasView] with zero PlatformViews.
///
/// Build the kitchen-sink cdylib first from the repo root:
/// ```bash
/// cargo build -p nve-example-kitchen-sink
/// ```
void main() {
  runApp(const NveCanvasExampleApp());
}

String _defaultLibraryPath() {
  if (Platform.isMacOS) return '../../target/debug/libnve_example_kitchen_sink.dylib';
  if (Platform.isWindows) return r'..\..\target\debug\nve_example_kitchen_sink.dll';
  return '../../target/debug/libnve_example_kitchen_sink.so';
}

class NveCanvasExampleApp extends StatelessWidget {
  const NveCanvasExampleApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'NVE Direct Canvas Host',
      theme: ThemeData.dark(useMaterial3: true),
      home: const NveCanvasHostScreen(),
    );
  }
}

class NveCanvasHostScreen extends StatefulWidget {
  const NveCanvasHostScreen({super.key});

  @override
  State<NveCanvasHostScreen> createState() => _NveCanvasHostScreenState();
}

class _NveCanvasHostScreenState extends State<NveCanvasHostScreen> {
  late final TextEditingController _pathController;
  NveCanvasController? _controller;
  String? _loadError;

  @override
  void initState() {
    super.initState();
    _pathController = TextEditingController(text: _defaultLibraryPath());
    _openLibrary(_pathController.text);
  }

  @override
  void dispose() {
    _controller?.removeListener(_onControllerUpdate);
    _controller?.dispose();
    _pathController.dispose();
    super.dispose();
  }

  void _onControllerUpdate() {
    if (mounted) {
      setState(() {});
    }
  }

  void _openLibrary(String path) {
    final trimmed = path.trim();
    if (trimmed.isEmpty) return;

    try {
      final existing = _controller;
      if (existing != null && !existing.isDisposed) {
        existing.reloadLibrary(trimmed);
        setState(() => _loadError = null);
        return;
      }

      final next = NveCanvasController.open(
        surfaceId: 1,
        libraryPath: trimmed,
      )..addListener(_onControllerUpdate);
      next.renderNextFrame();
      setState(() {
        _controller = next;
        _loadError = null;
      });
    } catch (err) {
      setState(() => _loadError = err.toString());
    }
  }

  @override
  Widget build(BuildContext context) {
    final ctrl = _controller;

    return Scaffold(
      backgroundColor: const Color(0xFF020617),
      appBar: AppBar(
        title: const Text('NVE Direct Canvas Host (Zero PlatformViews)'),
        backgroundColor: const Color(0xFF0F172A),
      ),
      body: Column(
        children: <Widget>[
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
            color: const Color(0xFF1E293B),
            child: Row(
              children: <Widget>[
                Expanded(
                  child: TextField(
                    controller: _pathController,
                    decoration: const InputDecoration(
                      labelText: 'NVE Module (.dylib / .so / .dll)',
                      border: OutlineInputBorder(),
                      isDense: true,
                    ),
                    onSubmitted: _openLibrary,
                  ),
                ),
                const SizedBox(width: 12),
                FilledButton.icon(
                  onPressed: () => _openLibrary(_pathController.text),
                  icon: const Icon(Icons.refresh, size: 18),
                  label: Text(ctrl == null ? 'Load' : 'Hot Reload'),
                ),
                const SizedBox(width: 16),
                if (ctrl != null)
                  Text(
                    'Frames: ${ctrl.frameCount}  •  Nodes: ${ctrl.nodeCount}',
                    style: const TextStyle(color: Color(0xFF38BDF8), fontSize: 13),
                  ),
              ],
            ),
          ),
          if (_loadError != null)
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(12),
              color: const Color(0xFF7F1D1D),
              child: Text(
                _loadError!,
                style: const TextStyle(color: Colors.white, fontSize: 12),
              ),
            ),
          Expanded(
            child: ctrl != null
                ? NveCanvasView(
                    controller: ctrl,
                    backgroundColor: const Color(0xFF0F172A),
                  )
                : const Center(
                    child: Text(
                      'Enter the path to a compiled NVE cdylib and press Load.',
                      style: TextStyle(color: Colors.white70),
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}
