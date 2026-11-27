// VM-only FFI tests for `HybridArena`, `NveValueTree`, `HybridPropsWriter`, `HybridOwnedBytes`,
// and multi-Option argument scoping.
import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'check.dart';

/// Synthetic POD optional struct matching codegen's `NveHybridOptU64` layout.
final class _OptU64 extends ffi.Struct {
  @ffi.Bool()
  external bool isSome;

  @ffi.Uint64()
  external int value;
}

/// Synthetic POD optional struct matching codegen's `NveHybridOptBool` layout.
final class _OptBool extends ffi.Struct {
  @ffi.Bool()
  external bool isSome;

  @ffi.Bool()
  external bool value;
}

/// Synthetic `HybridPropsEncodable` mixing direct scalars, `DynamicValue` (`HybridType::Value`),
/// and binary-encoded `Struct` / `Vec` payloads.
final class _SampleViewProps implements HybridPropsEncodable {
  const _SampleViewProps({
    required this.title,
    required this.muted,
    required this.chapters,
    required this.metadata,
  });

  final String title;
  final bool? muted;
  final List<int> chapters;
  final DynamicValue metadata;

  @override
  int get directPropCount => 4;

  @override
  void encodeDirect(HybridPropsBuilder builder) {
    builder
      ..writeString('title', title)
      ..writeBool('muted', muted);
    final bw = BinWriter();
    final mark = bw.beginArray(chapters.length);
    for (final ch in chapters) {
      bw.writeI64(ch);
    }
    bw.end(mark);
    builder
      ..writeBytes('chapters', bw.bytes)
      ..writeDynamicValue('metadata', metadata);
  }
}

final class _EmptyProps implements HybridPropsEncodable {
  const _EmptyProps();

  @override
  int get directPropCount => 0;

  @override
  void encodeDirect(HybridPropsBuilder builder) {}
}

final class _OverflowProps implements HybridPropsEncodable {
  const _OverflowProps();

  @override
  int get directPropCount => 1;

  @override
  void encodeDirect(HybridPropsBuilder builder) {
    builder
      ..writeNull('a')
      ..writeNull('b');
  }
}

String _readNveStr(NveStrSlice s) {
  if (s.len == 0) {
    return '';
  }
  return decodeUtf8PreserveBom(s.ptr.asTypedList(s.len));
}

bool _sameBytes(List<int> a, List<int> b) {
  if (a.length != b.length) {
    return false;
  }
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) {
      return false;
    }
  }
  return true;
}

void runHybridFfi(Check c) {
  _testOwnedBytesLayout(c);
  _testArenaLifecycleAndUtf8(c);
  _testValueTreeAllTagsAndDepth(c);
  _testPropsNoDoubleWrapping(c);
  _testOwnedBytesHelpersAndCursorDecode(c);
  _testMultiOptionMethodScoping(c);
}

void _testOwnedBytesLayout(Check c) {
  final ptrSize = ffi.sizeOf<ffi.Pointer<ffi.Void>>();
  c.eq(ffi.sizeOf<HybridOwnedBytes>(), ptrSize * 3, 'HybridOwnedBytes size matches 3*sizeof(size_t)');
}

