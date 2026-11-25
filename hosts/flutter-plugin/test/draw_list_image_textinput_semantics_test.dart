import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_canvas.dart';

class _NativeSlabAlloc {
  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  static final ffi.Pointer<ffi.Void> Function(int, int) _calloc = _libc.lookupFunction<
      ffi.Pointer<ffi.Void> Function(ffi.Size, ffi.Size),
      ffi.Pointer<ffi.Void> Function(int, int)>('calloc');
  static final void Function(ffi.Pointer<ffi.Void>) _free = _libc.lookupFunction<
      ffi.Void Function(ffi.Pointer<ffi.Void>),
      void Function(ffi.Pointer<ffi.Void>)>('free');

  final List<ffi.Pointer<ffi.Void>> _ptrs = <ffi.Pointer<ffi.Void>>[];

  ffi.Pointer<T> allocate<T extends ffi.NativeType>(int byteCount) {
    final ptr = _calloc(1, byteCount <= 0 ? 1 : byteCount);
    if (ptr == ffi.nullptr) throw StateError('calloc($byteCount) failed');
    _ptrs.add(ptr);
    return ptr.cast<T>();
  }

  void dispose() {
    for (final p in _ptrs) {
      _free(p);
    }
    _ptrs.clear();
  }
}

int _packRgba(int r, int g, int b, [int a = 255]) =>
    (r & 0xFF) | ((g & 0xFF) << 8) | ((b & 0xFF) << 16) | ((a & 0xFF) << 24);

/// Minimal binary `NVE_DRAW1` encoder matching `packages/core/components/src/canvas_ir.rs`.
class _DrawListEncoder {
  _DrawListEncoder() {
    _bytes.add(nveDrawMagic);
    _writeU16(nveDrawVersion);
    _writeU32(0); // opCount placeholder
  }

  final BytesBuilder _bytes = BytesBuilder(copy: false);
  int _opCount = 0;

  void _writeU8(int v) => _bytes.addByte(v & 0xFF);

  void _writeU16(int v) {
    final bd = ByteData(2)..setUint16(0, v, Endian.little);
    _bytes.add(bd.buffer.asUint8List());
  }

  void _writeU32(int v) {
    final bd = ByteData(4)..setUint32(0, v, Endian.little);
    _bytes.add(bd.buffer.asUint8List());
  }

  void _writeF32(double v) {
    final bd = ByteData(4)..setFloat32(0, v, Endian.little);
    _bytes.add(bd.buffer.asUint8List());
  }

  void clear(int rgba) {
    _writeU8(NveDrawOpcode.clear);
    _writeU32(rgba);
    _opCount++;
  }

  void fillRect(double x, double y, double w, double h, int rgba) {
    _writeU8(NveDrawOpcode.fillRect);
    _writeF32(x);
    _writeF32(y);
    _writeF32(w);
    _writeF32(h);
    _writeU32(rgba);
    _opCount++;
  }

  void fillRoundedRect(double x, double y, double w, double h, double radius, int rgba) {
    _writeU8(NveDrawOpcode.fillRoundedRect);
    _writeF32(x);
    _writeF32(y);
    _writeF32(w);
    _writeF32(h);
    _writeF32(radius);
    _writeU32(rgba);
    _opCount++;
  }

  void fillCircle(double cx, double cy, double radius, int rgba) {
    _writeU8(NveDrawOpcode.fillCircle);
    _writeF32(cx);
    _writeF32(cy);
    _writeF32(radius);
    _writeU32(rgba);
    _opCount++;
  }

  void strokeLine(double x0, double y0, double x1, double y1, double width, int rgba) {
    _writeU8(NveDrawOpcode.strokeLine);
    _writeF32(x0);
    _writeF32(y0);
    _writeF32(x1);
    _writeF32(y1);
    _writeF32(width);
    _writeU32(rgba);
    _opCount++;
  }

  void strokePolyline(List<(double, double)> points, double width, int rgba) {
    _writeU8(NveDrawOpcode.strokePolyline);
    _writeF32(width);
    _writeU32(rgba);
    _writeU32(points.length);
    for (final (px, py) in points) {
      _writeF32(px);
      _writeF32(py);
    }
    _opCount++;
  }

  void clipRect(double x, double y, double w, double h) {
    _writeU8(NveDrawOpcode.clipRect);
    _writeF32(x);
    _writeF32(y);
    _writeF32(w);
    _writeF32(h);
    _opCount++;
  }

