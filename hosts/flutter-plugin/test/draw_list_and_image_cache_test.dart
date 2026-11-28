import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

import 'src/test_support.dart';

Uint8List _wordsToBytes(List<int> words) {
  final out = Uint8List(words.length * 4);
  final view = ByteData.sublistView(out);
  for (var i = 0; i < words.length; i++) {
    view.setUint32(i * 4, words[i], Endian.little);
  }
  return out;
}

int _f32Bits(double v) => (ByteData(4)..setFloat32(0, v, Endian.little)).getUint32(0, Endian.little);

void main() {
  group('NveDrawListPainter (flat u32-word nve.res.draw_list decoder + Canvas renderer)', () {
    test('paints all 8 opcodes accurately onto Flutter Canvas from flat word payloads', () async {
      final enc = DrawListEncoder()
        ..clear(packRgba(10, 20, 30, 255))
        ..fillRect(4, 4, 24, 24, packRgba(220, 38, 38, 255))
        ..fillRoundedRect(34, 4, 24, 24, 6, packRgba(16, 185, 129, 255))
        ..fillCircle(76, 16, 10, packRgba(37, 99, 235, 255))
        ..strokeLine(4, 42, 92, 42, 4, packRgba(245, 158, 11, 255))
        ..strokePolyline(
          const <(double, double)>[(6, 62), (48, 52), (90, 62)],
          4,
          packRgba(236, 72, 153, 255),
        )
        ..clipRect(10, 70, 30, 20)
        ..fillRect(0, 70, 96, 20, packRgba(255, 255, 255, 255))
        ..restoreClip()
        ..fillRect(70, 74, 16, 12, packRgba(56, 189, 248, 255));

      final bytes = enc.finish();
      expect(bytes.length, 56 * 4, reason: 'exact 4-byte-aligned flat word count');
      final painter = NveDrawListPainter();

      final validation = NveDrawListPainter.validate(bytes);
      expect(validation.isOk, isTrue);
      expect(validation.opCount, 10);

      final rgba = await rasterize(96, 96, (Canvas canvas) {
        final res = painter.paint(canvas, const Rect.fromLTWH(0, 0, 96, 96), bytes);
        expect(res.isOk, isTrue);
        expect(res.opCount, 10);
      });

      // Clear background corner pixel (2, 2)
      expect(pixelAt(rgba, 96, 2, 2), (10, 20, 30, 255));
      // FillRect interior (16, 16) -> red
      expect(pixelAt(rgba, 96, 16, 16), (220, 38, 38, 255));
      // FillRoundedRect center (46, 16) -> green
      expect(pixelAt(rgba, 96, 46, 16), (16, 185, 129, 255));
      // FillCircle center (76, 16) -> blue
      expect(pixelAt(rgba, 96, 76, 16), (37, 99, 235, 255));
      // StrokeLine midpoint (48, 42) -> amber
      expect(pixelAt(rgba, 96, 48, 42), (245, 158, 11, 255));
      // StrokePolyline vertex (48, 52) -> pink
      final (pr, pg, pb, pa) = pixelAt(rgba, 96, 48, 52);
      expect(pr, greaterThan(200));
      expect(pb, greaterThan(120));
      expect(pa, 255);
      // Inside ClipRect (25, 80) -> white
      expect(pixelAt(rgba, 96, 25, 80), (255, 255, 255, 255));
      // Outside ClipRect (55, 80) -> remains Clear color!
      expect(pixelAt(rgba, 96, 55, 80), (10, 20, 30, 255));
      // After RestoreClip (78, 80) -> sky blue!
      expect(pixelAt(rgba, 96, 78, 80), (56, 189, 248, 255));

      // Retired `nve-draw1;b64,` ASCII base64 strings must be rejected as flat draw lists.
      final retiredAttrBytes = Uint8List.fromList(utf8.encode('nve-draw1;b64,${base64.encode(bytes)}'));
      expect(NveDrawListPainter.validate(retiredAttrBytes).isOk, isFalse);
      final rgbaRetired = await rasterize(96, 96, (Canvas canvas) {
        expect(painter.paint(canvas, const Rect.fromLTWH(0, 0, 96, 96), retiredAttrBytes).isOk, isFalse);
      });
      expect(pixelAt(rgbaRetired, 96, 16, 16).$4, 0);

      // Verify non-zero byte offset slice view decodes identically.
      final padded = Uint8List(bytes.length + 6)..setRange(3, 3 + bytes.length, bytes);
      final sliced = Uint8List.sublistView(padded, 3, 3 + bytes.length);
      expect(NveDrawListPainter.validate(sliced).opCount, 10);
      final rgbaSliced = await rasterize(96, 96, (Canvas canvas) {
        expect(painter.paint(canvas, const Rect.fromLTWH(0, 0, 96, 96), sliced).isOk, isTrue);
      });
      expect(rgbaSliced, equals(rgba));
    });

    test('matches packages/modules/canvas/src/flat.rs wire word layout byte-for-byte', () {
      final red = packRgba(255, 0, 0, 255);
      final blue = packRgba(0, 0, 255, 255);
      final green = packRgba(0, 255, 0, 255);
      final encoded = (DrawListEncoder()
            ..clear(red)
            ..fillRect(1.5, 2.5, 10.0, 20.0, blue)
            ..strokePolyline(const [(3.0, 4.0)], 2.0, green)
            ..restoreClip())
          .finish();

      final expected = _wordsToBytes(<int>[
        4, // opCount
        NveDrawOpcode.clear, red,
        NveDrawOpcode.fillRect, _f32Bits(1.5), _f32Bits(2.5), _f32Bits(10.0), _f32Bits(20.0), blue,
        NveDrawOpcode.strokePolyline, _f32Bits(2.0), green, 1, _f32Bits(3.0), _f32Bits(4.0),
        NveDrawOpcode.restoreClip,
      ]);
      expect(encoded, equals(expected));

      // Empty draw list (word 0 == 0) and 0-point polyline are valid flat payloads.
      final empty = DrawListEncoder().finish();
      expect(empty, equals(Uint8List(4)));
      final emptyRes = NveDrawListPainter.validate(empty);
      expect(emptyRes.isOk, isTrue);
      expect(emptyRes.opCount, 0);

      final emptyPoly = (DrawListEncoder()..strokePolyline(const [], 2.0, red)).finish();
      expect(emptyPoly.length, (1 + 4) * 4);
      expect(NveDrawListPainter.validate(emptyPoly).isOk, isTrue);
    });

    test('rejects truncated, unaligned, legacy, non-finite, oversized, and trailing flat buffers', () async {
      // Truncated header (< 4 bytes)
      for (var n = 0; n < 4; n++) {
        expect(
          NveDrawListPainter.validate(Uint8List(n)).errorCode,
          NveDrawListErrorCode.truncated,
          reason: '$n-byte buffer shorter than 4-byte header',
        );
      }

      // Legacy 15-byte-header NVE_DRAW1 buffer is rejected ('NVE_' LE u32 > nveDrawMaxOps)
      final legacy = Uint8List.fromList(<int>[
        ...ascii.encode('NVE_DRAW1'),
        1, 0, // version 1
        0, 0, 0, 0, // opCount 0
      ]);
      expect(
        NveDrawListPainter.validate(legacy).errorCode,
        NveDrawListErrorCode.opCountExceedsLimit,
      );

      // Op count > MAX_DRAW_OPS
      final hugeOps = _wordsToBytes(<int>[nveDrawMaxOps + 1]);
      expect(
        NveDrawListPainter.validate(hugeOps).errorCode,
        NveDrawListErrorCode.opCountExceedsLimit,
      );

      // Lying op count exceeding remaining bytes & unaligned opcode prefix
      expect(
        NveDrawListPainter.validate(_wordsToBytes(<int>[100])).errorCode,
        NveDrawListErrorCode.truncated,
      );
      expect(
        NveDrawListPainter.validate(Uint8List.fromList(<int>[1, 0, 0, 0, 8, 0])).errorCode,
        NveDrawListErrorCode.truncated,
      );

      // Unknown u32 opcode (including 0, 99, and high bits set on valid low byte)
      for (final badOp in <int>[0, 9, 99, 0x00010001]) {
        final unknownOp = _wordsToBytes(<int>[1, badOp]);
        expect(
          NveDrawListPainter.validate(unknownOp).errorCode,
          NveDrawListErrorCode.unknownOpcode,
          reason: 'opcode 0x${badOp.toRadixString(16)}',
        );
      }

      // Truncated fixed-size op payload (both word-short and unaligned byte-short)
      expect(
        NveDrawListPainter.validate(_wordsToBytes(<int>[1, NveDrawOpcode.clear])).errorCode,
        NveDrawListErrorCode.truncated,
      );
      expect(
        NveDrawListPainter.validate(Uint8List.fromList(<int>[1, 0, 0, 0, 1, 0, 0, 0, 0xFF, 0x00])).errorCode,
        NveDrawListErrorCode.truncated,
      );

      // Non-finite NaN & Infinity across geometry ops and polyline width/vertices
      final nanRect = (DrawListEncoder()..fillRect(double.nan, 0, 10, 10, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(nanRect).errorCode, NveDrawListErrorCode.nonFiniteFloat);
      final infRadius = (DrawListEncoder()..fillRoundedRect(0, 0, 10, 10, double.infinity, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(infRadius).errorCode, NveDrawListErrorCode.nonFiniteFloat);
      final infCircle = (DrawListEncoder()..fillCircle(0, double.negativeInfinity, 5, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(infCircle).errorCode, NveDrawListErrorCode.nonFiniteFloat);
      final nanLine = (DrawListEncoder()..strokeLine(0, 0, 10, 10, double.nan, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(nanLine).errorCode, NveDrawListErrorCode.nonFiniteFloat);
      final infPolyWidth = (DrawListEncoder()..strokePolyline(const [(0, 0)], double.infinity, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(infPolyWidth).errorCode, NveDrawListErrorCode.nonFiniteFloat);
      final nanPolyPt = (DrawListEncoder()..strokePolyline(const [(0, double.nan)], 2.0, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(nanPolyPt).errorCode, NveDrawListErrorCode.nonFiniteFloat);
      final nanClip = (DrawListEncoder()..clipRect(0, 0, double.nan, 10)).finish();
      expect(NveDrawListPainter.validate(nanClip).errorCode, NveDrawListErrorCode.nonFiniteFloat);

      // Negative radii and stroke widths rejected with negativeFloat; -0.0 accepted
      final negRRect = (DrawListEncoder()..fillRoundedRect(0, 0, 10, 10, -0.5, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(negRRect).errorCode, NveDrawListErrorCode.negativeFloat);
      final negCircle = (DrawListEncoder()..fillCircle(0, 0, -1.0, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(negCircle).errorCode, NveDrawListErrorCode.negativeFloat);
      final negLine = (DrawListEncoder()..strokeLine(0, 0, 10, 10, -0.25, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(negLine).errorCode, NveDrawListErrorCode.negativeFloat);
      final negPoly = (DrawListEncoder()..strokePolyline(const [(0, 0)], -2.0, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(negPoly).errorCode, NveDrawListErrorCode.negativeFloat);
      final negZero = (DrawListEncoder()..fillCircle(0, 0, -0.0, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(negZero).isOk, isTrue);

      // Polyline vertex count > MAX_POLYLINE_POINTS
      final badPoly = _wordsToBytes(<int>[
        1,
        NveDrawOpcode.strokePolyline,
        _f32Bits(2.0),
        packRgba(255, 255, 255),
        nveDrawMaxPolylinePoints + 1,
      ]);
      expect(
        NveDrawListPainter.validate(badPoly).errorCode,
        NveDrawListErrorCode.polylinePointCountExceedsLimit,
      );

      // Truncated polyline header and vertices (aligned & unaligned)
      final truncPolyHdr = _wordsToBytes(<int>[1, NveDrawOpcode.strokePolyline, _f32Bits(2.0)]);
      expect(NveDrawListPainter.validate(truncPolyHdr).errorCode, NveDrawListErrorCode.truncated);

      final truncPoly = _wordsToBytes(<int>[
        1,
        NveDrawOpcode.strokePolyline,
        _f32Bits(2.0),
        packRgba(255, 255, 255),
        4,
      ]);
      expect(NveDrawListPainter.validate(truncPoly).errorCode, NveDrawListErrorCode.truncated);

      // Trailing bytes (unaligned 2 bytes and full extra 4-byte word, after ops and empty list)
      final valid = (DrawListEncoder()..clear(packRgba(0, 0, 0))).finish();
      expect(
        NveDrawListPainter.validate(Uint8List.fromList(<int>[...valid, 0xAA, 0xBB])).errorCode,
        NveDrawListErrorCode.trailingBytes,
      );
      expect(
        NveDrawListPainter.validate(Uint8List.fromList(<int>[...valid, 0, 0, 0, 0])).errorCode,
        NveDrawListErrorCode.trailingBytes,
      );
      expect(
        NveDrawListPainter.validate(Uint8List.fromList(<int>[0, 0, 0, 0, 0xFF])).errorCode,
        NveDrawListErrorCode.trailingBytes,
      );

      // Unbalanced ClipRect + extra RestoreClip + > max clip depth must preserve outer Canvas saveCount!
      final painter = NveDrawListPainter();
      final recorder = ui.PictureRecorder();
      final canvas = Canvas(recorder);
      final unbalancedEnc = DrawListEncoder()..restoreClip();
      for (var i = 0; i < nveDrawMaxClipDepth + 5; i++) {
        unbalancedEnc.clipRect(0, 0, 10, 10);
      }
      final beforeCount = canvas.getSaveCount();
      final res = painter.paint(canvas, const Rect.fromLTWH(0, 0, 32, 32), unbalancedEnc.finish());
      expect(res.isOk, isTrue);
      expect(canvas.getSaveCount(), beforeCount);
      recorder.endRecording().dispose();
    });

    test('tracks overflowClipDepth past 64 nested clips so restores unwind overflow before popping clip #64', () async {
      final enc = DrawListEncoder()..clear(packRgba(0, 0, 0, 255));
      for (var i = 0; i < nveDrawMaxClipDepth - 1; i++) {
        enc.clipRect(0, 0, 96, 96);
      }
      // Clip #64 narrows to [0..20); clips #65 and #66 overflow the 64-deep stack.
      enc
        ..clipRect(0, 0, 20, 20)
        ..clipRect(0, 0, 10, 10)
        ..clipRect(0, 0, 5, 5)
        // Two restores unwind the 2 overflow clips only; clip #64 [0..20) must remain active!
        ..restoreClip()
        ..restoreClip()
        ..fillRect(0, 0, 96, 96, packRgba(255, 0, 0, 255))
        // Third restore pops clip #64, reopening [0..96).
        ..restoreClip()
        ..fillRect(30, 30, 20, 20, packRgba(0, 0, 255, 255));

      final painter = NveDrawListPainter();
      final rgba = await rasterize(96, 96, (Canvas canvas) {
        final before = canvas.getSaveCount();
        final res = painter.paint(canvas, const Rect.fromLTWH(0, 0, 96, 96), enc.finish());
        expect(res.isOk, isTrue);
        expect(canvas.getSaveCount(), before);
      });
      expect(pixelAt(rgba, 96, 10, 10), (255, 0, 0, 255), reason: 'inside clip #64');
      expect(pixelAt(rgba, 96, 70, 70), (0, 0, 0, 255), reason: 'clip #64 was NOT popped by overflow restores');
      expect(pixelAt(rgba, 96, 40, 40), (0, 0, 255, 255), reason: '3rd restore popped clip #64');
    });
  });

  group('NveSceneImageCache & NveImagePlacement', () {
    test('decodes PNG data URI, raw RGBA data URI, file:// PNG, and enforces LRU eviction', () async {
      final cache = NveSceneImageCache(maxEntries: 2);
      addTearDown(cache.dispose);

      // 1. data:image/png;base64,...
      final pngRed = makeSolidPngBytes(255, 0, 0, width: 4, height: 2);
      final pngRedUri = 'data:image/png;base64,${base64.encode(pngRed)}';
      final img1 = await cache.load(pngRedUri);
      expect(img1, isNotNull);
      expect(img1!.width, 4);
      expect(img1.height, 2);
      expect(cache.length, 1);

      // 2. data:image/rgba;width=2;height=2;base64,...
      final rawPixels = Uint8List.fromList(<int>[
        0, 255, 0, 255, 0, 255, 0, 255,
        0, 255, 0, 255, 0, 255, 0, 255,
      ]);
      final rgbaUri = 'data:image/rgba;width=2;height=2;base64,${base64.encode(rawPixels)}';
      final img2 = await cache.load(rgbaUri);
      expect(img2, isNotNull);
      expect(img2!.width, 2);
      expect(img2.height, 2);
      expect(cache.length, 2);

      // Touch img1 so img2 becomes LRU.
      expect(identical(cache.getCached(pngRedUri), img1), isTrue);

      // 3. file://... PNG -> evicts img2, keeps img1.
      final tempDir = await Directory.systemTemp.createTemp('nve_flutter_img_test_');
      addTearDown(() => tempDir.delete(recursive: true));
      final pngFile = File('${tempDir.path}/blue.png');
      await pngFile.writeAsBytes(makeSolidPngBytes(0, 0, 255, width: 3, height: 3));
      final fileUri = pngFile.uri.toString();

      final img3 = await cache.load(fileUri);
      expect(img3, isNotNull);
      expect(img3!.width, 3);
      expect(img3.height, 3);
      expect(cache.length, 2);
      expect(cache.containsKey(pngRedUri), isTrue);
      expect(cache.containsKey(fileUri), isTrue);
      expect(cache.containsKey(rgbaUri), isFalse);

      // Corrupt / mismatched RGBA length fails cleanly.
      final badRgba = 'data:image/rgba;width=4;height=4;base64,${base64.encode(Uint8List(4))}';
      expect(await cache.load(badRgba), isNull);
    });

    test('evict and clear dispose stale in-flight decodes and never invoke onDecoded', () async {
      final cache = NveSceneImageCache(maxEntries: 8);
      addTearDown(cache.dispose);

      Future<ui.Image> makeImage(int r, int g, int b) async =>
          (await NveSceneImageCache.decodeEncodedBytes(makeSolidPngBytes(r, g, b, width: 2, height: 2)))!;

      // 1. evict() while in flight: stale image is disposed, not cached, and onDecoded never fires.
      final staleCompleter = Completer<ui.Image?>();
      final unrelatedCompleter = Completer<ui.Image?>();
      var staleDecoded = 0;
      var unrelatedDecoded = 0;
      expect(
        cache.resolveOrRequest('res:a', onDecoded: () => staleDecoded++, decode: () => staleCompleter.future),
        isNull,
      );
      expect(
        cache.resolveOrRequest('res:b', onDecoded: () => unrelatedDecoded++, decode: () => unrelatedCompleter.future),
        isNull,
      );
      expect(cache.isInFlight('res:a'), isTrue);
      cache.evict('res:a');
      expect(cache.isInFlight('res:a'), isFalse);

      // Re-request the evicted key before the stale decode finishes: fresh decode must win.
      final freshCompleter = Completer<ui.Image?>();
      var freshDecoded = 0;
      expect(
        cache.resolveOrRequest('res:a', onDecoded: () => freshDecoded++, decode: () => freshCompleter.future),
        isNull,
      );

      final staleImg = await makeImage(255, 0, 0);
      final freshImg = await makeImage(0, 255, 0);
      final unrelatedImg = await makeImage(0, 0, 255);
      staleCompleter.complete(staleImg);
      await Future<void>.delayed(Duration.zero);
      expect(staleImg.debugDisposed, isTrue, reason: 'stale evicted image must be disposed');
      expect(staleDecoded, 0, reason: 'evicted decode must not invoke onDecoded');
      expect(cache.containsKey('res:a'), isFalse, reason: 'stale image must not populate cache');
      expect(cache.isInFlight('res:a'), isTrue, reason: 'stale completion must not clear newer in-flight');

      freshCompleter.complete(freshImg);
      unrelatedCompleter.complete(unrelatedImg);
      await Future<void>.delayed(Duration.zero);
      expect(freshImg.debugDisposed, isFalse);
      expect(identical(cache.getCached('res:a'), freshImg), isTrue);
      expect(freshDecoded, 1);
      expect(identical(cache.getCached('res:b'), unrelatedImg), isTrue);
      expect(unrelatedDecoded, 1);

      // 2. clear() while in flight: disposes and discards stale completion without invoking onDecoded.
      final clearedCompleter = Completer<ui.Image?>();
      var clearedDecoded = 0;
      expect(
        cache.resolveOrRequest('res:c', onDecoded: () => clearedDecoded++, decode: () => clearedCompleter.future),
        isNull,
      );
      cache.clear();
      final clearedImg = await makeImage(128, 128, 0);
      clearedCompleter.complete(clearedImg);
      await Future<void>.delayed(Duration.zero);
      expect(clearedImg.debugDisposed, isTrue);
      expect(clearedDecoded, 0);
      expect(cache.containsKey('res:c'), isFalse);

      // 3. Stale decode returning null or throwing after evict()/clear() must NOT poison _failedUris.
      for (final useClear in <bool>[false, true]) {
        for (final failWithThrow in <bool>[false, true]) {
          final key = 'res:stale_${useClear ? "clear" : "evict"}_${failWithThrow ? "err" : "null"}';
          final stale = Completer<ui.Image?>();
          expect(cache.resolveOrRequest(key, decode: () => stale.future), isNull);
          expect(cache.isInFlight(key), isTrue);
          if (useClear) {
            cache.clear();
          } else {
            cache.evict(key);
          }
          if (failWithThrow) {
            stale.completeError(StateError('stale decode failed'));
          } else {
            stale.complete(null);
          }
          await Future<void>.delayed(Duration.zero);

          // A fresh request for the same key must still start and succeed (not short-circuited by _failedUris).
          final retry = Completer<ui.Image?>();
          var retryDecoded = 0;
          expect(
            cache.resolveOrRequest(key, onDecoded: () => retryDecoded++, decode: () => retry.future),
            isNull,
          );
          expect(cache.isInFlight(key), isTrue, reason: '$key must not be poisoned in _failedUris');
          final img = await makeImage(64, 128, 255);
          retry.complete(img);
          await Future<void>.delayed(Duration.zero);
          expect(identical(cache.getCached(key), img), isTrue);
          expect(retryDecoded, 1);
        }
      }

      // 4. Overlapping-generation stale null/throw completion while a newer generation is actively
      //    in-flight (`_inFlight[key]?.gen != gen`): never clobbers the newer in-flight entry or
      //    poisons `_failedUris`, and the newer decode succeeds.
      for (final useClear in <bool>[false, true]) {
        for (final failWithThrow in <bool>[false, true]) {
          final key = 'res:overlap_${useClear ? "clear" : "evict"}_${failWithThrow ? "err" : "null"}';
          final stale = Completer<ui.Image?>();
          expect(cache.resolveOrRequest(key, decode: () => stale.future), isNull);
          expect(cache.isInFlight(key), isTrue);
          if (useClear) {
            cache.clear();
          } else {
            cache.evict(key);
          }
          // Start a newer generation decode BEFORE the stale one completes.
          final fresh = Completer<ui.Image?>();
          var freshDecoded = 0;
          expect(
            cache.resolveOrRequest(key, onDecoded: () => freshDecoded++, decode: () => fresh.future),
            isNull,
          );
          expect(cache.isInFlight(key), isTrue);

          // Now finish the stale generation with null or throw while `fresh` is actively in-flight.
          if (failWithThrow) {
            stale.completeError(StateError('stale overlapping decode failed'));
          } else {
            stale.complete(null);
          }
          await Future<void>.delayed(Duration.zero);
          expect(
            cache.isInFlight(key),
            isTrue,
            reason: 'stale completion must not remove newer in-flight entry for $key',
          );
          expect(freshDecoded, 0);

          final freshImg = await makeImage(32, 192, 96);
          fresh.complete(freshImg);
          await Future<void>.delayed(Duration.zero);
          expect(freshImg.debugDisposed, isFalse);
          expect(identical(cache.getCached(key), freshImg), isTrue);
          expect(freshDecoded, 1);
          expect(cache.isInFlight(key), isFalse);

          // Also verify a stale failure finishing AFTER the newer generation succeeded does not
          // poison `_failedUris` (checked by LRU-evicting without calling `evict()` / `clear()`).
          final lruCache = NveSceneImageCache(maxEntries: 1);
          try {
            final staleLate = Completer<ui.Image?>();
            expect(lruCache.resolveOrRequest(key, decode: () => staleLate.future), isNull);
            if (useClear) {
              lruCache.clear();
            } else {
              lruCache.evict(key);
            }
            final winner = Completer<ui.Image?>();
            expect(lruCache.resolveOrRequest(key, decode: () => winner.future), isNull);
            winner.complete(await makeImage(16, 64, 128));
            await Future<void>.delayed(Duration.zero);
            if (failWithThrow) {
              staleLate.completeError(StateError('late stale failure'));
            } else {
              staleLate.complete(null);
            }
            await Future<void>.delayed(Duration.zero);
            // LRU-evict `key` via an unrelated entry (does not touch `_failedUris`).
            final other = Completer<ui.Image?>();
            expect(lruCache.resolveOrRequest('res:other', decode: () => other.future), isNull);
            other.complete(await makeImage(200, 100, 50));
            await Future<void>.delayed(Duration.zero);
            expect(lruCache.containsKey(key), isFalse);
            // Re-requesting `key` must start a fresh decode (not short-circuited by `_failedUris`).
            final afterLru = Completer<ui.Image?>();
            expect(lruCache.resolveOrRequest(key, decode: () => afterLru.future), isNull);
            expect(lruCache.isInFlight(key), isTrue, reason: '$key must not be poisoned in _failedUris');
            afterLru.complete(await makeImage(90, 180, 200));
            await Future<void>.delayed(Duration.zero);
            expect(lruCache.containsKey(key), isTrue);
          } finally {
            lruCache.dispose();
          }
        }
      }
    });

    test('computes exact cover/contain/stretch/center placement geometry', () {
      const dst = Rect.fromLTWH(10, 20, 100, 50);

      // Stretch fills destination completely
      final stretch = NveImagePlacement.compute(dst, 200, 200, NveImageResizeMode.stretch)!;
      expect(stretch.srcRect, const Rect.fromLTWH(0, 0, 200, 200));
      expect(stretch.dstRect, dst);

      // Contain fits 200x200 into 100x50 -> 50x50 centered horizontally at x=35
      final contain = NveImagePlacement.compute(dst, 200, 200, NveImageResizeMode.contain)!;
      expect(contain.srcRect, const Rect.fromLTWH(0, 0, 200, 200));
      expect(contain.dstRect.left, closeTo(35, 0.01));
      expect(contain.dstRect.top, closeTo(20, 0.01));
      expect(contain.dstRect.width, closeTo(50, 0.01));
      expect(contain.dstRect.height, closeTo(50, 0.01));

      // Cover fills 100x50 from 200x200 -> crops vertical center band [0, 50, 200, 100]
      final cover = NveImagePlacement.compute(dst, 200, 200, NveImageResizeMode.cover)!;
      expect(cover.dstRect, dst);
      expect(cover.srcRect.left, closeTo(0, 0.01));
      expect(cover.srcRect.top, closeTo(50, 0.01));
      expect(cover.srcRect.width, closeTo(200, 0.01));
      expect(cover.srcRect.height, closeTo(100, 0.01));

      // Center with smaller image (40x20) centers without scaling
      final centerSmall = NveImagePlacement.compute(dst, 40, 20, NveImageResizeMode.center)!;
      expect(centerSmall.srcRect, const Rect.fromLTWH(0, 0, 40, 20));
      expect(centerSmall.dstRect, const Rect.fromLTWH(40, 35, 40, 20));

      // Center with larger image (200x100) crops centered 100x50 window
      final centerLarge = NveImagePlacement.compute(dst, 200, 100, NveImageResizeMode.center)!;
      expect(centerLarge.srcRect, const Rect.fromLTWH(50, 25, 100, 50));
      expect(centerLarge.dstRect, dst);
    });
  });
}
