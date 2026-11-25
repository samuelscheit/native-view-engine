import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

Uint8List _buildEventRing(List<void Function(BytesBuilder)> writers) {
  final builder = BytesBuilder(copy: false);
  final countHeader = ByteData(4)..setUint32(0, writers.length, Endian.little);
  builder.add(countHeader.buffer.asUint8List());
  for (final write in writers) {
    write(builder);
  }
  return builder.toBytes();
}

void _writeEventHeader(
  BytesBuilder builder, {
  required int surfaceId,
  required int nodeId,
  required int timestampUs,
  required int kindCode,
  required int payloadTag,
}) {
  final meta = ByteData(18)
    ..setUint32(0, surfaceId, Endian.little)
    ..setUint32(4, nodeId, Endian.little)
    ..setUint64(8, timestampUs, Endian.little)
    ..setUint8(16, kindCode)
    ..setUint8(17, payloadTag);
  builder.add(meta.buffer.asUint8List());
}

void main() {
  group('ABI constants and enums', () {
    test('interface IDs match nve_abi.h', () {
      expect(nveIfaceSurface, 1);
      expect(nveIfaceLayout, 2);
      expect(nveIfaceComponent, 3);
      expect(nveIfaceDev, 4);
    });

    test('NveGestureDisposition codes and fromCode mapping', () {
      expect(NveGestureDisposition.claim.code, 1);
      expect(NveGestureDisposition.passThrough.code, 2);
      expect(NveGestureDisposition.deferToParent.code, 3);

      expect(NveGestureDisposition.fromCode(1), NveGestureDisposition.claim);
      expect(NveGestureDisposition.fromCode(2), NveGestureDisposition.passThrough);
      expect(NveGestureDisposition.fromCode(3), NveGestureDisposition.deferToParent);
      expect(NveGestureDisposition.fromCode(99), NveGestureDisposition.passThrough);
    });

    test('NveEventKind codes and fromCode mapping', () {
      expect(NveEventKind.fromCode(1), NveEventKind.press);
      expect(NveEventKind.fromCode(2), NveEventKind.longPress);
      expect(NveEventKind.fromCode(3), NveEventKind.scroll);
      expect(NveEventKind.fromCode(4), NveEventKind.textChange);
      expect(NveEventKind.fromCode(5), NveEventKind.focus);
      expect(NveEventKind.fromCode(6), NveEventKind.blur);
      expect(NveEventKind.fromCode(7), NveEventKind.layout);
      expect(NveEventKind.fromCode(8), NveEventKind.custom);
      expect(NveEventKind.fromCode(255), NveEventKind.custom);
    });

    test('NveRenderMode enum values', () {
      expect(NveRenderMode.values, <NveRenderMode>[
        NveRenderMode.texture,
        NveRenderMode.platformView,
      ]);
    });
  });

  group('NveEvent', () {
    test('defaults rawPayload to empty Uint8List', () {
      const event = NveEvent(
        surfaceId: 1,
        nodeId: 2,
        timestampUs: 3,
        kind: NveEventKind.press,
      );
      expect(event.rawPayload, isEmpty);
      expect(event.name, isNull);
      expect(event.payloadJson, isNull);
    });

    test('fromPlatformMap parses map payload, raw JSON payload, and defaults', () {
      final fromMap = NveEvent.fromPlatformMap(<Object?, Object?>{
        'surfaceId': 7,
        'nodeId': 14,
        'timestampUs': 9001,
        'kind': NveEventKind.custom.code,
        'name': 'onIncrement',
        'payload': <Object?, Object?>{'count': 3, 'ok': true},
      });
      expect(fromMap.surfaceId, 7);
      expect(fromMap.nodeId, 14);
      expect(fromMap.timestampUs, 9001);
      expect(fromMap.kind, NveEventKind.custom);
      expect(fromMap.name, 'onIncrement');
      expect(fromMap.payloadJson, <String, dynamic>{'count': 3, 'ok': true});

      final rawJson = Uint8List.fromList(utf8.encode('{"value":"abc"}'));
      final fromRawJson = NveEvent.fromPlatformMap(<Object?, Object?>{
        'surfaceId': 2,
        'nodeId': 3,
        'timestampUs': 4,
        'kind': NveEventKind.custom.code,
        'rawPayload': rawJson,
      });
      expect(fromRawJson.payloadJson, <String, dynamic>{'value': 'abc'});
      expect(fromRawJson.rawPayload, rawJson);

      final fromBinary = NveEvent.fromPlatformMap(<Object?, Object?>{
        'kind': 'not-an-int',
        'rawPayload': Uint8List.fromList(<int>[0xFF, 0x00, 0x01]),
      });
      expect(fromBinary.surfaceId, 0);
      expect(fromBinary.nodeId, 0);
      expect(fromBinary.timestampUs, 0);
      expect(fromBinary.kind, NveEventKind.custom);
      expect(fromBinary.payloadJson, isNull);
      expect(fromBinary.rawPayload, Uint8List.fromList(<int>[0xFF, 0x00, 0x01]));
    });
  });

  group('NveEventRingCodec', () {
    test('decodes all 6 payload variants matching crates/nve-abi/src/events.rs', () {
      final bytes = _buildEventRing(<void Function(BytesBuilder)>[
        // 0: Empty
        (b) => _writeEventHeader(
              b,
              surfaceId: 1,
              nodeId: 10,
              timestampUs: 100,
              kindCode: NveEventKind.blur.code,
              payloadTag: 0,
            ),
        // 1: Point
        (b) {
          _writeEventHeader(
            b,
            surfaceId: 1,
            nodeId: 11,
            timestampUs: 200,
            kindCode: NveEventKind.press.code,
            payloadTag: 1,
          );
          final pt = ByteData(8)
            ..setFloat32(0, 42.5, Endian.little)
            ..setFloat32(4, 18.25, Endian.little);
          b.add(pt.buffer.asUint8List());
        },
        // 2: Scroll
        (b) {
          _writeEventHeader(
            b,
            surfaceId: 2,
            nodeId: 12,
            timestampUs: 300,
            kindCode: NveEventKind.scroll.code,
            payloadTag: 2,
          );
          final sc = ByteData(16)
            ..setFloat32(0, 12.0, Endian.little)
            ..setFloat32(4, 240.5, Endian.little)
            ..setFloat32(8, 390.0, Endian.little)
            ..setFloat32(12, 1600.0, Endian.little);
          b.add(sc.buffer.asUint8List());
        },
        // 3: Text (UTF-8)
        (b) {
          _writeEventHeader(
            b,
            surfaceId: 2,
            nodeId: 13,
            timestampUs: 400,
            kindCode: NveEventKind.textChange.code,
            payloadTag: 3,
          );
          final textBytes = utf8.encode('Hello Flutter 🚀');
          final len = ByteData(4)..setUint32(0, textBytes.length, Endian.little);
          b.add(len.buffer.asUint8List());
          b.add(textBytes);
        },
        // 4: Layout
        (b) {
          _writeEventHeader(
            b,
            surfaceId: 3,
            nodeId: 14,
            timestampUs: 500,
            kindCode: NveEventKind.layout.code,
            payloadTag: 4,
          );
          final layout = ByteData(16)
            ..setFloat32(0, 8.0, Endian.little)
            ..setFloat32(4, 16.0, Endian.little)
            ..setFloat32(8, 320.0, Endian.little)
            ..setFloat32(12, 180.0, Endian.little);
          b.add(layout.buffer.asUint8List());
        },
        // 5: Custom with JSON payload
        (b) {
          _writeEventHeader(
            b,
            surfaceId: 3,
            nodeId: 15,
            timestampUs: 600,
            kindCode: NveEventKind.custom.code,
            payloadTag: 5,
          );
          final nameBytes = utf8.encode('onIncrement');
          final dataBytes = utf8.encode('{"delta":5}');
          final nameLen = ByteData(2)..setUint16(0, nameBytes.length, Endian.little);
          final dataLen = ByteData(4)..setUint32(0, dataBytes.length, Endian.little);
          b.add(nameLen.buffer.asUint8List());
          b.add(nameBytes);
          b.add(dataLen.buffer.asUint8List());
          b.add(dataBytes);
        },
      ]);

      final events = NveEventRingCodec.decode(bytes);
      expect(events, hasLength(6));

      expect(events[0].kind, NveEventKind.blur);
      expect(events[0].payloadJson, isNull);

      expect(events[1].kind, NveEventKind.press);
      expect(events[1].payloadJson?['x'], closeTo(42.5, 0.001));
      expect(events[1].payloadJson?['y'], closeTo(18.25, 0.001));

      expect(events[2].kind, NveEventKind.scroll);
      expect(events[2].payloadJson?['x'], closeTo(12.0, 0.001));
      expect(events[2].payloadJson?['y'], closeTo(240.5, 0.001));
      expect(events[2].payloadJson?['contentWidth'], closeTo(390.0, 0.001));
      expect(events[2].payloadJson?['contentHeight'], closeTo(1600.0, 0.001));

      expect(events[3].kind, NveEventKind.textChange);
      expect(events[3].payloadJson?['text'], 'Hello Flutter 🚀');

      expect(events[4].kind, NveEventKind.layout);
      expect(events[4].payloadJson?['x'], closeTo(8.0, 0.001));
      expect(events[4].payloadJson?['y'], closeTo(16.0, 0.001));
      expect(events[4].payloadJson?['width'], closeTo(320.0, 0.001));
      expect(events[4].payloadJson?['height'], closeTo(180.0, 0.001));

      expect(events[5].kind, NveEventKind.custom);
      expect(events[5].name, 'onIncrement');
      expect(events[5].payloadJson, <String, dynamic>{'delta': 5});
    });

    test('encodeCustomEvents roundtrips JSON, raw binary, and empty payloads', () {
      final rawBlob = Uint8List.fromList(<int>[9, 8, 7, 6]);
      final input = <NveEvent>[
        const NveEvent(
          surfaceId: 10,
          nodeId: 2,
          timestampUs: 123456,
          kind: NveEventKind.custom,
          name: 'onSave',
          payloadJson: <String, dynamic>{'title': 'Draft', 'count': 4},
        ),
        NveEvent(
          surfaceId: 10,
          nodeId: 3,
          timestampUs: 123789,
          kind: NveEventKind.custom,
          name: 'onRaw',
          payloadJson: const <String, dynamic>{'ignored': true},
          rawPayload: rawBlob,
        ),
        const NveEvent(
          surfaceId: 10,
          nodeId: 4,
          timestampUs: 123999,
          kind: NveEventKind.custom,
          name: 'onEmpty',
        ),
      ];

      final encoded = NveEventRingCodec.encodeCustomEvents(input);
      final decoded = NveEventRingCodec.decode(encoded);
      expect(decoded, hasLength(3));

      expect(decoded[0].surfaceId, 10);
      expect(decoded[0].nodeId, 2);
      expect(decoded[0].timestampUs, 123456);
      expect(decoded[0].name, 'onSave');
      expect(decoded[0].payloadJson, <String, dynamic>{'title': 'Draft', 'count': 4});

      // rawPayload takes precedence over payloadJson when non-empty.
      expect(decoded[1].name, 'onRaw');
      expect(decoded[1].rawPayload, rawBlob);
      expect(decoded[1].payloadJson, isNull);

      expect(decoded[2].name, 'onEmpty');
      expect(decoded[2].rawPayload, isEmpty);
      expect(decoded[2].payloadJson, isNull);
    });

    test('handles short, empty, and truncated buffers safely', () {
      expect(NveEventRingCodec.decode(Uint8List(0)), isEmpty);
      expect(NveEventRingCodec.decode(Uint8List.fromList(<int>[1, 2, 3])), isEmpty);

      final zeroCount = Uint8List(4);
      expect(NveEventRingCodec.decode(zeroCount), isEmpty);

      // Header claims 2 events, only 1 complete event + 4 trailing bytes exist.
      final truncated = BytesBuilder()
        ..add((ByteData(4)..setUint32(0, 2, Endian.little)).buffer.asUint8List());
      _writeEventHeader(
        truncated,
        surfaceId: 4,
        nodeId: 5,
        timestampUs: 6,
        kindCode: NveEventKind.focus.code,
        payloadTag: 0,
      );
      truncated.add(<int>[1, 2, 3, 4]);

      final decoded = NveEventRingCodec.decode(truncated.toBytes());
      expect(decoded, hasLength(1));
      expect(decoded[0].surfaceId, 4);
      expect(decoded[0].kind, NveEventKind.focus);

      // Every prefix truncation of a rich multi-payload EventRing must return cleanly without throwing.
      final full = _buildEventRing(<void Function(BytesBuilder)>[
        (b) {
          _writeEventHeader(b, surfaceId: 1, nodeId: 1, timestampUs: 1, kindCode: 1, payloadTag: 1);
          b.add((ByteData(8)..setFloat32(0, 1.0, Endian.little)..setFloat32(4, 2.0, Endian.little))
              .buffer
              .asUint8List());
        },
        (b) {
          _writeEventHeader(b, surfaceId: 1, nodeId: 2, timestampUs: 2, kindCode: 3, payloadTag: 2);
          b.add(Uint8List(16));
        },
        (b) {
          _writeEventHeader(b, surfaceId: 1, nodeId: 3, timestampUs: 3, kindCode: 4, payloadTag: 3);
          final txt = utf8.encode('hello');
          b.add((ByteData(4)..setUint32(0, txt.length, Endian.little)).buffer.asUint8List());
          b.add(txt);
        },
        (b) {
          _writeEventHeader(b, surfaceId: 1, nodeId: 4, timestampUs: 4, kindCode: 7, payloadTag: 4);
          b.add(Uint8List(16));
        },
        (b) {
          _writeEventHeader(b, surfaceId: 1, nodeId: 5, timestampUs: 5, kindCode: 8, payloadTag: 5);
          final name = utf8.encode('ev');
          final data = utf8.encode('{"a":1}');
          b.add((ByteData(2)..setUint16(0, name.length, Endian.little)).buffer.asUint8List());
          b.add(name);
          b.add((ByteData(4)..setUint32(0, data.length, Endian.little)).buffer.asUint8List());
          b.add(data);
        },
      ]);
      for (var cut = 0; cut < full.length; cut++) {
        final partial = NveEventRingCodec.decode(Uint8List.sublistView(full, 0, cut));
        expect(partial.length, lessThan(5));
      }

      // Oversized length (0xFFFFFFFF) in Text and Custom payloads must never throw RangeError.
      final oversizedText = BytesBuilder()
        ..add((ByteData(4)..setUint32(0, 1, Endian.little)).buffer.asUint8List());
      _writeEventHeader(
        oversizedText,
        surfaceId: 1,
        nodeId: 9,
        timestampUs: 10,
        kindCode: NveEventKind.textChange.code,
        payloadTag: 3,
      );
      oversizedText.add((ByteData(4)..setUint32(0, 0xFFFFFFFF, Endian.little)).buffer.asUint8List());
      expect(NveEventRingCodec.decode(oversizedText.toBytes()), isEmpty);
    });
  });
}
