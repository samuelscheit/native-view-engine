import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

import 'check.dart';

void _expectErr(Check c, EventBatchErrorKind kind, void Function() body, String what) {
  final e = c.throwsA<EventBatchException>(body, what);
  if (e != null) {
    c.eq(e.kind, kind, '$what kind');
  }
}

void runEventBatch(Check c) {
  final tsBig = const Key64(0x12345678, 0xABCDEF01); // > 2^53 for dart2js safety
  final customTable = Key64.of('app.slider');

  final events = <RuntimeEvent>[
    const RuntimeEvent.press(10, 12.5, 34.25, timestampUs: 100),
    const RuntimeEvent.longPress(11, 1.0, 2.0, timestampUs: 101),
    const RuntimeEvent.focus(12, focused: true, timestampUs: 102),
    const RuntimeEvent.focus(12, focused: false, timestampUs: 103),
    RuntimeEvent.pointer(
      NveInteractionEvent.pointerDown,
      13,
      50.5,
      60.5,
      pointerId: 7,
      button: 2,
      modifiers: nveEventFlagModShift | nveEventFlagModMeta,
      capture: true,
      timestampUs: 104,
    ),
    const RuntimeEvent.wheel(14, -3.5, 4.5, modifiers: nveEventFlagModCtrl, timestampUs: 105),
    // Inline key (<= 16 B UTF-8) with BOM and physical code in arena.
    RuntimeEvent.key(
      NveInteractionEvent.keyDown,
      15,
      '\uFEFFEnter',
      'Enter',
      modifiers: nveEventFlagModAlt | nveEventFlagModRepeat,
      keyCode: 13,
      composing: true,
      timestampUs: 106,
    ),
    // Exact 16-byte UTF-8 inline key boundary.
    RuntimeEvent.key(NveInteractionEvent.keyUp, 15, '0123456789abcdef', '', keyCode: 65, timestampUs: 107),
    // Long key (> 16 B UTF-8) -> KEY_SPLIT_BYTES in arena.
    RuntimeEvent.key(
      NveInteractionEvent.keyDown,
      15,
      'DeadCircumflexKey123',
      'BracketLeft',
      keyCode: 219,
      timestampUs: 108,
    ),
    const RuntimeEvent.textChange(16, '\uFEFFhéllo 🚀', timestampUs: 109),
    const RuntimeEvent.submit(16, 'done', timestampUs: 110),
    RuntimeEvent.composition(NveTextInputEvent.compositionUpdate, 16, '漢', timestampUs: 111),
    const RuntimeEvent.scroll(17, 10.0, 20.5, 300.0, 900.0, timestampUs: 112),
    const RuntimeEvent.layout(18, 4.0, 8.0, 120.0, 48.0, timestampUs: 113),
    const RuntimeEvent.custom(1, 'props', value: {'n': 2, 'ok': true, 'ratio': 0.5}, timestampUs: 114),
    RuntimeEvent.custom(1, 'malformed', data: Uint8List.fromList([0xFF]), timestampUs: 115),
    RuntimeEvent.table(
      customTable,
      3,
      surfaceId: 9,
      nodeId: 20,
      payload: const [11, 22, 33, 44, 55, 66],
      data: Uint8List.fromList([1, 2, 3]),
      timestamp: tsBig,
    ),
  ];

  final batch = EventBatch.encode(events, defaultSurfaceId: 4);
  c.eq(batch.eventCount, events.length, 'encoded event count');
  c.eq(batch.records.length, events.length * nveEventSize, '64 bytes per event');

  // Wire round-trip (`NVEV` v2).
  final wire = batch.toWireBytes();
  c.eq(wire.length, nveEventWireHeaderSize + batch.records.length + batch.bytes.length, 'wire length');
  final h = ByteData.sublistView(wire);
  c.eq(h.getUint32(0, Endian.little), nveEventWireMagic, 'wire magic NVEV');
  c.eq(h.getUint16(4, Endian.little), nveEventWireVersion, 'wire version 2');
  c.eq(h.getUint32(6, Endian.little), events.length, 'wire count');
  c.eq(h.getUint32(10, Endian.little), batch.bytes.length, 'wire bytesLen');

  final decoded = EventBatch.fromWireBytes(wire).decodeEvents();
  c.eq(decoded.length, events.length, 'decoded count');

  // Press / longPress / focus / blur.
  c.eq(decoded[0].interaction, NveInteractionEvent.press, 'press enum');
  c.eq((decoded[0].surfaceId, decoded[0].nodeId, decoded[0].x, decoded[0].y), (4, 10, 12.5, 34.25), 'press fields');
  c.eq(decoded[1].interaction, NveInteractionEvent.longPress, 'longPress enum');
  c.eq(decoded[2].interaction, NveInteractionEvent.focus, 'focus enum');
  c.eq(decoded[3].interaction, NveInteractionEvent.blur, 'blur enum');

  // Pointer.
  final ptr = decoded[4];
  c.eq(ptr.interaction, NveInteractionEvent.pointerDown, 'pointerDown');
  c.eq((ptr.x, ptr.y, ptr.pointerId, ptr.pointerButton, ptr.isCapture), (50.5, 60.5, 7, 2, true), 'pointer fields');
  c.eq(ptr.modifiers, nveEventFlagModShift | nveEventFlagModMeta, 'pointer modifiers');

  // Wheel.
  c.eq(decoded[5].interaction, NveInteractionEvent.wheel, 'wheel');
  c.eq((decoded[5].x, decoded[5].y, decoded[5].modifiers), (-3.5, 4.5, nveEventFlagModCtrl), 'wheel fields');

  // Keys: inline (<= 16 B), exact 16 B, and split (> 16 B).
  final kInline = decoded[6];
  c.that((kInline.flags & nveEventFlagInlineKeyUtf8) != 0, 'inline key flag');
  c.eq((kInline.key, kInline.code, kInline.keyCode, kInline.isComposing), ('\uFEFFEnter', 'Enter', 13, true), 'inline key');
  c.eq(kInline.text, '', 'inline key code never leaks into text');
  final k16 = decoded[7];
  c.that((k16.flags & nveEventFlagInlineKeyUtf8) != 0 && (k16.flags & nveEventFlagUtf8Bytes) == 0, '16 B inline key, empty code');
  c.eq((k16.key, k16.code), ('0123456789abcdef', ''), '16 B key exact');
  final kSplit = decoded[8];
  c.that((kSplit.flags & nveEventFlagKeySplitBytes) != 0, 'split key flag');
  c.eq((kSplit.key, kSplit.code, kSplit.keyCode), ('DeadCircumflexKey123', 'BracketLeft', 219), 'split key');

  // TextInput.
  c.eq(decoded[9].textInput, NveTextInputEvent.change, 'textChange');
  c.eq(decoded[9].text, '\uFEFFhéllo 🚀', 'textChange preserves BOM + emoji');
  c.eq((decoded[10].textInput, decoded[10].text), (NveTextInputEvent.submit, 'done'), 'submit');
  c.eq((decoded[11].textInput, decoded[11].text), (NveTextInputEvent.compositionUpdate, '漢'), 'composition');

  // Scroll + Layout.
  c.eq(decoded[12].scrollEvent, NveScrollEvent.scroll, 'scroll');
  c.eq((decoded[12].x, decoded[12].y, decoded[12].width, decoded[12].height), (10.0, 20.5, 300.0, 900.0), 'scroll fields');
  c.eq(decoded[13].layoutEvent, NveLayoutEvent.layout, 'layout');
  c.eq((decoded[13].x, decoded[13].y, decoded[13].width, decoded[13].height), (4.0, 8.0, 120.0, 48.0), 'layout fields');

  // Custom: valid NveBinValue and invalid NveBinValue (`[0xFF]`).
  c.that(decoded[14].isCustom && decoded[14].name == 'props', 'custom props');
  c.eq((decoded[14].value as Map<String, Object?>)['n'], 2, 'custom BinValue decoded');
  c.that(decoded[15].isCustom && decoded[15].name == 'malformed', 'custom malformed');
  c.eq(decoded[15].value, null, 'invalid BinValue yields null value');
  c.eq(decoded[15].data?.toList().join(','), '255', 'invalid BinValue raw bytes preserved');

  // Generic table event + exact 64-bit timestamp.
  final gen = decoded[16];
  c.eq((gen.table, gen.event, gen.surfaceId, gen.nodeId), (customTable, 3, 9, 20), 'generic table event');
  c.eq(gen.timestamp, tsBig, '64-bit timestamp exact above 2^53');
  c.eq(gen.rawPayload?.join(','), '11,22,33,44,55,66', 'raw payload preserved');
  c.eq(gen.data?.join(','), '1,2,3', 'generic data preserved');

  // Empty batch round-trip.
  c.eq(EventBatch.fromWireBytes(EventBatch.empty.toWireBytes()).decodeEvents().length, 0, 'empty wire round-trip');

  // Lone surrogate rejected on encode.
  c.throwsA<BinCodecException>(() => EventBatch.encode([const RuntimeEvent.textChange(1, '\uD800')]), 'lone surrogate text');
  c.throwsA<BinCodecException>(() => EventBatch.encode([RuntimeEvent.key(NveInteractionEvent.keyDown, 1, '\uD800', '')]), 'lone surrogate key');

  // Wire / validation error cases.
  _expectErr(c, EventBatchErrorKind.truncated, () => EventBatch.fromWireBytes(Uint8List(10)), 'truncated header');
  final badMagic = Uint8List.fromList(wire)..[0] = 0x58;
  _expectErr(c, EventBatchErrorKind.invalidMagic, () => EventBatch.fromWireBytes(badMagic), 'bad magic');
  final badVer = Uint8List.fromList(wire)..[4] = 1;
  _expectErr(c, EventBatchErrorKind.unsupportedVersion, () => EventBatch.fromWireBytes(badVer), 'v1 rejected');
  _expectErr(c, EventBatchErrorKind.truncated, () => EventBatch.fromWireBytes(Uint8List.sublistView(wire, 0, wire.length - 1)), 'truncated body');
  final trailing = Uint8List(wire.length + 1)..setRange(0, wire.length, wire);
  _expectErr(c, EventBatchErrorKind.trailingBytes, () => EventBatch.fromWireBytes(trailing), 'trailing byte');

  // Record-level validation failures.
  Uint8List oneRecord({int flags = 0, int p0 = 0, int p1 = 0, int off = 0, int len = 0, List<int> inline = const []}) {
    final rec = Uint8List(nveEventSize);
    final v = ByteData.sublistView(rec);
    v
      ..setUint32(28, flags, Endian.little)
      ..setUint32(32, p0, Endian.little)
      ..setUint32(36, p1, Endian.little)
      ..setUint32(56, off, Endian.little)
      ..setUint32(60, len, Endian.little);
    rec.setRange(40, 40 + inline.length, inline);
    return rec;
  }

  _expectErr(c, EventBatchErrorKind.spanOutOfBounds, () => EventBatch.validateRaw(oneRecord(off: 1, len: 2), Uint8List(2)), 'span oob');
  _expectErr(
      c, EventBatchErrorKind.spanOutOfBounds, () => EventBatch.validateRaw(oneRecord(off: 0xFFFFFFFF, len: 1), Uint8List(4)), 'span wrap');
  _expectErr(
      c,
      EventBatchErrorKind.conflictingFlags,
      () => EventBatch.validateRaw(oneRecord(flags: nveEventFlagInlineKeyUtf8 | nveEventFlagKeySplitBytes), Uint8List(0)),
      'inline+split conflict');
  _expectErr(
      c,
      EventBatchErrorKind.conflictingFlags,
      () => EventBatch.validateRaw(oneRecord(flags: nveEventFlagCustomNamed | nveEventFlagInlineKeyUtf8), Uint8List(0)),
      'custom+inline conflict');
  _expectErr(
      c, EventBatchErrorKind.invalidUtf8, () => EventBatch.validateRaw(oneRecord(flags: nveEventFlagUtf8Bytes, len: 1), Uint8List.fromList([0xFF])), 'bad utf8 bytes');
  _expectErr(
      c, EventBatchErrorKind.invalidInlineKeyLen, () => EventBatch.validateRaw(oneRecord(flags: nveEventFlagInlineKeyUtf8, p1: 17), Uint8List(0)), 'inline key len 17');
  _expectErr(
      c,
      EventBatchErrorKind.invalidUtf8,
      () => EventBatch.validateRaw(oneRecord(flags: nveEventFlagInlineKeyUtf8, p1: 2, inline: [0xC0, 0xAF]), Uint8List(0)),
      'bad inline key utf8');
  _expectErr(
      c, EventBatchErrorKind.invalidSplitKeyLen, () => EventBatch.validateRaw(oneRecord(flags: nveEventFlagKeySplitBytes, p1: 3, len: 2), Uint8List.fromList([65, 66])), 'split key len > len');
  _expectErr(
      c, EventBatchErrorKind.invalidCustomNameLen, () => EventBatch.validateRaw(oneRecord(flags: nveEventFlagCustomNamed, p0: 5, len: 2), Uint8List.fromList([65, 66])), 'custom name len > len');
  _expectErr(
      c, EventBatchErrorKind.invalidUtf8, () => EventBatch.validateRaw(oneRecord(flags: nveEventFlagCustomNamed, p0: 1, len: 1), Uint8List.fromList([0xFF])), 'bad custom name utf8');

  // Non-canonical index-0 handles (`0x01000000`) canonicalize to `0`, and `CUSTOM_NAMED | UTF8_BYTES`
  // keeps `text == ''`.
  final canonRec = oneRecord(flags: nveEventFlagCustomNamed | nveEventFlagUtf8Bytes, p0: 4, len: 8);
  ByteData.sublistView(canonRec)
    ..setUint32(16, 0x01000000, Endian.little)
    ..setUint32(20, 0x01000000, Endian.little);
  final canonEv = EventBatch.validateAndDecode(canonRec, Uint8List.fromList('pingdata'.codeUnits)).single;
  c.eq((canonEv.surfaceId, canonEv.nodeId), (0, 0), 'non-canonical 0x01000000 handles canonicalize to 0');
  c.eq((canonEv.text, canonEv.name), ('', 'ping'), 'CUSTOM_NAMED | UTF8_BYTES never leaks into text');
  final encCanon = EventBatch.encode([const RuntimeEvent.press(0x01000000, 0, 0, surfaceId: 0x01000000)]).decodeEvents().single;
  c.eq((encCanon.surfaceId, encCanon.nodeId), (0, 0), 'encode canonicalizes index-0 handles');
}
