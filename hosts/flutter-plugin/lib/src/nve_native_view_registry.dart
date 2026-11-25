import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui';

import 'package:flutter/widgets.dart';

abstract final class NveNativeViewConstants {
  static const List<int> nvnpMagic = <int>[0x4E, 0x56, 0x4E, 0x50];
  static const int nvnpVersion = 1, nvnpHeaderSize = 7, maxPropFields = 1024;
  static const List<int> nvncMagic = <int>[0x4E, 0x56, 0x4E, 0x43];
  static const int nvncVersion = 1, nvncHeaderSize = 8, maxCmdArgs = 64;
  static const String kindPropKey = 'nve.native_kind', kindPropKeyAlias = 'nve.native.kind';
  static const String propsPropKey = 'nve.native_props', propsPropKeyAlias = 'nve.native.props';
  static const String kindBlurSurface = 'nve.blur_surface', kindBlurAlias = 'nve.blur';
  static const String kindWebView = 'nve.web_view', kindWebViewAlias = 'nve.webview';
  static const String kindVideoPlayer = 'nve.video_player', kindVideoAlias = 'nve.video';
  static const String kindWebWildcard = 'web.*';
}

enum NveNativeViewCodecErrorKind {
  truncated, invalidMagic, unsupportedVersion, invalidTag, invalidBool, invalidEnum, invalidUtf8,
  missingField, missingArg, unexpectedArgCount, typeMismatch, trailingBytes, unknownCommandOpcode, tooManyFields,
}

class NveNativeViewCodecException implements Exception {
  const NveNativeViewCodecException(this.kind, this.message);
  final NveNativeViewCodecErrorKind kind;
  final String message;
  @override
  String toString() => 'NveNativeViewCodecException($kind): $message';
}

enum NveNativeValueTag {
  nullValue(0), boolValue(1), u32(2), i64(3), u64(4), f32(5), f64(6), str(7), bytes(8);
  const NveNativeValueTag(this.wireCode);
  final int wireCode;
}

class NveNativeValue {
  const NveNativeValue._(this.tag, this._value);
  const NveNativeValue.nullValue() : this._(NveNativeValueTag.nullValue, null);
  const NveNativeValue.boolValue(bool v) : this._(NveNativeValueTag.boolValue, v);
  const NveNativeValue.u32(int v) : this._(NveNativeValueTag.u32, v);
  const NveNativeValue.i64(int v) : this._(NveNativeValueTag.i64, v);
  const NveNativeValue.u64(int v) : this._(NveNativeValueTag.u64, v);
  const NveNativeValue.f32(double v) : this._(NveNativeValueTag.f32, v);
  const NveNativeValue.f64(double v) : this._(NveNativeValueTag.f64, v);
  const NveNativeValue.str(String v) : this._(NveNativeValueTag.str, v);
  const NveNativeValue.bytes(Uint8List v) : this._(NveNativeValueTag.bytes, v);
  final NveNativeValueTag tag;
  final Object? _value;
  bool get isNull => tag == NveNativeValueTag.nullValue;
  bool? asBool() => tag == NveNativeValueTag.boolValue ? _value as bool : null;
  int? asU32() => tag == NveNativeValueTag.u32 ? _value as int : null;
  int? asI64() => tag == NveNativeValueTag.i64 ? _value as int : null;
  int? asU64() => tag == NveNativeValueTag.u64 ? _value as int : null;
  double? asF32() => tag == NveNativeValueTag.f32 ? _value as double : null;
  double? asF64() => tag == NveNativeValueTag.f64 ? _value as double : null;
  String? asString() => tag == NveNativeValueTag.str ? _value as String : null;
  Uint8List? asBytes() => tag == NveNativeValueTag.bytes ? _value as Uint8List : null;
}

