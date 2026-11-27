import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:nve_host/nve_native.dart';

import 'check.dart';

/// Reads a native `NveValue` tree back into Dart (`Map` / `List` / scalars) for comparison.
Object? readNative(NveValue v) {
  String str(ffi.Pointer<ffi.Uint8> p, int n) => n == 0 ? '' : String.fromCharCodes(p.asTypedList(n));
  switch (NveValueTag.values.firstWhere((t) => t.code == v.tag)) {
    case NveValueTag.nullValue:
      return null;
    case NveValueTag.boolValue:
      return v.data.boolVal != 0;
    case NveValueTag.i64:
      return v.data.i64Val;
    case NveValueTag.f64:
      return v.data.f64Val;
    case NveValueTag.str:
      return str(v.data.strVal.ptr, v.data.strVal.len);
    case NveValueTag.bytes:
      final b = v.data.bytesVal;
      return b.len == 0 ? Uint8List(0) : Uint8List.fromList(b.ptr.asTypedList(b.len));
    case NveValueTag.array:
      final a = v.data.arrayVal;
      return [for (var i = 0; i < a.len; i++) readNative(a.ptr[i])];
    case NveValueTag.map:
      final m = v.data.mapVal;
      return {for (var i = 0; i < m.len; i++) str(m.ptr[i].key.ptr, m.ptr[i].key.len): readNative(m.ptr[i].value)};
  }
}

final class _Props implements DirectEncodable {
  const _Props(this.extra);
  final int extra;

  @override
  int get directPropCount => 6;

  @override
  void encodeDirect(DirectPropsBuilder b) {
    b
      ..writeBool('on', true)
      ..writeI64('n', -7)
      ..writeF64('f', 0.25)
      ..writeStr('s', 'hé')
      ..writeBytes('b', null)
      ..writeNull('z');
    for (var i = 0; i < extra; i++) {
      b.writeNull('overflow$i');
    }
  }
}

void runNativeValues(Check c) {
  final arena = NativeArena();
  final root = arena.alloc<NveValue>(ffi.sizeOf<NveValue>());
  final value = <String, Object?>{
    'a': [1, 2.5, null, true, 'x'],
    'b': Uint8List.fromList(List<int>.generate(10000, (i) => i & 0xFF)), // forces an oversized chunk
    'empty': <Object?>[],
    'nested': {'k': 'v'},
  };
  arena.writeValue(root.ref, value);
  final back = readNative(root.ref)! as Map<String, Object?>;
  c.eq(back.keys.join(','), 'a,b,empty,nested', 'native map entry order');
  c.eq((back['a']! as List).join(','), '1,2.5,null,true,x', 'native array scalars');
  c.eq((back['b']! as Uint8List).length, 10000, 'large bytes copied');
  c.eq((back['b']! as Uint8List)[9999], 9999 & 0xFF, 'large bytes content');
  c.eq((back['empty']! as List).length, 0, 'empty array');
  c.eq(root.ref.data.mapVal.ptr[2].value.data.arrayVal.ptr.address, 0, 'empty array has null ptr');
  c.eq((back['nested']! as Map)['k'], 'v', 'nested map');

  // A DynamicValue streams through BinReader into the same native shape.
  final viaBin = arena.alloc<NveValue>(ffi.sizeOf<NveValue>());
  arena.writeValue(viaBin.ref, DynamicValue(encodeDynamic(value)));
  final backBin = readNative(viaBin.ref)! as Map<String, Object?>;
  c.eq(backBin.keys.join(','), back.keys.join(','), 'bin → native keys');
  c.eq((backBin['a']! as List).join(','), (back['a']! as List).join(','), 'bin → native array');
  c.eq((backBin['b']! as Uint8List).length, 10000, 'bin → native bytes');

  for (var align in const [1, 2, 4, 8, 16]) {
    arena.alloc<ffi.Uint8>(1, align: 1);
    c.eq(arena.alloc<ffi.Uint8>(3, align: align).address % align, 0, 'alignment $align');
  }
  c.throwsA<BinCodecException>(() => arena.writeValue(root.ref, '\uDC00'), 'lone surrogate rejected');
  c.throwsA<ArgumentError>(() => arena.writeEntries({1: 2}), 'non-string key rejected');
  c.throwsA<ArgumentError>(() => arena.writeValue(root.ref, Object()), 'unsupported type rejected');
  Object? deep = 0;
  for (var i = 0; i < binValueMaxDepth + 1; i++) {
    deep = [deep];
  }
  c.throwsA<BinCodecException>(() => arena.writeValue(root.ref, deep), 'native depth limit');
  arena.dispose();

  final props = DirectProps.withEncoded(const _Props(0), (entries, count) {
    final m = {for (var i = 0; i < count; i++) String.fromCharCodes(entries[i].key.ptr.asTypedList(entries[i].key.len)): readNative(entries[i].value)};
    return m;
  });
  c.eq(props.length, 6, 'direct props count');
  c.eq(props['on'], true, 'bool prop');
  c.eq(props['n'], -7, 'i64 prop');
  c.eq(props['f'], 0.25, 'f64 prop');
  c.eq(props['s'], 'h\u00c3\u00a9', 'str prop is raw UTF-8');
  c.that(props.containsKey('b') && props['b'] == null, 'null bytes prop writes null');
  c.throwsA<StateError>(() => DirectProps.withEncoded(const _Props(1), (_, __) => 0), 'capacity overflow');
  c.eq(DirectProps.withMap(const {}, (e, n) => e.address + n), 0, 'empty map passes nullptr/0');
  c.eq(DirectProps.withMap(const {'x': 1}, (e, n) => readNative(e[0].value)), 1, 'map props');
}
