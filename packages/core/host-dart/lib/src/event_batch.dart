/// Uniform 64-byte `NveEvent` batch encoder/validator/decoder and `NVEV` v2 binary wire codec
/// (web-safe, 53-bit-safe, bit-identical to Rust `nve_abi::event`).
library;

import 'dart:typed_data';

import 'bin_codec.dart' show BinCodecException;
import 'generated/nve_abi_const.generated.dart';
import 'hybrid_runtime.dart' show DynamicValue;
import 'key64.dart';
import 'runtime_event.dart';
import 'table_reader.dart' show f32FromBits, f32ToBits, nodeIndexMask;
import 'u64_math.dart' show isWeb, two32;
import 'utf8.dart' show decodeUtf8PreserveBom, encodeUtf8CodePointAt, isValidUtf8, utf16UnitsAt, utf8EncodedLength;

export 'runtime_event.dart';

/// Wire magic bytes for a binary `EventBatch` frame (`"NVEV"`, little-endian).
const int nveEventWireMagic = 0x5645564E;

/// Wire format version for 64-byte `NveEvent` batches.
const int nveEventWireVersion = 2;

/// Fixed `NVEV` v2 header size (`magic[4] + version:u16 + count:u32 + bytes_len:u32`).
const int nveEventWireHeaderSize = 14;

const int _internalFramingMask =
    nveEventFlagUtf8Bytes | nveEventFlagInlineKeyUtf8 | nveEventFlagCustomNamed | nveEventFlagKeySplitBytes;

int _canonicalHandle(int raw) => (raw & nodeIndexMask) == 0 ? 0 : (raw & 0xFFFFFFFF);

enum EventBatchErrorKind {
  truncated,
  trailingBytes,
  invalidMagic,
  unsupportedVersion,
  spanOutOfBounds,
  invalidUtf8,
  invalidInlineKeyLen,
  invalidSplitKeyLen,
  invalidCustomNameLen,
  conflictingFlags,
}

final class EventBatchException implements Exception {
  const EventBatchException(this.kind, this.message);

  final EventBatchErrorKind kind;
  final String message;

  @override
  String toString() => 'EventBatchException(${kind.name}): $message';
}

/// Encoded batch of uniform 64-byte little-endian `NveEvent` records + companion byte arena.
final class EventBatch {
  const EventBatch._(this.eventCount, this.records, this.bytes);

  static final EventBatch empty = EventBatch._(0, Uint8List(0), Uint8List(0));

  final int eventCount;
  final Uint8List records;
  final Uint8List bytes;

