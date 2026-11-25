import 'dart:async';

import 'package:flutter/services.dart';

import 'nve_bin_value.dart';

/// Imperative controller attached to [NveTextureHostWidget] or [NvePlatformViewWidget].
class NveSurfaceController {
  void Function(String name, Map<String, Object?> args)? _commandHandler;
  VoidCallback? _renderHandler;
  Uint8List? Function()? _snapshotHandler;

  bool get isAttached => _commandHandler != null;

  void attachHandlers({
    required void Function(String name, Map<String, Object?> args) onCommand,
    required VoidCallback onRender,
    required Uint8List? Function() onSnapshot,
  }) {
    _commandHandler = onCommand;
    _renderHandler = onRender;
    _snapshotHandler = onSnapshot;
  }

  void detachHandlers() {
    _commandHandler = null;
    _renderHandler = null;
    _snapshotHandler = null;
  }

  /// Dispatches an imperative component command (matching `codegenNativeCommands` / `NveExportSpec.commands`).
  void dispatchCommand(String name, [Map<String, Object?> args = const <String, Object?>{}]) {
    _commandHandler?.call(name, args);
  }

  /// Requests an immediate frame render from the underlying NVE surface.
  void requestRender() {
    _renderHandler?.call();
  }

  /// Captures a state snapshot if backed by a direct FFI instance.
  Uint8List? snapshotState() => _snapshotHandler?.call();
}

/// Message delivered across [NveChannelClient] and Rust's `NveChannelBus`.
class NveChannelMessage {
  const NveChannelMessage({
    required this.channelId,
    required this.topic,
    required this.seq,
    required this.payload,
    this.senderSurfaceId,
    this.isBinValue = true,
  });

  final int channelId;
  final String topic;
  final int seq;
  final Uint8List payload;
  final int? senderSurfaceId;
  final bool isBinValue;

  /// Parses [payload] as a zero-copy [NveBinValueView], or returns `null` if raw/non-BinValue bytes.
  NveBinValueView? decodeBinValue() {
    if (payload.isEmpty) return null;
    try {
      return NveBinValueView.parse(payload);
    } on NveBinValueException {
      return null;
    }
  }
}

/// Bridges Dart `Stream`s to `NveChannelBus` (`theme`, `locale`, `auth`, `navigation`, custom topics).
class NveChannelClient {
  NveChannelClient({
    MethodChannel? methodChannel,
    EventChannel? eventChannel,
  })  : _methodChannel = methodChannel ?? const MethodChannel('dev.nve/channel_bus'),
        _eventChannel = eventChannel ?? const EventChannel('dev.nve/channel_bus/events');

  static const String topicTheme = 'theme';
  static const String topicLocale = 'locale';
  static const String topicAuth = 'auth';
  static const String topicNavigation = 'navigation';

  final MethodChannel _methodChannel;
  final EventChannel _eventChannel;
  final Map<String, StreamController<NveChannelMessage>> _topicControllers =
      <String, StreamController<NveChannelMessage>>{};
  final Map<String, NveChannelMessage> _retained = <String, NveChannelMessage>{};
  final Map<String, int> _topicIds = <String, int>{
    topicTheme: 1,
    topicLocale: 2,
    topicAuth: 3,
    topicNavigation: 4,
  };
  int _nextCustomId = 100;
  int _seq = 0;
  StreamSubscription<dynamic>? _platformSub;

  /// Returns a broadcast stream of messages for `topic`, immediately replaying the latest
  /// retained shared-state value when `replayRetained` is `true`.
  Stream<NveChannelMessage> subscribe(String topic, {bool replayRetained = true}) {
    _ensurePlatformListener();
    final ctrl = _topicControllers.putIfAbsent(
      topic,
      () => StreamController<NveChannelMessage>.broadcast(),
    );
    _methodChannel.invokeMethod<void>('subscribe', <String, dynamic>{'topic': topic}).catchError(
      (_) {},
    );

    if (replayRetained && _retained.containsKey(topic)) {
      final latest = _retained[topic]!;
      return _prependStream(latest, ctrl.stream);
    }
    return ctrl.stream;
  }

  /// Publishes raw bytes to `topic` across both local Dart listeners and the native `NveChannelBus`.
  Future<void> publish(
    String topic,
    Uint8List payload, {
    bool retain = false,
    bool isBinValue = false,
    int? senderSurfaceId,
  }) async {
    final id = _topicIds.putIfAbsent(topic, () => _nextCustomId++);
    _seq += 1;
    final msg = NveChannelMessage(
      channelId: id,
      topic: topic,
      seq: _seq,
      payload: payload,
      senderSurfaceId: senderSurfaceId,
      isBinValue: isBinValue,
    );
    if (retain) {
      _retained[topic] = msg;
    }
    _topicControllers[topic]?.add(msg);
    try {
      await _methodChannel.invokeMethod<void>('publish', <String, dynamic>{
        'channelId': id,
        'topic': topic,
        'payload': payload,
        'retain': retain,
        'isBinValue': isBinValue,
        if (senderSurfaceId != null) 'senderSurfaceId': senderSurfaceId,
      });
    } on MissingPluginException {
      // Allow pure-Dart / widget-test usage when native plugin host is not registered.
    }
  }

  /// Publishes a compact binary `NveBinValue` payload to `topic` (zero JSON).
  Future<void> publishValue(
    String topic,
    Object? value, {
    bool retain = false,
    int? senderSurfaceId,
  }) {
    final bytes = NveBinValueEncoder.encode(value);
    return publish(
      topic,
      bytes,
      retain: retain,
      isBinValue: true,
      senderSurfaceId: senderSurfaceId,
    );
  }

  /// Returns the last retained message for `topic`, if any.
  NveChannelMessage? retained(String topic) => _retained[topic];

  Future<void> unsubscribe(String topic) async {
    final ctrl = _topicControllers.remove(topic);
    await ctrl?.close();
    try {
      await _methodChannel.invokeMethod<void>('unsubscribe', <String, dynamic>{'topic': topic});
    } on MissingPluginException {
      // Ignored in tests/headless mode.
    }
  }

  Future<void> dispose() async {
    await _platformSub?.cancel();
    _platformSub = null;
    for (final ctrl in _topicControllers.values) {
      await ctrl.close();
    }
    _topicControllers.clear();
  }

  void _ensurePlatformListener() {
    if (_platformSub != null) return;
    try {
      _platformSub = _eventChannel.receiveBroadcastStream().listen(
        (dynamic raw) {
          if (raw is! Map) return;
          final topic = raw['topic'] as String?;
          if (topic == null) return;
          final bytes = raw['payload'] is Uint8List ? raw['payload'] as Uint8List : Uint8List(0);
          final msg = NveChannelMessage(
            channelId: (raw['channelId'] as int?) ??
                _topicIds.putIfAbsent(topic, () => _nextCustomId++),
            topic: topic,
            seq: (raw['seq'] as int?) ?? (++_seq),
            payload: bytes,
            senderSurfaceId: raw['senderSurfaceId'] as int?,
            isBinValue: (raw['isBinValue'] as bool?) ?? true,
          );
          if (raw['retained'] == true) {
            _retained[topic] = msg;
          }
          _topicControllers[topic]?.add(msg);
        },
        onError: (_) {},
      );
    } on MissingPluginException {
      // Pure-Dart local bus fallback.
    }
  }

  static Stream<NveChannelMessage> _prependStream(
    NveChannelMessage first,
    Stream<NveChannelMessage> rest,
  ) async* {
    yield first;
    yield* rest;
  }
}
