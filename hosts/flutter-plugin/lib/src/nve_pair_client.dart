import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'event_codec.dart';
import 'package:nve_host/nve_host.dart';

typedef NvePairedFrameCallback = void Function(int seq, Uint8List frameBytes);

enum NveWireChannel {
  control(0),
  uiScene(1),
  asyncSlot(2),
  inputEvents(3),
  stateSync(4),
  presence(5);

  const NveWireChannel(this.code);
  final int code;

  static NveWireChannel? fromCode(int code) {
    for (final v in values) {
      if (v.code == code) return v;
    }
    return null;
  }
}

enum NveWireKind {
  hello(1),
  helloAck(2),
  reject(3),
  sceneKeyframe(4),
  commandDelta(5),
  nodePatchDelta(6),
  slotChunk(7),
  eventBatch(8),
  crdtDelta(9),
  presence(10),
  hotPatch(11),
  ping(12),
  pong(13),
  keyframeRequest(14),
  bye(15);

  const NveWireKind(this.code);
  final int code;

  static NveWireKind? fromCode(int code) {
    for (final v in values) {
      if (v.code == code) return v;
    }
    return null;
  }
}

class NvePairProtocolException implements Exception {
  const NvePairProtocolException(this.message);
  final String message;

  @override
  String toString() => 'NvePairProtocolException($message)';
}

class NveDecodedWireFrame {
  const NveDecodedWireFrame({
    required this.channel,
    required this.kind,
    required this.sequence,
    required this.payload,
  });

  final NveWireChannel channel;
  final NveWireKind kind;
  final int sequence;
  final Uint8List payload;
}

/// Thin `NveWireFrame` v2 codec (16-byte LE header + payload + optional 8-byte `XXH64` trailer).
abstract final class NveWireCodec {
  static const int magic0 = 0x4E; // 'N'
  static const int magic1 = 0x57; // 'W'
  static const int version = 2;
  static const int headerSize = 16;
  static const int trailerSize = 8;
  static const int flagXxh64 = 1 << 0;
  static const int defaultSchemaVersion = 2;
  static const int maxFrameBytes = 16 * 1024 * 1024;
  static const Key64 xxh64Seed = Key64(0x49524532, 0x4E564557);

  static Key64 xxh64(Uint8List bytes, {Key64 seed = xxh64Seed}) =>
      (Xxh64Hasher(seed)..update(bytes)).digest();

  static Uint8List encodeFrame({
    required NveWireChannel channel,
    required NveWireKind kind,
    required int sequence,
    Uint8List? payload,
    int flags = flagXxh64,
  }) {
    final body = payload ?? Uint8List(0);
    final hasCk = (flags & flagXxh64) != 0;
    final out = Uint8List(headerSize + body.length + (hasCk ? trailerSize : 0));
    final bd = ByteData.sublistView(out);
    out[0] = magic0;
    out[1] = magic1;
    out[2] = version;
    out[3] = channel.code;
    out[4] = kind.code;
    out[5] = flags;
    bd.setUint16(6, 0, Endian.little);
    bd.setUint32(8, sequence, Endian.little);
    bd.setUint32(12, body.length, Endian.little);
    out.setRange(headerSize, headerSize + body.length, body);
    if (hasCk) {
      final bodyEnd = headerSize + body.length;
      xxh64(Uint8List.sublistView(out, 0, bodyEnd)).writeLE(out, bodyEnd);
    }
    return out;
  }

  static NveDecodedWireFrame decodeFrame(Uint8List bytes, {int maxBytes = maxFrameBytes}) {
    if (bytes.length < headerSize) {
      throw const NvePairProtocolException('truncated NveWireFrame header');
    }
    if (bytes[0] != magic0 || bytes[1] != magic1) {
      throw const NvePairProtocolException('invalid NveWireFrame magic');
    }
    if (bytes[2] != version) {
      throw NvePairProtocolException('unsupported NveWireFrame version ${bytes[2]}');
    }
    final ch = NveWireChannel.fromCode(bytes[3]);
    final kind = NveWireKind.fromCode(bytes[4]);
    if (ch == null || kind == null) {
      throw const NvePairProtocolException('unknown NveWireFrame channel or kind');
    }
    final bd = ByteData.sublistView(bytes);
    if (bd.getUint16(6, Endian.little) != 0) {
      throw const NvePairProtocolException('non-zero reserved header');
    }
    final seq = bd.getUint32(8, Endian.little);
    final len = bd.getUint32(12, Endian.little);
    if (len > maxBytes) {
      throw NvePairProtocolException('frame payload $len exceeds limit $maxBytes');
    }
    final hasCk = (bytes[5] & flagXxh64) != 0;
    final total = headerSize + len + (hasCk ? trailerSize : 0);
    if (bytes.length != total) {
      throw const NvePairProtocolException('NveWireFrame length mismatch');
    }
    final bodyEnd = headerSize + len;
    if (hasCk) {
      final expected = Key64.readLE(bytes, bodyEnd);
      final actual = xxh64(Uint8List.sublistView(bytes, 0, bodyEnd));
      if (actual != expected) {
        throw const NvePairProtocolException('NveWireFrame XXH64 checksum mismatch');
      }
    }
    return NveDecodedWireFrame(
      channel: ch,
      kind: kind,
      sequence: seq,
      payload: Uint8List.fromList(bytes.sublist(headerSize, bodyEnd)),
    );
  }