bool _isValidUtf8(Uint8List b, int start, int end) {
  int i = start;
  bool c(int idx) => (b[idx] & 0xC0) == 0x80;
  while (i < end) {
    final int b0 = b[i];
    if (b0 <= 0x7F) { i += 1; }
    else if (b0 >= 0xC2 && b0 <= 0xDF) { if (i + 1 >= end || !c(i + 1)) return false; i += 2; }
    else if (b0 == 0xE0) { if (i + 2 >= end || b[i + 1] < 0xA0 || b[i + 1] > 0xBF || !c(i + 2)) return false; i += 3; }
    else if ((b0 >= 0xE1 && b0 <= 0xEC) || (b0 >= 0xEE && b0 <= 0xEF)) { if (i + 2 >= end || !c(i + 1) || !c(i + 2)) return false; i += 3; }
    else if (b0 == 0xED) { if (i + 2 >= end || b[i + 1] < 0x80 || b[i + 1] > 0x9F || !c(i + 2)) return false; i += 3; }
    else if (b0 == 0xF0) { if (i + 3 >= end || b[i + 1] < 0x90 || b[i + 1] > 0xBF || !c(i + 2) || !c(i + 3)) return false; i += 4; }
    else if (b0 >= 0xF1 && b0 <= 0xF3) { if (i + 3 >= end || !c(i + 1) || !c(i + 2) || !c(i + 3)) return false; i += 4; }
    else if (b0 == 0xF4) { if (i + 3 >= end || b[i + 1] < 0x80 || b[i + 1] > 0x8F || !c(i + 2) || !c(i + 3)) return false; i += 4; }
    else { return false; }
  }
  return true;
}

Never _fail(NveNativeViewCodecErrorKind k, String m) => throw NveNativeViewCodecException(k, m);

(NveNativeValue, int) _decodeValueAt(Uint8List bytes, ByteData data, int pos) {
  if (pos >= bytes.length) _fail(NveNativeViewCodecErrorKind.truncated, 'EOF');
  final int tag = bytes[pos++];
  switch (tag) {
    case 0: return (const NveNativeValue.nullValue(), pos);
    case 1:
      if (bytes.length - pos < 1) _fail(NveNativeViewCodecErrorKind.truncated, 'bool');
      final int v = bytes[pos++];
      if (v > 1) _fail(NveNativeViewCodecErrorKind.invalidBool, 'bool');
      return (NveNativeValue.boolValue(v == 1), pos);
    case 2:
      if (bytes.length - pos < 4) _fail(NveNativeViewCodecErrorKind.truncated, 'u32');
      return (NveNativeValue.u32(data.getUint32(pos, Endian.little)), pos + 4);
    case 3:
      if (bytes.length - pos < 8) _fail(NveNativeViewCodecErrorKind.truncated, 'i64');
      return (NveNativeValue.i64(data.getInt64(pos, Endian.little)), pos + 8);
    case 4:
      if (bytes.length - pos < 8) _fail(NveNativeViewCodecErrorKind.truncated, 'u64');
      return (NveNativeValue.u64(data.getUint64(pos, Endian.little)), pos + 8);
    case 5:
      if (bytes.length - pos < 4) _fail(NveNativeViewCodecErrorKind.truncated, 'f32');
      return (NveNativeValue.f32(data.getFloat32(pos, Endian.little)), pos + 4);
    case 6:
      if (bytes.length - pos < 8) _fail(NveNativeViewCodecErrorKind.truncated, 'f64');
      return (NveNativeValue.f64(data.getFloat64(pos, Endian.little)), pos + 8);
    case 7:
    case 8:
      if (bytes.length - pos < 4) _fail(NveNativeViewCodecErrorKind.truncated, 'len');
      final int n = data.getUint32(pos, Endian.little);
      pos += 4;
      if (bytes.length - pos < n) _fail(NveNativeViewCodecErrorKind.truncated, 'payload');
      final Uint8List slice = Uint8List.sublistView(bytes, pos, pos + n);
      if (tag == 7) {
        if (!_isValidUtf8(bytes, pos, pos + n)) _fail(NveNativeViewCodecErrorKind.invalidUtf8, 'utf8');
        return (NveNativeValue.str(utf8.decode(slice, allowMalformed: false)), pos + n);
      }
      return (NveNativeValue.bytes(slice), pos + n);
    default: _fail(NveNativeViewCodecErrorKind.invalidTag, 'tag $tag');
  }
}

