import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'nve_bin_value.dart';
import 'nve_core_abi.generated.dart' hide NveEvent;
import 'nve_wire_codecs.generated.dart';

/// High-level Flutter event classification mapped onto uniform 64-byte `NveEvent` records.
enum NveEventKind {
  press(1),
  longPress(2),
  scroll(3),
  textChange(4),
  focus(5),
  blur(6),
  layout(7),
  custom(8),
  compositionStart(9),
  compositionUpdate(10),
  compositionEnd(11),
  keyDown(12),
  keyUp(13),
  submit(14),
  pointerDown(15),
  pointerMove(16),
  pointerUp(17),
  pointerEnter(18),
  pointerLeave(19),
  wheel(20),
  tableEvent(21);

  const NveEventKind(this.code);
  final int code;

  static NveEventKind fromCode(int code) => switch (code) {
        1 => NveEventKind.press,
        2 => NveEventKind.longPress,
        3 => NveEventKind.scroll,
        4 => NveEventKind.textChange,
        5 => NveEventKind.focus,
        6 => NveEventKind.blur,
        7 => NveEventKind.layout,
        9 => NveEventKind.compositionStart,
        10 => NveEventKind.compositionUpdate,
        11 => NveEventKind.compositionEnd,
        12 => NveEventKind.keyDown,
        13 => NveEventKind.keyUp,
        14 => NveEventKind.submit,
        15 => NveEventKind.pointerDown,
        16 => NveEventKind.pointerMove,
        17 => NveEventKind.pointerUp,
        18 => NveEventKind.pointerEnter,
        19 => NveEventKind.pointerLeave,
        20 => NveEventKind.wheel,
        21 => NveEventKind.tableEvent,
        _ => NveEventKind.custom,
      };
}

/// Strongly typed binary event payload for Flutter host events.
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

final class NveKeyPayload extends NveEventPayload {
  const NveKeyPayload({
    required this.key,
    this.code = '',
    this.modifiers = 0,
    this.keyCode = 0,
    this.isComposing = false,
  });

  final String key;
  final String code;
  final int modifiers;
  final int keyCode;
  final bool isComposing;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is NveKeyPayload &&
          other.key == key &&
          other.code == code &&
          other.modifiers == modifiers &&
          other.keyCode == keyCode &&
          other.isComposing == isComposing);

  @override
  int get hashCode => Object.hash(key, code, modifiers, keyCode, isComposing);
}

final class NvePointerPayload extends NveEventPayload {
  const NvePointerPayload({
    required this.x,
    required this.y,
    this.modifiers = 0,
    this.flags = 0,
  });

  final double x;
  final double y;
  final int modifiers;
  final int flags;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is NvePointerPayload &&
          other.x == x &&
          other.y == y &&
          other.modifiers == modifiers &&
          other.flags == flags);

  @override
  int get hashCode => Object.hash(x, y, modifiers, flags);
}

final class NveWheelPayload extends NveEventPayload {
  const NveWheelPayload({
    required this.deltaX,
    required this.deltaY,
    this.modifiers = 0,
  });

  final double deltaX;
  final double deltaY;
  final int modifiers;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is NveWheelPayload &&
          other.deltaX == deltaX &&
          other.deltaY == deltaY &&
          other.modifiers == modifiers);

  @override
  int get hashCode => Object.hash(deltaX, deltaY, modifiers);
}

final class NveTableEventPayload extends NveEventPayload {
  const NveTableEventPayload({
    required this.tableKey,
    required this.eventId,
    required this.bytes,
  });

  final int tableKey;
  final int eventId;
  final Uint8List bytes;

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    if (other is! NveTableEventPayload ||
        other.tableKey != tableKey ||
        other.eventId != eventId ||
        other.bytes.length != bytes.length) {
      return false;
    }
    for (var i = 0; i < bytes.length; i++) {
      if (other.bytes[i] != bytes[i]) return false;
    }
    return true;
  }

  @override
  int get hashCode => Object.hash(tableKey, eventId, Object.hashAll(bytes));
}

/// Decoded outbound or inbound NVE event with strongly typed payload (binary).
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

/// Encodes a single pointer `Press` event in `NVEV` v2 wire format.
Uint8List encodePressEvent({
  required int surfaceId,
  required int nodeId,
  required double x,
  required double y,
  int? timestampUs,
}) {
  final w = NveWireEventBatchWriter();
  w.pushPress(surfaceId, nodeId, timestampUs ?? DateTime.now().microsecondsSinceEpoch, x, y);
  return w.finish();
}

