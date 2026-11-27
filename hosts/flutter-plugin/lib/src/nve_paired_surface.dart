/// Live-pairing surface: pairs a [NveCanvasController] with an `nve dev` server (`nve://pair?…`)
/// through `libnve_runtime`'s built-in pair client and renders the mirrored app with
/// [NveCanvasView]; input flows back through the same runtime.
library;

import 'package:flutter/material.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'nve_canvas_controller.dart';
import 'nve_canvas_view.dart';

class NvePairedSurface extends StatefulWidget {
  const NvePairedSurface({
    super.key,
    required this.controller,
    this.initialUri = 'nve://pair?host=127.0.0.1&ws=8092',
    this.initialToken = '',
    this.backgroundColor,
  });

  final NveCanvasController controller;
  final String initialUri;
  final String initialToken;
  final Color? backgroundColor;

  @override
  State<NvePairedSurface> createState() => _NvePairedSurfaceState();
}

class _NvePairedSurfaceState extends State<NvePairedSurface> {
  late final TextEditingController _uri = TextEditingController(text: widget.initialUri);
  late final TextEditingController _token = TextEditingController(text: widget.initialToken);
  String? _error;

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_refresh);
  }

  @override
  void didUpdateWidget(covariant NvePairedSurface oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!identical(oldWidget.controller, widget.controller)) {
      oldWidget.controller.removeListener(_refresh);
      widget.controller.addListener(_refresh);
    }
  }

  @override
  void dispose() {
    widget.controller.removeListener(_refresh);
    _uri.dispose();
    _token.dispose();
    super.dispose();
  }

  void _refresh() {
    if (mounted) {
      setState(() {});
    }
  }

  void _toggle() {
    final c = widget.controller;
    setState(() {
      _error = null;
      try {
        if (c.isPairConnected) {
          c.unpair();
        } else {
          c.pair(_uri.text.trim(), token: _token.text.trim());
        }
      } on RuntimeException catch (e) {
        _error = e.message;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.controller;
    final connected = !c.isDisposed && c.isPairConnected;
    final metrics = c.isDisposed ? null : c.session.metrics();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.all(8),
          child: Row(children: [
            Expanded(
              flex: 3,
              child: TextField(
                controller: _uri,
                enabled: !connected,
                decoration: const InputDecoration(labelText: 'Pair URI', isDense: true),
              ),
            ),
            const SizedBox(width: 8),
            Expanded(
              child: TextField(
                controller: _token,
                enabled: !connected,
                obscureText: true,
                decoration: const InputDecoration(labelText: 'Token', isDense: true),
              ),
            ),
            const SizedBox(width: 8),
            FilledButton(onPressed: _toggle, child: Text(connected ? 'Disconnect' : 'Pair')),
          ]),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: Row(children: [
            Icon(Icons.circle, size: 10, color: connected ? Colors.green : Colors.grey),
            const SizedBox(width: 6),
            Expanded(
              child: Text(
                _error ??
                    (connected
                        ? 'Paired · frame ${metrics?.frameId ?? 0} · ${metrics?.liveNodes ?? 0} nodes · '
                            '${((metrics?.wireBytesRx ?? 0) / 1024).toStringAsFixed(1)} KiB received'
                        : 'Not paired'),
                style: TextStyle(color: _error == null ? null : Theme.of(context).colorScheme.error),
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ]),
        ),
        const SizedBox(height: 8),
        Expanded(child: NveCanvasView(controller: c, backgroundColor: widget.backgroundColor)),
      ],
    );
  }
}
