import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

Uint8List _buildNvevV2(List<Uint8List> records, Uint8List arena) {
  final builder = BytesBuilder(copy: false);
  final hdr = ByteData(14)
    ..setUint8(0, 0x4E)
    ..setUint8(1, 0x56)
    ..setUint8(2, 0x45)
    ..setUint8(3, 0x56)
    ..setUint16(4, 2, Endian.little)
    ..setUint32(6, records.length, Endian.little)
    ..setUint32(10, arena.length, Endian.little);
  builder.add(hdr.buffer.asUint8List());
  for (final r in records) {
    builder.add(r);
  }
  builder.add(arena);
  return builder.toBytes();
}

int _f32Bits(double v) => (ByteData(4)..setFloat32(0, v, Endian.little)).getUint32(0, Endian.little);

Uint8List _makeRecord({
  required int timestampUs,
  required int table,
  required int surfaceId,
  required int nodeId,
  required int eventId,
  int flags = 0,
  List<int> payload = const <int>[],
  int bytesOffset = 0,
  int bytesLen = 0,
}) {
  final bd = ByteData(64)
    ..setUint64(0, timestampUs, Endian.little)
    ..setUint64(8, table, Endian.little)
    ..setUint32(16, surfaceId, Endian.little)
    ..setUint32(20, nodeId, Endian.little)
    ..setUint32(24, eventId, Endian.little)
    ..setUint32(28, flags, Endian.little);
  for (var i = 0; i < payload.length && i < 6; i++) {
    bd.setUint32(32 + i * 4, payload[i], Endian.little);
  }
  bd
    ..setUint32(56, bytesOffset, Endian.little)
    ..setUint32(60, bytesLen, Endian.little);
  return bd.buffer.asUint8List();
}

void main() {
  group('ABI constants and enums', () {
    test('interface IDs match nve_abi.generated.h', () {
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
    test('decodes all 6 typed payload variants matching NVEV v2 wire layout', () {
      final customBin = NveBinValueEncoder.encode(<String, Object?>{'delta': 5});
      final textBytes = Uint8List.fromList(utf8.encode('Hello Flutter 🚀'));
      final nameBytes = Uint8List.fromList(utf8.encode('onIncrement'));
      final arena = BytesBuilder(copy: false);
      final textOff = arena.length;
      arena.add(textBytes);
      final customOff = arena.length;
      arena
        ..add(nameBytes)
        ..add(customBin);
      final bytes = _buildNvevV2(
        <Uint8List>[
          _makeRecord(
            timestampUs: 100,
            table: nveTableInteraction,
            surfaceId: 1,
            nodeId: 10,
            eventId: NveInteractionEvent.blur.code,
          ),
          _makeRecord(
            timestampUs: 200,
            table: nveTableInteraction,
            surfaceId: 1,
            nodeId: 11,
            eventId: NveInteractionEvent.press.code,
            payload: <int>[_f32Bits(42.5), _f32Bits(18.25)],
          ),
          _makeRecord(
            timestampUs: 300,
            table: nveTableScroll,
            surfaceId: 2,
            nodeId: 12,
            eventId: NveScrollEvent.scroll.code,
            payload: <int>[_f32Bits(12.0), _f32Bits(240.5), _f32Bits(390.0), _f32Bits(1600.0)],
          ),
          _makeRecord(
            timestampUs: 400,
            table: nveTableTextInput,
            surfaceId: 2,
            nodeId: 13,
            eventId: NveTextInputEvent.change.code,
            flags: nveEventFlagUtf8Bytes,
            bytesOffset: textOff,
            bytesLen: textBytes.length,
          ),
          _makeRecord(
            timestampUs: 500,
            table: nveTableLayout,
            surfaceId: 3,
            nodeId: 14,
            eventId: NveLayoutEvent.layout.code,
            payload: <int>[_f32Bits(8.0), _f32Bits(16.0), _f32Bits(320.0), _f32Bits(180.0)],
          ),
          _makeRecord(
            timestampUs: 600,
            table: nveTableCustom,
            surfaceId: 3,
            nodeId: 15,
            eventId: 0,
            flags: nveEventFlagCustomNamed,
            payload: <int>[nameBytes.length],
            bytesOffset: customOff,
            bytesLen: nameBytes.length + customBin.length,
          ),
        ],
        arena.takeBytes(),
      );

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
      expect(bytes.length, 78);
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

      // Oversized arena length (0xFFFFFFFF) must never throw RangeError.
      final oversizedArena = ByteData(14)
        ..setUint8(0, 0x4E)
        ..setUint8(1, 0x56)
        ..setUint8(2, 0x45)
        ..setUint8(3, 0x56)
        ..setUint16(4, 2, Endian.little)
        ..setUint32(6, 1, Endian.little)
        ..setUint32(10, 0xFFFFFFFF, Endian.little);
      expect(NveEventRingCodec.decode(oversizedArena.buffer.asUint8List()), isEmpty);
    });
  });
}
