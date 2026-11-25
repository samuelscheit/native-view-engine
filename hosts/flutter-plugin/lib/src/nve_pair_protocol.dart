import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'event_codec.dart';
import 'nve_core_abi.generated.dart';

/// Wire constants for `NVT1` transport framing and `NVD1` frame-delta packets.
/// Matches `crates/nve-interop/src/sync_transport/protocol.rs` and `crates/nve-interop/src/sync_delta/`.
abstract final class NvePairProtocol {
  static const int transportMagic0 = 0x4E; // 'N'
  static const int transportMagic1 = 0x54; // 'T'
  static const int transportVersion = 1;
  static const int transportHeaderSize = 8;
  static const int helloPayloadSize = 32;
  static const int helloAckPayloadSize = 20;
  static const List<int> deltaMagic = <int>[0x4E, 0x56, 0x44, 0x31]; // "NVD1"
  static const int deltaVersion = 1;
  static const int deltaHeaderSize = 32;
  static const int flagRetransmit = 1 << 0;
  static const int defaultSchemaVersion = 1;
  static const int defaultSyncWindow = 64;
  static const int minSyncWindow = 4;
  static const int maxSyncWindow = 1024;
  static const int maxDecodedFrameBytes = 16 * 1024 * 1024;
  static const int maxWireFrameBytes = maxDecodedFrameBytes + deltaHeaderSize;
  static const int maxInputEventBytes = 64 * 1024;
  static const int minEncodedEventBytes = 18;
  static const int nveCmdHeaderSize = 24;
  static const int cmdDeltaCopyRun = 1;
  static const int cmdDeltaLiteralOp = 2;
  static const int cmdDeltaXorOp = 3;
}

enum NveTransportMsgKind {
  hello(1), helloAck(2), reject(3), deltaSingle(4), deltaFrag(5), fragNack(6),
  ping(7), pong(8), keyframeRequest(9), bye(10), inputEvents(11);
  const NveTransportMsgKind(this.code);
  final int code;
  static NveTransportMsgKind? fromCode(int code) {
    for (final v in values) { if (v.code == code) return v; }
    return null;
  }
}

enum NveRejectReason {
  invalidPairingToken(1), schemaVersionMismatch(2), unsupportedProtocolVersion(3), serverBusy(4);
  const NveRejectReason(this.code);
  final int code;
  static NveRejectReason? fromCode(int code) {
    for (final v in values) { if (v.code == code) return v; }
    return null;
  }
}

enum NveDeltaPacketKind {
  keyframe(1), xorRleDelta(2), commandOpDelta(3), ack(4), nack(5);
  const NveDeltaPacketKind(this.code);
  final int code;
  static NveDeltaPacketKind? fromCode(int code) {
    for (final v in values) { if (v.code == code) return v; }
    return null;
  }
}

enum NvePairProtocolErrorCode {
  invalidUri, truncatedFrame, invalidTransportMagic, unsupportedTransportVersion,
  invalidTransportKind, malformedFrame, frameTooLarge, invalidDeltaMagic,
  unsupportedDeltaVersion, invalidDeltaKind, lengthMismatch, checksumMismatch,
  baseSequenceMismatch, corruptPayload,
}

class NvePairProtocolException implements Exception {
  const NvePairProtocolException(this.code, this.message);
  final NvePairProtocolErrorCode code;
  final String message;
  @override
  String toString() => 'NvePairProtocolException(${code.name}): $message';
}

/// Allocation-free unsigned 64-bit integer split into unsigned 32-bit `(lo, hi)`.
class NveU64 {
  const NveU64(this.lo, this.hi);
  static const NveU64 zero = NveU64(0, 0);
  final int lo, hi;

  factory NveU64.fromInt(int value) => NveU64(value & 0xFFFFFFFF, (value >>> 32) & 0xFFFFFFFF);
  factory NveU64.readLE(ByteData bd, int offset) =>
      NveU64(bd.getUint32(offset, Endian.little), bd.getUint32(offset + 4, Endian.little));

  void writeLE(ByteData bd, int offset) {
    bd.setUint32(offset, lo & 0xFFFFFFFF, Endian.little);
    bd.setUint32(offset + 4, hi & 0xFFFFFFFF, Endian.little);
  }

  String toHex() =>
      '0x${(hi & 0xFFFFFFFF).toRadixString(16).padLeft(8, '0')}${(lo & 0xFFFFFFFF).toRadixString(16).padLeft(8, '0')}';