class NvnpDecoder {
  NvnpDecoder._(this._fields);
  final List<(int, NveNativeValue)> _fields;
  int get fieldCount => _fields.length;
  bool get isEmpty => _fields.isEmpty;

  factory NvnpDecoder.parse(Uint8List bytes) {
    if (bytes.length < NveNativeViewConstants.nvnpHeaderSize) _fail(NveNativeViewCodecErrorKind.truncated, 'hdr');
    for (int i = 0; i < 4; i++) {
      if (bytes[i] != NveNativeViewConstants.nvnpMagic[i]) _fail(NveNativeViewCodecErrorKind.invalidMagic, 'magic');
    }
    if (bytes[4] != NveNativeViewConstants.nvnpVersion) _fail(NveNativeViewCodecErrorKind.unsupportedVersion, 'ver');
    final ByteData data = ByteData.sublistView(bytes);
    final int count = data.getUint16(5, Endian.little);
    if (count > NveNativeViewConstants.maxPropFields) _fail(NveNativeViewCodecErrorKind.tooManyFields, 'count');
    int pos = NveNativeViewConstants.nvnpHeaderSize;
    if (count > (bytes.length - pos) ~/ 3) _fail(NveNativeViewCodecErrorKind.truncated, 'count');
    final List<(int, NveNativeValue)> out = <(int, NveNativeValue)>[];
    for (int i = 0; i < count; i++) {
      if (bytes.length - pos < 2) _fail(NveNativeViewCodecErrorKind.truncated, 'id');
      final int id = data.getUint16(pos, Endian.little);
      pos += 2;
      final (NveNativeValue v, int nextPos) = _decodeValueAt(bytes, data, pos);
      pos = nextPos;
      out.add((id, v));
    }
    if (pos != bytes.length) _fail(NveNativeViewCodecErrorKind.trailingBytes, 'trailing');
    return NvnpDecoder._(out);
  }

  NveNativeValue? find(int fieldId) { for (final (int id, NveNativeValue v) in _fields) { if (id == fieldId) return v; } return null; }
  NveNativeValue _req(int id) => find(id) ?? _fail(NveNativeViewCodecErrorKind.missingField, 'field $id');
  bool requireBool(int id) => _req(id).asBool() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'bool');
  int requireU32(int id) => _req(id).asU32() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'u32');
  int? requireOptU32(int id) { final NveNativeValue v = _req(id); return v.isNull ? null : (v.asU32() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'u32?')); }
  double requireF32(int id) => _req(id).asF32() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'f32');
  String requireString(int id) => _req(id).asString() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'str');
  Uint8List requireBytes(int id) => _req(id).asBytes() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'bytes');
}

class NvncDecoder {
  NvncDecoder._(this.opcode, this._args);
  final int opcode;
  final List<NveNativeValue> _args;
  int get argCount => _args.length;

  factory NvncDecoder.parse(Uint8List bytes) {
    if (bytes.length < NveNativeViewConstants.nvncHeaderSize) _fail(NveNativeViewCodecErrorKind.truncated, 'hdr');
    for (int i = 0; i < 4; i++) {
      if (bytes[i] != NveNativeViewConstants.nvncMagic[i]) _fail(NveNativeViewCodecErrorKind.invalidMagic, 'magic');
    }
    if (bytes[4] != NveNativeViewConstants.nvncVersion) _fail(NveNativeViewCodecErrorKind.unsupportedVersion, 'ver');
    final ByteData data = ByteData.sublistView(bytes);
    final int op = data.getUint16(5, Endian.little), count = bytes[7];
    if (count > NveNativeViewConstants.maxCmdArgs) _fail(NveNativeViewCodecErrorKind.tooManyFields, 'args');
    int pos = NveNativeViewConstants.nvncHeaderSize;
    if (count > bytes.length - pos) _fail(NveNativeViewCodecErrorKind.truncated, 'args');
    final List<NveNativeValue> out = <NveNativeValue>[];
    for (int i = 0; i < count; i++) {
      final (NveNativeValue v, int nextPos) = _decodeValueAt(bytes, data, pos);
      pos = nextPos;
      out.add(v);
    }
    if (pos != bytes.length) _fail(NveNativeViewCodecErrorKind.trailingBytes, 'trailing');
    return NvncDecoder._(op, out);
  }