  /// Encodes [events] into contiguous 64-byte LE `NveEvent` records + shared byte arena.
  ///
  /// Throws [BinCodecException] on lone UTF-16 surrogates and [ArgumentError] on unsupported
  /// structured values.
  factory EventBatch.encode(List<RuntimeEvent> events, {int defaultSurfaceId = 0}) {
    if (events.isEmpty) {
      return empty;
    }
    final records = Uint8List(events.length * nveEventSize);
    final view = ByteData.sublistView(records);
    final arena = _ArenaBuilder();

    for (var i = 0; i < events.length; i++) {
      final e = events[i];
      final base = i * nveEventSize;
      final ts = e.timestamp;
      final rawSurface = _canonicalHandle(e.surfaceId);
      final surface = rawSurface != 0 ? rawSurface : _canonicalHandle(defaultSurfaceId);
      var flags = e.flags & ~_internalFramingMask;
      var p0 = 0, p1 = 0, p2 = 0, p3 = 0, p4 = 0, p5 = 0;
      var byteOff = 0, byteLen = 0;

      if (e.table == nveTableCustom) {
        final nameLen = utf8EncodedLength(e.name);
        final payloadBytes = e.data ?? (e.value != null ? DynamicValue.fromObject(e.value).bytes : null);
        final dataLen = payloadBytes?.length ?? 0;
        flags |= nveEventFlagCustomNamed;
        p0 = nameLen;
        if (nameLen + dataLen > 0) {
          byteOff = arena.length;
          arena.writeUtf8(e.name, nameLen);
          if (payloadBytes != null && dataLen > 0) {
            arena.writeBytes(payloadBytes);
          }
          byteLen = nameLen + dataLen;
        }
      } else if (e.table == nveTableInteraction &&
          (e.event == NveInteractionEvent.keyDown.code || e.event == NveInteractionEvent.keyUp.code)) {
        final keyLen = utf8EncodedLength(e.key);
        final codeLen = utf8EncodedLength(e.code);
        p0 = e.keyCode & 0xFFFFFFFF;
        p1 = keyLen;
        if (keyLen <= nveEventInlineKeyCap) {
          flags |= nveEventFlagInlineKeyUtf8;
          _encodeUtf8Into(e.key, records, base + 40);
          if (codeLen > 0) {
            flags |= nveEventFlagUtf8Bytes;
            byteOff = arena.length;
            arena.writeUtf8(e.code, codeLen);
            byteLen = codeLen;
          }
        } else {
          flags |= nveEventFlagKeySplitBytes | nveEventFlagUtf8Bytes;
          byteOff = arena.length;
          arena
            ..writeUtf8(e.key, keyLen)
            ..writeUtf8(e.code, codeLen);
          byteLen = keyLen + codeLen;
        }
      } else if (e.table == nveTableTextInput) {
        final textLen = utf8EncodedLength(e.text);
        flags |= nveEventFlagUtf8Bytes;
        if (textLen > 0) {
          byteOff = arena.length;
          arena.writeUtf8(e.text, textLen);
          byteLen = textLen;
        }
      } else if (e.table == nveTableInteraction) {
        switch (NveInteractionEvent.fromCode(e.event)) {
          case NveInteractionEvent.press:
          case NveInteractionEvent.longPress:
          case NveInteractionEvent.wheel:
            p0 = f32ToBits(e.x);
            p1 = f32ToBits(e.y);
          case NveInteractionEvent.pointerDown:
          case NveInteractionEvent.pointerMove:
          case NveInteractionEvent.pointerUp:
          case NveInteractionEvent.pointerEnter:
          case NveInteractionEvent.pointerLeave:
          case NveInteractionEvent.pointerCancel:
            p0 = f32ToBits(e.x);
            p1 = f32ToBits(e.y);
            p2 = e.pointerId & 0xFFFFFFFF;
          case NveInteractionEvent.focus:
          case NveInteractionEvent.blur:
          case NveInteractionEvent.keyDown:
          case NveInteractionEvent.keyUp:
          case null:
            break;
        }
      } else if (e.table == nveTableScroll || e.table == nveTableLayout) {
        p0 = f32ToBits(e.x);
        p1 = f32ToBits(e.y);
        p2 = f32ToBits(e.width);
        p3 = f32ToBits(e.height);
      } else {
        final rp = e.rawPayload;
        if (rp != null) {
          if (rp.isNotEmpty) p0 = rp[0] & 0xFFFFFFFF;
          if (rp.length > 1) p1 = rp[1] & 0xFFFFFFFF;
          if (rp.length > 2) p2 = rp[2] & 0xFFFFFFFF;
          if (rp.length > 3) p3 = rp[3] & 0xFFFFFFFF;
          if (rp.length > 4) p4 = rp[4] & 0xFFFFFFFF;
          if (rp.length > 5) p5 = rp[5] & 0xFFFFFFFF;
        }
        final rawData = e.data;
        if (rawData != null && rawData.isNotEmpty) {
          byteOff = arena.length;
          arena.writeBytes(rawData);
          byteLen = rawData.length;
        }
      }

      view
        ..setUint32(base + 0, ts.lo, Endian.little)
        ..setUint32(base + 4, ts.hi, Endian.little)
        ..setUint32(base + 8, e.table.lo, Endian.little)
        ..setUint32(base + 12, e.table.hi, Endian.little)
        ..setUint32(base + 16, surface, Endian.little)
        ..setUint32(base + 20, _canonicalHandle(e.nodeId), Endian.little)
        ..setUint32(base + 24, e.event & 0xFFFFFFFF, Endian.little)
        ..setUint32(base + 28, flags & 0xFFFFFFFF, Endian.little)
        ..setUint32(base + 32, p0, Endian.little)
        ..setUint32(base + 36, p1, Endian.little);
      // Preserve inline key bytes already written to `records[base + 40 .. base + 56]`.
      if ((flags & nveEventFlagInlineKeyUtf8) == 0) {
        view
          ..setUint32(base + 40, p2, Endian.little)
          ..setUint32(base + 44, p3, Endian.little)
          ..setUint32(base + 48, p4, Endian.little)
          ..setUint32(base + 52, p5, Endian.little);
      }
      view
        ..setUint32(base + 56, byteOff, Endian.little)
        ..setUint32(base + 60, byteLen, Endian.little);
    }

    return EventBatch._(events.length, records, arena.takeBytes());
  }

