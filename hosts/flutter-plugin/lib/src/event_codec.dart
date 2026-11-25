import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'nve_bin_value.dart';
import 'nve_core_abi.generated.dart';

/// Strongly typed zero-JSON event payload mirroring `EventPayload` in `packages/core/abi/src/events.rs`.
sealed class NveEventPayload {
  const NveEventPayload();

  /// Parses this payload as a zero-copy [NveBinValueView] when it is an [NveBinValuePayload].
  NveBinValueView? parseBinValue() => null;
}

final class NveEmptyPayload extends NveEventPayload {
  const NveEmptyPayload();

  @override
  bool operator ==(Object other) => other is NveEmptyPayload;

  @override
  int get hashCode => 0;
}

final class NvePointPayload extends NveEventPayload {
  const NvePointPayload({required this.x, required this.y});

  final double x;
  final double y;

  @override
  bool operator ==(Object other) =>
      identical(this, other) || (other is NvePointPayload && other.x == x && other.y == y);

  @override
  int get hashCode => Object.hash(x, y);
}

final class NveScrollPayload extends NveEventPayload {
  const NveScrollPayload({
    required this.x,
    required this.y,
    this.contentWidth = 0.0,
    this.contentHeight = 0.0,
  });

  final double x;
  final double y;
  final double contentWidth;
  final double contentHeight;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is NveScrollPayload &&
          other.x == x &&
          other.y == y &&
          other.contentWidth == contentWidth &&
          other.contentHeight == contentHeight);

  @override
  int get hashCode => Object.hash(x, y, contentWidth, contentHeight);
}

final class NveTextPayload extends NveEventPayload {
  const NveTextPayload(this.text);

  final String text;

  @override
  bool operator ==(Object other) =>
      identical(this, other) || (other is NveTextPayload && other.text == text);

  @override
  int get hashCode => text.hashCode;
}

final class NveLayoutPayload extends NveEventPayload {
  const NveLayoutPayload({
    this.x = 0.0,
    this.y = 0.0,
    required this.width,
    required this.height,
  });

  final double x;
  final double y;
  final double width;
  final double height;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is NveLayoutPayload &&
          other.x == x &&
          other.y == y &&
          other.width == width &&
          other.height == height);

  @override
  int get hashCode => Object.hash(x, y, width, height);
}

final class NveBinValuePayload extends NveEventPayload {
  const NveBinValuePayload(this.bytes);

  factory NveBinValuePayload.encode(Object? value) =>
      NveBinValuePayload(NveBinValueEncoder.encode(value));

  final Uint8List bytes;

  @override
  NveBinValueView? parseBinValue() {
    if (bytes.isEmpty) return null;
    try {
      return NveBinValueView.parse(bytes);
    } on NveBinValueException {
      return null;
    }
  }
}

/// Decoded outbound or inbound NVE event with strongly typed payload (zero JSON).
class NveEvent {
  const NveEvent({
    required this.surfaceId,
    required this.nodeId,
    required this.timestampUs,
    required this.kind,
    this.name,
    this.payload = const NveEmptyPayload(),
  });

  /// Convenience factory encoding a custom event whose payload is a compact binary `NveBinValue`.
  factory NveEvent.custom({
    required int surfaceId,
    int nodeId = 1,
    int? timestampUs,
    required String name,
    Object? value,
    Uint8List? rawBytes,
  }) {
    final payloadBytes = rawBytes ??
        (value != null ? NveBinValueEncoder.encode(value) : _emptyBytes);
    return NveEvent(
      surfaceId: surfaceId,
      nodeId: nodeId,
      timestampUs: timestampUs ?? DateTime.now().microsecondsSinceEpoch,
      kind: NveEventKind.custom,
      name: name,
      payload: NveBinValuePayload(payloadBytes),
    );
  }

  static final Uint8List _emptyBytes = Uint8List(0);

  final int surfaceId;
  final int nodeId;
  final int timestampUs;
  final NveEventKind kind;

  /// Custom event name (e.g. `"onIncrement"`) when [kind] is [NveEventKind.custom].
  final String? name;

