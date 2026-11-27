import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';

import 'check.dart';

/// Untyped Dart ⇄ `NveBinValue` bridge: exact bytes, round trips, and every rejection path.
void runBinDynamic(Check c) {
  // Exact wire bytes against the hand-assembled canonical encoding.
  c.eq(_hex(encodeDynamic(null)), '00', 'null');
  c.eq(_hex(encodeDynamic(true)), '0101', 'bool');
  c.eq(_hex(encodeDynamic(-2)), '02feffffffffffffff', 'i64 -2');
  c.eq(_hex(encodeDynamic(1.5)), '03000000000000f83f', 'f64 1.5');
  c.eq(_hex(encodeDynamic('é')), '0402000000c3a9', 'str');
  c.eq(_hex(encodeDynamic(Uint8List.fromList([7, 8]))), '05020000000708', 'bytes');
  c.eq(_hex(encodeDynamic(const <Object?>[])), '060400000000000000', 'empty array: payload counts only the count');
  c.eq(_hex(encodeDynamic(const {'a': null})), '070a000000010000000100000061' '00', 'map key untagged');

  // Round trips (entry order kept, nested containers, integral doubles stay doubles).
  final value = <String, Object?>{
    'z': 1,
    'a': [true, null, 2.0, 'x', <String, Object?>{'n': -1}],
    'b': Uint8List.fromList([1, 2, 3]),
    'max': 9007199254740991,
  };
  final decoded = decodeDynamic(encodeDynamic(value))! as Map<String, Object?>;
  c.eq(decoded.keys.join(','), 'z,a,b,max', 'map entry order preserved');
  c.eq(decoded['z'], 1, 'int');
  final list = decoded['a']! as List<Object?>;
  c.eq(list.length, 5, 'array length');
  c.that(list[2] is double && list[2] == 2.0, 'integral double stays f64');
  c.eq((list[4]! as Map<String, Object?>)['n'], -1, 'nested map');
  c.eq((decoded['b']! as Uint8List).join(','), '1,2,3', 'bytes');
  c.eq(decoded['max'], 9007199254740991, '2^53-1 exact');

  // DynamicValue embeds its bytes verbatim.
  final dv = DynamicValue(encodeDynamic([1, 'two']));
  c.eq(_hex(encodeDynamic([dv])), _hex(encodeDynamic([
    [1, 'two']
  ])), 'DynamicValue raw splice');

  // Rejections write nothing usable and name the cause.
  c.throwsA<ArgumentError>(() => encodeDynamic({1: 'x'}), 'non-string map key');
  c.throwsA<ArgumentError>(() => encodeDynamic(Object()), 'unsupported type');
  c.eq(c.throwsA<BinCodecException>(() => encodeDynamic('\uD800'), 'lone surrogate')?.kind, BinErrorKind.invalidUtf8,
      'lone surrogate kind');
  Object? deep = 0;
  for (var i = 0; i < binValueMaxDepth; i++) {
    deep = [deep];
  }
  c.that(decodeDynamic(encodeDynamic(deep)) is List, '64 nested containers accepted');
  c.eq(c.throwsA<BinCodecException>(() => encodeDynamic([deep]), '65 nested rejected')?.kind, BinErrorKind.maxDepth,
      'max depth kind');
  c.eq(c.throwsA<BinCodecException>(() => decodeDynamic(Uint8List.fromList([0, 0])), 'trailing')?.kind,
      BinErrorKind.trailingBytes, 'decode validates trailing bytes');
  c.eq(c.throwsA<BinCodecException>(() => decodeDynamic(Uint8List.fromList([4, 1, 0, 0, 0, 0xFF])), 'bad utf8')?.kind,
      BinErrorKind.invalidUtf8, 'decode validates utf8');
}

String _hex(Uint8List b) => b.map((x) => x.toRadixString(16).padLeft(2, '0')).join();
