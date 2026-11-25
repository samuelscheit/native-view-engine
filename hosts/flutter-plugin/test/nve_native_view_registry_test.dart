import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

class _NvnpBuilder {
  final BytesBuilder _b = BytesBuilder()..add(<int>[0x4E, 0x56, 0x4E, 0x50, 1, 0, 0]);
  int _count = 0;

  void _u16(int v) => _b.add(<int>[v & 0xFF, (v >> 8) & 0xFF]);
  void _u32(int v) => _b.add(Uint8List(4)..buffer.asByteData().setUint32(0, v, Endian.little));
  void _i64(int v) => _b.add(Uint8List(8)..buffer.asByteData().setInt64(0, v, Endian.little));
  void _u64(int v) => _b.add(Uint8List(8)..buffer.asByteData().setUint64(0, v, Endian.little));
  void _f32(double v) => _b.add(Uint8List(4)..buffer.asByteData().setFloat32(0, v, Endian.little));
  void _f64(double v) => _b.add(Uint8List(8)..buffer.asByteData().setFloat64(0, v, Endian.little));

  void addNull(int id) {
    _u16(id);
    _b.addByte(0);
    _count++;
  }

  void addBool(int id, bool v) {
    _u16(id);
    _b.addByte(1);
    _b.addByte(v ? 1 : 0);
    _count++;
  }

  void addU32(int id, int v) {
    _u16(id);
    _b.addByte(2);
    _u32(v);
    _count++;
  }

  void addI64(int id, int v) {
    _u16(id);
    _b.addByte(3);
    _i64(v);
    _count++;
  }

  void addU64(int id, int v) {
    _u16(id);
    _b.addByte(4);
    _u64(v);
    _count++;
  }

  void addF32(int id, double v) {
    _u16(id);
    _b.addByte(5);
    _f32(v);
    _count++;
  }

  void addF64(int id, double v) {
    _u16(id);
    _b.addByte(6);
    _f64(v);
    _count++;
  }

  void addStr(int id, String s) {
    final List<int> bytes = utf8.encode(s);
    _u16(id);
    _b.addByte(7);
    _u32(bytes.length);
    _b.add(bytes);
    _count++;
  }

  void addBytes(int id, Uint8List bytes) {
    _u16(id);
    _b.addByte(8);
    _u32(bytes.length);
    _b.add(bytes);
    _count++;
  }

  Uint8List finish() {
    final Uint8List out = _b.toBytes();
    out[5] = _count & 0xFF;
    out[6] = (_count >> 8) & 0xFF;
    return out;
  }
}

class _NvncBuilder {
  _NvncBuilder(int opcode) {
    _b.add(<int>[0x4E, 0x56, 0x4E, 0x43, 1, opcode & 0xFF, (opcode >> 8) & 0xFF, 0]);
  }

  final BytesBuilder _b = BytesBuilder();
  int _count = 0;

  void argU64(int v) {
    _b.addByte(4);
    _b.add(Uint8List(8)..buffer.asByteData().setUint64(0, v, Endian.little));
    _count++;
  }

  void argF32(double v) {
    _b.addByte(5);
    _b.add(Uint8List(4)..buffer.asByteData().setFloat32(0, v, Endian.little));
    _count++;
  }

  void argStr(String s) {
    final List<int> bytes = utf8.encode(s);
    _b.addByte(7);
    _b.add(Uint8List(4)..buffer.asByteData().setUint32(0, bytes.length, Endian.little));
    _b.add(bytes);
    _count++;
  }

  Uint8List finish() {
    final Uint8List out = _b.toBytes();
    out[7] = _count;
    return out;
  }
}