  /// Strongly typed event payload.
  final NveEventPayload payload;

  /// Raw payload bytes for custom binary payloads (or UTF-8 bytes for text payloads).
  Uint8List get rawPayload => switch (payload) {
        NveBinValuePayload(:final bytes) => bytes,
        NveTextPayload(:final text) => Uint8List.fromList(utf8.encode(text)),
        _ => _emptyBytes,
      };

  /// Zero-copy `NveBinValueView` parser for custom binary payloads.
  NveBinValueView? parseBinValue() => payload.parseBinValue();

  factory NveEvent.fromPlatformMap(Map<Object?, Object?> map) {
    final kindRaw = map['kind'];
    final kind = kindRaw is int ? NveEventKind.fromCode(kindRaw) : NveEventKind.custom;
    final rawPayloadObj = map['rawPayload'] ?? map['payloadBin'];
    final payloadObj = map['payload'];

    final NveEventPayload payload;
    if (rawPayloadObj is Uint8List) {
      payload = NveBinValuePayload(rawPayloadObj);
    } else if (payloadObj is Map) {
      final typed = <String, Object?>{
        for (final entry in payloadObj.entries)
          if (entry.key is String) entry.key as String: entry.value,
      };
      payload = NveBinValuePayload.encode(typed);
    } else if (map['text'] is String) {
      payload = NveTextPayload(map['text'] as String);
    } else if (map['x'] is num && map['y'] is num) {
      final x = (map['x'] as num).toDouble();
      final y = (map['y'] as num).toDouble();
      if (kind == NveEventKind.scroll) {
        payload = NveScrollPayload(
          x: x,
          y: y,
          contentWidth: (map['contentWidth'] as num?)?.toDouble() ?? 0.0,
          contentHeight: (map['contentHeight'] as num?)?.toDouble() ?? 0.0,
        );
      } else if (kind == NveEventKind.layout) {
        payload = NveLayoutPayload(
          x: x,
          y: y,
          width: (map['width'] as num?)?.toDouble() ?? 0.0,
          height: (map['height'] as num?)?.toDouble() ?? 0.0,
        );
      } else {
        payload = NvePointPayload(x: x, y: y);
      }
    } else {
      payload = const NveEmptyPayload();
    }

    return NveEvent(
      surfaceId: (map['surfaceId'] as int?) ?? 0,
      nodeId: (map['nodeId'] as int?) ?? 0,
      timestampUs: (map['timestampUs'] as int?) ?? 0,
      kind: kind,
      name: map['name'] as String?,
      payload: payload,
    );
  }
}

typedef NveEventCallback = void Function(NveEvent event);

/// Encodes a single pointer `Press` event matching `packages/core/abi/src/events.rs`
/// (`NativeEventKind::Press = 1`, `EventPayload::Point { x, y } = tag 1`).
Uint8List encodePressEvent({
  required int surfaceId,
  required int nodeId,
  required double x,
  required double y,
  int? timestampUs,
}) {
  final ts = timestampUs ?? DateTime.now().microsecondsSinceEpoch;
  // 4 (count) + 4 (surfaceId) + 4 (nodeId) + 8 (timestampUs) + 1 (kind) + 1 (payloadTag) + 8 (x, y)
  final out = Uint8List(30);
  final bd = ByteData.sublistView(out)
    ..setUint32(0, 1, Endian.little)
    ..setUint32(4, surfaceId, Endian.little)
    ..setUint32(8, nodeId, Endian.little)
    ..setUint64(12, ts, Endian.little)
    ..setUint8(20, NveEventKind.press.code)
    ..setUint8(21, NveEventPayloadTag.point.code)
    ..setFloat32(22, x, Endian.little)
    ..setFloat32(26, y, Endian.little);
  assert(bd.lengthInBytes == 30);
  return out;
}

