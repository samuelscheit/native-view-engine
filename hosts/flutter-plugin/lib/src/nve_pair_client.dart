import 'dart:async';
import 'dart:io' show WebSocket;

import 'package:flutter/foundation.dart';

import 'event_codec.dart';
import 'nve_pair_protocol.dart';

class NveAppliedFrame {
  const NveAppliedFrame({required this.seq, required this.frame, required this.kind});
  final int seq;
  final Uint8List frame;
  final NveDeltaPacketKind kind;
}

class NveDeltaReceiveOutcome {
  const NveDeltaReceiveOutcome({required this.appliedFrames, required this.ackPacket, required this.nackPackets, required this.gapDetected});
  final List<NveAppliedFrame> appliedFrames;
  final Uint8List? ackPacket;
  final List<Uint8List> nackPackets;
  final bool gapDetected;
}

class NveDeltaReceiver {
  NveDeltaReceiver({int maxWindow = NvePairProtocol.defaultSyncWindow})
      : _maxWindow = maxWindow.clamp(NvePairProtocol.minSyncWindow, NvePairProtocol.maxSyncWindow);
  int _maxWindow, _lastAppliedSeq = 0;
  Uint8List _lastAppliedFrame = Uint8List(0);
  final Map<int, Uint8List> _reorder = <int, Uint8List>{};

  int get maxWindow => _maxWindow;
  int get lastAppliedSeq => _lastAppliedSeq;
  Uint8List get lastAppliedFrame => _lastAppliedFrame;
  int get reorderBufferCount => _reorder.length;

  void reset({int? maxWindow}) {
    if (maxWindow != null) _maxWindow = maxWindow.clamp(NvePairProtocol.minSyncWindow, NvePairProtocol.maxSyncWindow);
    _lastAppliedSeq = 0; _lastAppliedFrame = Uint8List(0); _reorder.clear();
  }

  NveDeltaReceiveOutcome receivePacket(Uint8List packetBytes) {
    final pkt = NveDeltaPacket.parse(packetBytes); final hdr = pkt.header;
    if (hdr.kind == NveDeltaPacketKind.ack || hdr.kind == NveDeltaPacketKind.nack) {
      return const NveDeltaReceiveOutcome(appliedFrames: <NveAppliedFrame>[], ackPacket: null, nackPackets: <Uint8List>[], gapDetected: false);
    }
    if (hdr.seq <= _lastAppliedSeq) {
      return NveDeltaReceiveOutcome(appliedFrames: const <NveAppliedFrame>[], ackPacket: _buildAck(), nackPackets: const <Uint8List>[], gapDetected: false);
    }
    final applied = <NveAppliedFrame>[], nacks = <Uint8List>[]; var gap = false;
    final expected = _lastAppliedSeq + 1;
    if (hdr.kind == NveDeltaPacketKind.keyframe && hdr.seq >= expected) {
      final f = Uint8List.fromList(pkt.payload); _lastAppliedSeq = hdr.seq; _lastAppliedFrame = f;
      applied.add(NveAppliedFrame(seq: hdr.seq, frame: f, kind: NveDeltaPacketKind.keyframe));
      _reorder.removeWhere((seq, _) => seq <= _lastAppliedSeq); _drainReorder(applied);
    } else if (hdr.seq == expected) {
      final f = _applySingle(pkt); _lastAppliedSeq = hdr.seq; _lastAppliedFrame = f;
      applied.add(NveAppliedFrame(seq: hdr.seq, frame: f, kind: hdr.kind)); _drainReorder(applied);
    } else {
      gap = true;
      if (_reorder.length < _maxWindow) _reorder[hdr.seq] = Uint8List.fromList(packetBytes);
      final end = hdr.seq < expected + 8 ? hdr.seq : expected + 8;
      for (var m = expected; m < end; m++) {
        if (!_reorder.containsKey(m)) {
          final p = Uint8List(4); ByteData.sublistView(p).setUint32(0, m & 0xFFFFFFFF, Endian.little);
          nacks.add(NveDeltaPacket.encode(kind: NveDeltaPacketKind.nack, ackSeq: _lastAppliedSeq, payload: p));
        }
      }
    }
    return NveDeltaReceiveOutcome(appliedFrames: applied, ackPacket: _buildAck(), nackPackets: nacks, gapDetected: gap);
  }