  void expectArgCount(int n) { if (_args.length != n) _fail(NveNativeViewCodecErrorKind.unexpectedArgCount, 'args'); }
  NveNativeValue? arg(int i) => (i >= 0 && i < _args.length) ? _args[i] : null;
  NveNativeValue _req(int i) => arg(i) ?? _fail(NveNativeViewCodecErrorKind.missingArg, 'arg $i');
  String requireString(int i) => _req(i).asString() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'str');
  int requireU64(int i) => _req(i).asU64() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'u64');
  double requireF32(int i) => _req(i).asF32() ?? _fail(NveNativeViewCodecErrorKind.typeMismatch, 'f32');
}

String _normalizeScheme(String raw) {
  final StringBuffer sb = StringBuffer();
  for (final int u in raw.codeUnits) { if (u > 0x20 && u != 0x7F) sb.writeCharCode((u >= 0x41 && u <= 0x5A) ? u + 32 : u); }
  return sb.toString();
}
bool isSafeNveWebViewUrl(String url) { final String n = _normalizeScheme(url); return n.isNotEmpty && (n.startsWith('https://') || n.startsWith('http://') || n == 'about:blank'); }
bool isSafeNveMediaUri(String uri) { final String n = _normalizeScheme(uri); return n.isNotEmpty && (n.startsWith('https://') || n.startsWith('http://') || n.startsWith('file://')); }
Color nveColorFromRrggbbaa(int rgba) => Color.fromARGB(rgba & 0xFF, (rgba >> 24) & 0xFF, (rgba >> 16) & 0xFF, (rgba >> 8) & 0xFF);

enum NveBlurMaterial {
  ultraThin(0, 8.0), thin(1, 14.0), regular(2, 20.0), thick(3, 30.0), chrome(4, 24.0), systemMaterial(5, 20.0);
  const NveBlurMaterial(this.wireCode, this.baseSigma);
  final int wireCode;
  final double baseSigma;
  static NveBlurMaterial fromWire(int c) => (c >= 0 && c < values.length) ? values[c] : _fail(NveNativeViewCodecErrorKind.invalidEnum, 'mat $c');
}

class NveBlurSurfaceProps {
  const NveBlurSurfaceProps({this.material = NveBlurMaterial.regular, this.intensity = 1.0, this.tint});
  final NveBlurMaterial material; final double intensity; final int? tint;
  double get clampedIntensity => intensity.isFinite ? intensity.clamp(0.0, 1.0) : 1.0;
  double get sigma => material.baseSigma * clampedIntensity;
  factory NveBlurSurfaceProps.decode(NvnpDecoder d) =>
      NveBlurSurfaceProps(material: NveBlurMaterial.fromWire(d.requireU32(1)), intensity: d.requireF32(2), tint: d.requireOptU32(3));
}

class NveWebViewProps {
  const NveWebViewProps({this.url = '', this.allowsBackForward = true});
  final String url; final bool allowsBackForward;
  factory NveWebViewProps.decode(NvnpDecoder d) => NveWebViewProps(url: d.requireString(1), allowsBackForward: d.requireBool(2));
}

enum NveWebViewCommandKind { navigate, reload, goBack, goForward, evaluateScript }