/// Encodes a single `Scroll` event matching `packages/core/abi/src/events.rs`
/// (`NativeEventKind::Scroll = 3`, `EventPayload::Scroll { x, y, content_width, content_height } = tag 2`).
Uint8List encodeScrollEvent({
  required int surfaceId,
  required int nodeId,
  required double x,
  required double y,
  required double contentWidth,
  required double contentHeight,
  int? timestampUs,
}) {
  final ts = timestampUs ?? DateTime.now().microsecondsSinceEpoch;
  // 4 (count) + 18 (header) + 16 (x, y, contentWidth, contentHeight) = 38 bytes
  final out = Uint8List(38);
  final bd = ByteData.sublistView(out)
    ..setUint32(0, 1, Endian.little)
    ..setUint32(4, surfaceId, Endian.little)
    ..setUint32(8, nodeId, Endian.little)
    ..setUint64(12, ts, Endian.little)
    ..setUint8(20, NveEventKind.scroll.code)
    ..setUint8(21, NveEventPayloadTag.scroll.code)
    ..setFloat32(22, x, Endian.little)
    ..setFloat32(26, y, Endian.little)
    ..setFloat32(30, contentWidth, Endian.little)
    ..setFloat32(34, contentHeight, Endian.little);
  assert(bd.lengthInBytes == 38);
  return out;
}

/// Binary encoder/decoder for `EventRing` buffers matching `packages/core/abi/src/events.rs`.
class NveEventRingCodec {
  const NveEventRingCodec._();

  static List<NveEvent> decode(Uint8List bytes) {
    if (bytes.length < 4) return const <NveEvent>[];
    final bd = ByteData.sublistView(bytes);
    var pos = 0;
    final count = bd.getUint32(pos, Endian.little);
    pos += 4;
    final events = <NveEvent>[];

    for (var i = 0; i < count; i++) {
      if (pos + 18 > bytes.length) break;
      final surfaceId = bd.getUint32(pos, Endian.little);
      pos += 4;
      final nodeId = bd.getUint32(pos, Endian.little);
      pos += 4;
      final timestampUs = bd.getUint64(pos, Endian.little);
      pos += 8;
      final kind = NveEventKind.fromCode(bytes[pos++]);
      final payloadTag = bytes[pos++];

      String? name;
      NveEventPayload payload = const NveEmptyPayload();

      var truncated = false;
      switch (payloadTag) {
        case 0: // Empty
          payload = const NveEmptyPayload();
        case 1: // Point { x, y }
          if (pos + 8 > bytes.length) {
            truncated = true;
            break;
          }
          final x = bd.getFloat32(pos, Endian.little);
          final y = bd.getFloat32(pos + 4, Endian.little);
          pos += 8;
          payload = NvePointPayload(x: x, y: y);
        case 2: // Scroll { x, y, content_width, content_height }
          if (pos + 16 > bytes.length) {
            truncated = true;
            break;
          }
          final x = bd.getFloat32(pos, Endian.little);
          final y = bd.getFloat32(pos + 4, Endian.little);
          final cw = bd.getFloat32(pos + 8, Endian.little);
          final ch = bd.getFloat32(pos + 12, Endian.little);
          pos += 16;
          payload = NveScrollPayload(x: x, y: y, contentWidth: cw, contentHeight: ch);
        case 3: // Text(String)
          if (pos + 4 > bytes.length) {
            truncated = true;
            break;
          }
          final len = bd.getUint32(pos, Endian.little);
          pos += 4;
          if (len > bytes.length - pos) {
            truncated = true;
            break;
          }
          final rawText = Uint8List.sublistView(bytes, pos, pos + len);
          pos += len;
          payload = NveTextPayload(utf8.decode(rawText, allowMalformed: true));
        case 4: // Layout { x, y, width, height }
          if (pos + 16 > bytes.length) {
            truncated = true;
            break;
          }
          final x = bd.getFloat32(pos, Endian.little);
          final y = bd.getFloat32(pos + 4, Endian.little);
          final w = bd.getFloat32(pos + 8, Endian.little);
          final h = bd.getFloat32(pos + 12, Endian.little);
          pos += 16;
          payload = NveLayoutPayload(x: x, y: y, width: w, height: h);
        case 5: // Custom { name, data }
          if (pos + 2 > bytes.length) {
            truncated = true;
            break;
          }
          final nameLen = bd.getUint16(pos, Endian.little);
          pos += 2;
          if (nameLen > bytes.length - pos) {
            truncated = true;
            break;
          }
          name = utf8.decode(
            Uint8List.sublistView(bytes, pos, pos + nameLen),
            allowMalformed: true,
          );
          pos += nameLen;
          if (pos + 4 > bytes.length) {
            truncated = true;
            break;
          }
          final dataLen = bd.getUint32(pos, Endian.little);
          pos += 4;
          if (dataLen > bytes.length - pos) {
            truncated = true;
            break;
          }
          final rawData = Uint8List.fromList(Uint8List.sublistView(bytes, pos, pos + dataLen));
          pos += dataLen;
          payload = NveBinValuePayload(rawData);
        default:
          truncated = true;
      }

      if (truncated) break;

      events.add(
        NveEvent(
          surfaceId: surfaceId,
          nodeId: nodeId,
          timestampUs: timestampUs,
          kind: kind,
          name: name,
          payload: payload,
        ),
      );
    }

    return events;
  }

