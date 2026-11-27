import 'dart:ffi' as ffi;
import 'dart:io' show Platform;

import 'package:flutter/material.dart';
import 'package:nve_flutter/nve_flutter.dart';

/// Runs a compiled NVE app (`nve_get_interface` cdylib) on `libnve_runtime`, rendered directly on
/// Flutter's Canvas by [NveCanvasView] (zero PlatformViews).
///
/// Build both libraries first from the repo root:
/// ```bash
/// cargo build -p nve-runtime -p nve-example-kitchen-sink
/// ```
void main() {
  runApp(const NveCanvasExampleApp());
}

String _targetLibrary(String crate) {
  if (Platform.isMacOS) return '../../target/debug/lib$crate.dylib';
  if (Platform.isWindows) return '..\\..\\target\\debug\\$crate.dll';
  return '../../target/debug/lib$crate.so';
}

class NveCanvasExampleApp extends StatelessWidget {
  const NveCanvasExampleApp({super.key});

  @override
  Widget build(BuildContext context) => MaterialApp(
        title: 'NVE Canvas Host',
        theme: ThemeData.dark(useMaterial3: true),
        home: const NveCanvasHostScreen(),
      );
}

class NveCanvasHostScreen extends StatefulWidget {
  const NveCanvasHostScreen({super.key});

  @override
  State<NveCanvasHostScreen> createState() => _NveCanvasHostScreenState();
}

class _NveCanvasHostScreenState extends State<NveCanvasHostScreen> {
  final TextEditingController _runtimePath = TextEditingController(text: _targetLibrary('nve_runtime'));
  final TextEditingController _appPath = TextEditingController(text: _targetLibrary('nve_example_kitchen_sink'));
  NveCanvasController? _controller;
  String? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _controller?.dispose();
    _runtimePath.dispose();
    _appPath.dispose();
    super.dispose();
  }

  void _refresh() {
    if (mounted) {
      setState(() {});
    }
  }

  /// Opens the runtime once, then (re)attaches the app library.
  void _load() {
    try {
      final ctrl = _controller ??= NveCanvasController.open(runtimeLibrary: _runtimePath.text.trim())
        ..addListener(_refresh);
      ctrl.attachApp(ffi.DynamicLibrary.open(_appPath.text.trim()));
      setState(() => _error = null);
    } on Object catch (e) {
      setState(() => _error = e.toString());
    }
  }

  @override
  Widget build(BuildContext context) {
    final ctrl = _controller;
    final metrics = ctrl == null || ctrl.isDisposed ? null : ctrl.session.metrics();
    return Scaffold(
      backgroundColor: const Color(0xFF020617),
      appBar: AppBar(title: const Text('NVE Canvas Host'), backgroundColor: const Color(0xFF0F172A)),
      body: Column(
        children: [
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
            color: const Color(0xFF1E293B),
            child: Row(
              children: [
                Expanded(child: _pathField(_runtimePath, 'libnve_runtime', enabled: ctrl == null)),
                const SizedBox(width: 12),
                Expanded(child: _pathField(_appPath, 'NVE app (.dylib / .so / .dll)')),
                const SizedBox(width: 12),
                FilledButton.icon(
                  onPressed: _load,
                  icon: const Icon(Icons.play_arrow, size: 18),
                  label: Text(ctrl == null ? 'Load' : 'Re-attach'),
                ),
                const SizedBox(width: 16),
                if (metrics != null)
                  Text('Frame ${metrics.frameId} • ${metrics.liveNodes} nodes',
                      style: const TextStyle(color: Color(0xFF38BDF8), fontSize: 13)),
              ],
            ),
          ),
          if (_error != null)
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(12),
              color: const Color(0xFF7F1D1D),
              child: Text(_error!, style: const TextStyle(color: Colors.white, fontSize: 12)),
            ),
          Expanded(
            child: ctrl != null
                ? NveCanvasView(controller: ctrl, backgroundColor: const Color(0xFF0F172A))
                : const Center(child: Text('Build libnve_runtime and an NVE app, then press Load.')),
          ),
        ],
      ),
    );
  }

  Widget _pathField(TextEditingController controller, String label, {bool enabled = true}) => TextField(
        controller: controller,
        enabled: enabled,
        decoration: InputDecoration(labelText: label, border: const OutlineInputBorder(), isDense: true),
        onSubmitted: (_) => _load(),
      );
}