class NveWebViewCommand {
  const NveWebViewCommand(this.kind, [this.text = '']);
  final NveWebViewCommandKind kind; final String text;
  factory NveWebViewCommand.decode(NvncDecoder d) => switch (d.opcode) {
        1 => (() { d.expectArgCount(1); return NveWebViewCommand(NveWebViewCommandKind.navigate, d.requireString(0)); })(),
        2 => (() { d.expectArgCount(0); return const NveWebViewCommand(NveWebViewCommandKind.reload); })(),
        3 => (() { d.expectArgCount(0); return const NveWebViewCommand(NveWebViewCommandKind.goBack); })(),
        4 => (() { d.expectArgCount(0); return const NveWebViewCommand(NveWebViewCommandKind.goForward); })(),
        5 => (() { d.expectArgCount(1); return NveWebViewCommand(NveWebViewCommandKind.evaluateScript, d.requireString(0)); })(),
        _ => _fail(NveNativeViewCodecErrorKind.unknownCommandOpcode, 'op'),
      };
}

class NveVideoPlayerProps {
  const NveVideoPlayerProps({this.sourceUri = '', this.autoplay = false, this.muted = false, this.looping = false, this.controls = true});
  final String sourceUri; final bool autoplay, muted, looping, controls;
  factory NveVideoPlayerProps.decode(NvnpDecoder d) => NveVideoPlayerProps(
      sourceUri: d.requireString(1), autoplay: d.requireBool(2), muted: d.requireBool(3), looping: d.requireBool(4), controls: d.requireBool(5));
}

enum NveVideoPlayerCommandKind { play, pause, seekMs, setVolume }

class NveVideoPlayerCommand {
  const NveVideoPlayerCommand(this.kind, {this.seekMs = 0, this.volume = 1.0});
  final NveVideoPlayerCommandKind kind; final int seekMs; final double volume;
  factory NveVideoPlayerCommand.decode(NvncDecoder d) => switch (d.opcode) {
        1 => (() { d.expectArgCount(0); return const NveVideoPlayerCommand(NveVideoPlayerCommandKind.play); })(),
        2 => (() { d.expectArgCount(0); return const NveVideoPlayerCommand(NveVideoPlayerCommandKind.pause); })(),
        3 => (() { d.expectArgCount(1); return NveVideoPlayerCommand(NveVideoPlayerCommandKind.seekMs, seekMs: d.requireU64(0)); })(),
        4 => (() { d.expectArgCount(1); return NveVideoPlayerCommand(NveVideoPlayerCommandKind.setVolume, volume: d.requireF32(0)); })(),
        _ => _fail(NveNativeViewCodecErrorKind.unknownCommandOpcode, 'op'),
      };
}

class NveWebElementProps {
  const NveWebElementProps(this.tag, this.attrs);
  final String tag; final List<(String, String)> attrs;
  factory NveWebElementProps.decode(NvnpDecoder d) {
    final String tag = d.requireString(1); final int count = d.requireU32(2); final Uint8List raw = d.requireBytes(3);
    if (count > raw.length ~/ 8) _fail(NveNativeViewCodecErrorKind.truncated, 'attrs');
    final ByteData data = ByteData.sublistView(raw); int pos = 0;
    String readStr() {
      if (raw.length - pos < 4) _fail(NveNativeViewCodecErrorKind.truncated, 'len');
      final int n = data.getUint32(pos, Endian.little); pos += 4;
      if (raw.length - pos < n) _fail(NveNativeViewCodecErrorKind.truncated, 'str');
      if (!_isValidUtf8(raw, pos, pos + n)) _fail(NveNativeViewCodecErrorKind.invalidUtf8, 'utf8');
      final String s = utf8.decode(Uint8List.sublistView(raw, pos, pos + n), allowMalformed: false); pos += n; return s;
    }
    final List<(String, String)> attrs = <(String, String)>[for (int i = 0; i < count; i++) (readStr(), readStr())];
    if (pos != raw.length) _fail(NveNativeViewCodecErrorKind.trailingBytes, 'trailing');
    return NveWebElementProps(tag, attrs);
  }
}