  /// Encodes this batch as a little-endian `NVEV` v2 wire frame.
  Uint8List toWireBytes() {
    final out = Uint8List(nveEventWireHeaderSize + records.length + bytes.length);
    final h = ByteData.sublistView(out);
    h
      ..setUint32(0, nveEventWireMagic, Endian.little)
      ..setUint16(4, nveEventWireVersion, Endian.little)
      ..setUint32(6, eventCount, Endian.little)
      ..setUint32(10, bytes.length, Endian.little);
    out.setRange(nveEventWireHeaderSize, nveEventWireHeaderSize + records.length, records);
    out.setRange(nveEventWireHeaderSize + records.length, out.length, bytes);
    return out;
  }

  /// Decodes and validates a little-endian `NVEV` v2 wire frame (mirrors Rust `EventBatchBuf::decode_wire`).
  factory EventBatch.fromWireBytes(Uint8List raw) {
    if (raw.length < nveEventWireHeaderSize) {
      throw const EventBatchException(EventBatchErrorKind.truncated, 'NVEV header truncated');
    }
    final h = ByteData.sublistView(raw);
    final magic = h.getUint32(0, Endian.little);
    if (magic != nveEventWireMagic) {
      throw EventBatchException(EventBatchErrorKind.invalidMagic, 'magic 0x${magic.toRadixString(16)}');
    }
    final version = h.getUint16(4, Endian.little);
    if (version != nveEventWireVersion) {
      throw EventBatchException(EventBatchErrorKind.unsupportedVersion, 'NVEV version $version != $nveEventWireVersion');
    }
    final count = h.getUint32(6, Endian.little);
    final bytesLen = h.getUint32(10, Endian.little);
    final bodyRem = raw.length - nveEventWireHeaderSize;
    if (count > bodyRem ~/ nveEventSize) {
      throw EventBatchException(EventBatchErrorKind.truncated, 'NVEV truncated: $count events');
    }
    final evBytes = count * nveEventSize;
    if (bytesLen > bodyRem - evBytes) {
      throw EventBatchException(EventBatchErrorKind.truncated, 'NVEV truncated: $bytesLen arena bytes');
    }
    final expectedTotal = nveEventWireHeaderSize + evBytes + bytesLen;
    if (raw.length > expectedTotal) {
      throw EventBatchException(EventBatchErrorKind.trailingBytes, '${raw.length - expectedTotal} trailing bytes');
    }
    final recView = Uint8List.sublistView(raw, nveEventWireHeaderSize, nveEventWireHeaderSize + evBytes);
    final arenaView = Uint8List.sublistView(raw, nveEventWireHeaderSize + evBytes, expectedTotal);
    validateRaw(recView, arenaView);
    return EventBatch._(count, Uint8List.fromList(recView), Uint8List.fromList(arenaView));
  }