  /// Encodes a batch of typed events into the binary `EventRing` wire format (zero JSON).
  static Uint8List encodeEvents(List<NveEvent> events) {
    final builder = BytesBuilder(copy: false);
    final header = ByteData(4)..setUint32(0, events.length, Endian.little);
    builder.add(header.buffer.asUint8List());

    for (final ev in events) {
      final int payloadTag = switch (ev.payload) {
        NveEmptyPayload() => NveEventPayloadTag.empty.code,
        NvePointPayload() => NveEventPayloadTag.point.code,
        NveScrollPayload() => NveEventPayloadTag.scroll.code,
        NveTextPayload() => NveEventPayloadTag.text.code,
        NveLayoutPayload() => NveEventPayloadTag.layout.code,
        NveBinValuePayload() => NveEventPayloadTag.custom.code,
      };
      final meta = ByteData(18)
        ..setUint32(0, ev.surfaceId, Endian.little)
        ..setUint32(4, ev.nodeId, Endian.little)
        ..setUint64(8, ev.timestampUs, Endian.little)
        ..setUint8(16, ev.kind.code)
        ..setUint8(17, payloadTag);
      builder.add(meta.buffer.asUint8List());

      switch (ev.payload) {
        case NveEmptyPayload():
          break;
        case NvePointPayload(:final x, :final y):
          final pt = ByteData(8)
            ..setFloat32(0, x, Endian.little)
            ..setFloat32(4, y, Endian.little);
          builder.add(pt.buffer.asUint8List());
        case NveScrollPayload(:final x, :final y, :final contentWidth, :final contentHeight):
          final sc = ByteData(16)
            ..setFloat32(0, x, Endian.little)
            ..setFloat32(4, y, Endian.little)
            ..setFloat32(8, contentWidth, Endian.little)
            ..setFloat32(12, contentHeight, Endian.little);
          builder.add(sc.buffer.asUint8List());
        case NveTextPayload(:final text):
          final textBytes = utf8.encode(text);
          final len = ByteData(4)..setUint32(0, textBytes.length, Endian.little);
          builder.add(len.buffer.asUint8List());
          builder.add(textBytes);
        case NveLayoutPayload(:final x, :final y, :final width, :final height):
          final lay = ByteData(16)
            ..setFloat32(0, x, Endian.little)
            ..setFloat32(4, y, Endian.little)
            ..setFloat32(8, width, Endian.little)
            ..setFloat32(12, height, Endian.little);
          builder.add(lay.buffer.asUint8List());
        case NveBinValuePayload(:final bytes):
          final nameBytes = utf8.encode(ev.name ?? '');
          final nameLen = ByteData(2)..setUint16(0, nameBytes.length, Endian.little);
          builder.add(nameLen.buffer.asUint8List());
          builder.add(nameBytes);
          final dataLen = ByteData(4)..setUint32(0, bytes.length, Endian.little);
          builder.add(dataLen.buffer.asUint8List());
          builder.add(bytes);
      }
    }

    return builder.toBytes();
  }
}