  Uint8List _applySingle(NveDeltaPacket pkt) {
    final hdr = pkt.header;
    if (hdr.kind == NveDeltaPacketKind.keyframe) return Uint8List.fromList(pkt.payload);
    if (hdr.kind == NveDeltaPacketKind.xorRleDelta || hdr.kind == NveDeltaPacketKind.commandOpDelta) {
      if (hdr.baseSeq != _lastAppliedSeq) throw NvePairProtocolException(NvePairProtocolErrorCode.baseSequenceMismatch, 'expected $_lastAppliedSeq != ${hdr.baseSeq}');
      return hdr.kind == NveDeltaPacketKind.xorRleDelta
          ? NveDeltaCodec.decodeXorRleDelta(_lastAppliedFrame, pkt.payload)
          : NveDeltaCodec.decodeCommandOpDelta(_lastAppliedFrame, pkt.payload);
    }
    throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidDeltaKind, 'unexpected control');
  }

  void _drainReorder(List<NveAppliedFrame> applied) {
    while (true) {
      final raw = _reorder.remove(_lastAppliedSeq + 1);
      if (raw == null) break;
      final pkt = NveDeltaPacket.parse(raw); final f = _applySingle(pkt);
      _lastAppliedSeq = pkt.header.seq; _lastAppliedFrame = f;
      applied.add(NveAppliedFrame(seq: pkt.header.seq, frame: f, kind: pkt.header.kind));
    }
  }

  Uint8List _buildAck() {
    var sackLo = 0, sackHi = 0;
    for (final seq in _reorder.keys) {
      if (seq > _lastAppliedSeq) {
        final d = seq - _lastAppliedSeq;
        if (d >= 1 && d <= 32) { sackLo |= 1 << (d - 1); } else if (d >= 33 && d <= 64) { sackHi |= 1 << (d - 33); }
      }
    }
    final p = Uint8List(8); NveU64(sackLo & 0xFFFFFFFF, sackHi & 0xFFFFFFFF).writeLE(ByteData.sublistView(p), 0);
    return NveDeltaPacket.encode(kind: NveDeltaPacketKind.ack, ackSeq: _lastAppliedSeq, payload: p);
  }
}

enum NvePairConnectionState { disconnected, connecting, paired, reconnecting, error }

class NveSyncTelemetry {
  const NveSyncTelemetry({
    this.sessionId = 0,
    this.framesReceived = 0,
    this.keyframesReceived = 0,
    this.deltasReceived = 0,
    this.keyframeRequestsSent = 0,
    this.eventsSent = 0,
    this.wireBytesRx = 0,
    this.rawBytesRx = 0,
    this.wireBytesTx = 0,
    this.lastRttUs,
    this.smoothedRttUs,
    this.minRttUs,
    this.fps = 0.0,
    this.bytesPerSecRx = 0.0,
  });

  final int sessionId, framesReceived, keyframesReceived, deltasReceived, keyframeRequestsSent;
  final int eventsSent, wireBytesRx, rawBytesRx, wireBytesTx;
  final int? lastRttUs, smoothedRttUs, minRttUs;
  final double fps, bytesPerSecRx;

  double get compressionRatio => wireBytesRx == 0 ? 1.0 : rawBytesRx / wireBytesRx;
  double? get smoothedRttMs => smoothedRttUs != null ? smoothedRttUs! / 1000.0 : null;
}

abstract class NvePairSocket {
  Stream<Uint8List> get binaryMessages;
  Future<void> get done;
  void send(Uint8List bytes);
  Future<void> close();
}

typedef NvePairSocketConnector = Future<NvePairSocket> Function(Uri wsUri);