void _testArenaLifecycleAndUtf8(Check c) {
  late HybridArena escaped;
  final result = HybridArena.using<int>((arena) {
    escaped = arena;
    final p1 = arena.allocBytes(7, align: 8);
    final p2 = arena.allocBytes(16, align: 8);
    c.eq(p1.address % 8, 0, 'first allocation 8-byte aligned');
    c.eq(p2.address % 8, 0, 'second allocation 8-byte aligned');
    // Force multi-chunk growth with an allocation larger than the initial slab.
    final big = arena.allocBytes(8192, align: 8);
    c.eq(big.address % 8, 0, 'large chunk allocation 8-byte aligned');
    c.that(arena.allocatedChunkCount >= 2, 'arena chained a second chunk');

    // Empty slices produce nullptr + len 0.
    final emptyStr = arena.strSlice('');
    c.eq(emptyStr.ptr, ffi.nullptr, 'empty strSlice ptr is nullptr');
    c.eq(emptyStr.len, 0, 'empty strSlice len is 0');
    final emptyBuf = arena.bufferSlice(const <int>[]);
    c.eq(emptyBuf.ptr, ffi.nullptr, 'empty bufferSlice ptr is nullptr');
    c.eq(emptyBuf.len, 0, 'empty bufferSlice len is 0');

    // BOM + emoji UTF-8 encoding preserves leading U+FEFF cleanly.
    const rich = '\uFEFFhéllo 🚀 \u{10FFFF}';
    final s = arena.strSlice(rich);
    c.eq(_readNveStr(s), rich, 'strSlice preserves BOM and emoji');

    // Lone surrogates are rejected with BinCodecException(invalidUtf8).
    final err = c.throwsA<BinCodecException>(() => arena.strSlice('\uD800'), 'strSlice lone surrogate');
    c.eq(err?.kind, BinErrorKind.invalidUtf8, 'lone surrogate error kind');
    return 42;
  }, initialChunkBytes: 256);

  c.eq(result, 42, 'HybridArena.using returns value');
  c.that(escaped.isDisposed, 'arena disposed after using()');
  c.eq(escaped.allocatedChunkCount, 0, 'all chunks freed after using()');
  c.throwsA<StateError>(() => escaped.allocBytes(8), 'use-after-dispose rejected');

  // Exception inside HybridArena.using still disposes all slabs.
  late HybridArena failedArena;
  c.throwsA<FormatException>(
    () => HybridArena.using<void>((arena) {
      failedArena = arena;
      arena.allocBytes(512);
      throw const FormatException('boom');
    }),
    'exception propagates from HybridArena.using',
  );
  c.that(failedArena.isDisposed, 'arena disposed on exception');
  c.eq(failedArena.allocatedChunkCount, 0, 'chunks freed on exception');
}