/// Encodes a single `Scroll` event in `NVEV` v2 wire format.
Uint8List encodeScrollEvent({
  required int surfaceId,
  required int nodeId,
  required double x,
  required double y,
  required double contentWidth,
  required double contentHeight,
  int? timestampUs,
}) {
  final w = NveWireEventBatchWriter();
  w.pushScroll(
    surfaceId,
    nodeId,
    timestampUs ?? DateTime.now().microsecondsSinceEpoch,
    x,
    y,
    contentWidth,
    contentHeight,
  );
  return w.finish();
}

/// Binary encoder/decoder for `NVEV` v2 event batch wire frames matching `EventBatchBuf`.
class NveEventRingCodec {
  const NveEventRingCodec._();

  static double _bitsF32(int bits) {
    final bd = ByteData(4)..setUint32(0, bits, Endian.little);
    return bd.getFloat32(0, Endian.little);
  }

  static int _f32Bits(double v) {
    final bd = ByteData(4)..setFloat32(0, v, Endian.little);
    return bd.getUint32(0, Endian.little);
  }

  static List<NveEvent> decode(Uint8List bytes) {
    final hdr = nveWireDecodeEventBatchHeader(bytes);
    if (hdr == null) return const <NveEvent>[];
    final evBytes = hdr.eventCount * nveWireEventRecordSize;
    final arenaStart = nveWireEventHeaderSize + evBytes;
    if (arenaStart + hdr.bytesLen > bytes.length) return const <NveEvent>[];

    final bd = ByteData.sublistView(bytes);
    final arena = Uint8List.sublistView(bytes, arenaStart, arenaStart + hdr.bytesLen);
    final events = <NveEvent>[];

    for (var i = 0; i < hdr.eventCount; i++) {
      final base = nveWireEventHeaderSize + i * nveWireEventRecordSize;
      final ts = bd.getUint64(base + 0, Endian.little);
      final table = bd.getUint64(base + 8, Endian.little);
      final surfaceId = bd.getUint32(base + 16, Endian.little);
      final nodeId = bd.getUint32(base + 20, Endian.little);
      final eventId = bd.getUint32(base + 24, Endian.little);
      final flags = bd.getUint32(base + 28, Endian.little);
      final p0 = bd.getUint32(base + 32, Endian.little);
      final p1 = bd.getUint32(base + 36, Endian.little);
      final p2 = bd.getUint32(base + 40, Endian.little);
      final p3 = bd.getUint32(base + 44, Endian.little);
      final p4 = bd.getUint32(base + 48, Endian.little);
      final p5 = bd.getUint32(base + 52, Endian.little);
      final bytesOff = bd.getUint32(base + 56, Endian.little);
      final bytesLen = bd.getUint32(base + 60, Endian.little);
      final slice = (bytesLen == 0 || bytesOff + bytesLen > arena.length)
          ? Uint8List(0)
          : Uint8List.fromList(arena.sublist(bytesOff, bytesOff + bytesLen));

      String? name;
      final NveEventKind kind;
      final NveEventPayload payload;

      if (table == nveTableCustom) {
        kind = NveEventKind.custom;
        final nlen = p0 <= slice.length ? p0 : slice.length;
        name = utf8.decode(slice.sublist(0, nlen), allowMalformed: true);
        payload = NveBinValuePayload(Uint8List.fromList(slice.sublist(nlen)));
      } else if (table == nveTableInteraction) {
        final ie = NveInteractionEvent.fromCode(eventId);
        switch (ie) {
          case NveInteractionEvent.press:
            kind = NveEventKind.press;
            payload = NvePointPayload(x: _bitsF32(p0), y: _bitsF32(p1));
          case NveInteractionEvent.longPress:
            kind = NveEventKind.longPress;
            payload = NvePointPayload(x: _bitsF32(p0), y: _bitsF32(p1));
          case NveInteractionEvent.focus:
            kind = NveEventKind.focus;
            payload = const NveEmptyPayload();
          case NveInteractionEvent.blur:
            kind = NveEventKind.blur;
            payload = const NveEmptyPayload();
          case NveInteractionEvent.keyDown || NveInteractionEvent.keyUp:
            kind = ie == NveInteractionEvent.keyDown ? NveEventKind.keyDown : NveEventKind.keyUp;
            final String key;
            final String code;
            if ((flags & nveEventFlagInlineKeyUtf8) != 0) {
              final klen = p1 <= nveEventInlineKeyCap ? p1 : nveEventInlineKeyCap;
              final kb = Uint8List(16);
              final kbd = ByteData.sublistView(kb);
              kbd.setUint32(0, p2, Endian.little);
              kbd.setUint32(4, p3, Endian.little);
              kbd.setUint32(8, p4, Endian.little);
              kbd.setUint32(12, p5, Endian.little);
              key = utf8.decode(kb.sublist(0, klen), allowMalformed: true);
              code = utf8.decode(slice, allowMalformed: true);
            } else {
              final klen = p1 <= slice.length ? p1 : slice.length;
              key = utf8.decode(slice.sublist(0, klen), allowMalformed: true);
              code = utf8.decode(slice.sublist(klen), allowMalformed: true);
            }
            payload = NveKeyPayload(
              key: key,
              code: code,
              modifiers: flags & 0x1F,
              keyCode: p0,
              isComposing: (flags & nveEventFlagComposing) != 0,
            );
          case NveInteractionEvent.pointerDown ||
              NveInteractionEvent.pointerMove ||
              NveInteractionEvent.pointerUp ||
              NveInteractionEvent.pointerEnter ||
              NveInteractionEvent.pointerLeave:
            kind = switch (ie) {
              NveInteractionEvent.pointerDown => NveEventKind.pointerDown,
              NveInteractionEvent.pointerMove => NveEventKind.pointerMove,
              NveInteractionEvent.pointerUp => NveEventKind.pointerUp,
              NveInteractionEvent.pointerEnter => NveEventKind.pointerEnter,
              _ => NveEventKind.pointerLeave,
            };
            final btn = (flags & nveEventFlagButtonMask) >> nveEventFlagButtonShift;
            final cap = (flags & nveEventFlagCapture) != 0 ? 0x08 : 0;
            payload = NvePointerPayload(
              x: _bitsF32(p0),
              y: _bitsF32(p1),
              modifiers: flags & 0x1F,
              flags: btn | cap,
            );
          case NveInteractionEvent.wheel:
            kind = NveEventKind.wheel;
            payload = NveWheelPayload(deltaX: _bitsF32(p0), deltaY: _bitsF32(p1), modifiers: flags & 0x1F);
          case null:
            continue;
        }
      } else if (table == nveTableTextInput) {
        kind = switch (NveTextInputEvent.fromCode(eventId)) {
          NveTextInputEvent.change => NveEventKind.textChange,
          NveTextInputEvent.submit => NveEventKind.submit,
          NveTextInputEvent.compositionStart => NveEventKind.compositionStart,
          NveTextInputEvent.compositionUpdate => NveEventKind.compositionUpdate,
          NveTextInputEvent.compositionEnd => NveEventKind.compositionEnd,
          null => NveEventKind.textChange,
        };
        payload = NveTextPayload(utf8.decode(slice, allowMalformed: true));
      } else if (table == nveTableScroll) {
        kind = NveEventKind.scroll;
        payload = NveScrollPayload(
          x: _bitsF32(p0),
          y: _bitsF32(p1),
          contentWidth: _bitsF32(p2),
          contentHeight: _bitsF32(p3),
        );
      } else if (table == nveTableLayout) {
        kind = NveEventKind.layout;
        payload = NveLayoutPayload(
          x: _bitsF32(p0),
          y: _bitsF32(p1),
          width: _bitsF32(p2),
          height: _bitsF32(p3),
        );
      } else {
        kind = NveEventKind.tableEvent;
        payload = NveTableEventPayload(tableKey: table, eventId: eventId, bytes: slice);
      }

      events.add(
        NveEvent(
          surfaceId: surfaceId,
          nodeId: nodeId,
          timestampUs: ts,
          kind: kind,
          name: name,
          payload: payload,
        ),
      );
    }

    return events;
  }