  void restoreClip() {
    _writeU8(NveDrawOpcode.restoreClip);
    _opCount++;
  }

  Uint8List finish() {
    final out = _bytes.toBytes();
    ByteData.sublistView(out).setUint32(11, _opCount, Endian.little);
    return out;
  }
}

/// Builds a valid uncompressed RFC 2083 2x2 RGBA PNG data URI with distinct corner colors:
/// (0,0)=red, (1,0)=green, (0,1)=blue, (1,1)=yellow.
Uint8List _makeSolidPngBytes(int r, int g, int b, {int width = 2, int height = 2}) {
  final raw = BytesBuilder(copy: false);
  for (var y = 0; y < height; y++) {
    raw.addByte(0); // filter type None
    for (var x = 0; x < width; x++) {
      raw.addByte(r & 0xFF);
      raw.addByte(g & 0xFF);
      raw.addByte(b & 0xFF);
      raw.addByte(255);
    }
  }
  final idatPayload = Uint8List.fromList(zlib.encode(raw.toBytes()));

  int crc32(List<int> tag, List<int> data) {
    var crc = 0xFFFFFFFF;
    for (final byte in <int>[...tag, ...data]) {
      crc ^= byte & 0xFF;
      for (var k = 0; k < 8; k++) {
        crc = (crc & 1) != 0 ? (0xEDB88320 ^ (crc >>> 1)) : (crc >>> 1);
      }
    }
    return (~crc) & 0xFFFFFFFF;
  }

  final out = BytesBuilder(copy: false);
  out.add(<int>[137, 80, 78, 71, 13, 10, 26, 10]);

  void writeChunk(String name, Uint8List payload) {
    final tag = ascii.encode(name);
    final lenBd = ByteData(4)..setUint32(0, payload.length, Endian.big);
    out.add(lenBd.buffer.asUint8List());
    out.add(tag);
    out.add(payload);
    final crcBd = ByteData(4)..setUint32(0, crc32(tag, payload), Endian.big);
    out.add(crcBd.buffer.asUint8List());
  }

  final ihdr = ByteData(13)
    ..setUint32(0, width, Endian.big)
    ..setUint32(4, height, Endian.big)
    ..setUint8(8, 8) // 8-bit depth
    ..setUint8(9, 6) // RGBA
    ..setUint8(10, 0)
    ..setUint8(11, 0)
    ..setUint8(12, 0);
  writeChunk('IHDR', ihdr.buffer.asUint8List());
  writeChunk('IDAT', idatPayload);
  writeChunk('IEND', Uint8List(0));
  return out.toBytes();
}

Future<Uint8List> _rasterize(
  int width,
  int height,
  void Function(Canvas canvas) draw,
) async {
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder, Rect.fromLTWH(0, 0, width.toDouble(), height.toDouble()));
  draw(canvas);
  final picture = recorder.endRecording();
  final img = await picture.toImage(width, height);
  final bytes = await img.toByteData(format: ui.ImageByteFormat.rawRgba);
  img.dispose();
  picture.dispose();
  return bytes!.buffer.asUint8List();
}

(int, int, int, int) _pixelAt(Uint8List rgba, int width, int x, int y) {
  final off = (y * width + x) * 4;
  return (rgba[off], rgba[off + 1], rgba[off + 2], rgba[off + 3]);
}

class _SceneHostWidget extends LeafRenderObjectWidget {
  const _SceneHostWidget({
    required this.onCreated,
    this.onSceneEvent,
    this.backgroundColor,
    this.imageCache,
  });

  final void Function(RenderNveCanvas renderBox) onCreated;
  final NveEventCallback? onSceneEvent;
  final Color? backgroundColor;
  final NveSceneImageCache? imageCache;

  @override
  RenderNveCanvas createRenderObject(BuildContext context) {
    final box = RenderNveCanvas(
      surfaceId: 91,
      backgroundColor: backgroundColor,
      onSceneEvent: onSceneEvent,
      imageCache: imageCache,
    );
    onCreated(box);
    return box;
  }

  @override
  void updateRenderObject(BuildContext context, RenderNveCanvas renderObject) {
    renderObject
      ..backgroundColor = backgroundColor
      ..onSceneEvent = onSceneEvent;
  }
}

