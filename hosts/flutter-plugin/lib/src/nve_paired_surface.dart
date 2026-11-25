import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/material.dart';

import 'command_decoder.dart';
import 'event_codec.dart';
import 'flex_layout.dart';
import 'nve_core_abi.generated.dart';
import 'nve_pair_client.dart';
import 'nve_pair_protocol.dart';

typedef NvePairedSurfaceFrameBuilder = Widget Function(
  BuildContext context,
  int seq,
  Uint8List frameBytes,
);

/// Zero-cdylib live thin-client Flutter widget paired over `NVT1`/`NVD1` binary WebSockets.
///
/// Displays pairing URI/token controls, a live status & telemetry bar (RTT, FPS, RX throughput),
/// and renders incoming reconstructed `NVE1` `CommandBuffer` frames directly onto a Flutter
/// [Canvas] (while forwarding tap gestures back over `InputEvents = 11`).
class NvePairedSurface extends StatefulWidget {
  const NvePairedSurface({
    super.key,
    required this.client,
    this.initialUri = 'nve://pair?host=127.0.0.1&ws=8092&token=dev',
    this.initialToken = '',
    this.surfaceId = 1,
    this.onFrame,
    this.frameBuilder,
  });

  final NvePairClient client;
  final String initialUri;
  final String initialToken;
  final int surfaceId;
  final NvePairedFrameCallback? onFrame;
  final NvePairedSurfaceFrameBuilder? frameBuilder;

  @override
  State<NvePairedSurface> createState() => _NvePairedSurfaceState();
}

class _NvePairedSurfaceState extends State<NvePairedSurface> {
  late final TextEditingController _uriController;
  late final TextEditingController _tokenController;
  final NveRetainedTree _tree = NveRetainedTree();
  final NveFlexLayoutSolver _layoutSolver = NveFlexLayoutSolver();

  int _lastDecodedSeq = -1;
  String? _validationError;

  @override
  void initState() {
    super.initState();
    _uriController = TextEditingController(text: widget.initialUri);
    _tokenController = TextEditingController(text: widget.initialToken);
    widget.client.addListener(_onClientChanged);
    _syncLatestFrame();
  }