void main() {
  test('NVNP decodes all 9 value types and WebElement packed attributes', () {
    final _NvnpBuilder all = _NvnpBuilder()
      ..addNull(1)
      ..addBool(2, true)
      ..addU32(3, 0xDEADBEEF)
      ..addI64(4, -9876543210)
      ..addU64(5, 0x1122334455667788)
      ..addF32(6, 0.75)
      ..addF64(7, 3.1415926535)
      ..addStr(8, 'hello UTF-8 ✓ 🚀')
      ..addBytes(9, Uint8List.fromList(<int>[0xAA, 0xBB, 0xCC]));

    final NvnpDecoder dec = NvnpDecoder.parse(all.finish());
    expect(dec.fieldCount, 9);
    expect(dec.find(1)?.isNull, isTrue);
    expect(dec.find(2)?.asBool(), isTrue);
    expect(dec.find(3)?.asU32(), 0xDEADBEEF);
    expect(dec.find(4)?.asI64(), -9876543210);
    expect(dec.find(5)?.asU64(), 0x1122334455667788);
    expect(dec.find(6)?.asF32(), closeTo(0.75, 1e-6));
    expect(dec.find(7)?.asF64(), closeTo(3.1415926535, 1e-9));
    expect(dec.find(8)?.asString(), 'hello UTF-8 ✓ 🚀');
    expect(dec.find(9)?.asBytes(), <int>[0xAA, 0xBB, 0xCC]);

    final BytesBuilder packed = BytesBuilder();
    void pushStr(String s) {
      final List<int> b = utf8.encode(s);
      packed.add(Uint8List(4)..buffer.asByteData().setUint32(0, b.length, Endian.little));
      packed.add(b);
    }

    pushStr('viewBox');
    pushStr('0 0 24 24');
    pushStr('aria-hidden');
    pushStr('true');

    final _NvnpBuilder elem = _NvnpBuilder()
      ..addStr(1, 'svg')
      ..addU32(2, 2)
      ..addBytes(3, packed.toBytes());
    final NveWebElementProps props = NveWebElementProps.decode(NvnpDecoder.parse(elem.finish()));
    expect(props.tag, 'svg');
    expect(props.attrs, <(String, String)>[('viewBox', '0 0 24 24'), ('aria-hidden', 'true')]);
  });

  test('NVNP and NVNC reject corrupt and adversarial buffers', () {
    void expectCodecErr(void Function() fn, NveNativeViewCodecErrorKind kind) {
      expect(fn, throwsA(isA<NveNativeViewCodecException>().having((NveNativeViewCodecException e) => e.kind, 'kind', kind)));
    }

    expectCodecErr(() => NvnpDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E])), NveNativeViewCodecErrorKind.truncated);
    expectCodecErr(() => NvncDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E])), NveNativeViewCodecErrorKind.truncated);
    expectCodecErr(() => NvnpDecoder.parse(Uint8List.fromList(<int>[0x42, 0x41, 0x44, 0x21, 1, 0, 0])), NveNativeViewCodecErrorKind.invalidMagic);
    expectCodecErr(() => NvncDecoder.parse(Uint8List.fromList(<int>[0x42, 0x41, 0x44, 0x21, 1, 0, 0, 0])), NveNativeViewCodecErrorKind.invalidMagic);
    expectCodecErr(() => NvnpDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x50, 2, 0, 0])), NveNativeViewCodecErrorKind.unsupportedVersion);
    expectCodecErr(() => NvncDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x43, 9, 1, 0, 0])), NveNativeViewCodecErrorKind.unsupportedVersion);
    expectCodecErr(() => NvnpDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x50, 1, 1, 0, 1, 0, 1, 2])), NveNativeViewCodecErrorKind.invalidBool);
    expectCodecErr(() => NvnpDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x50, 1, 1, 0, 1, 0, 99])), NveNativeViewCodecErrorKind.invalidTag);
    expectCodecErr(() => NvnpDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x50, 1, 1, 0, 1, 0, 7, 2, 0, 0, 0, 0xC0, 0xAF])), NveNativeViewCodecErrorKind.invalidUtf8);
    expectCodecErr(() => NvncDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x43, 1, 1, 0, 1, 7, 3, 0, 0, 0, 0xED, 0xA0, 0x80])), NveNativeViewCodecErrorKind.invalidUtf8);
    expectCodecErr(() => NvnpDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x50, 1, 0, 0, 0xFF])), NveNativeViewCodecErrorKind.trailingBytes);
    expectCodecErr(() => NvncDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x43, 1, 2, 0, 0, 0x00])), NveNativeViewCodecErrorKind.trailingBytes);
    expectCodecErr(() => NvnpDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x50, 1, 0xFF, 0x7F])), NveNativeViewCodecErrorKind.tooManyFields);
    expectCodecErr(() => NvncDecoder.parse(Uint8List.fromList(<int>[0x4E, 0x56, 0x4E, 0x43, 1, 1, 0, 200])), NveNativeViewCodecErrorKind.tooManyFields);
  });

  testWidgets('Registry wildcards, URL safety, BlurSurface BackdropFilter, WebView & VideoPlayer', (WidgetTester tester) async {
    expect(isSafeNveWebViewUrl('https://flutter.nve.dev'), isTrue);
    expect(isSafeNveWebViewUrl('http://localhost:8080'), isTrue);
    expect(isSafeNveWebViewUrl('  ABOUT:BLANK\n'), isTrue);
    expect(isSafeNveWebViewUrl('javascript:alert(1)'), isFalse);
    expect(isSafeNveWebViewUrl('JaVa\tScRiPt:alert(1)'), isFalse);
    expect(isSafeNveWebViewUrl('vbscript:msgbox(1)'), isFalse);
    expect(isSafeNveWebViewUrl('data:text/html,<b>x</b>'), isFalse);

    final NveNativeViewRegistry registry = NveNativeViewRegistry();
    registry.registerDefaultFlutterNativeViews(
      webViewBuilder: (_, NveFlutterWebViewState s) => Text(s.lastRequestedUrl, textDirection: TextDirection.ltr),
      videoPlayerBuilder: (_, NveFlutterVideoPlayerState s) => Text(s.props.sourceUri, textDirection: TextDirection.ltr),
    );

    // Wildcard precedence
    registry.registerAdapter('web.*', NveNativeViewAdapter(create: (_, __, ___) => 'web-wild'));
    registry.registerAdapter('web.media.*', NveNativeViewAdapter(create: (_, __, ___) => 'media-wild'));
    registry.registerAdapter('web.media.audio', NveNativeViewAdapter(create: (_, __, ___) => 'exact-audio'));
    expect(registry.supports('web.canvas'), isTrue);
    expect(registry.supports('web.'), isFalse);
    expect(registry.mountOrUpdate(1, 'web.canvas', Uint8List(0)), 'web-wild');
    expect(registry.mountOrUpdate(2, 'web.media.video', Uint8List(0)), 'media-wild');
    expect(registry.mountOrUpdate(3, 'web.media.audio', Uint8List(0)), 'exact-audio');

    // BlurSurface with BackdropFilter + tint
    final _NvnpBuilder blur = _NvnpBuilder()
      ..addU32(1, NveBlurMaterial.thick.wireCode)
      ..addF32(2, 0.5)
      ..addU32(3, 0x10203080);
    final NveFlutterBlurSurfaceState blurState =
        registry.mountOrUpdate(10, NveNativeViewConstants.kindBlurAlias, blur.finish())! as NveFlutterBlurSurfaceState;
    expect(blurState.props.material, NveBlurMaterial.thick);
    expect(blurState.props.sigma, closeTo(15.0, 1e-4));

    await tester.pumpWidget(
      Directionality(
        textDirection: TextDirection.ltr,
        child: Builder(builder: (BuildContext ctx) => registry.buildWidgetFor(ctx, 10)!),
      ),
    );
    expect(find.byType(BackdropFilter), findsOneWidget);
    expect(find.byType(ColoredBox), findsOneWidget);

    // WebView props + NVNC commands + URL sanitization
    final _NvnpBuilder wv = _NvnpBuilder()
      ..addStr(1, 'https://initial.nve.dev')
      ..addBool(2, true);
    final NveFlutterWebViewState wvState =
        registry.mountOrUpdate(11, NveNativeViewConstants.kindWebView, wv.finish())! as NveFlutterWebViewState;
    expect(wvState.lastRequestedUrl, 'https://initial.nve.dev');

    expect(registry.dispatchCommand(11, 'navigate', (_NvncBuilder(1)..argStr('javascript:bad()')).finish()), isFalse);
    expect(registry.dispatchCommand(11, 'navigate', (_NvncBuilder(1)..argStr('https://next.nve.dev')).finish()), isTrue);
    expect(registry.dispatchCommand(11, 'reload', _NvncBuilder(2).finish()), isTrue);
    expect(registry.dispatchCommand(11, 'go_back', _NvncBuilder(3).finish()), isTrue);
    expect(registry.dispatchCommand(11, 'evaluate_script', (_NvncBuilder(5)..argStr('2 + 2')).finish()), isTrue);
    expect(wvState.lastRequestedUrl, 'https://next.nve.dev');
    expect(wvState.reloadCount, 1);
    expect(wvState.goBackCount, 1);
    expect(wvState.lastEvaluatedScript, '2 + 2');

    // VideoPlayer props + NVNC commands
    final _NvnpBuilder vid = _NvnpBuilder()
      ..addStr(1, 'https://cdn.nve.dev/demo.mp4')
      ..addBool(2, false)
      ..addBool(3, true)
      ..addBool(4, true)
      ..addBool(5, true);
    final NveFlutterVideoPlayerState vidState =
        registry.mountOrUpdate(12, NveNativeViewConstants.kindVideoAlias, vid.finish())! as NveFlutterVideoPlayerState;
    expect(registry.dispatchCommand(12, 'play', _NvncBuilder(1).finish()), isTrue);
    expect(registry.dispatchCommand(12, 'seek_ms', (_NvncBuilder(3)..argU64(5500)).finish()), isTrue);
    expect(registry.dispatchCommand(12, 'set_volume', (_NvncBuilder(4)..argF32(0.25)).finish()), isTrue);
    expect(vidState.isPlaying, isTrue);
    expect(vidState.positionMs, 5500);
    expect(vidState.volume, closeTo(0.25, 1e-5));

    // Unmount widget tree before disposing ChangeNotifiers
    await tester.pumpWidget(const SizedBox.shrink());
    registry.unmountAll();
    expect(registry.mountedCount, 0);
    expect(blurState.unmounted, isTrue);
    expect(wvState.unmounted, isTrue);
    expect(vidState.unmounted, isTrue);
  });
}