void main() {
  group('NveDrawListPainter (NVE_DRAW1 binary decoder + Canvas renderer)', () {
    test('paints all 8 opcodes accurately onto Flutter Canvas and handles b64 fallback', () async {
      final enc = _DrawListEncoder()
        ..clear(_packRgba(10, 20, 30, 255))
        ..fillRect(4, 4, 24, 24, _packRgba(220, 38, 38, 255))
        ..fillRoundedRect(34, 4, 24, 24, 6, _packRgba(16, 185, 129, 255))
        ..fillCircle(76, 16, 10, _packRgba(37, 99, 235, 255))
        ..strokeLine(4, 42, 92, 42, 4, _packRgba(245, 158, 11, 255))
        ..strokePolyline(
          const <(double, double)>[(6, 62), (48, 52), (90, 62)],
          4,
          _packRgba(236, 72, 153, 255),
        )
        ..clipRect(10, 70, 30, 20)
        ..fillRect(0, 70, 96, 20, _packRgba(255, 255, 255, 255))
        ..restoreClip()
        ..fillRect(70, 74, 16, 12, _packRgba(56, 189, 248, 255));

      final bytes = enc.finish();
      final painter = NveDrawListPainter();

      final validation = NveDrawListPainter.validate(bytes);
      expect(validation.isOk, isTrue);
      expect(validation.version, nveDrawVersion);
      expect(validation.opCount, 10);

      final rgba = await _rasterize(96, 96, (Canvas canvas) {
        final res = painter.paint(canvas, const Rect.fromLTWH(0, 0, 96, 96), bytes);
        expect(res.isOk, isTrue);
        expect(res.opCount, 10);
      });

      // Clear background corner pixel (2, 2)
      expect(_pixelAt(rgba, 96, 2, 2), (10, 20, 30, 255));
      // FillRect interior (16, 16) -> red
      expect(_pixelAt(rgba, 96, 16, 16), (220, 38, 38, 255));
      // FillRoundedRect center (46, 16) -> green
      expect(_pixelAt(rgba, 96, 46, 16), (16, 185, 129, 255));
      // FillCircle center (76, 16) -> blue
      expect(_pixelAt(rgba, 96, 76, 16), (37, 99, 235, 255));
      // StrokeLine midpoint (48, 42) -> amber
      expect(_pixelAt(rgba, 96, 48, 42), (245, 158, 11, 255));
      // StrokePolyline vertex (48, 52) -> pink
      final (pr, pg, pb, pa) = _pixelAt(rgba, 96, 48, 52);
      expect(pr, greaterThan(200));
      expect(pb, greaterThan(120));
      expect(pa, 255);
      // Inside ClipRect (25, 80) -> white
      expect(_pixelAt(rgba, 96, 25, 80), (255, 255, 255, 255));
      // Outside ClipRect (55, 80) -> remains Clear color!
      expect(_pixelAt(rgba, 96, 55, 80), (10, 20, 30, 255));
      // After RestoreClip (78, 80) -> sky blue!
      expect(_pixelAt(rgba, 96, 78, 80), (56, 189, 248, 255));

      // Verify `nve-draw1;b64,` ASCII base64 transport decodes identically.
      final attrBytes = Uint8List.fromList(utf8.encode('$nveDrawListAttrPrefix${base64.encode(bytes)}'));
      final rgbaB64 = await _rasterize(96, 96, (Canvas canvas) {
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
      final badVer = _DrawListEncoder().finish();
      ByteData.sublistView(badVer).setUint16(9, 99, Endian.little);
      expect(
        NveDrawListPainter.validate(badVer).errorCode,
        NveDrawListErrorCode.unsupportedVersion,
      );

      // Op count > MAX_DRAW_OPS
      final hugeOps = _DrawListEncoder().finish();
      ByteData.sublistView(hugeOps).setUint32(11, nveDrawMaxOps + 1, Endian.little);
      expect(
        NveDrawListPainter.validate(hugeOps).errorCode,
        NveDrawListErrorCode.opCountExceedsLimit,
      );

      // Lying op count exceeding remaining bytes
      final lyingOps = _DrawListEncoder().finish();
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
      final nanRect = (_DrawListEncoder()..fillRect(double.nan, 0, 10, 10, _packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(nanRect).errorCode, NveDrawListErrorCode.nonFiniteFloat);
      final infCircle = (_DrawListEncoder()..fillCircle(0, double.infinity, 5, _packRgba(255, 0, 0))).finish();
      expect(NveDrawListPainter.validate(infCircle).errorCode, NveDrawListErrorCode.nonFiniteFloat);

      // Polyline vertex count > MAX_POLYLINE_POINTS
      final badPoly = Uint8List(nveDrawHeaderSize + 13)..setAll(0, nveDrawMagic);
      ByteData.sublistView(badPoly)
        ..setUint16(9, nveDrawVersion, Endian.little)
        ..setUint32(11, 1, Endian.little)
        ..setUint8(15, NveDrawOpcode.strokePolyline)
        ..setFloat32(16, 2.0, Endian.little)
        ..setUint32(20, _packRgba(255, 255, 255), Endian.little)
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
        ..setUint32(20, _packRgba(255, 255, 255), Endian.little)
        ..setUint32(24, 4, Endian.little);
      expect(NveDrawListPainter.validate(truncPoly).errorCode, NveDrawListErrorCode.truncated);

      // Trailing bytes
      final valid = (_DrawListEncoder()..clear(_packRgba(0, 0, 0))).finish();
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
      final unbalanced = (_DrawListEncoder()
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
      final pngRed = _makeSolidPngBytes(255, 0, 0, width: 4, height: 2);
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
      await pngFile.writeAsBytes(_makeSolidPngBytes(0, 0, 255, width: 3, height: 3));
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

  group('Scene canvas SkiaCanvas, Image, TextInput, and Semantics integration', () {
    testWidgets(
      'paints SkiaCanvas draw lists, async PNG Images with repaint, TextInput value/placeholder/secure, and exposes Semantics',
      (WidgetTester tester) async {
        final alloc = _NativeSlabAlloc();
        addTearDown(alloc.dispose);
        final semHandle = tester.ensureSemantics();
        try {

        final drawBytes = (_DrawListEncoder()
              ..fillRoundedRect(0, 0, 80, 40, 6, _packRgba(37, 99, 235, 255))
              ..fillCircle(40, 20, 10, _packRgba(250, 204, 21, 255)))
            .finish();
        final pngBytes = _makeSolidPngBytes(16, 185, 129, width: 8, height: 8);
        final imgUri = 'data:image/png;base64,${base64.encode(pngBytes)}';

        final headingBytes = utf8.encode('Dashboard');
        final buttonBytes = utf8.encode('Deploy');
        final inputValBytes = utf8.encode('secret123');
        final imgUriBytes = utf8.encode(imgUri);

        // Concatenate text_slab: [heading, button, inputVal, imgUri, rawDrawBytes]
        final slab = BytesBuilder(copy: false);
        final headingOff = 0;
        slab.add(headingBytes);
        final buttonOff = headingBytes.length;
        slab.add(buttonBytes);
        final inputOff = buttonOff + buttonBytes.length;
        slab.add(inputValBytes);
        final imgOff = inputOff + inputValBytes.length;
        slab.add(imgUriBytes);
        final drawOff = imgOff + imgUriBytes.length;
        slab.add(drawBytes);
        final slabBytes = slab.toBytes();

        final textPtr = alloc.allocate<ffi.Uint8>(slabBytes.length);
        textPtr.asTypedList(slabBytes.length).setAll(0, slabBytes);

        // root(1) -> [2(heading), 3(button), 4(textInput with value), 5(textInput with placeholder), 6(image), 7(skiaCanvas)]
        final childrenPtr = alloc.allocate<ffi.Uint32>(6 * 4);
        childrenPtr.asTypedList(6).setAll(0, <int>[2, 3, 4, 5, 6, 7]);

        final nodesPtr = alloc.allocate<NveRenderNode>(7 * ffi.sizeOf<NveRenderNode>());
        nodesPtr[0]
          ..nodeId = 1
          ..kind = nveNodeKindRoot
          ..flags = 0
          ..opacityU8 = 255
          ..x = 0
          ..y = 0
          ..width = 320
          ..height = 240
          ..bgRgba = _packRgba(15, 23, 42)
          ..childCount = 6
          ..childOffset = 0;

        nodesPtr[1]
          ..nodeId = 2
          ..kind = nveNodeKindText
          ..flags = nveNodeLeafText
          ..opacityU8 = 255
          ..x = 10
          ..y = 10
          ..width = 200
          ..height = 30
          ..textRgba = _packRgba(248, 250, 252)
          ..fontSize = 24
          ..textOffset = headingOff
          ..textLen = headingBytes.length;

        nodesPtr[2]
          ..nodeId = 3
          ..kind = nveNodeKindPressable
          ..flags = nveNodeInteractive | nveNodeLeafText
          ..opacityU8 = 255
          ..x = 10
          ..y = 48
          ..width = 120
          ..height = 36
          ..bgRgba = _packRgba(37, 99, 235)
          ..textRgba = _packRgba(255, 255, 255)
          ..borderRadius = 6
          ..fontSize = 14
          ..textOffset = buttonOff
          ..textLen = buttonBytes.length;

        // Secure TextInput with value ("secret123")
        nodesPtr[3]
          ..nodeId = 4
          ..kind = nveNodeKindTextInput
          ..flags = nveNodeInteractive
          ..opacityU8 = 255
          ..x = 10
          ..y = 92
          ..width = 180
          ..height = 32
          ..bgRgba = _packRgba(30, 41, 59)
          ..textRgba = _packRgba(255, 255, 255)
          ..borderRgba = _packRgba(100, 116, 139)
          ..borderWidth = 1
          ..borderRadius = 4
          ..fontSize = 14
          ..textOffset = inputOff
          ..textLen = inputValBytes.length;

        // Empty TextInput showing muted placeholder
        nodesPtr[4]
          ..nodeId = 5
          ..kind = nveNodeKindTextInput
          ..flags = nveNodeInteractive
          ..opacityU8 = 255
          ..x = 10
          ..y = 132
          ..width = 180
          ..height = 32
          ..bgRgba = _packRgba(30, 41, 59)
          ..textRgba = _packRgba(255, 255, 255)
          ..fontSize = 14
          ..textOffset = 0
          ..textLen = 0;

        // Image node
        nodesPtr[5]
          ..nodeId = 6
          ..kind = nveNodeKindImage
          ..flags = 0
          ..opacityU8 = 255
          ..x = 210
          ..y = 10
          ..width = 80
          ..height = 60
          ..borderRadius = 8
          ..textOffset = imgOff
          ..textLen = imgUriBytes.length;

        // SkiaCanvas node carrying raw binary NVE_DRAW1 bytes
        nodesPtr[6]
          ..nodeId = 7
          ..kind = nveNodeKindSkiaCanvas
          ..flags = 0
          ..opacityU8 = 255
          ..x = 210
          ..y = 90
          ..width = 80
          ..height = 40
          ..textOffset = drawOff
          ..textLen = drawBytes.length;

        final dirtyPtr = alloc.allocate<ffi.Uint32>(7 * 4);
        dirtyPtr.asTypedList(7).setAll(0, <int>[0, 1, 2, 3, 4, 5, 6]);

        final viewPtr = alloc.allocate<NveSceneView>(ffi.sizeOf<NveSceneView>());
        viewPtr.ref
          ..schemaHash = nveSceneSchemaHash
          ..frameId = 1
          ..nodeCount = 7
          ..dirtyCount = 7
          ..removedCount = 0
          ..textSlabLen = slabBytes.length
          ..childrenSlabLen = 6
          ..nodes = nodesPtr
          ..textSlab = textPtr
          ..childrenSlab = childrenPtr
          ..dirtyIndices = dirtyPtr
          ..removedIds = ffi.nullptr;

        late RenderNveCanvas renderBox;
        final events = <NveEvent>[];
        final boundaryKey = GlobalKey();
        final imageCache = NveSceneImageCache(maxEntries: 16);
        addTearDown(imageCache.dispose);

        await tester.pumpWidget(
          Directionality(
            textDirection: TextDirection.ltr,
            child: Align(
              alignment: Alignment.topLeft,
              child: RepaintBoundary(
                key: boundaryKey,
                child: SizedBox(
                  width: 320,
                  height: 240,
                  child: _SceneHostWidget(
                    backgroundColor: const Color(0xFF020617),
                    imageCache: imageCache,
                    onSceneEvent: events.add,
                    onCreated: (RenderNveCanvas box) => renderBox = box,
                  ),
                ),
              ),
            ),
          ),
        );

        renderBox.sceneStore
          ..setAccessibility(2, role: NveAccessibilityRole.header)
          ..setTextInputConfig(4, placeholder: 'Password', secureTextEntry: true)
          ..setAccessibility(4, label: 'Password')
          ..setTextInputConfig(5, placeholder: 'Search files...')
          ..setImageResizeMode(6, NveImageResizeMode.cover)
          ..setAccessibility(6, label: 'Status Avatar');

        renderBox.updateFromSceneView(NveSceneReader.fromPointer(viewPtr));
        await tester.pump();

        // Wait for async PNG image decode triggered during paint, then pump repaint!
        await tester.runAsync(() async {
          await imageCache.load(imgUri);
        });
        await tester.pump();

        // Rasterize the real RepaintBoundary and verify SkiaCanvas + Image pixels!
        final rgba = await tester.runAsync(() async {
          final boundary = boundaryKey.currentContext!.findRenderObject()! as RenderRepaintBoundary;
          final img = await boundary.toImage(pixelRatio: 1.0);
          final data = await img.toByteData(format: ui.ImageByteFormat.rawRgba);
          img.dispose();
          return data!.buffer.asUint8List();
        });
        expect(rgba, isNotNull);
        // Image node (210..290, 10..70) painted solid emerald (16, 185, 129)
        expect(_pixelAt(rgba!, 320, 250, 40), (16, 185, 129, 255));
        // SkiaCanvas node (210..290, 90..130): center circle at (250, 110) is yellow (250, 204, 21)
        expect(_pixelAt(rgba, 320, 250, 110), (250, 204, 21, 255));
        // SkiaCanvas corner interior (220, 100) is blue (37, 99, 235)
        expect(_pixelAt(rgba, 320, 220, 100), (37, 99, 235, 255));

        // Verify live Flutter Semantics tree assembled by RenderNveCanvas.assembleSemanticsNode!
        final rootSem = renderBox.debugSemantics!;
        final childSemData = <SemanticsData>[];
        rootSem.visitChildren((SemanticsNode child) {
          childSemData.add(child.getSemanticsData());
          return true;
        });

        // Heading
        final headingSem = childSemData.firstWhere((d) => d.label == 'Dashboard');
        expect(headingSem.flagsCollection.isHeader, isTrue);

        // Interactive button with SemanticsAction.tap
        final buttonSem = childSemData.firstWhere((d) => d.label == 'Deploy');
        expect(buttonSem.flagsCollection.isButton, isTrue);
        expect(buttonSem.hasAction(SemanticsAction.tap), isTrue);

        // Secure TextInput exposes masked value + isTextField + isObscured
        final passSem = childSemData.firstWhere((d) => d.label == 'Password');
        expect(passSem.flagsCollection.isTextField, isTrue);
        expect(passSem.flagsCollection.isObscured, isTrue);
        expect(passSem.value, '\u2022' * 9);

        // Placeholder TextInput exposes placeholder as label
        final searchSem = childSemData.firstWhere((d) => d.label == 'Search files...');
        expect(searchSem.flagsCollection.isTextField, isTrue);
        expect(searchSem.flagsCollection.isObscured, isFalse);

        // Image with accessibility label exposes isImage
        final imageSem = childSemData.firstWhere((d) => d.label == 'Status Avatar');
        expect(imageSem.flagsCollection.isImage, isTrue);

        // Performing semantic tap action on Deploy button triggers press event on node 3!
        final deployNode = <SemanticsNode>[];
        rootSem.visitChildren((SemanticsNode child) {
          if (child.getSemanticsData().label == 'Deploy') {
            deployNode.add(child);
          }
          return true;
        });
        expect(deployNode, hasLength(1));
        renderBox.owner!.semanticsOwner!.performAction(
          deployNode.single.id,
          SemanticsAction.tap,
        );
        await tester.pump();
        expect(events, hasLength(1));
        expect(events.single.kind, NveEventKind.press);
        expect(events.single.nodeId, 3);

        // Verify TextInput pixels are actually painted inside both secure-value and placeholder boxes!
        int countNonBgPixels(Rect box, (int, int, int) bg) {
          var count = 0;
          for (var y = box.top.toInt() + 2; y < box.bottom.toInt() - 2; y++) {
            for (var x = box.left.toInt() + 2; x < box.right.toInt() - 2; x++) {
              final (r, g, b, _) = _pixelAt(rgba, 320, x, y);
              if (r != bg.$1 || g != bg.$2 || b != bg.$3) count++;
            }
          }
          return count;
        }

        expect(countNonBgPixels(const Rect.fromLTWH(10, 92, 180, 32), (30, 41, 59)), greaterThan(20));
        expect(countNonBgPixels(const Rect.fromLTWH(10, 132, 180, 32), (30, 41, 59)), greaterThan(20));
        } finally {
          semHandle.dispose();
        }
      },
    );
  });
}