class NveNativeViewAdapter {
  const NveNativeViewAdapter({required this.create, this.updateProps, this.dispatchCommand, this.unmount, this.buildWidget});
  final Object Function(int nodeId, String kind, NvnpDecoder? initialProps) create;
  final void Function(Object instance, int nodeId, NvnpDecoder props)? updateProps;
  final bool Function(Object instance, int nodeId, String commandName, NvncDecoder cmd)? dispatchCommand;
  final void Function(Object instance, int nodeId)? unmount;
  final Widget Function(BuildContext context, Object instance)? buildWidget;
}

class _MountedEntry {
  _MountedEntry(this.kind, this.instance, this.adapter);
  final String kind; final Object instance; final NveNativeViewAdapter adapter;
}

String _trimAsciiWs(String s) {
  int a = 0, b = s.length;
  bool ws(int c) => c == 0x20 || c == 0x09 || c == 0x0D || c == 0x0A;
  while (a < b && ws(s.codeUnitAt(a))) { a++; }
  while (b > a && ws(s.codeUnitAt(b - 1))) { b--; }
  return s.substring(a, b);
}

class NveNativeViewRegistry {
  final Map<String, NveNativeViewAdapter> _exact = <String, NveNativeViewAdapter>{};
  final List<(String, NveNativeViewAdapter)> _wildcards = <(String, NveNativeViewAdapter)>[];
  final Map<int, _MountedEntry> _mounted = <int, _MountedEntry>{};

  void registerAdapter(String pattern, NveNativeViewAdapter adapter) {
    final String t = _trimAsciiWs(pattern); if (t.isEmpty) return;
    if (t.endsWith('*')) {
      final String p = t.substring(0, t.length - 1);
      final int idx = _wildcards.indexWhere(((String, NveNativeViewAdapter) e) => e.$1 == p);
      if (idx >= 0) { _wildcards[idx] = (p, adapter); return; }
      _wildcards.add((p, adapter));
      _wildcards.sort(((String, NveNativeViewAdapter) a, (String, NveNativeViewAdapter) b) {
        final int cmp = b.$1.length.compareTo(a.$1.length); return cmp != 0 ? cmp : a.$1.compareTo(b.$1);
      });
    } else { _exact[t] = adapter; }
  }

  bool unregisterAdapter(String pattern) {
    final String t = _trimAsciiWs(pattern); if (t.isEmpty) return false;
    if (t.endsWith('*')) {
      final String p = t.substring(0, t.length - 1); final int n = _wildcards.length;
      _wildcards.removeWhere(((String, NveNativeViewAdapter) e) => e.$1 == p); return _wildcards.length != n;
    }
    return _exact.remove(t) != null;
  }

  NveNativeViewAdapter? findAdapter(String kind) {
    final String t = _trimAsciiWs(kind); if (t.isEmpty) return null;
    final NveNativeViewAdapter? hit = _exact[t]; if (hit != null) return hit;
    for (final (String p, NveNativeViewAdapter a) in _wildcards) {
      if (p.isEmpty || (t.length > p.length && t.startsWith(p))) return a;
    }
    return null;
  }

  bool supports(String kind) => findAdapter(kind) != null;

  Object? mountOrUpdate(int nodeId, String kind, Uint8List nvnpBytes) {
    final NveNativeViewAdapter? adapter = findAdapter(kind); if (adapter == null) return null;
    final NvnpDecoder? dec = nvnpBytes.isEmpty ? null : NvnpDecoder.parse(nvnpBytes);
    final _MountedEntry? prev = _mounted[nodeId]; if (prev != null && prev.kind != kind) unmount(nodeId);
    final _MountedEntry e = _mounted.putIfAbsent(nodeId, () => _MountedEntry(kind, adapter.create(nodeId, kind, dec), adapter));
    if (dec != null) e.adapter.updateProps?.call(e.instance, nodeId, dec);
    return e.instance;
  }