void _testValueTreeAllTagsAndDepth(Check c) {
  final minI64 = const Key64(0, 0x80000000).toSignedInt();
  final maxI64 = const Key64(0xFFFFFFFF, 0x7FFFFFFF).toSignedInt();

  // Exercise every NveValueTag (null, bool, i64, f64, str, bytes, array, map) + empty collections.
  final sampleObject = <String, Object?>{
    'null': null,
    'boolTrue': true,
    'boolFalse': false,
    'intZero': 0,
    'intNeg': -123456,
    'intMin': minI64,
    'intMax': maxI64,
    'floatVal': 3.25,
    'floatNegZero': -0.0,
    'strEmpty': '',
    'strEmoji': '\uFEFFhéllo 🚀 \u{10FFFF}',
    'bytesEmpty': Uint8List(0),
    'bytesRaw': Uint8List.fromList(<int>[0x00, 0x7F, 0x80, 0xFF]),
    'listEmpty': <Object?>[],
    'mapEmpty': <String, Object?>{},
    'nestedList': <Object?>[
      null,
      true,
      99,
      -1.5,
      'elem',
      Uint8List.fromList(<int>[1, 2]),
      <Object?>['deep'],
      <String, Object?>{'k': 'v'},
    ],
    'nestedMap': <String, Object?>{
      '🚀': <Object?>[10, 20],
      'sub': <String, Object?>{'flag': false},
    },
  };
  final dv = DynamicValue.fromObject(sampleObject);

  HybridArena.using<void>((arena) {
    final fromDyn = NveValueTree.fromDynamic(arena, dv);
    final fromObj = NveValueTree.fromObject(arena, sampleObject);
    final fromRdr = NveValueTree.fromReader(arena, BinReader(dv.bytes));

    for (final (label, tree) in <(String, NveValueTree)>[
      ('fromDynamic', fromDyn),
      ('fromObject', fromObj),
      ('fromReader', fromRdr),
    ]) {
      final root = tree.root;
      c.eq(root.tag, NveValueTag.map.code, '$label root tag is MAP (never BYTES)');
      final map = root.data.mapVal;
      c.eq(map.len, sampleObject.length, '$label map length');

      // Index entries by key to inspect raw C-ABI NveValue tags directly.
      final byKey = <String, NveValue>{};
      for (var i = 0; i < map.len; i++) {
        byKey[_readNveStr(map.ptr[i].key)] = map.ptr[i].value;
      }
      c.eq(byKey['null']!.tag, NveValueTag.nullValue.code, '$label null tag');
      c.eq(byKey['boolTrue']!.tag, NveValueTag.boolValue.code, '$label boolTrue tag');
      c.eq(byKey['boolTrue']!.data.boolVal, 1, '$label boolTrue val');
      c.eq(byKey['boolFalse']!.data.boolVal, 0, '$label boolFalse val');
      c.eq(byKey['intMin']!.tag, NveValueTag.i64.code, '$label intMin tag');
      c.eq(byKey['intMin']!.data.i64Val, minI64, '$label intMin val');
      c.eq(byKey['intMax']!.data.i64Val, maxI64, '$label intMax val');
      c.eq(byKey['floatVal']!.tag, NveValueTag.f64.code, '$label floatVal tag');
      c.eq(byKey['floatVal']!.data.f64Val, 3.25, '$label floatVal val');
      c.that(byKey['floatNegZero']!.data.f64Val.isNegative, '$label -0.0 sign preserved');
      c.eq(byKey['strEmpty']!.tag, NveValueTag.str.code, '$label strEmpty tag');
      c.eq(byKey['strEmpty']!.data.strVal.ptr, ffi.nullptr, '$label strEmpty nullptr');
      c.eq(byKey['strEmpty']!.data.strVal.len, 0, '$label strEmpty len');
      c.eq(byKey['strEmoji']!.tag, NveValueTag.str.code, '$label strEmoji tag');
      c.eq(_readNveStr(byKey['strEmoji']!.data.strVal), '\uFEFFhéllo 🚀 \u{10FFFF}', '$label strEmoji text');
      c.eq(byKey['bytesEmpty']!.tag, NveValueTag.bytes.code, '$label bytesEmpty tag');
      c.eq(byKey['bytesEmpty']!.data.bytesVal.ptr, ffi.nullptr, '$label bytesEmpty nullptr');
      c.eq(byKey['bytesRaw']!.tag, NveValueTag.bytes.code, '$label bytesRaw tag');
      c.that(
        _sameBytes(
          byKey['bytesRaw']!.data.bytesVal.ptr.asTypedList(byKey['bytesRaw']!.data.bytesVal.len),
          <int>[0x00, 0x7F, 0x80, 0xFF],
        ),
        '$label bytesRaw contents',
      );
      c.eq(byKey['listEmpty']!.tag, NveValueTag.array.code, '$label listEmpty tag');
      c.eq(byKey['listEmpty']!.data.arrayVal.ptr, ffi.nullptr, '$label listEmpty nullptr');
      c.eq(byKey['listEmpty']!.data.arrayVal.len, 0, '$label listEmpty len');
      c.eq(byKey['mapEmpty']!.tag, NveValueTag.map.code, '$label mapEmpty tag');
      c.eq(byKey['mapEmpty']!.data.mapVal.ptr, ffi.nullptr, '$label mapEmpty nullptr');
      c.eq(byKey['mapEmpty']!.data.mapVal.len, 0, '$label mapEmpty len');
      c.eq(byKey['nestedList']!.tag, NveValueTag.array.code, '$label nestedList tag');
      c.eq(byKey['nestedList']!.data.arrayVal.len, 8, '$label nestedList len');
      c.eq(byKey['nestedMap']!.tag, NveValueTag.map.code, '$label nestedMap tag');

      // Round-trip back to DynamicValue must be byte-identical to `dv`.
      final rt = NveValueTree.toDynamicValue(root);
      c.eq(rt, dv, '$label toDynamicValue byte-identical round-trip');
    }

    // Depth limit: 64 passes, 65 fails on fromObject, fromReader, and embedded DynamicValue.
    Object? nestedList(int depth) {
      Object? cur;
      for (var i = 0; i < depth; i++) {
        cur = <Object?>[cur];
      }
      return cur;
    }

    final ok64 = NveValueTree.fromObject(arena, nestedList(binValueMaxDepth));
    c.eq(ok64.root.tag, NveValueTag.array.code, 'NveValueTree depth 64 succeeds');

    final e65Obj = c.throwsA<BinCodecException>(
      () => NveValueTree.fromObject(arena, nestedList(binValueMaxDepth + 1)),
      'NveValueTree.fromObject rejects depth 65',
    );
    c.eq(e65Obj?.kind, BinErrorKind.maxDepth, 'fromObject depth 65 error kind');

    final dv64 = DynamicValue.fromObject(nestedList(binValueMaxDepth));
    final e65Embed = c.throwsA<BinCodecException>(
      () => NveValueTree.fromObject(arena, <Object?>[dv64]),
      'NveValueTree.fromObject rejects nested DynamicValue past depth 64',
    );
    c.eq(e65Embed?.kind, BinErrorKind.maxDepth, 'nested DynamicValue depth 65 error kind');

    // Malformed reader input rejected by fromReader.
    final badTag = c.throwsA<BinCodecException>(
      () => NveValueTree.fromReader(arena, BinReader(Uint8List.fromList(<int>[99]))),
      'fromReader rejects invalid tag',
    );
    c.eq(badTag?.kind, BinErrorKind.invalidTag, 'fromReader invalid tag kind');

    // Unsupported object / non-String map key rejected cleanly.
    c.throwsA<ArgumentError>(() => NveValueTree.fromObject(arena, Object()), 'unsupported object');
    c.throwsA<ArgumentError>(
      () => NveValueTree.fromObject(arena, <Object?, Object?>{42: 'bad'}),
      'non-String map key',
    );
  });
}