/// Default zero-dependency `dart:io` `WebSocket` adapter.
class NveIoWebSocketAdapter implements NvePairSocket {
  NveIoWebSocketAdapter._(this._socket) : _controller = StreamController<Uint8List>.broadcast() {
    _sub = _socket.listen(
      (dynamic data) {
        if (data is Uint8List) {
          _controller.add(data);
        } else if (data is List<int>) {
          _controller.add(Uint8List.fromList(data));
        }
      },
      onError: _controller.addError,
      onDone: _controller.close,
    );
  }

  static Future<NvePairSocket> connect(Uri wsUri) async =>
      NveIoWebSocketAdapter._(await WebSocket.connect(wsUri.toString()));

  final WebSocket _socket;
  final StreamController<Uint8List> _controller;
  late final StreamSubscription<dynamic> _sub;

  @override
  Stream<Uint8List> get binaryMessages => _controller.stream;
  @override
  Future<void> get done => _socket.done.then((_) {});
  @override
  void send(Uint8List bytes) {
    if (_socket.readyState == WebSocket.open) _socket.add(bytes);
  }

  @override
  Future<void> close() async {
    await _sub.cancel();
    await _socket.close();
    if (!_controller.isClosed) await _controller.close();
  }
}

typedef NvePairedFrameCallback = void Function(int seq, Uint8List frameBytes);

class NvePairClient extends ChangeNotifier {
  NvePairClient({
    NvePairSocketConnector? connector,
    this.autoReconnect = true,
    this.maxReconnectAttempts = 5,
    this.heartbeatInterval = const Duration(seconds: 1),
    int Function()? nowMicros,
    this.onFrame,
  })  : _connector = connector ?? NveIoWebSocketAdapter.connect,
        _nowMicros = nowMicros ?? _defaultNowMicros;

  static int _defaultNowMicros() => DateTime.now().microsecondsSinceEpoch;

  final NvePairSocketConnector _connector;
  final int Function() _nowMicros;
  final bool autoReconnect;
  final int maxReconnectAttempts;
  final Duration heartbeatInterval;
  NvePairedFrameCallback? onFrame;

  NvePairConnectionState _state = NvePairConnectionState.disconnected;
  String? _errorMessage;
  NvePairUri? _activeTarget;
  int _sessionId = 0, _latestFrameSeq = 0, _nonceCounter = 1, _reconnectAttempt = 0;
  Uint8List? _latestFrame;

  final NveDeltaReceiver _deltaReceiver = NveDeltaReceiver();
  NvePairSocket? _socket;
  StreamSubscription<Uint8List>? _socketSub;
  Timer? _heartbeatTimer, _reconnectTimer;
  NveU64? _pendingClientNonce;
  bool _manualDisconnect = false, _disposed = false;

  int _framesRx = 0, _keyframesRx = 0, _deltasRx = 0, _kfReqsTx = 0, _eventsTx = 0;
  int _wireRx = 0, _rawRx = 0, _wireTx = 0;
  int? _lastRttUs, _smoothedRttUs, _minRttUs;
  double _fps = 0.0, _bytesPerSecRx = 0.0;
  final List<(int, int)> _rxSamples = <(int, int)>[];
  final List<int> _frameSamples = <int>[];

  NvePairConnectionState get state => _state;
  String? get errorMessage => _errorMessage;
  NvePairUri? get activeTarget => _activeTarget;
  int get sessionId => _sessionId;
  int get latestFrameSeq => _latestFrameSeq;
  Uint8List? get latestFrame => _latestFrame;
  bool get isPaired => _state == NvePairConnectionState.paired;

  String get stateLabel => switch (_state) {
        NvePairConnectionState.disconnected => 'Disconnected',
        NvePairConnectionState.connecting => 'Connecting…',
        NvePairConnectionState.paired =>
          'Paired (0x${_sessionId.toRadixString(16).toUpperCase().padLeft(8, '0')})',
        NvePairConnectionState.reconnecting => 'Reconnecting (#$_reconnectAttempt)…',
        NvePairConnectionState.error => 'Error: ${_errorMessage ?? 'unknown'}',
      };