  bool dispatchCommand(int nodeId, String commandName, Uint8List nvncBytes) {
    final _MountedEntry? e = _mounted[nodeId];
    if (e == null || e.adapter.dispatchCommand == null) return false;
    return e.adapter.dispatchCommand!(e.instance, nodeId, commandName, NvncDecoder.parse(nvncBytes));
  }

  bool unmount(int nodeId) {
    final _MountedEntry? e = _mounted.remove(nodeId); if (e == null) return false;
    e.adapter.unmount?.call(e.instance, nodeId); return true;
  }
  void unmountAll() { for (final MapEntry<int, _MountedEntry> e in _mounted.entries) { e.value.adapter.unmount?.call(e.value.instance, e.key); } _mounted.clear(); }
  Object? instanceFor(int nodeId) => _mounted[nodeId]?.instance;
  String? kindFor(int nodeId) => _mounted[nodeId]?.kind;
  int get mountedCount => _mounted.length;
  Widget? buildWidgetFor(BuildContext ctx, int nodeId) => _mounted[nodeId]?.adapter.buildWidget?.call(ctx, _mounted[nodeId]!.instance);

  void registerDefaultFlutterNativeViews({
    Widget Function(BuildContext context, NveFlutterWebViewState state)? webViewBuilder,
    Widget Function(BuildContext context, NveFlutterVideoPlayerState state)? videoPlayerBuilder,
  }) {
    final NveNativeViewAdapter blur = NveNativeViewAdapter(
      create: (_, __, ___) => NveFlutterBlurSurfaceState(),
      updateProps: (Object i, _, NvnpDecoder d) { if (i is NveFlutterBlurSurfaceState) i.apply(NveBlurSurfaceProps.decode(d)); },
      unmount: (Object i, _) { if (i is NveFlutterBlurSurfaceState) i.markUnmounted(); },
      buildWidget: (_, Object i) => NveFlutterBlurSurfaceWidget(state: i as NveFlutterBlurSurfaceState),
    );
    final NveNativeViewAdapter web = NveNativeViewAdapter(
      create: (_, __, ___) => NveFlutterWebViewState(),
      updateProps: (Object i, _, NvnpDecoder d) { if (i is NveFlutterWebViewState) i.apply(NveWebViewProps.decode(d)); },
      dispatchCommand: (Object i, _, __, NvncDecoder d) => i is NveFlutterWebViewState && i.dispatch(NveWebViewCommand.decode(d)),
      unmount: (Object i, _) { if (i is NveFlutterWebViewState) i.markUnmounted(); },
      buildWidget: (BuildContext c, Object i) => webViewBuilder != null ? webViewBuilder(c, i as NveFlutterWebViewState) : const SizedBox.expand(),
    );
    final NveNativeViewAdapter video = NveNativeViewAdapter(
      create: (_, __, ___) => NveFlutterVideoPlayerState(),
      updateProps: (Object i, _, NvnpDecoder d) { if (i is NveFlutterVideoPlayerState) i.apply(NveVideoPlayerProps.decode(d)); },
      dispatchCommand: (Object i, _, __, NvncDecoder d) => i is NveFlutterVideoPlayerState && i.dispatch(NveVideoPlayerCommand.decode(d)),
      unmount: (Object i, _) { if (i is NveFlutterVideoPlayerState) i.markUnmounted(); },
      buildWidget: (BuildContext c, Object i) => videoPlayerBuilder != null ? videoPlayerBuilder(c, i as NveFlutterVideoPlayerState) : const SizedBox.expand(),
    );
    registerAdapter(NveNativeViewConstants.kindBlurSurface, blur);
    registerAdapter(NveNativeViewConstants.kindBlurAlias, blur);
    registerAdapter(NveNativeViewConstants.kindWebView, web);
    registerAdapter(NveNativeViewConstants.kindWebViewAlias, web);
    registerAdapter(NveNativeViewConstants.kindVideoPlayer, video);
    registerAdapter(NveNativeViewConstants.kindVideoAlias, video);
  }
}