  static Uint8List encodeHello(String bearerToken, {String clientName = 'nve-flutter'}) {
    final tok = utf8.encode(bearerToken);
    final name = utf8.encode(clientName);
    final out = Uint8List(10 + tok.length + name.length);
    final bd = ByteData.sublistView(out);
    bd.setUint16(0, defaultSchemaVersion, Endian.little);
    bd.setUint32(2, 0x0F, Endian.little);
    bd.setUint16(6, tok.length, Endian.little);
    bd.setUint16(8, name.length, Endian.little);
    out.setRange(10, 10 + tok.length, tok);
    out.setRange(10 + tok.length, 10 + tok.length + name.length, name);
    return out;
  }

  static int decodeHelloAckSessionId(Uint8List payload) {
    if (payload.length != 12) {
      throw const NvePairProtocolException('invalid HelloAck length');
    }
    return ByteData.sublistView(payload).getUint32(0, Endian.little);
  }
}

class NveParsedPairUri {
  const NveParsedPairUri({
    required this.webSocketUri,
    required this.token,
    this.certHashHex,
  });

  final Uri webSocketUri;
  final String token;
  final String? certHashHex;

  static NveParsedPairUri parse(String raw, {String? fallbackToken}) {
    final trimmed = raw.trim();
    if (trimmed.isEmpty) {
      throw const NvePairProtocolException('pairing URI cannot be empty');
    }
    final uri = Uri.tryParse(trimmed);
    if (uri == null) {
      throw const NvePairProtocolException('malformed pairing URI');
    }
    if (uri.scheme.toLowerCase() == 'nve') {
      final host = uri.queryParameters['host'] ?? '127.0.0.1';
      final port = uri.queryParameters['wss'] ??
          uri.queryParameters['ws'] ??
          uri.queryParameters['quic'] ??
          uri.queryParameters['udp'] ??
          '';
      final tok = (uri.queryParameters['token']?.isNotEmpty == true)
          ? uri.queryParameters['token']!
          : (fallbackToken ?? '');
      if (port.isEmpty || tok.isEmpty) {
        throw const NvePairProtocolException('nve://pair URI requires port and token');
      }
      final scheme = uri.queryParameters.containsKey('wss') ? 'wss' : 'ws';
      return NveParsedPairUri(
        webSocketUri: Uri.parse('$scheme://$host:$port/nve-wire'),
        token: tok,
        certHashHex: uri.queryParameters['cert_hash'],
      );
    }
    final tok = fallbackToken ?? uri.queryParameters['token'] ?? '';
    if (tok.isEmpty) {
      throw const NvePairProtocolException('pairing token is required');
    }
    return NveParsedPairUri(webSocketUri: uri, token: tok);
  }
}

enum NvePairConnectionState {
  disconnected,
  connecting,
  paired,
  reconnecting,
  error,
}

class NveSyncTelemetry {
  int sessionId = 0;
  int framesReceived = 0;
  int keyframesReceived = 0;
  int bytesReceived = 0;
  int eventsSent = 0;
  double? smoothedRttMs;
  double fps = 0;
  double bytesPerSecRx = 0;
  double compressionRatio = 1.0;
}

/// Thin Flutter pairing client speaking `NveWireFrame` v2 over WebSocket (or `nve-wire` FFI).
class NvePairClient extends ChangeNotifier {
  NvePairConnectionState _state = NvePairConnectionState.disconnected;
  String? _lastError;
  final NveSyncTelemetry telemetry = NveSyncTelemetry();
  Uint8List? _latestFrame;
  int _latestFrameSeq = 0;
  int _nextSeq = 2;
  WebSocket? _socket;

  NvePairConnectionState get state => _state;
  bool get isPaired => _state == NvePairConnectionState.paired;
  Uint8List? get latestFrame => _latestFrame;
  int get latestFrameSeq => _latestFrameSeq;