  NveSyncTelemetry get telemetry => NveSyncTelemetry(
        sessionId: _sessionId, framesReceived: _framesRx, keyframesReceived: _keyframesRx,
        deltasReceived: _deltasRx, keyframeRequestsSent: _kfReqsTx, eventsSent: _eventsTx,
        wireBytesRx: _wireRx, rawBytesRx: _rawRx, wireBytesTx: _wireTx,
        lastRttUs: _lastRttUs, smoothedRttUs: _smoothedRttUs, minRttUs: _minRttUs,
        fps: _fps, bytesPerSecRx: _bytesPerSecRx,
      );

  Future<void> connect(String uriString, {String? fallbackToken}) =>
      connectTarget(NvePairUri.parse(uriString, fallbackToken: fallbackToken));

  Future<void> connectTarget(NvePairUri target) async {
    _cancelTimers();
    await _closeSocket();
    _manualDisconnect = false;
    _reconnectAttempt = 0;
    _activeTarget = target;
    _errorMessage = null;
    _sessionId = 0;
    _latestFrameSeq = 0;
    _latestFrame = null;
    _deltaReceiver.reset();
    _resetCounters();
    await _openSocket(target, isReconnect: false);
  }

  Future<void> disconnect() async {
    _manualDisconnect = true;
    _cancelTimers();
    if (_state == NvePairConnectionState.paired && _sessionId != 0) {
      _sendEnvelope(NvePairCodec.encodeBye(_sessionId));
    }
    await _closeSocket();
    _pendingClientNonce = null;
    _sessionId = 0;
    _state = NvePairConnectionState.disconnected;
    _errorMessage = null;
    _notify();
  }

  Uint8List? requestKeyframe({int reason = 1}) {
    if (!isPaired) return null;
    final pkt = NvePairCodec.encodeKeyframeRequest(_sessionId, _deltaReceiver.lastAppliedSeq, reason);
    _kfReqsTx += 1;
    _sendEnvelope(pkt);
    _notify();
    return pkt;
  }

  Uint8List? sendPing() {
    if (!isPaired) return null;
    final pkt = NvePairCodec.encodePing(_sessionId, _nowMicros());
    _sendEnvelope(pkt);
    return pkt;
  }