  @override
  bool operator ==(Object other) => identical(this, other) || (other is NveU64 && other.lo == lo && other.hi == hi);
  @override
  int get hashCode => Object.hash(lo, hi);
  @override
  String toString() => toHex();
}

class NvePairUri {
  const NvePairUri({
    required this.wsUri, required this.token, required this.host,
    this.wsPort, this.udpPort, this.width, this.height,
    this.schemaVersion = NvePairProtocol.defaultSchemaVersion,
  });

  final Uri wsUri;
  final String token, host;
  final int? wsPort, udpPort;
  final double? width, height;
  final int schemaVersion;

  static NvePairUri parse(String raw, {String? fallbackToken}) {
    final trimmed = raw.trim();
    if (trimmed.isEmpty) throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'URI is empty');
    final fallback = fallbackToken?.trim() ?? '';

    if (trimmed.toLowerCase().startsWith('nve://')) {
      final parsed = Uri.tryParse(trimmed);
      if (parsed == null || parsed.host.toLowerCase() != 'pair') {
        throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'expected nve://pair URI');
      }
      final params = <String, String>{for (final e in parsed.queryParameters.entries) e.key.toLowerCase(): e.value.trim()};
      final wsParam = params['ws'] ?? params['endpoint'] ?? params['connect'] ?? '';
      if (wsParam.isEmpty) throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'missing ws');
      final token = fallback.isNotEmpty ? fallback : (params['token'] ?? '');
      if (token.isEmpty) throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'missing token');
      var schema = NvePairProtocol.defaultSchemaVersion;
      if ((params['schema'] ?? '').isNotEmpty) {
        final s = int.tryParse(params['schema']!);
        if (s == null || s <= 0 || s > 0xFFFF) {
          throw NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'invalid schema "${params['schema']}"');
        }
        schema = s;
      }
      final udp = params['udp'] != null ? int.tryParse(params['udp']!) : null;
      final w = params['w'] != null ? double.tryParse(params['w']!) : null;
      final h = params['h'] != null ? double.tryParse(params['h']!) : null;
      final lw = wsParam.toLowerCase();
      if (lw.startsWith('ws://') || lw.startsWith('wss://')) {
        final wsUri = Uri.tryParse(wsParam);
        if (wsUri == null || wsUri.host.isEmpty) throw NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'invalid ws URL');
        return NvePairUri(wsUri: wsUri, token: token, host: wsUri.host, wsPort: wsUri.hasPort ? wsUri.port : null, udpPort: udp, width: w, height: h, schemaVersion: schema);
      }
      final port = int.tryParse(wsParam);
      if (port == null || port < 1 || port > 65535) throw NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'invalid ws port');
      final host = (params['host'] ?? '').isNotEmpty ? params['host']! : '127.0.0.1';
      final hostPart = (host.contains(':') && !host.startsWith('[')) ? '[$host]' : host;
      return NvePairUri(wsUri: Uri.parse('ws://$hostPart:$port/ws/nvd'), token: token, host: host, wsPort: port, udpPort: udp, width: w, height: h, schemaVersion: schema);
    }

    final lower = trimmed.toLowerCase();
    final uri = Uri.tryParse((lower.startsWith('ws://') || lower.startsWith('wss://')) ? trimmed : 'ws://$trimmed');
    if (uri == null || (uri.scheme != 'ws' && uri.scheme != 'wss') || uri.host.isEmpty) {
      throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'unsupported scheme or host');
    }
    final token = fallback.isNotEmpty ? fallback : (uri.queryParameters['token'] ?? '').trim();
    if (token.isEmpty) throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidUri, 'missing token');
    final finalUri = uri.path.isEmpty ? uri.replace(path: '/ws/nvd') : uri;
    return NvePairUri(wsUri: finalUri, token: token, host: finalUri.host, wsPort: finalUri.hasPort ? finalUri.port : null);
  }
}

abstract final class NvePairCrypto {
  static const int _offsetLo = 0x84222325, _offsetHi = 0xcbf29ce4;

  static NveU64 fnv1a64(List<int> bytes) => fnv1a64Pair(bytes, const <int>[]);