void _testPropsNoDoubleWrapping(Check c) {
  // Verify EVERY DynamicValue tag passed through HybridPropsBuilder.writeDynamicValue arrives
  // with its TRUE NveValueTag (NEVER double-wrapped in NVE_VAL_BYTES), while Struct/Vec props
  // stay NVE_VAL_BYTES.
  final tagCases = <(String, DynamicValue, NveValueTag)>[
    ('null', DynamicValue.nullValue, NveValueTag.nullValue),
    ('bool', DynamicValue.fromObject(true), NveValueTag.boolValue),
    ('i64', DynamicValue.fromObject(-77), NveValueTag.i64),
    ('f64', DynamicValue.fromObject(1.25), NveValueTag.f64),
    ('str', DynamicValue.fromObject('emoji 🚀'), NveValueTag.str),
    ('bytes', DynamicValue.fromObject(Uint8List.fromList(<int>[9, 8])), NveValueTag.bytes),
    ('array', DynamicValue.fromObject(<Object?>[1, 'two']), NveValueTag.array),
    ('map', DynamicValue.fromObject(<String, Object?>{'a': <Object?>[true, null]}), NveValueTag.map),
  ];

  for (final (name, dv, expectedTag) in tagCases) {
    final props = _SampleViewProps(
      title: 'clip-$name',
      muted: name == 'null' ? null : true,
      chapters: const <int>[10, 20, 30],
      metadata: dv,
    );
    HybridPropsWriter.withEncodedProps<void>(props, (entries, count) {
      c.eq(count, 4, 'props count for $name');
      c.eq(_readNveStr(entries[0].key), 'title', '$name key 0');
      c.eq(entries[0].value.tag, NveValueTag.str.code, '$name title tag');
      c.eq(_readNveStr(entries[1].key), 'muted', '$name key 1');
      c.eq(
        entries[1].value.tag,
        name == 'null' ? NveValueTag.nullValue.code : NveValueTag.boolValue.code,
        '$name muted tag',
      );
      c.eq(_readNveStr(entries[2].key), 'chapters', '$name key 2');
      c.eq(entries[2].value.tag, NveValueTag.bytes.code, '$name chapters stays NVE_VAL_BYTES');
      c.eq(_readNveStr(entries[3].key), 'metadata', '$name key 3');
      c.eq(entries[3].value.tag, expectedTag.code, '$name metadata arrives with true tag (no double-wrapping)');
      if (expectedTag != NveValueTag.bytes) {
        c.that(
          entries[3].value.tag != NveValueTag.bytes.code,
          '$name metadata must NOT be wrapped as NVE_VAL_BYTES',
        );
      }
      c.eq(NveValueTree.toDynamicValue(entries[3].value), dv, '$name metadata lossless round-trip');
    });
  }

  // Empty props passes (nullptr, 0) cleanly.
  HybridPropsWriter.withEncodedProps<void>(const _EmptyProps(), (entries, count) {
    c.eq(entries, ffi.nullptr, 'empty props nullptr');
    c.eq(count, 0, 'empty props count 0');
  });

  // Overflowing directPropCount throws StateError.
  c.throwsA<StateError>(
    () => HybridPropsWriter.withEncodedProps<void>(const _OverflowProps(), (_, __) {}),
    'HybridPropsBuilder overflow rejected',
  );
}