  Uint8List? sendInputEvents(List<NveEvent> events) {
    if (events.isEmpty) return null;
    if (!isPaired) {
      throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'not paired');
    }
    final pkt = NvePairCodec.encodeInputEvents(events, _sessionId);
    _eventsTx += 1;
    _sendEnvelope(pkt);
    _notify();
    return pkt;
  }

  Uint8List sendRawInputEvents(Uint8List eventRing) {
    if (!isPaired) {
      throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'not paired');
    }
    final pkt = NvePairCodec.encodeRawInputEvents(eventRing, _sessionId);
    _eventsTx += 1;
    _sendEnvelope(pkt);
    _notify();
    return pkt;
  }

  Uint8List? beginHandshake() {
    final target = _activeTarget;
    if (target == null) return null;
    final seq = _nonceCounter++;
    final lastSeq = _deltaReceiver.lastAppliedSeq;
    final clientNonce = NveU64(
      ((seq * 0x9E3779B9) ^ lastSeq ^ 0x464C5554) & 0xFFFFFFFF,
      ((_nowMicros() ^ (seq << 7)) ^ 0x54455201) & 0xFFFFFFFF,
    );
    _pendingClientNonce = clientNonce;
    final hello = NveHelloPayload(
      schemaVersion: target.schemaVersion,
      maxWindow: NvePairProtocol.defaultSyncWindow,
      clientLastSeq: lastSeq,
      clientNonce: clientNonce,
      tokenProof: NvePairCrypto.deriveTokenProof(target.token, schemaVersion: target.schemaVersion),
    );
    final env = NveTransportEnvelope.encode(kind: NveTransportMsgKind.hello, sessionId: 0, payload: hello.encode());
    _sendEnvelope(env);
    return env;
  }

  List<Uint8List> handleInboundBinary(Uint8List bytes) {
    final now = _nowMicros();
    _wireRx += bytes.length;
    _rxSamples.add((now, bytes.length));
    _pruneWindow(now);

    final env = NveTransportEnvelope.parse(bytes);
    final outbound = <Uint8List>[];

    switch (env.kind) {
      case NveTransportMsgKind.helloAck:
        final target = _activeTarget, clientNonce = _pendingClientNonce;
        if (target == null || clientNonce == null) {
          throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'unexpected HelloAck');
        }
        final ack = NveHelloAckPayload.decode(env.payload);
        if (ack.schemaVersion != target.schemaVersion) {
          _failWith('schema mismatch: expected ${target.schemaVersion}, got ${ack.schemaVersion}');
        }
        final expectedSid = NvePairCrypto.deriveSessionId(
          pairingToken: target.token, schemaVersion: target.schemaVersion,
          clientNonce: clientNonce, serverNonce: ack.serverNonce,
        );
        if (env.sessionId != expectedSid) _failWith('session_id verification failed');
        _pendingClientNonce = null;
        _reconnectAttempt = 0;
        _deltaReceiver.reset(maxWindow: ack.serverMaxWindow);
        _sessionId = env.sessionId;
        _state = NvePairConnectionState.paired;
        _errorMessage = null;
        _startHeartbeat();

      case NveTransportMsgKind.reject:
        final reject = NveRejectPayload.decode(env.payload);
        _pendingClientNonce = null;
        _manualDisconnect = true;
        _cancelTimers();
        _failWith('pairing rejected: ${reject.reason.name} (schema v${reject.expectedSchema})');

      case NveTransportMsgKind.deltaSingle:
        final sid = _requirePairedSessionId(env.sessionId);
        try {
          final outcome = _deltaReceiver.receivePacket(env.payload);
          for (final entry in outcome.appliedFrames) {
            _framesRx += 1;
            _rawRx += entry.frame.length;
            if (entry.kind == NveDeltaPacketKind.keyframe) { _keyframesRx += 1; } else { _deltasRx += 1; }
            _frameSamples.add(now);
            _latestFrameSeq = entry.seq;
            _latestFrame = entry.frame;
            onFrame?.call(entry.seq, entry.frame);
          }
          if (outcome.ackPacket != null) {
            outbound.add(NveTransportEnvelope.encode(kind: NveTransportMsgKind.deltaSingle, sessionId: sid, payload: outcome.ackPacket));
          }
          for (final nack in outcome.nackPackets) {
            outbound.add(NveTransportEnvelope.encode(kind: NveTransportMsgKind.deltaSingle, sessionId: sid, payload: nack));
          }
          if (outcome.gapDetected) {
            _kfReqsTx += 1;
            outbound.add(NvePairCodec.encodeKeyframeRequest(sid, _deltaReceiver.lastAppliedSeq, 2));
          }
        } on NvePairProtocolException catch (err) {
          if (err.code == NvePairProtocolErrorCode.baseSequenceMismatch) {
            _kfReqsTx += 1;
            outbound.add(NvePairCodec.encodeKeyframeRequest(sid, _deltaReceiver.lastAppliedSeq, 3));
          } else {
            rethrow;
          }
        }

      case NveTransportMsgKind.ping:
        final sid = _requirePairedSessionId(env.sessionId);
        NvePairCodec.decodeTimestampUs(env.payload);
        outbound.add(NvePairCodec.encodePong(sid, env.payload));

      case NveTransportMsgKind.pong:
        _requirePairedSessionId(env.sessionId);
        final sentUs = NvePairCodec.decodeTimestampUs(env.payload);
        if (now >= sentUs) {
          final rtt = now - sentUs;
          _lastRttUs = rtt;
          _minRttUs = _minRttUs == null ? rtt : (rtt < _minRttUs! ? rtt : _minRttUs);
          _smoothedRttUs = _smoothedRttUs == null ? rtt : ((_smoothedRttUs! * 7 + rtt) ~/ 8);
        }

      case NveTransportMsgKind.bye:
        _requirePairedSessionId(env.sessionId);
        _manualDisconnect = true;
        _cancelTimers();
        unawaited(_closeSocket());
        _sessionId = 0;
        _state = NvePairConnectionState.disconnected;

      default:
        throw NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'unsupported kind ${env.kind.name}');
    }

    for (final msg in outbound) { _sendEnvelope(msg); }
    _refreshRates(now);
    _notify();
    return outbound;
  }

  Future<void> _openSocket(NvePairUri target, {required bool isReconnect}) async {
    _state = isReconnect ? NvePairConnectionState.reconnecting : NvePairConnectionState.connecting;
    _notify();
    try {
      final sock = await _connector(target.wsUri);
      if (_disposed || _manualDisconnect) { await sock.close(); return; }
      _socket = sock;
      _socketSub = sock.binaryMessages.listen(
        (Uint8List msg) { if (!_disposed) { try { handleInboundBinary(msg); } catch (_) {} } },
        onError: (Object err) => _handleTransportClosed(err.toString()),
        onDone: () => _handleTransportClosed(null),
      );
      beginHandshake();
    } catch (err) {
      _handleTransportClosed(err.toString());
    }
  }

  void _handleTransportClosed(String? reason) {
    _heartbeatTimer?.cancel();
    _heartbeatTimer = null;
    if (_disposed || _manualDisconnect) return;
    final target = _activeTarget;
    if (!autoReconnect || target == null || _reconnectAttempt >= maxReconnectAttempts) {
      _state = NvePairConnectionState.error;
      _errorMessage = reason ?? 'connection closed';
      _notify();
      return;
    }
    _reconnectAttempt += 1;
    _state = NvePairConnectionState.reconnecting;
    _notify();
    _reconnectTimer?.cancel();
    _reconnectTimer = Timer(Duration(milliseconds: _reconnectAttempt.clamp(1, 5) * 400), () {
      if (!_disposed && !_manualDisconnect) unawaited(_openSocket(target, isReconnect: true));
    });
  }

  void _startHeartbeat() {
    _heartbeatTimer?.cancel();
    if (heartbeatInterval <= Duration.zero) return;
    _heartbeatTimer = Timer.periodic(heartbeatInterval, (_) {
      if (_disposed || !isPaired) { _heartbeatTimer?.cancel(); return; }
      sendPing();
    });
  }

  void _cancelTimers() {
    _heartbeatTimer?.cancel(); _heartbeatTimer = null;
    _reconnectTimer?.cancel(); _reconnectTimer = null;
  }

  Future<void> _closeSocket() async {
    await _socketSub?.cancel(); _socketSub = null;
    final sock = _socket; _socket = null;
    if (sock != null) { try { await sock.close(); } catch (_) {} }
  }

  Never _failWith(String message) {
    _state = NvePairConnectionState.error; _errorMessage = message; _notify();
    throw NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, message);
  }

  int _requirePairedSessionId(int incoming) {
    if (!isPaired) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'not paired');
    if (incoming != _sessionId) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'session_id mismatch');
    return _sessionId;
  }

  void _sendEnvelope(Uint8List bytes) { _wireTx += bytes.length; _socket?.send(bytes); }

  void _resetCounters() {
    _framesRx = 0; _keyframesRx = 0; _deltasRx = 0; _kfReqsTx = 0; _eventsTx = 0;
    _wireRx = 0; _rawRx = 0; _wireTx = 0; _lastRttUs = null; _smoothedRttUs = null; _minRttUs = null;
    _fps = 0.0; _bytesPerSecRx = 0.0; _rxSamples.clear(); _frameSamples.clear();
  }

  void _pruneWindow(int nowUs) {
    final cutoff = nowUs > 1000000 ? nowUs - 1000000 : 0;
    _rxSamples.removeWhere((s) => s.$1 < cutoff);
    _frameSamples.removeWhere((ts) => ts < cutoff);
  }

  void _refreshRates(int nowUs) {
    _pruneWindow(nowUs);
    var sum = 0;
    for (final s in _rxSamples) { sum += s.$2; }
    _bytesPerSecRx = sum.toDouble();
    _fps = _frameSamples.length.toDouble();
  }

  void _notify() { if (!_disposed) notifyListeners(); }

  @override
  void dispose() {
    _disposed = true; _manualDisconnect = true; _cancelTimers(); unawaited(_closeSocket());
    super.dispose();
  }
}