  String get stateLabel => switch (_state) {
        NvePairConnectionState.disconnected => 'Disconnected',
        NvePairConnectionState.connecting => 'Connecting…',
        NvePairConnectionState.paired =>
          'Paired (0x${telemetry.sessionId.toRadixString(16).toUpperCase().padLeft(8, '0')})',
        NvePairConnectionState.reconnecting => 'Reconnecting…',
        NvePairConnectionState.error => 'Error: ${_lastError ?? 'unknown'}',
      };

  Future<void> connect(String uriString, {String? fallbackToken}) async {
    await disconnect();
    final parsed = NveParsedPairUri.parse(uriString, fallbackToken: fallbackToken);
    _state = NvePairConnectionState.connecting;
    notifyListeners();
    try {
      final ws = await WebSocket.connect(parsed.webSocketUri.toString());
      _socket = ws;
      ws.add(NveWireCodec.encodeFrame(
        channel: NveWireChannel.control,
        kind: NveWireKind.hello,
        sequence: 1,
        payload: NveWireCodec.encodeHello(parsed.token),
      ));
      ws.listen(
        (dynamic msg) {
          if (msg is List<int>) {
            try {
              final reply = handleInboundBinary(Uint8List.fromList(msg));
              if (reply != null) _socket?.add(reply);
            } on NvePairProtocolException catch (e) {
              _lastError = e.message;
              _state = NvePairConnectionState.error;
              notifyListeners();
            }
          }
        },
        onDone: () {
          if (_state != NvePairConnectionState.disconnected) {
            _state = NvePairConnectionState.disconnected;
            notifyListeners();
          }
        },
        onError: (Object err) {
          _lastError = err.toString();
          _state = NvePairConnectionState.error;
          notifyListeners();
        },
      );
    } catch (e) {
      _lastError = e.toString();
      _state = NvePairConnectionState.error;
      notifyListeners();
      rethrow;
    }
  }

  Future<void> disconnect() async {
    if (_socket != null) {
      if (isPaired) {
        _socket!.add(NveWireCodec.encodeFrame(
          channel: NveWireChannel.control,
          kind: NveWireKind.bye,
          sequence: _allocSeq(),
        ));
      }
      await _socket!.close();
      _socket = null;
    }
    _state = NvePairConnectionState.disconnected;
    notifyListeners();
  }

  void requestKeyframe() {
    if (!isPaired) return;
    _socket?.add(NveWireCodec.encodeFrame(
      channel: NveWireChannel.control,
      kind: NveWireKind.keyframeRequest,
      sequence: _allocSeq(),
    ));
  }

  Uint8List sendInputEvents(List<NveEvent> events) {
    if (!isPaired) {
      throw const NvePairProtocolException('not paired');
    }
    final ring = NveEventRingCodec.encodeEvents(events);
    final frame = NveWireCodec.encodeFrame(
      channel: NveWireChannel.inputEvents,
      kind: NveWireKind.eventBatch,
      sequence: _allocSeq(),
      payload: ring,
    );
    telemetry.eventsSent += 1;
    _socket?.add(frame);
    return frame;
  }

  Uint8List? handleInboundBinary(Uint8List bytes) {
    final decoded = NveWireCodec.decodeFrame(bytes);
    telemetry.bytesReceived += bytes.length;
    switch (decoded.kind) {
      case NveWireKind.helloAck:
        telemetry.sessionId = NveWireCodec.decodeHelloAckSessionId(decoded.payload);
        _state = NvePairConnectionState.paired;
        notifyListeners();
        return null;
      case NveWireKind.reject:
        _lastError = 'Pairing rejected';
        _state = NvePairConnectionState.error;
        notifyListeners();
        throw const NvePairProtocolException('Pairing rejected');
      case NveWireKind.sceneKeyframe:
      case NveWireKind.commandDelta:
        telemetry.framesReceived += 1;
        if (decoded.kind == NveWireKind.sceneKeyframe) {
          telemetry.keyframesReceived += 1;
        }
        _latestFrame = decoded.payload;
        _latestFrameSeq = decoded.sequence;
        notifyListeners();
        return null;
      case NveWireKind.ping:
        return NveWireCodec.encodeFrame(
          channel: NveWireChannel.control,
          kind: NveWireKind.pong,
          sequence: _allocSeq(),
          payload: decoded.payload,
        );
      case NveWireKind.bye:
        _state = NvePairConnectionState.disconnected;
        notifyListeners();
        return null;
      default:
        return null;
    }
  }

  int _allocSeq() {
    final s = _nextSeq;
    _nextSeq = (_nextSeq + 1) & 0xFFFFFFFF;
    if (_nextSeq == 0) _nextSeq = 1;
    return s;
  }
}