  @override
  void didUpdateWidget(covariant NvePairedSurface oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!identical(oldWidget.client, widget.client)) {
      oldWidget.client.removeListener(_onClientChanged);
      widget.client.addListener(_onClientChanged);
      _lastDecodedSeq = -1;
      _tree.reset();
      _syncLatestFrame();
    }
  }

  @override
  void dispose() {
    widget.client.removeListener(_onClientChanged);
    _uriController.dispose();
    _tokenController.dispose();
    super.dispose();
  }

  void _onClientChanged() {
    _syncLatestFrame();
    if (mounted) {
      setState(() {});
    }
  }

  void _syncLatestFrame() {
    final frame = widget.client.latestFrame;
    final seq = widget.client.latestFrameSeq;
    if (frame == null) {
      if (_lastDecodedSeq != -1) {
        _lastDecodedSeq = -1;
        _tree.reset();
      }
      return;
    }
    if (seq != _lastDecodedSeq) {
      _lastDecodedSeq = seq;
      _tree.applyCommandBuffer(frame);
      widget.onFrame?.call(seq, frame);
    }
  }

  bool get _isActiveConnection =>
      widget.client.state == NvePairConnectionState.connecting ||
      widget.client.state == NvePairConnectionState.paired ||
      widget.client.state == NvePairConnectionState.reconnecting;

  Color get _statusColor => switch (widget.client.state) {
        NvePairConnectionState.disconnected => Colors.grey,
        NvePairConnectionState.connecting ||
        NvePairConnectionState.reconnecting =>
          Colors.orange,
        NvePairConnectionState.paired => Colors.green,
        NvePairConnectionState.error => Colors.red,
      };

  Future<void> _toggleConnection() async {
    setState(() => _validationError = null);
    if (_isActiveConnection) {
      await widget.client.disconnect();
      return;
    }
    final fallback = _tokenController.text.trim();
    try {
      await widget.client.connect(
        _uriController.text,
        fallbackToken: fallback.isEmpty ? null : fallback,
      );
    } on NvePairProtocolException catch (err) {
      if (mounted) {
        setState(() => _validationError = err.message);
      }
    }
  }

  void _handleSurfaceTap(Offset localPosition) {
    if (!widget.client.isPaired) return;
    final hitNode = _hitTestNode(nveRootNodeId, localPosition) ?? nveRootNodeId;
    widget.client.sendInputEvents(<NveEvent>[
      NveEvent(
        surfaceId: widget.surfaceId,
        nodeId: hitNode,
        timestampUs: DateTime.now().microsecondsSinceEpoch,
        kind: NveEventKind.press,
        payload: NvePointPayload(x: localPosition.dx, y: localPosition.dy),
      ),
    ]);
  }

  int? _hitTestNode(int nodeId, Offset pos) {
    final node = _tree.node(nodeId);
    if (node == null || !node.hasComputedLayout) return null;
    if (!node.computedRect.contains(pos) && nodeId != nveRootNodeId) return null;
    for (var i = node.children.length - 1; i >= 0; i--) {
      final childHit = _hitTestNode(node.children[i], pos);
      if (childHit != null) return childHit;
    }
    return node.computedRect.contains(pos) ? nodeId : null;
  }

  @override
  Widget build(BuildContext context) {
    final tel = widget.client.telemetry;
    final rttLabel =
        tel.smoothedRttMs != null ? '${tel.smoothedRttMs!.toStringAsFixed(1)}ms' : '—';
    final rxLabel = tel.bytesPerSecRx >= 1024.0
        ? '${(tel.bytesPerSecRx / 1024.0).toStringAsFixed(1)} KiB/s'
        : '${tel.bytesPerSecRx.toStringAsFixed(0)} B/s';

    return Padding(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          Row(
            children: <Widget>[
              Container(
                width: 9,
                height: 9,
                decoration: BoxDecoration(color: _statusColor, shape: BoxShape.circle),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  widget.client.stateLabel,
                  style: const TextStyle(fontSize: 12, fontWeight: FontWeight.w600),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (widget.client.isPaired) ...<Widget>[
                OutlinedButton(
                  key: const Key('nve_pair_keyframe_button'),
                  onPressed: () => widget.client.requestKeyframe(),
                  child: const Text('Keyframe'),
                ),
                const SizedBox(width: 8),
              ],
              FilledButton(
                key: const Key('nve_pair_connect_button'),
                onPressed: () => unawaited(_toggleConnection()),
                child: Text(_isActiveConnection ? 'Disconnect' : 'Connect'),
              ),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: <Widget>[
              Expanded(
                flex: 3,
                child: TextField(
                  key: const Key('nve_pair_uri_field'),
                  controller: _uriController,
                  decoration: const InputDecoration(
                    isDense: true,
                    border: OutlineInputBorder(),
                    labelText: 'Pairing URI',
                  ),
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                flex: 1,
                child: TextField(
                  key: const Key('nve_pair_token_field'),
                  controller: _tokenController,
                  decoration: const InputDecoration(
                    isDense: true,
                    border: OutlineInputBorder(),
                    labelText: 'Token',
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 8),
          Container(
            key: const Key('nve_pair_telemetry_pill'),
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
            decoration: BoxDecoration(
              color: Colors.blueGrey.withValues(alpha: 0.12),
              borderRadius: BorderRadius.circular(8),
            ),
            child: Wrap(
              spacing: 12,
              runSpacing: 4,
              children: <Widget>[
                Text('RTT $rttLabel', style: const TextStyle(fontSize: 11)),
                Text('FPS ${tel.fps.toStringAsFixed(0)}', style: const TextStyle(fontSize: 11)),
                Text('RX $rxLabel', style: const TextStyle(fontSize: 11)),
                Text('Frames ${tel.framesReceived}', style: const TextStyle(fontSize: 11)),
                Text(
                  'Ratio ${tel.compressionRatio.toStringAsFixed(1)}x',
                  style: const TextStyle(fontSize: 11),
                ),
              ],
            ),
          ),
          if (_validationError != null) ...<Widget>[
            const SizedBox(height: 6),
            Text(
              _validationError!,
              key: const Key('nve_pair_error_text'),
              style: const TextStyle(color: Colors.red, fontSize: 12),
            ),
          ],
          const SizedBox(height: 10),
          Expanded(child: _buildSurfaceBody(context)),
        ],
      ),
    );
  }

  Widget _buildSurfaceBody(BuildContext context) {
    final frame = widget.client.latestFrame;
    if (frame == null) {
      return DecoratedBox(
        decoration: BoxDecoration(
          border: Border.all(color: Colors.grey.withValues(alpha: 0.35)),
          borderRadius: BorderRadius.circular(10),
        ),
        child: Center(
          child: Text(
            widget.client.isPaired
                ? 'Waiting for first NVD1 keyframe…'
                : 'Not paired — enter an nve://pair URI to connect',
            style: const TextStyle(fontSize: 13, color: Colors.grey),
          ),
        ),
      );
    }

    if (widget.frameBuilder != null) {
      return widget.frameBuilder!(context, widget.client.latestFrameSeq, frame);
    }

    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final size = Size(
          constraints.maxWidth.isFinite ? constraints.maxWidth : 320,
          constraints.maxHeight.isFinite ? constraints.maxHeight : 200,
        );
        _layoutSolver.layout(_tree, size);
        return GestureDetector(
          key: const Key('nve_pair_surface_canvas'),
          behavior: HitTestBehavior.opaque,
          onTapUp: (TapUpDetails d) => _handleSurfaceTap(d.localPosition),
          child: CustomPaint(
            size: size,
            painter: _NvePairedTreePainter(tree: _tree),
          ),
        );
      },
    );
  }
}

class _NvePairedTreePainter extends CustomPainter {
  _NvePairedTreePainter({required this.tree});

  final NveRetainedTree tree;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    _paintNode(canvas, nveRootNodeId);
    canvas.restore();
  }

  void _paintNode(Canvas canvas, int nodeId) {
    final node = tree.node(nodeId);
    if (node == null || !node.hasComputedLayout) return;
    final rect = node.computedRect;
    final radius = math.max(0.0, node.style.borderRadius);

    if (rect.width > 0 && rect.height > 0 && node.style.background.a > 0) {
      final paint = Paint()
        ..color = node.style.background.toFlutterColor()
        ..style = PaintingStyle.fill;
      if (radius > 0) {
        canvas.drawRRect(RRect.fromRectAndRadius(rect, Radius.circular(radius)), paint);
      } else {
        canvas.drawRect(rect, paint);
      }
    }

    if (node.isTextLeaf && node.text != null && node.text!.isNotEmpty) {
      final fontSize = node.style.fontSize > 0 ? node.style.fontSize : 14.0;
      final painter = TextPainter(
        text: TextSpan(
          text: node.text!,
          style: TextStyle(
            color: node.style.textColor.a > 0
                ? node.style.textColor.toFlutterColor()
                : Colors.black,
            fontSize: fontSize,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout(maxWidth: math.max(0.0, rect.width));
      painter.paint(canvas, rect.topLeft);
      painter.dispose();
    }

    for (final childId in node.children) {
      _paintNode(canvas, childId);
    }
  }

  @override
  bool shouldRepaint(covariant _NvePairedTreePainter oldDelegate) => true;
}