  static NveU64 fnv1a64Pair(List<int> a, List<int> b) {
    var lo = _offsetLo, hi = _offsetHi;
    void feed(List<int> src) {
      for (var i = 0; i < src.length; i++) {
        lo = (lo ^ (src[i] & 0xFF)) & 0xFFFFFFFF;
        final p0 = lo * 0x1b3, newLo = p0 & 0xFFFFFFFF, carry = (p0 - newLo) ~/ 0x100000000;
        hi = (hi * 0x1b3 + ((lo & 0x00FFFFFF) << 8) + carry) & 0xFFFFFFFF;
        lo = newLo;
      }
    }
    feed(a); feed(b);
    return NveU64(lo, hi);
  }

  static Uint8List deriveTokenProof(String pairingToken, {int schemaVersion = 1}) {
    final tok = utf8.encode(pairingToken);
    final vLo = schemaVersion & 0xFF, vHi = (schemaVersion >> 8) & 0xFF;
    final h1 = fnv1a64Pair(<int>[...utf8.encode('NVE-PAIR-A-v1:'), vLo, vHi], tok);
    final h2 = fnv1a64Pair(tok, <int>[...utf8.encode(':NVE-PAIR-B-v1'), vLo, vHi]);
    final out = Uint8List(16), bd = ByteData.sublistView(out);
    h1.writeLE(bd, 0); h2.writeLE(bd, 8);
    return out;
  }

  static int deriveSessionId({required String pairingToken, int schemaVersion = 1, required NveU64 clientNonce, required NveU64 serverNonce}) {
    final mat = Uint8List(18), bd = ByteData.sublistView(mat)..setUint16(0, schemaVersion & 0xFFFF, Endian.little);
    clientNonce.writeLE(bd, 2); serverNonce.writeLE(bd, 10);
    final h = fnv1a64Pair(utf8.encode(pairingToken), mat);
    final folded = ((h.lo ^ h.hi) ^ 0x5A3C96F1) & 0xFFFFFFFF;
    return folded == 0 ? 1 : folded;
  }
}

class NveTransportEnvelope {
  const NveTransportEnvelope({required this.kind, required this.sessionId, required this.payload});
  final NveTransportMsgKind kind;
  final int sessionId;
  final Uint8List payload;
  static final Uint8List _empty = Uint8List(0);

  static NveTransportEnvelope parse(Uint8List bytes) {
    if (bytes.length < 8) throw const NvePairProtocolException(NvePairProtocolErrorCode.truncatedFrame, 'truncated NVT1');
    if (bytes[0] != NvePairProtocol.transportMagic0 || bytes[1] != NvePairProtocol.transportMagic1) {
      throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidTransportMagic, 'invalid NVT1 magic');
    }
    if (bytes[2] != NvePairProtocol.transportVersion) {
      throw NvePairProtocolException(NvePairProtocolErrorCode.unsupportedTransportVersion, 'bad version ${bytes[2]}');
    }
    final kind = NveTransportMsgKind.fromCode(bytes[3]);
    if (kind == null) throw NvePairProtocolException(NvePairProtocolErrorCode.invalidTransportKind, 'bad kind ${bytes[3]}');
    return NveTransportEnvelope(kind: kind, sessionId: ByteData.sublistView(bytes).getUint32(4, Endian.little), payload: Uint8List.sublistView(bytes, 8));
  }

  static Uint8List encode({required NveTransportMsgKind kind, required int sessionId, Uint8List? payload}) {
    final p = payload ?? _empty, out = Uint8List(8 + p.length);
    out[0] = NvePairProtocol.transportMagic0; out[1] = NvePairProtocol.transportMagic1;
    out[2] = NvePairProtocol.transportVersion; out[3] = kind.code;
    ByteData.sublistView(out).setUint32(4, sessionId & 0xFFFFFFFF, Endian.little);
    if (p.isNotEmpty) out.setRange(8, out.length, p);
    return out;
  }
}

class NveHelloPayload {
  const NveHelloPayload({this.schemaVersion = 1, this.maxWindow = 64, this.clientLastSeq = 0, required this.clientNonce, required this.tokenProof});
  final int schemaVersion, maxWindow, clientLastSeq;
  final NveU64 clientNonce;
  final Uint8List tokenProof;