  /// Encodes a batch of typed events into the binary `NVEV` v2 wire format.
  static Uint8List encodeEvents(List<NveEvent> events) {
    final w = NveWireEventBatchWriter();
    for (final ev in events) {
      switch (ev.payload) {
        case NveEmptyPayload():
          if (ev.kind == NveEventKind.blur) {
            w.pushBlur(ev.surfaceId, ev.nodeId, ev.timestampUs);
          } else {
            w.pushFocus(ev.surfaceId, ev.nodeId, ev.timestampUs);
          }
        case NvePointPayload(:final x, :final y):
          if (ev.kind == NveEventKind.longPress) {
            w.pushLongPress(ev.surfaceId, ev.nodeId, ev.timestampUs, x, y);
          } else {
            w.pushPress(ev.surfaceId, ev.nodeId, ev.timestampUs, x, y);
          }
        case NveScrollPayload(:final x, :final y, :final contentWidth, :final contentHeight):
          w.pushScroll(ev.surfaceId, ev.nodeId, ev.timestampUs, x, y, contentWidth, contentHeight);
        case NveTextPayload(:final text):
          final tid = switch (ev.kind) {
            NveEventKind.submit => NveWireTextInputEvent.submit,
            NveEventKind.compositionStart => NveWireTextInputEvent.compositionStart,
            NveEventKind.compositionUpdate => NveWireTextInputEvent.compositionUpdate,
            NveEventKind.compositionEnd => NveWireTextInputEvent.compositionEnd,
            _ => NveWireTextInputEvent.change,
          };
          w.pushEvent(
            nveWireTableKeyTextInput,
            ev.surfaceId,
            ev.nodeId,
            ev.timestampUs,
            tid,
            nveWireEventFlagUtf8Bytes,
            const <int>[0, 0, 0, 0, 0, 0],
            utf8.encode(text),
          );
        case NveLayoutPayload(:final x, :final y, :final width, :final height):
          w.pushLayout(ev.surfaceId, ev.nodeId, ev.timestampUs, x, y, width, height);
        case NveBinValuePayload(:final bytes):
          w.pushCustom(ev.surfaceId, ev.nodeId, ev.timestampUs, ev.name ?? '', bytes);
        case NveKeyPayload(:final key, :final code, :final modifiers, :final keyCode, :final isComposing):
          w.pushKeyDown(
            ev.surfaceId,
            ev.nodeId,
            ev.timestampUs,
            key,
            meta: (modifiers & nveWireEventFlagModMeta) != 0,
            ctrl: (modifiers & nveWireEventFlagModCtrl) != 0,
            shift: (modifiers & nveWireEventFlagModShift) != 0,
            alt: (modifiers & nveWireEventFlagModAlt) != 0,
            code: code,
            keyCode: keyCode,
            isComposing: isComposing,
          );
        case NvePointerPayload(:final x, :final y, :final modifiers, :final flags):
          final evId = switch (ev.kind) {
            NveEventKind.pointerDown => NveWireInteractionEvent.pointerDown,
            NveEventKind.pointerUp => NveWireInteractionEvent.pointerUp,
            NveEventKind.pointerEnter => NveWireInteractionEvent.pointerEnter,
            NveEventKind.pointerLeave => NveWireInteractionEvent.pointerLeave,
            _ => NveWireInteractionEvent.pointerMove,
          };
          final f = (modifiers & 0x1F) |
              (((flags & 0x07) << nveWireEventFlagButtonShift) & nveWireEventFlagButtonMask) |
              ((flags & 0x08) != 0 ? nveWireEventFlagCapture : 0);
          w.pushEvent(
            nveWireTableKeyInteraction,
            ev.surfaceId,
            ev.nodeId,
            ev.timestampUs,
            evId,
            f,
            <int>[_f32Bits(x), _f32Bits(y), 0, 0, 0, 0],
          );
        case NveWheelPayload(:final deltaX, :final deltaY, :final modifiers):
          w.pushEvent(
            nveWireTableKeyInteraction,
            ev.surfaceId,
            ev.nodeId,
            ev.timestampUs,
            NveWireInteractionEvent.wheel,
            modifiers & 0x1F,
            <int>[_f32Bits(deltaX), _f32Bits(deltaY), 0, 0, 0, 0],
          );
        case NveTableEventPayload(:final tableKey, :final eventId, :final bytes):
          w.pushEvent(
            tableKey,
            ev.surfaceId,
            ev.nodeId,
            ev.timestampUs,
            eventId,
            0,
            const <int>[0, 0, 0, 0, 0, 0],
            bytes,
          );
      }
    }
    return w.finish();
  }
}