void _testOwnedBytesHelpersAndCursorDecode(Check c) {
  var freeCount = 0;
  final freedAddrs = <int>[];
  final freeCallable = ffi.NativeCallable<HybridFreeBytesNative>.isolateLocal((HybridOwnedBytes buf) {
    freeCount++;
    freedAddrs.add(buf.ptr.address);
  });
  final freeFn = freeCallable.nativeFunction.asFunction<HybridFreeBytesDart>();

  try {
    HybridArena.using<void>((arena) {
      HybridOwnedBytes makeOwned(List<int> bytes) {
        final slot = arena.allocStruct<HybridOwnedBytes>(ffi.sizeOf<HybridOwnedBytes>());
        if (bytes.isEmpty) {
          return slot.ref;
        }
        final p = arena.allocBytes(bytes.length, align: 1);
        p.asTypedList(bytes.length).setAll(0, bytes);
        slot.ref
          ..ptr = p
          ..len = bytes.length
          ..cap = bytes.length;
        return slot.ref;
      }

      // 1. hybridTakeString preserves BOM and frees once.
      final bomStr = makeOwned(<int>[0xEF, 0xBB, 0xBF, 0x68, 0x69]);
      c.eq(hybridTakeString(bomStr, freeFn), '\uFEFFhi', 'hybridTakeString BOM');
      c.eq(freeCount, 1, 'freed after hybridTakeString');

      // Invalid UTF-8 throws BinCodecException(invalidUtf8) AND still frees once.
      final badUtf8 = makeOwned(<int>[0xC0, 0xAF]);
      final utf8Err = c.throwsA<BinCodecException>(
        () => hybridTakeString(badUtf8, freeFn),
        'hybridTakeString rejects invalid UTF-8',
      );
      c.eq(utf8Err?.kind, BinErrorKind.invalidUtf8, 'hybridTakeString invalidUtf8 kind');
      c.eq(freeCount, 2, 'freed even when hybridTakeString throws');

      // 2. hybridTakeBytes copies & frees once.
      final rawBytes = makeOwned(<int>[0xAA, 0xBB, 0xCC]);
      final copied = hybridTakeBytes(rawBytes, freeFn);
      c.that(_sameBytes(copied, <int>[0xAA, 0xBB, 0xCC]), 'hybridTakeBytes copy');
      c.eq(freeCount, 3, 'freed after hybridTakeBytes');

      // 3. hybridTakeDynamicValue validates & frees once (even on malformed payload).
      final dv = DynamicValue.fromObject(<String, Object?>{'ok': 1});
      c.eq(hybridTakeDynamicValue(makeOwned(dv.bytes), freeFn), dv, 'hybridTakeDynamicValue valid');
      c.eq(freeCount, 4, 'freed after hybridTakeDynamicValue');

      final badDv = makeOwned(<int>[6, 4, 0, 0, 0, 99, 0, 0, 0]);
      c.throwsA<BinCodecException>(
        () => hybridTakeDynamicValue(badDv, freeFn),
        'hybridTakeDynamicValue rejects malformed',
      );
      c.eq(freeCount, 5, 'freed after hybridTakeDynamicValue failure');

      // 4. hybridDecodeOwned decodes zero-copy from native memory, enforces requireDone, and frees.
      final bw = BinWriter();
      final m = bw.beginMap(2);
      bw
        ..writeMapKey('w')
        ..writeBytes(<int>[1, 2, 3])
        ..writeMapKey('meta')
        ..raw(dv.bytes)
        ..end(m);
      final ownedStruct = makeOwned(bw.bytes);
      final decoded = hybridDecodeOwned<(Uint8List, DynamicValue)>(ownedStruct, freeFn, (r) {
        var w = Uint8List(0);
        var meta = DynamicValue.nullValue;
        r.readMapScope<void>((n) {
          for (var i = 0; i < n; i++) {
            switch (r.readMapKey()) {
              case 'w':
                w = Uint8List.fromList(r.readBytes());
              case 'meta':
                meta = DynamicValue.read(r);
              default:
                r.skipValue();
            }
          }
        });
        return (w, meta);
      });
      // Mutate original native bytes to prove decoded Uint8List & DynamicValue are detached copies.
      ownedStruct.ptr.asTypedList(ownedStruct.len).fillRange(0, ownedStruct.len, 0xEE);
      c.that(_sameBytes(decoded.$1, <int>[1, 2, 3]), 'decoded bytes detached from native memory');
      c.eq(decoded.$2, dv, 'decoded DynamicValue detached from native memory');
      c.eq(freeCount, 6, 'freed after hybridDecodeOwned');

      // Trailing bytes trigger BinErrorKind.trailingBytes AND still free once.
      final trailing = Uint8List(bw.length + 1)
        ..setRange(0, bw.length, bw.bytes)
        ..[bw.length] = 0;
      final trailErr = c.throwsA<BinCodecException>(
        () => hybridDecodeOwned<void>(makeOwned(trailing), freeFn, (r) => r.skipValue()),
        'hybridDecodeOwned rejects trailing bytes',
      );
      c.eq(trailErr?.kind, BinErrorKind.trailingBytes, 'hybridDecodeOwned trailingBytes kind');
      c.eq(freeCount, 7, 'freed after hybridDecodeOwned trailingBytes');
      c.eq(freedAddrs.length, 7, 'freedAddrs count matches freeCount');
      c.eq(freedAddrs.toSet().length, 7, 'every native buffer freed at most once (zero double-frees)');
    });
  } finally {
    freeCallable.close();
  }
}