  Uint8List encode() {
    final out = Uint8List(32), bd = ByteData.sublistView(out)
      ..setUint16(0, schemaVersion & 0xFFFF, Endian.little)..setUint16(2, maxWindow & 0xFFFF, Endian.little)
      ..setUint32(4, clientLastSeq & 0xFFFFFFFF, Endian.little);
    clientNonce.writeLE(bd, 8);
    out.setRange(16, 16 + (tokenProof.length < 16 ? tokenProof.length : 16), tokenProof);
    return out;
  }

  static NveHelloPayload decode(Uint8List bytes) {
    if (bytes.length != 32) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'invalid Hello');
    final bd = ByteData.sublistView(bytes);
    return NveHelloPayload(
      schemaVersion: bd.getUint16(0, Endian.little), maxWindow: bd.getUint16(2, Endian.little),
      clientLastSeq: bd.getUint32(4, Endian.little), clientNonce: NveU64.readLE(bd, 8),
      tokenProof: Uint8List.fromList(Uint8List.sublistView(bytes, 16, 32)),
    );
  }
}

class NveHelloAckPayload {
  const NveHelloAckPayload({required this.schemaVersion, required this.serverMaxWindow, required this.serverSeq, required this.serverNonce, required this.resyncRequired});
  final int schemaVersion, serverMaxWindow, serverSeq;
  final NveU64 serverNonce;
  final bool resyncRequired;

  Uint8List encode() {
    final out = Uint8List(20), bd = ByteData.sublistView(out)
      ..setUint16(0, schemaVersion & 0xFFFF, Endian.little)..setUint16(2, serverMaxWindow & 0xFFFF, Endian.little)
      ..setUint32(4, serverSeq & 0xFFFFFFFF, Endian.little);
    serverNonce.writeLE(bd, 8); out[16] = resyncRequired ? 1 : 0;
    return out;
  }

  static NveHelloAckPayload decode(Uint8List bytes) {
    if (bytes.length != 20) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'invalid HelloAck');
    final bd = ByteData.sublistView(bytes);
    return NveHelloAckPayload(
      schemaVersion: bd.getUint16(0, Endian.little), serverMaxWindow: bd.getUint16(2, Endian.little),
      serverSeq: bd.getUint32(4, Endian.little), serverNonce: NveU64.readLE(bd, 8), resyncRequired: bytes[16] != 0,
    );
  }
}

class NveRejectPayload {
  const NveRejectPayload({required this.reason, required this.expectedSchema});
  final NveRejectReason reason;
  final int expectedSchema;
  Uint8List encode() {
    final out = Uint8List(4)..[0] = reason.code;
    ByteData.sublistView(out).setUint16(1, expectedSchema & 0xFFFF, Endian.little);
    return out;
  }
  static NveRejectPayload decode(Uint8List bytes) {
    if (bytes.length < 3) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'truncated Reject');
    final reason = NveRejectReason.fromCode(bytes[0]);
    if (reason == null) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'unknown Reject');
    return NveRejectPayload(reason: reason, expectedSchema: ByteData.sublistView(bytes).getUint16(1, Endian.little));
  }
}

abstract final class NvePairCodec {
  static Uint8List encodePing(int sessionId, int timestampUs) {
    final p = Uint8List(8); NveU64.fromInt(timestampUs).writeLE(ByteData.sublistView(p), 0);
    return NveTransportEnvelope.encode(kind: NveTransportMsgKind.ping, sessionId: sessionId, payload: p);
  }
  static Uint8List encodePong(int sessionId, Uint8List tsPayload) {
    if (tsPayload.length != 8) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'invalid Pong');
    return NveTransportEnvelope.encode(kind: NveTransportMsgKind.pong, sessionId: sessionId, payload: tsPayload);
  }
  static int decodeTimestampUs(Uint8List payload) {
    if (payload.length != 8) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'invalid Ping/Pong');
    return ByteData.sublistView(payload).getUint64(0, Endian.little);
  }
  static Uint8List encodeKeyframeRequest(int sessionId, int lastSeq, int reason) {
    final p = Uint8List(8)..[4] = reason & 0xFF; ByteData.sublistView(p).setUint32(0, lastSeq & 0xFFFFFFFF, Endian.little);
    return NveTransportEnvelope.encode(kind: NveTransportMsgKind.keyframeRequest, sessionId: sessionId, payload: p);
  }
  static ({int lastSeq, int reason}) decodeKeyframeRequest(Uint8List payload) {
    if (payload.length != 8) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'invalid KeyframeRequest');
    return (lastSeq: ByteData.sublistView(payload).getUint32(0, Endian.little), reason: payload[4]);
  }
  static Uint8List encodeBye(int sessionId) => NveTransportEnvelope.encode(kind: NveTransportMsgKind.bye, sessionId: sessionId);
  static Uint8List encodeInputEvents(List<NveEvent> events, int sessionId) =>
      encodeRawInputEvents(NveEventRingCodec.encodeEvents(events), sessionId);
  static Uint8List encodeRawInputEvents(Uint8List eventRing, int sessionId) {
    if (eventRing.length > NvePairProtocol.maxInputEventBytes) {
      throw const NvePairProtocolException(NvePairProtocolErrorCode.frameTooLarge, 'InputEvents exceeds 64 KiB');
    }
    if (eventRing.length < 4) throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'truncated InputEvents');
    final count = ByteData.sublistView(eventRing).getUint32(0, Endian.little);
    if (eventRing.length < 4 + count * NvePairProtocol.minEncodedEventBytes) {
      throw const NvePairProtocolException(NvePairProtocolErrorCode.malformedFrame, 'InputEvents count mismatch');
    }
    return NveTransportEnvelope.encode(kind: NveTransportMsgKind.inputEvents, sessionId: sessionId, payload: eventRing);
  }
}

