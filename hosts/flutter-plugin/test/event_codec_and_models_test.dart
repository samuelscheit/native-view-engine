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
    test('defaults payload to NveEmptyPayload and empty rawPayload', () {
      const event = NveEvent(
        surfaceId: 1,
        nodeId: 2,
        timestampUs: 3,
        kind: NveEventKind.press,
      );
      expect(event.payload, isA<NveEmptyPayload>());
      expect(event.rawPayload, isEmpty);
      expect(event.name, isNull);
      expect(event.parseBinValue(), isNull);
    });

    test('fromPlatformMap parses map payload (as NveBinValue), raw binary payload, and typed fields', () {
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
      final view = fromMap.parseBinValue();
      expect(view?.get('count')?.asInt64(), 3);
      expect(view?.get('ok')?.asBool(), isTrue);

      final rawBin = NveBinValueEncoder.encode(<String, Object?>{'value': 'abc'});
      final fromRawBin = NveEvent.fromPlatformMap(<Object?, Object?>{
        'surfaceId': 2,
        'nodeId': 3,
        'timestampUs': 4,
        'kind': NveEventKind.custom.code,
        'rawPayload': rawBin,
      });
      expect(fromRawBin.parseBinValue()?.get('value')?.asString(), 'abc');
      expect(fromRawBin.rawPayload, rawBin);

      final fromInvalidBinary = NveEvent.fromPlatformMap(<Object?, Object?>{
        'kind': 'not-an-int',
        'rawPayload': Uint8List.fromList(<int>[0xFF, 0x00, 0x01]),
      });
      expect(fromInvalidBinary.surfaceId, 0);
      expect(fromInvalidBinary.nodeId, 0);
      expect(fromInvalidBinary.timestampUs, 0);
      expect(fromInvalidBinary.kind, NveEventKind.custom);
      expect(fromInvalidBinary.parseBinValue(), isNull);
      expect(fromInvalidBinary.rawPayload, Uint8List.fromList(<int>[0xFF, 0x00, 0x01]));
    });
  });

  group('NveEventRingCodec', () {
    test('decodes all 6 typed payload variants matching crates/nve-abi/src/events.rs', () {
      final customBin = NveBinValueEncoder.encode(<String, Object?>{'delta': 5});
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
        // 5: Custom with binary NveBinValue payload
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
          final nameLen = ByteData(2)..setUint16(0, nameBytes.length, Endian.little);
          final dataLen = ByteData(4)..setUint32(0, customBin.length, Endian.little);
          b.add(nameLen.buffer.asUint8List());
          b.add(nameBytes);
          b.add(dataLen.buffer.asUint8List());
          b.add(customBin);
        },
      ]);

      final events = NveEventRingCodec.decode(bytes);
      expect(events, hasLength(6));

      expect(events[0].kind, NveEventKind.blur);
      expect(events[0].payload, isA<NveEmptyPayload>());

      expect(events[1].kind, NveEventKind.press);
      final pt = events[1].payload as NvePointPayload;
      expect(pt.x, closeTo(42.5, 0.001));
      expect(pt.y, closeTo(18.25, 0.001));

      expect(events[2].kind, NveEventKind.scroll);
      final sc = events[2].payload as NveScrollPayload;
      expect(sc.x, closeTo(12.0, 0.001));
      expect(sc.y, closeTo(240.5, 0.001));
      expect(sc.contentWidth, closeTo(390.0, 0.001));
      expect(sc.contentHeight, closeTo(1600.0, 0.001));

      expect(events[3].kind, NveEventKind.textChange);
      expect((events[3].payload as NveTextPayload).text, 'Hello Flutter 🚀');

      expect(events[4].kind, NveEventKind.layout);
      final lay = events[4].payload as NveLayoutPayload;
      expect(lay.x, closeTo(8.0, 0.001));
      expect(lay.y, closeTo(16.0, 0.001));
      expect(lay.width, closeTo(320.0, 0.001));
      expect(lay.height, closeTo(180.0, 0.001));

      expect(events[5].kind, NveEventKind.custom);
      expect(events[5].name, 'onIncrement');
      expect(events[5].parseBinValue()?.get('delta')?.asInt64(), 5);
    });

    test('encodeEvents roundtrips all payload types including binary NveBinValue and raw blobs', () {
      final rawBlob = Uint8List.fromList(<int>[9, 8, 7, 6]);
      final input = <NveEvent>[
        const NveEvent(
          surfaceId: 10,
          nodeId: 1,
          timestampUs: 100,
          kind: NveEventKind.focus,
        ),
        const NveEvent(
          surfaceId: 10,
          nodeId: 2,
          timestampUs: 200,
          kind: NveEventKind.press,
          payload: NvePointPayload(x: 15.5, y: 25.25),
        ),
        const NveEvent(
          surfaceId: 10,
          nodeId: 3,
          timestampUs: 300,
          kind: NveEventKind.scroll,
          payload: NveScrollPayload(x: 0.0, y: 120.5, contentWidth: 300.0, contentHeight: 900.0),
        ),
        const NveEvent(
          surfaceId: 10,
          nodeId: 4,
          timestampUs: 400,
          kind: NveEventKind.textChange,
          payload: NveTextPayload('Typed'),
        ),
        const NveEvent(
          surfaceId: 10,
          nodeId: 5,
          timestampUs: 500,
          kind: NveEventKind.layout,
          payload: NveLayoutPayload(x: 1.0, y: 2.0, width: 100.0, height: 50.0),
        ),
        NveEvent.custom(
          surfaceId: 10,
          nodeId: 6,
          timestampUs: 600,
          name: 'onSave',
          value: <String, Object?>{'title': 'Draft', 'count': 4},
        ),
        NveEvent.custom(
          surfaceId: 10,
          nodeId: 7,
          timestampUs: 700,
          name: 'onRaw',
          rawBytes: rawBlob,
        ),
      ];

      final encoded = NveEventRingCodec.encodeEvents(input);
      final decoded = NveEventRingCodec.decode(encoded);
      expect(decoded, hasLength(7));

      expect(decoded[0].payload, const NveEmptyPayload());
      expect(decoded[1].payload, const NvePointPayload(x: 15.5, y: 25.25));
      expect(
        decoded[2].payload,
        const NveScrollPayload(x: 0.0, y: 120.5, contentWidth: 300.0, contentHeight: 900.0),
      );
      expect(decoded[3].payload, const NveTextPayload('Typed'));
      expect(decoded[4].payload, const NveLayoutPayload(x: 1.0, y: 2.0, width: 100.0, height: 50.0));
      expect(decoded[5].name, 'onSave');
      expect(decoded[5].parseBinValue()?.get('title')?.asString(), 'Draft');
      expect(decoded[5].parseBinValue()?.get('count')?.asInt64(), 4);
      expect(decoded[6].name, 'onRaw');
      expect(decoded[6].rawPayload, rawBlob);
      expect(decoded[6].parseBinValue(), isNull);
    });

    test('encodeScrollEvent matches wire EventRing format', () {
      final bytes = encodeScrollEvent(
        surfaceId: 4,
        nodeId: 12,
        x: 10.5,
        y: 250.25,
        contentWidth: 400.0,
        contentHeight: 1200.0,
        timestampUs: 999999,
      );
      expect(bytes.length, 38);
      final decoded = NveEventRingCodec.decode(bytes);
      expect(decoded, hasLength(1));
      expect(decoded.single.surfaceId, 4);
      expect(decoded.single.nodeId, 12);
      expect(decoded.single.timestampUs, 999999);
      expect(decoded.single.kind, NveEventKind.scroll);
      expect(
        decoded.single.payload,
        const NveScrollPayload(x: 10.5, y: 250.25, contentWidth: 400.0, contentHeight: 1200.0),
      );
    });

    test('handles short, empty, and truncated buffers safely', () {
      expect(NveEventRingCodec.decode(Uint8List(0)), isEmpty);
      expect(NveEventRingCodec.decode(Uint8List.fromList(<int>[1, 2, 3])), isEmpty);
      expect(NveEventRingCodec.decode(Uint8List(4)), isEmpty);

      final full = NveEventRingCodec.encodeEvents(<NveEvent>[
        const NveEvent(
          surfaceId: 1,
          nodeId: 1,
          timestampUs: 1,
          kind: NveEventKind.press,
          payload: NvePointPayload(x: 1.0, y: 2.0),
        ),
        const NveEvent(
          surfaceId: 1,
          nodeId: 2,
          timestampUs: 2,
          kind: NveEventKind.scroll,
          payload: NveScrollPayload(x: 0.0, y: 50.0, contentWidth: 100.0, contentHeight: 400.0),
        ),
        const NveEvent(
          surfaceId: 1,
          nodeId: 3,
          timestampUs: 3,
          kind: NveEventKind.textChange,
          payload: NveTextPayload('hello'),
        ),
        const NveEvent(
          surfaceId: 1,
          nodeId: 4,
          timestampUs: 4,
          kind: NveEventKind.layout,
          payload: NveLayoutPayload(x: 0.0, y: 0.0, width: 100.0, height: 50.0),
        ),
        NveEvent.custom(
          surfaceId: 1,
          nodeId: 5,
          timestampUs: 5,
          name: 'ev',
          value: <String, Object?>{'a': 1},
        ),
      ]);
      for (var cut = 0; cut < full.length; cut++) {
        final partial = NveEventRingCodec.decode(Uint8List.sublistView(full, 0, cut));
        expect(partial.length, lessThan(5));
      }

      // Oversized length (0xFFFFFFFF) must never throw RangeError.
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