  /// Validates every 64-byte record in [records] against [arena] (mirrors Rust `validate_one_event`).
  static void validateRaw(Uint8List records, Uint8List arena) {
    if (records.length % nveEventSize != 0) {
      throw EventBatchException(
          EventBatchErrorKind.truncated, 'records length ${records.length} not multiple of $nveEventSize');
    }
    final count = records.length ~/ nveEventSize;
    final view = ByteData.sublistView(records);
    for (var i = 0; i < count; i++) {
      final base = i * nveEventSize;
      final flags = view.getUint32(base + 28, Endian.little);
      final p0 = view.getUint32(base + 32, Endian.little);
      final p1 = view.getUint32(base + 36, Endian.little);
      final off = view.getUint32(base + 56, Endian.little);
      final len = view.getUint32(base + 60, Endian.little);
      if (off > arena.length || len > arena.length - off) {
        throw EventBatchException(
            EventBatchErrorKind.spanOutOfBounds, 'event $i span [$off, +$len) outside arena (${arena.length})');
      }
      final hasUtf8 = (flags & nveEventFlagUtf8Bytes) != 0;
      final hasInlineKey = (flags & nveEventFlagInlineKeyUtf8) != 0;
      final hasSplitKey = (flags & nveEventFlagKeySplitBytes) != 0;
      final hasCustom = (flags & nveEventFlagCustomNamed) != 0;
      if ((hasInlineKey && hasSplitKey) || (hasCustom && (hasInlineKey || hasSplitKey))) {
        throw EventBatchException(
            EventBatchErrorKind.conflictingFlags, 'event $i conflicting framing flags 0x${flags.toRadixString(16)}');
      }
      if (hasUtf8 && !isValidUtf8(arena, off, off + len)) {
        throw EventBatchException(EventBatchErrorKind.invalidUtf8, 'event $i invalid UTF-8 bytes');
      }
      if (hasInlineKey) {
        if (p1 > nveEventInlineKeyCap) {
          throw EventBatchException(
              EventBatchErrorKind.invalidInlineKeyLen, 'event $i inline key len $p1 > $nveEventInlineKeyCap');
        }
        if (!isValidUtf8(records, base + 40, base + 40 + p1)) {
          throw EventBatchException(EventBatchErrorKind.invalidUtf8, 'event $i invalid inline key UTF-8');
        }
      }
      if (hasSplitKey) {
        if (p1 > len) {
          throw EventBatchException(EventBatchErrorKind.invalidSplitKeyLen, 'event $i split key len $p1 > $len');
        }
        if (!isValidUtf8(arena, off, off + p1) || !isValidUtf8(arena, off + p1, off + len)) {
          throw EventBatchException(EventBatchErrorKind.invalidUtf8, 'event $i invalid split key/code UTF-8');
        }
      }
      if (hasCustom) {
        if (p0 > len) {
          throw EventBatchException(EventBatchErrorKind.invalidCustomNameLen, 'event $i custom name len $p0 > $len');
        }
        if (!isValidUtf8(arena, off, off + p0)) {
          throw EventBatchException(EventBatchErrorKind.invalidUtf8, 'event $i invalid custom name UTF-8');
        }
      }
    }
  }