class NveDeltaPacketHeader {
  const NveDeltaPacketHeader({required this.version, required this.kind, required this.flags, required this.seq, required this.baseSeq, required this.ackSeq, required this.payloadLen, required this.checksum});
  final int version, flags, seq, baseSeq, ackSeq, payloadLen;
  final NveDeltaPacketKind kind;
  final NveU64 checksum;
}

class NveDeltaPacket {
  const NveDeltaPacket({required this.header, required this.payload});
  final NveDeltaPacketHeader header;
  final Uint8List payload;

  static NveDeltaPacket parse(Uint8List bytes) {
    if (bytes.length < 32) throw const NvePairProtocolException(NvePairProtocolErrorCode.truncatedFrame, 'truncated NVD1');
    for (var i = 0; i < 4; i++) {
      if (bytes[i] != NvePairProtocol.deltaMagic[i]) throw const NvePairProtocolException(NvePairProtocolErrorCode.invalidDeltaMagic, 'invalid NVD1 magic');
    }
    if (bytes[4] != NvePairProtocol.deltaVersion) throw NvePairProtocolException(NvePairProtocolErrorCode.unsupportedDeltaVersion, 'bad NVD1 ${bytes[4]}');
    final kind = NveDeltaPacketKind.fromCode(bytes[5]);
    if (kind == null) throw NvePairProtocolException(NvePairProtocolErrorCode.invalidDeltaKind, 'bad NVD1 kind ${bytes[5]}');
    final bd = ByteData.sublistView(bytes), payloadLen = bd.getUint32(20, Endian.little), checksum = NveU64.readLE(bd, 24);
    if (bytes.length - 32 != payloadLen) throw NvePairProtocolException(NvePairProtocolErrorCode.lengthMismatch, 'length mismatch');
    final payload = Uint8List.sublistView(bytes, 32);
    if (NvePairCrypto.fnv1a64Pair(Uint8List.sublistView(bytes, 0, 24), payload) != checksum) {
      throw const NvePairProtocolException(NvePairProtocolErrorCode.checksumMismatch, 'checksum mismatch');
    }
    return NveDeltaPacket(
      header: NveDeltaPacketHeader(
        version: bytes[4], kind: kind, flags: bd.getUint16(6, Endian.little), seq: bd.getUint32(8, Endian.little),
        baseSeq: bd.getUint32(12, Endian.little), ackSeq: bd.getUint32(16, Endian.little), payloadLen: payloadLen, checksum: checksum,
      ),
      payload: payload,
    );
  }

  static Uint8List encode({required NveDeltaPacketKind kind, int flags = 0, int seq = 0, int baseSeq = 0, int ackSeq = 0, Uint8List? payload}) {
    final p = payload ?? Uint8List(0), out = Uint8List(32 + p.length);
    for (var i = 0; i < 4; i++) { out[i] = NvePairProtocol.deltaMagic[i]; }
    out[4] = NvePairProtocol.deltaVersion; out[5] = kind.code;
    final bd = ByteData.sublistView(out)
      ..setUint16(6, flags & 0xFFFF, Endian.little)..setUint32(8, seq & 0xFFFFFFFF, Endian.little)
      ..setUint32(12, baseSeq & 0xFFFFFFFF, Endian.little)..setUint32(16, ackSeq & 0xFFFFFFFF, Endian.little)
      ..setUint32(20, p.length & 0xFFFFFFFF, Endian.little);
    NvePairCrypto.fnv1a64Pair(Uint8List.sublistView(out, 0, 24), p).writeLE(bd, 24);
    if (p.isNotEmpty) out.setRange(32, out.length, p);
    return out;
  }
}

