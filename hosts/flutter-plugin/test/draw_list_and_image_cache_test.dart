import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

import 'src/test_support.dart';

void main() {
  group('NveDrawListPainter (NVE_DRAW1 binary decoder + Canvas renderer)', () {
    test('paints all 8 opcodes accurately onto Flutter Canvas and handles b64 fallback', () async {
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
      final painter = NveDrawListPainter();

      final validation = NveDrawListPainter.validate(bytes);
      expect(validation.isOk, isTrue);
      expect(validation.version, nveDrawVersion);
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

      // Verify `nve-draw1;b64,` ASCII base64 transport decodes identically.
      final attrBytes = Uint8List.fromList(utf8.encode('$nveDrawListAttrPrefix${base64.encode(bytes)}'));
      final rgbaB64 = await rasterize(96, 96, (Canvas canvas) {
        final res = painter.paint(canvas, const Rect.fromLTWH(0, 0, 96, 96), attrBytes);
        expect(res.isOk, isTrue);
        expect(res.opCount, 10);
      });
      expect(rgbaB64, equals(rgba));
    });

    test('rejects truncated, corrupt, non-finite, oversized, and trailing NVE_DRAW1 buffers', () async {
      // Truncated header
      expect(
        NveDrawListPainter.validate(Uint8List(8)).errorCode,
        NveDrawListErrorCode.truncated,
      );

      // Bad magic
      final badMagic = Uint8List(nveDrawHeaderSize)..setAll(0, ascii.encode('NVE_DRAWX'));
      expect(NveDrawListPainter.validate(badMagic).errorCode, NveDrawListErrorCode.badMagic);

      // Unsupported version
      final badVer = DrawListEncoder().finish();
      ByteData.sublistView(badVer).setUint16(9, 99, Endian.little);
      expect(
        NveDrawListPainter.validate(badVer).errorCode,
        NveDrawListErrorCode.unsupportedVersion,
      );

      // Op count > MAX_DRAW_OPS
      final hugeOps = DrawListEncoder().finish();
      ByteData.sublistView(hugeOps).setUint32(11, nveDrawMaxOps + 1, Endian.little);
      expect(
        NveDrawListPainter.validate(hugeOps).errorCode,
        NveDrawListErrorCode.opCountExceedsLimit,
      );

      // Lying op count exceeding remaining bytes
      final lyingOps = DrawListEncoder().finish();
      ByteData.sublistView(lyingOps).setUint32(11, 100, Endian.little);
      expect(NveDrawListPainter.validate(lyingOps).errorCode, NveDrawListErrorCode.truncated);

      // Unknown opcode
      final unknownOp = Uint8List(nveDrawHeaderSize + 1)
        ..setAll(0, nveDrawMagic);
      ByteData.sublistView(unknownOp)
        ..setUint16(9, nveDrawVersion, Endian.little)
        ..setUint32(11, 1, Endian.little);
      unknownOp[nveDrawHeaderSize] = 99;
      expect(
        NveDrawListPainter.validate(unknownOp).errorCode,
        NveDrawListErrorCode.unknownOpcode,
      );

      // Non-finite NaN & Infinity in FillRect
      final nanRect = (DrawListEncoder()..fillRect(double.nan, 0, 10, 10, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(nanRect).errorCode, NveDrawListErrorCode.nonFiniteFloat);
      final infCircle = (DrawListEncoder()..fillCircle(0, double.infinity, 5, packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(infCircle).errorCode, NveDrawListErrorCode.nonFiniteFloat);

      // Polyline vertex count > MAX_POLYLINE_POINTS
      final badPoly = Uint8List(nveDrawHeaderSize + 13)..setAll(0, nveDrawMagic);
      ByteData.sublistView(badPoly)
        ..setUint16(9, nveDrawVersion, Endian.little)
        ..setUint32(11, 1, Endian.little)
        ..setUint8(15, NveDrawOpcode.strokePolyline)
        ..setFloat32(16, 2.0, Endian.little)
        ..setUint32(20, packRgba(255, 255, 255), Endian.little)
        ..setUint32(24, nveDrawMaxPolylinePoints + 1, Endian.little);
      expect(
        NveDrawListPainter.validate(badPoly).errorCode,
        NveDrawListErrorCode.polylinePointCountExceedsLimit,
      );

      // Truncated polyline vertices
      final truncPoly = Uint8List(nveDrawHeaderSize + 13)..setAll(0, nveDrawMagic);
      ByteData.sublistView(truncPoly)
        ..setUint16(9, nveDrawVersion, Endian.little)
        ..setUint32(11, 1, Endian.little)
        ..setUint8(15, NveDrawOpcode.strokePolyline)
        ..setFloat32(16, 2.0, Endian.little)
        ..setUint32(20, packRgba(255, 255, 255), Endian.little)
        ..setUint32(24, 4, Endian.little);
      expect(NveDrawListPainter.validate(truncPoly).errorCode, NveDrawListErrorCode.truncated);

      // Trailing bytes
      final valid = (DrawListEncoder()..clear(packRgba(0, 0, 0))).finish();
      final trailing = Uint8List.fromList(<int>[...valid, 0xAA, 0xBB]);
      expect(
        NveDrawListPainter.validate(trailing).errorCode,
        NveDrawListErrorCode.trailingBytes,
      );

      // Invalid base64 after prefix
      final painter = NveDrawListPainter();
      final recorder = ui.PictureRecorder();
      final canvas = Canvas(recorder);
      final badB64 = Uint8List.fromList(ascii.encode('${nveDrawListAttrPrefix}!!!not_base64!!!'));
      expect(
        painter.paint(canvas, const Rect.fromLTWH(0, 0, 32, 32), badB64).errorCode,
        NveDrawListErrorCode.invalidBase64,
      );

      // Unbalanced ClipRect + extra RestoreClip must preserve outer Canvas saveCount!
      final unbalanced = (DrawListEncoder()
            ..restoreClip()
            ..clipRect(0, 0, 10, 10)
            ..clipRect(2, 2, 6, 6))
          .finish();
      final beforeCount = canvas.getSaveCount();
      final res = painter.paint(canvas, const Rect.fromLTWH(0, 0, 32, 32), unbalanced);
      expect(res.isOk, isTrue);
      expect(canvas.getSaveCount(), beforeCount);
      recorder.endRecording().dispose();
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