  /// Validates and decodes raw `(records, arena)` into owned [RuntimeEvent]s (shared by wire + FFI).
  static List<RuntimeEvent> validateAndDecode(Uint8List records, Uint8List arena) {
    validateRaw(records, arena);
    final count = records.length ~/ nveEventSize;
    if (count == 0) {
      return const <RuntimeEvent>[];
    }
    final view = ByteData.sublistView(records);
    final out = <RuntimeEvent>[];
    for (var i = 0; i < count; i++) {
      final base = i * nveEventSize;
      final ts = Key64(view.getUint32(base + 0, Endian.little), view.getUint32(base + 4, Endian.little));
      final table = Key64(view.getUint32(base + 8, Endian.little), view.getUint32(base + 12, Endian.little));
      final surface = _canonicalHandle(view.getUint32(base + 16, Endian.little));
      final node = _canonicalHandle(view.getUint32(base + 20, Endian.little));
      final event = view.getUint32(base + 24, Endian.little);
      final flags = view.getUint32(base + 28, Endian.little);
      final p0 = view.getUint32(base + 32, Endian.little);
      final p1 = view.getUint32(base + 36, Endian.little);
      final p2 = view.getUint32(base + 40, Endian.little);
      final p3 = view.getUint32(base + 44, Endian.little);
      final p4 = view.getUint32(base + 48, Endian.little);
      final p5 = view.getUint32(base + 52, Endian.little);
      final off = view.getUint32(base + 56, Endian.little);
      final len = view.getUint32(base + 60, Endian.little);
      final tsUs = ts.hi < (isWeb ? 0x200000 : 0x80000000) ? ts.hi * two32 + ts.lo : 0;

      String utf8Slice(Uint8List src, int start, int n) =>
          n == 0 ? '' : decodeUtf8PreserveBom(Uint8List.sublistView(src, start, start + n));

      final hasUtf8 = (flags & nveEventFlagUtf8Bytes) != 0;
      final hasInlineKey = (flags & nveEventFlagInlineKeyUtf8) != 0;
      final hasSplitKey = (flags & nveEventFlagKeySplitBytes) != 0;
      final hasCustom = (flags & nveEventFlagCustomNamed) != 0;

      var x = 0.0, y = 0.0, width = 0.0, height = 0.0;
      var pointerId = 0, keyCode = 0;
      var key = '', code = '', text = '', name = '';
      Uint8List? data;
      Object? value;

      if (hasCustom) {
        name = utf8Slice(arena, off, p0);
        final rawData = Uint8List.fromList(Uint8List.sublistView(arena, off + p0, off + len));
        data = rawData;
        if (rawData.isNotEmpty) {
          try {
            value = DynamicValue(rawData).toObject();
          } on BinCodecException {
            // Malformed `NveBinValue` payload stays raw in `data` without dropping the batch.
            value = null;
          }
        }
      } else if (hasInlineKey) {
        keyCode = p0;
        key = utf8Slice(records, base + 40, p1);
        code = hasUtf8 ? utf8Slice(arena, off, len) : '';
      } else if (hasSplitKey) {
        keyCode = p0;
        key = utf8Slice(arena, off, p1);
        code = utf8Slice(arena, off + p1, len - p1);
      } else if (hasUtf8) {
        text = utf8Slice(arena, off, len);
      } else if (len > 0) {
        data = Uint8List.fromList(Uint8List.sublistView(arena, off, off + len));
      }

      if (table == nveTableInteraction) {
        switch (NveInteractionEvent.fromCode(event)) {
          case NveInteractionEvent.press:
          case NveInteractionEvent.longPress:
          case NveInteractionEvent.wheel:
            x = f32FromBits(p0);
            y = f32FromBits(p1);
          case NveInteractionEvent.pointerDown:
          case NveInteractionEvent.pointerMove:
          case NveInteractionEvent.pointerUp:
          case NveInteractionEvent.pointerEnter:
          case NveInteractionEvent.pointerLeave:
          case NveInteractionEvent.pointerCancel:
            x = f32FromBits(p0);
            y = f32FromBits(p1);
            pointerId = p2;
          case NveInteractionEvent.focus:
          case NveInteractionEvent.blur:
          case NveInteractionEvent.keyDown:
          case NveInteractionEvent.keyUp:
          case null:
            break;
        }
      } else if (table == nveTableScroll || table == nveTableLayout) {
        x = f32FromBits(p0);
        y = f32FromBits(p1);
        width = f32FromBits(p2);
        height = f32FromBits(p3);
      }

      out.add(RuntimeEvent(
        table: table,
        event: event,
        surfaceId: surface,
        nodeId: node,
        timestampUs: tsUs,
        timestamp: ts,
        flags: flags,
        x: x,
        y: y,
        width: width,
        height: height,
        pointerId: pointerId,
        keyCode: keyCode,
        key: key,
        code: code,
        text: text,
        name: name,
        data: data,
        value: value,
        rawPayload: List<int>.unmodifiable(<int>[p0, p1, p2, p3, p4, p5]),
      ));
    }
    return out;
  }

  /// Decodes this batch into owned [RuntimeEvent]s.
  List<RuntimeEvent> decodeEvents() => validateAndDecode(records, bytes);
}

int _encodeUtf8Into(String s, Uint8List out, int offset) {
  var o = offset;
  for (var i = 0; i < s.length; i += utf16UnitsAt(s, i)) {
    o = encodeUtf8CodePointAt(s, i, out, o);
  }
  return o;
}

final class _ArenaBuilder {
  Uint8List _buf = Uint8List(0);
  int _len = 0;

  int get length => _len;

  void _grow(int extra) {
    final need = _len + extra;
    if (need <= _buf.length) {
      return;
    }
    var cap = _buf.isEmpty ? 64 : _buf.length * 2;
    while (cap < need) {
      cap *= 2;
    }
    final next = Uint8List(cap)..setRange(0, _len, _buf);
    _buf = next;
  }

  void writeUtf8(String s, int byteLen) {
    if (byteLen == 0) {
      return;
    }
    _grow(byteLen);
    _len = _encodeUtf8Into(s, _buf, _len);
  }

  void writeBytes(Uint8List src) {
    if (src.isEmpty) {
      return;
    }
    _grow(src.length);
    _buf.setRange(_len, _len + src.length, src);
    _len += src.length;
  }

  Uint8List takeBytes() => _len == 0 ? Uint8List(0) : Uint8List.sublistView(_buf, 0, _len);
}