class NveFlutterBlurSurfaceState extends ChangeNotifier {
  NveBlurSurfaceProps props = const NveBlurSurfaceProps();
  bool unmounted = false;
  void apply(NveBlurSurfaceProps next) { props = next; notifyListeners(); }
  void markUnmounted() { unmounted = true; dispose(); }
}

class NveFlutterBlurSurfaceWidget extends StatelessWidget {
  const NveFlutterBlurSurfaceWidget({super.key, required this.state, this.child});
  final NveFlutterBlurSurfaceState state; final Widget? child;
  @override
  Widget build(BuildContext context) => ListenableBuilder(
        listenable: state,
        builder: (_, __) {
          final double s = state.props.sigma; final int? tint = state.props.tint;
          return ClipRect(
            child: BackdropFilter(
              filter: ImageFilter.blur(sigmaX: s, sigmaY: s),
              child: tint != null ? ColoredBox(color: nveColorFromRrggbbaa(tint), child: child ?? const SizedBox.expand()) : (child ?? const SizedBox.expand()),
            ),
          );
        },
      );
}

class NveFlutterWebViewState extends ChangeNotifier {
  String lastRequestedUrl = '', lastEvaluatedScript = '';
  bool allowsBackForward = true, unmounted = false;
  int navigationCount = 0, reloadCount = 0, goBackCount = 0, goForwardCount = 0;
  bool _loadSafeUrl(String u) { if (!isSafeNveWebViewUrl(u)) return false; lastRequestedUrl = u; navigationCount += 1; notifyListeners(); return true; }
  void apply(NveWebViewProps p) { allowsBackForward = p.allowsBackForward; if (lastRequestedUrl != p.url) { _loadSafeUrl(p.url); } else { notifyListeners(); } }
  bool dispatch(NveWebViewCommand cmd) {
    switch (cmd.kind) {
      case NveWebViewCommandKind.navigate: return _loadSafeUrl(cmd.text);
      case NveWebViewCommandKind.reload: reloadCount += 1; notifyListeners(); return true;
      case NveWebViewCommandKind.goBack: if (!allowsBackForward) return false; goBackCount += 1; notifyListeners(); return true;
      case NveWebViewCommandKind.goForward: if (!allowsBackForward) return false; goForwardCount += 1; notifyListeners(); return true;
      case NveWebViewCommandKind.evaluateScript: lastEvaluatedScript = cmd.text; notifyListeners(); return true;
    }
  }
  void markUnmounted() { unmounted = true; dispose(); }
}

class NveFlutterVideoPlayerState extends ChangeNotifier {
  NveVideoPlayerProps props = const NveVideoPlayerProps();
  bool isPlaying = false, unmounted = false;
  int positionMs = 0, playCount = 0, pauseCount = 0, seekCount = 0;
  double volume = 1.0;
  void apply(NveVideoPlayerProps n) {
    final String uri = isSafeNveMediaUri(n.sourceUri) ? n.sourceUri : props.sourceUri;
    props = NveVideoPlayerProps(sourceUri: uri, autoplay: n.autoplay, muted: n.muted, looping: n.looping, controls: n.controls);
    if (props.autoplay && props.sourceUri.isNotEmpty && !isPlaying) { isPlaying = true; playCount += 1; }
    notifyListeners();
  }
  bool dispatch(NveVideoPlayerCommand cmd) {
    switch (cmd.kind) {
      case NveVideoPlayerCommandKind.play: isPlaying = true; playCount += 1; notifyListeners(); return true;
      case NveVideoPlayerCommandKind.pause: isPlaying = false; pauseCount += 1; notifyListeners(); return true;
      case NveVideoPlayerCommandKind.seekMs: positionMs = cmd.seekMs; seekCount += 1; notifyListeners(); return true;
      case NveVideoPlayerCommandKind.setVolume:
        if (!cmd.volume.isFinite) return false;
        volume = cmd.volume.clamp(0.0, 1.0); notifyListeners(); return true;
    }
  }
  void markUnmounted() { unmounted = true; isPlaying = false; dispose(); }
}