/// Regression guard: encoding multiple `Option<T>` method arguments (POD + non-POD) in one
/// `HybridArena.using` scope produces independent slices and structs with zero shadowing.
void _testMultiOptionMethodScoping(Check c) {
  (bool, int, bool, bool, String?, List<int>?) encodeAndDecodeMultiOption(
    int? a,
    bool? b,
    String? s,
    List<int>? nums,
  ) {
    return HybridArena.using<(bool, int, bool, bool, String?, List<int>?)>((arena) {
      final opt0 = ffi.Struct.create<_OptU64>();
      if (a != null) {
        opt0
          ..isSome = true
          ..value = a;
      }
      final opt1 = ffi.Struct.create<_OptBool>();
      if (b != null) {
        opt1
          ..isSome = true
          ..value = b;
      }
      final w2 = BinWriter();
      {
        final ov = s;
        if (ov != null) {
          w2.writeStr(ov);
        } else {
          w2.writeNull();
        }
      }
      final arg2 = arena.bufferSlice(w2.bytes);

      final w3 = BinWriter();
      {
        final ov = nums;
        if (ov != null) {
          final arr = w3.beginArray(ov.length);
          for (final el in ov) {
            w3.writeI64(el);
          }
          w3.end(arr);
        } else {
          w3.writeNull();
        }
      }
      final arg3 = arena.bufferSlice(w3.bytes);

      // Decode back straight from arena slices.
      final r2 = BinReader(arg2.ptr.asTypedList(arg2.len));
      final outS = r2.readNull() ? null : r2.readStr();
      r2.requireDone();

      final r3 = BinReader(arg3.ptr.asTypedList(arg3.len));
      final outNums = r3.readNull()
          ? null
          : r3.readArrayScope<List<int>>((n) {
              final list = <int>[];
              for (var i = 0; i < n; i++) {
                list.add(r3.readI64());
              }
              return list;
            });
      r3.requireDone();

      return (opt0.isSome, opt0.value, opt1.isSome, opt1.value, outS, outNums);
    });
  }

  final some = encodeAndDecodeMultiOption(99, true, 'hi', const <int>[4, 5]);
  c.that(some.$1 && some.$2 == 99, 'multi-option POD u64 some');
  c.that(some.$3 && some.$4, 'multi-option POD bool some');
  c.eq(some.$5, 'hi', 'multi-option String some');
  c.eq(some.$6?.length, 2, 'multi-option List<int> some len');
  c.eq(some.$6?[1], 5, 'multi-option List<int> some elem');

  final none = encodeAndDecodeMultiOption(null, null, null, null);
  c.that(!none.$1 && !none.$3, 'multi-option POD none');
  c.eq(none.$5, null, 'multi-option String none');
  c.eq(none.$6, null, 'multi-option List<int> none');
}