class _VarintCursor {
  _VarintCursor(this.bytes, [this.pos = 0]);
  final Uint8List bytes;
  int pos;
  int readVarint() {
    var shift = 0, result = 0;
    for (var i = 0; i < 10; i++) {
      if (pos >= bytes.length) throw const NvePairProtocolException(NvePairProtocolErrorCode.truncatedFrame, 'truncated varint');
      final b = bytes[pos++], low = b & 0x7F;
      if (shift == 63 && low > 1) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'varint overflow');
      result |= low << shift;
      if ((b & 0x80) == 0) return result;
      shift += 7;
    }
    throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'unterminated varint');
  }
}

abstract final class NveDeltaCodec {
  static Uint8List decodeXorRleDelta(Uint8List base, Uint8List deltaPayload) {
    final cur = _VarintCursor(deltaPayload), targetLen = cur.readVarint();
    if (targetLen < 0 || targetLen > NvePairProtocol.maxDecodedFrameBytes) {
      throw const NvePairProtocolException(NvePairProtocolErrorCode.frameTooLarge, 'frame exceeds limit');
    }
    final out = Uint8List(targetLen); var outPos = 0;
    while (outPos < targetLen) {
      if (cur.pos >= deltaPayload.length) throw const NvePairProtocolException(NvePairProtocolErrorCode.truncatedFrame, 'truncated xor-rle');
      final tag = deltaPayload[cur.pos++];
      if ((tag & 0x80) == 0) {
        final run = tag == 0 ? cur.readVarint() : tag;
        if (run <= 0 || outPos + run > targetLen) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'zero-run');
        final end = outPos + run, fb = end < base.length ? end : base.length;
        if (outPos < fb) out.setRange(outPos, fb, base, outPos);
        outPos = end;
      } else {
        final lit = tag == 0x80 ? cur.readVarint() : (tag & 0x7F);
        if (lit <= 0 || outPos + lit > targetLen) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'literal');
        if (deltaPayload.length - cur.pos < lit) throw const NvePairProtocolException(NvePairProtocolErrorCode.truncatedFrame, 'truncated literal');
        for (var k = 0; k < lit; k++) { out[outPos + k] = ((outPos + k) < base.length ? base[outPos + k] : 0) ^ deltaPayload[cur.pos + k]; }
        cur.pos += lit; outPos += lit;
      }
    }
    if (cur.pos != deltaPayload.length) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'trailing xor-rle');
    return out;
  }

  static List<(int, int)>? scanNveCommandSlices(Uint8List bytes) {
    if (bytes.length < 24) return null;
    final bd = ByteData.sublistView(bytes);
    if (bd.getUint32(0, Endian.little) != nveAbiMagic || bd.getUint16(4, Endian.little) != nveAbiVersion) return null;
    final opCount = bd.getUint32(20, Endian.little); var pos = 24; final ranges = <(int, int)>[];
    bool take(int n) { if (n < 0 || n > bytes.length - pos) return false; pos += n; return true; }
    int? u16() { if (pos + 2 > bytes.length) return null; final v = bd.getUint16(pos, Endian.little); pos += 2; return v; }
    int? u32() { if (pos + 4 > bytes.length) return null; final v = bd.getUint32(pos, Endian.little); pos += 4; return v; }
    for (var i = 0; i < opCount; i++) {
      final start = pos; if (pos >= bytes.length) return null;
      switch (bytes[pos++]) {
        case 1: if (!take(6)) return null; final n = u16(); if (n == null || !take(n)) return null;
        case 2: if (!take(4)) return null;
        case 3: if (!take(4)) return null; final c = u32(); if (c == null || c > (bytes.length - pos) ~/ 4 || !take(c * 4)) return null;
        case 4: if (!take(4)) return null; final l = u32(); if (l == null || !take(l)) return null;
        case 5: if (!take(81)) return null;
        case 6: if (!take(36)) return null;
        case 7:
          if (!take(4)) return null; final kl = u16(); if (kl == null || !take(kl) || pos >= bytes.length) return null;
          switch (bytes[pos++]) {
            case 0: break;
            case 1: if (!take(1)) return null;
            case 2 || 3: if (!take(8)) return null;
            case 4 || 5: final l = u32(); if (l == null || !take(l)) return null;
            default: return null;
          }
        case 8: if (!take(12)) return null;
        case 9: if (!take(4)) return null; final nl = u16(); if (nl == null || !take(nl)) return null; final pl = u32(); if (pl == null || !take(pl)) return null;
        case 10: if (!take(1)) return null;
        default: return null;
      }
      ranges.add((start, pos));
    }
    return pos == bytes.length ? ranges : null;
  }

  static Uint8List decodeCommandOpDelta(Uint8List base, Uint8List deltaPayload) {
    final prev = scanNveCommandSlices(base);
    if (prev == null) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'invalid NVE1 base');
    if (deltaPayload.length < 2) throw const NvePairProtocolException(NvePairProtocolErrorCode.truncatedFrame, 'truncated CommandOpDelta');
    final flags = ByteData.sublistView(deltaPayload).getUint16(0, Endian.little);
    final cur = _VarintCursor(deltaPayload, 2);
    final sid = cur.readVarint(), fid = cur.readVarint(), ops = cur.readVarint();
    if (sid < 0 || sid > 0xFFFFFFFF || ops < 0 || ops > 0xFFFFFFFF) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'header overflow');
    final out = BytesBuilder(copy: false), hdr = Uint8List(24);
    ByteData.sublistView(hdr)
      ..setUint32(0, nveAbiMagic, Endian.little)..setUint16(4, nveAbiVersion, Endian.little)
      ..setUint16(6, flags, Endian.little)..setUint32(8, sid, Endian.little)
      ..setUint64(12, fid, Endian.little)..setUint32(20, ops, Endian.little);
    out.add(hdr);

    var emitted = 0;
    while (cur.pos < deltaPayload.length) {
      switch (deltaPayload[cur.pos++]) {
        case NvePairProtocol.cmdDeltaCopyRun:
          final s = cur.readVarint(), c = cur.readVarint();
          if (c <= 0 || s < 0 || s + c > prev.length) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'copy_run');
          for (var i = s; i < s + c; i++) {
            final (ps, pe) = prev[i];
            if (out.length + (pe - ps) > NvePairProtocol.maxDecodedFrameBytes) throw const NvePairProtocolException(NvePairProtocolErrorCode.frameTooLarge, 'frame too large');
            out.add(Uint8List.sublistView(base, ps, pe)); emitted++;
          }
        case NvePairProtocol.cmdDeltaLiteralOp:
          final l = cur.readVarint();
          if (l <= 0 || deltaPayload.length - cur.pos < l) throw const NvePairProtocolException(NvePairProtocolErrorCode.truncatedFrame, 'literal');
          if (out.length + l > NvePairProtocol.maxDecodedFrameBytes) throw const NvePairProtocolException(NvePairProtocolErrorCode.frameTooLarge, 'frame too large');
          out.add(Uint8List.sublistView(deltaPayload, cur.pos, cur.pos + l)); cur.pos += l; emitted++;
        case NvePairProtocol.cmdDeltaXorOp:
          final pi = cur.readVarint(), xl = cur.readVarint();
          if (pi < 0 || pi >= prev.length) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'xor_op index');
          if (xl <= 0 || deltaPayload.length - cur.pos < xl) throw const NvePairProtocolException(NvePairProtocolErrorCode.truncatedFrame, 'xor_op');
          final (ps, pe) = prev[pi];
          final dec = decodeXorRleDelta(Uint8List.sublistView(base, ps, pe), Uint8List.sublistView(deltaPayload, cur.pos, cur.pos + xl));
          cur.pos += xl;
          if (out.length + dec.length > NvePairProtocol.maxDecodedFrameBytes) throw const NvePairProtocolException(NvePairProtocolErrorCode.frameTooLarge, 'frame too large');
          out.add(dec); emitted++;
        default: throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'unknown delta op');
      }
    }
    if (emitted != ops) throw const NvePairProtocolException(NvePairProtocolErrorCode.corruptPayload, 'op_count mismatch');
    return out.toBytes();
  }
}
