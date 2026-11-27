// VM-only FFI edge-case and negative-path tests for `HybridArena`, `NveValueTree.toDynamicValue`,
// `hybridVerifyVtable`, `hybridTake*`, and `HybridPropsBuilder`.
import 'dart:ffi' as ffi;

import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'check.dart';

final class _LoneSurrogateProp implements HybridPropsEncodable {
  const _LoneSurrogateProp({required this.key, required this.value});

  final String key;
  final String value;

  @override
  int get directPropCount => 1;

  @override
  void encodeDirect(HybridPropsBuilder builder) {
    builder.writeString(key, value);
  }
}

void runHybridFfiEdgeCases(Check c) {
  _testArenaBoundsAndZeroAllocEmptySlices(c);
  _testArenaUnwindMidTree(c);
  _testVerifyVtable(c);
  _testOwnedBytesNullAndSignedSizeBounds(c);
  _testToDynamicValueErrorPathsAndSpecialNumbers(c);
  _testPropsLoneSurrogate(c);
}

void _testArenaBoundsAndZeroAllocEmptySlices(Check c) {
  // Invalid initialChunkBytes rejected up front.
  c.throwsA<RangeError>(() => HybridArena(initialChunkBytes: -1), 'negative initialChunkBytes');
  c.throwsA<RangeError>(() => HybridArena(initialChunkBytes: 0x40000000), 'oversized initialChunkBytes');

  // Empty strSlice / bufferSlice allocate zero native slabs.
  HybridArena.using<void>((arena) {
    final s = arena.strSlice('');
    final b = arena.bufferSlice(const <int>[]);
    c.eq(s.ptr, ffi.nullptr, 'empty strSlice ptr');
    c.eq(s.len, 0, 'empty strSlice len');
    c.eq(b.ptr, ffi.nullptr, 'empty bufferSlice ptr');
    c.eq(b.len, 0, 'empty bufferSlice len');
    c.eq(arena.allocatedChunkCount, 0, 'empty slices allocate 0 native slabs');

    // Runtime alignment & negative byteCount checks (must throw RangeError even without asserts).
    for (final badAlign in const <int>[0, -1, 3, 5, 16]) {
      c.throwsA<RangeError>(() => arena.allocBytes(8, align: badAlign), 'allocBytes invalid align $badAlign');
    }
    c.throwsA<RangeError>(() => arena.allocBytes(-1), 'allocBytes negative byteCount');
    c.throwsA<RangeError>(() => arena.allocStructArray<NveValue>(ffi.sizeOf<NveValue>(), -1), 'negative array count');

    for (final goodAlign in const <int>[1, 2, 4, 8]) {
      final p = arena.allocBytes(5, align: goodAlign);
      c.eq(p.address % goodAlign, 0, 'allocBytes align=$goodAlign honored');
    }
  });

  // Async callback returning Future is caught in assert-enabled builds (`--enable-asserts`).
  c.throwsA<AssertionError>(
    () => HybridArena.using<Future<int>>((_) async => 1),
    'HybridArena.using rejects async Future callback under --enable-asserts',
  );
}

void _testArenaUnwindMidTree(Check c) {
  // Mid-tree lone surrogate throw disposes all chunks allocated so far.
  late HybridArena arena1;
  final err1 = c.throwsA<BinCodecException>(
    () => HybridArena.using<void>((arena) {
      arena1 = arena;
      NveValueTree.fromObject(arena, <String, Object?>{
        'ok': 'allocated-before-throw',
        'bad': '\uD800',
      });
    }),
    'mid-tree lone surrogate throws',
  );
  c.eq(err1?.kind, BinErrorKind.invalidUtf8, 'mid-tree lone surrogate error kind');
  c.that(arena1.isDisposed && arena1.allocatedChunkCount == 0, 'arena disposed after mid-tree UTF-8 error');

  // Mid-tree maxDepth throw disposes all chunks allocated so far.
  Object? deep(int n) {
    Object? cur = 'leaf';
    for (var i = 0; i < n; i++) {
      cur = <Object?>[cur];
    }
    return cur;
  }

  late HybridArena arena2;
  final err2 = c.throwsA<BinCodecException>(
    () => HybridArena.using<void>((arena) {
      arena2 = arena;
      NveValueTree.fromObject(arena, deep(binValueMaxDepth + 1));
    }),
    'mid-tree maxDepth throws',
  );
  c.eq(err2?.kind, BinErrorKind.maxDepth, 'mid-tree maxDepth error kind');
  c.that(arena2.isDisposed && arena2.allocatedChunkCount == 0, 'arena disposed after mid-tree maxDepth error');
}

void _testVerifyVtable(Check c) {
  const expected = Key64(0x11223344, 0x55667788);
  const wrong = Key64(0x11223344, 0x55667789);
  final rawExpected = expected.toSignedInt();
  final rawWrong = wrong.toSignedInt();

  hybridVerifyVtable(1, 1, rawExpected, expected, 'SampleMod');

  final abiErr = c.throwsA<HybridException>(
    () => hybridVerifyVtable(1, 2, rawExpected, expected, 'SampleMod'),
    'hybridVerifyVtable ABI mismatch',
  );
  c.eq(abiErr?.status, HybridStatus.schemaMismatch, 'ABI mismatch status');
  c.that(abiErr?.message.contains('SampleMod') ?? false, 'ABI mismatch mentions interface name');

  final schemaErr = c.throwsA<HybridException>(
    () => hybridVerifyVtable(1, 1, rawWrong, expected, 'SampleMod'),
    'hybridVerifyVtable schema mismatch',
  );
  c.eq(schemaErr?.status, HybridStatus.schemaMismatch, 'schema mismatch status');
  c.that(schemaErr?.message.contains('SampleMod') ?? false, 'schema mismatch mentions interface name');
}

void _testOwnedBytesNullAndSignedSizeBounds(Check c) {
  final freedAddrs = <int>[];
  final freeCallable = ffi.NativeCallable<HybridFreeBytesNative>.isolateLocal((HybridOwnedBytes buf) {
    freedAddrs.add(buf.ptr.address);
  });
  final freeFn = freeCallable.nativeFunction.asFunction<HybridFreeBytesDart>();

  try {
    HybridArena.using<void>((arena) {
      // (nullptr, 0, 0): returns empty, never calls free.
      final nullEmpty = ffi.Struct.create<HybridOwnedBytes>();
      c.eq(hybridTakeString(nullEmpty, freeFn), '', 'nullptr len 0 string');
      c.eq(hybridTakeBytes(nullEmpty, freeFn).length, 0, 'nullptr len 0 bytes');
      c.eq(freedAddrs.length, 0, 'nullptr never passed to freeBytes');

      // (nonNull, 0, cap): returns empty AND frees once.
      final nonNullEmpty = arena.allocStruct<HybridOwnedBytes>(ffi.sizeOf<HybridOwnedBytes>());
      final dummyBuf = arena.allocBytes(8);
      nonNullEmpty.ref
        ..ptr = dummyBuf
        ..len = 0
        ..cap = 8;
      c.eq(hybridTakeString(nonNullEmpty.ref, freeFn), '', 'nonNull len 0 string');
      c.eq(freedAddrs.length, 1, 'nonNull len 0 string still freed');
      c.eq(freedAddrs.last, dummyBuf.address, 'exact ptr freed for nonNull len 0 string');

      final dummyBuf2 = arena.allocBytes(8);
      nonNullEmpty.ref.ptr = dummyBuf2;
      c.eq(hybridTakeBytes(nonNullEmpty.ref, freeFn).length, 0, 'nonNull len 0 bytes');
      c.eq(freedAddrs.length, 2, 'nonNull len 0 bytes still freed');
      c.eq(freedAddrs.last, dummyBuf2.address, 'exact ptr freed for nonNull len 0 bytes');

      // (nullptr, len > 0): throws truncated, never calls free.
      final nullNonEmpty = ffi.Struct.create<HybridOwnedBytes>()..len = 4;
      final e1 = c.throwsA<BinCodecException>(
        () => hybridTakeString(nullNonEmpty, freeFn),
        'hybridTakeString rejects (nullptr, len > 0)',
      );
      c.eq(e1?.kind, BinErrorKind.truncated, 'null string truncated kind');
      final e2 = c.throwsA<BinCodecException>(
        () => hybridTakeBytes(nullNonEmpty, freeFn),
        'hybridTakeBytes rejects (nullptr, len > 0)',
      );
      c.eq(e2?.kind, BinErrorKind.truncated, 'null bytes truncated kind');
      c.eq(freedAddrs.length, 2, 'nullptr error never calls free');

      // Negative & >u32::MAX signed ffi.Size rejected with tooLarge AND still freed once.
      for (final badLen in const <int>[-1, 0x100000000]) {
        final buf = arena.allocBytes(8);
        final owned = ffi.Struct.create<HybridOwnedBytes>()
          ..ptr = buf
          ..len = badLen
          ..cap = 8;
        final es = c.throwsA<BinCodecException>(
          () => hybridTakeString(owned, freeFn),
          'hybridTakeString rejects len=$badLen',
        );
        c.eq(es?.kind, BinErrorKind.tooLarge, 'string bad len=$badLen tooLarge');
        final buf2 = arena.allocBytes(8);
        owned.ptr = buf2;
        final eb = c.throwsA<BinCodecException>(
          () => hybridTakeBytes(owned, freeFn),
          'hybridTakeBytes rejects len=$badLen',
        );
        c.eq(eb?.kind, BinErrorKind.tooLarge, 'bytes bad len=$badLen tooLarge');
      }
      c.eq(freedAddrs.length, 6, 'all 4 bad-len non-null buffers freed once');
      c.eq(freedAddrs.toSet().length, 6, 'all 6 freed addresses distinct');
    });
  } finally {
    freeCallable.close();
  }
}

void _testToDynamicValueErrorPathsAndSpecialNumbers(Check c) {
  // Special floats (`NaN`, `+-infinity`) and `u64 > i64::MAX` bit-pattern round-trip through NveValueTree.
  final u64MaxAsI64 = const Key64(0xFFFFFFFF, 0xFFFFFFFF).toSignedInt();
  final u64HighBitAsI64 = const Key64(0x00000001, 0x80000000).toSignedInt();
  HybridArena.using<void>((arena) {
    final special = DynamicValue.fromObject(<String, Object?>{
      'nan': double.nan,
      'inf': double.infinity,
      'ninf': double.negativeInfinity,
      'u64Max': u64MaxAsI64,
      'u64High': u64HighBitAsI64,
    });
    final tree = NveValueTree.fromDynamic(arena, special);
    final rt = NveValueTree.toDynamicValue(tree.root);
    final map = rt.toObject() as Map<String, Object?>;
    c.that((map['nan'] as double).isNaN, 'NveValueTree preserves NaN');
    c.eq(map['inf'], double.infinity, 'NveValueTree preserves +inf');
    c.eq(map['ninf'], double.negativeInfinity, 'NveValueTree preserves -inf');
    c.eq(map['u64Max'], u64MaxAsI64, 'NveValueTree preserves u64::MAX bits');
    c.eq(map['u64High'], u64HighBitAsI64, 'NveValueTree preserves 2^63 + 1 bits');

    // Invalid tag.
    final badTag = ffi.Struct.create<NveValue>()..tag = 99;
    c.eq(
      c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(badTag), 'invalid tag')?.kind,
      BinErrorKind.invalidTag,
      'invalidTag error kind',
    );

    // Invalid boolVal > 1.
    final badBool = ffi.Struct.create<NveValue>()
      ..tag = NveValueTag.boolValue.code
      ..data.boolVal = 2;
    c.eq(
      c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(badBool), 'boolVal=2')?.kind,
      BinErrorKind.invalidBool,
      'invalidBool error kind',
    );

    // Negative & >u32::MAX len on str / bytes / array / map.
    final dummy = arena.allocBytes(8);
    for (final badLen in const <int>[-1, 0x100000000]) {
      final badStr = ffi.Struct.create<NveValue>()
        ..tag = NveValueTag.str.code
        ..data.strVal.ptr = dummy
        ..data.strVal.len = badLen;
      c.eq(
        c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(badStr), 'str len=$badLen')?.kind,
        BinErrorKind.tooLarge,
        'str badLen=$badLen tooLarge',
      );
      final badBytes = ffi.Struct.create<NveValue>()
        ..tag = NveValueTag.bytes.code
        ..data.bytesVal.ptr = dummy
        ..data.bytesVal.len = badLen;
      c.eq(
        c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(badBytes), 'bytes len=$badLen')?.kind,
        BinErrorKind.tooLarge,
        'bytes badLen=$badLen tooLarge',
      );
      final badArr = ffi.Struct.create<NveValue>()
        ..tag = NveValueTag.array.code
        ..data.arrayVal.ptr = dummy.cast<NveValue>()
        ..data.arrayVal.len = badLen;
      c.eq(
        c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(badArr), 'array len=$badLen')?.kind,
        BinErrorKind.tooLarge,
        'array badLen=$badLen tooLarge',
      );
      final badMap = ffi.Struct.create<NveValue>()
        ..tag = NveValueTag.map.code
        ..data.mapVal.ptr = dummy.cast<NveMapEntry>()
        ..data.mapVal.len = badLen;
      c.eq(
        c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(badMap), 'map len=$badLen')?.kind,
        BinErrorKind.tooLarge,
        'map badLen=$badLen tooLarge',
      );
    }

    // (nullptr, len > 0) on str / bytes / array / map.
    for (final tag in const <NveValueTag>[NveValueTag.str, NveValueTag.bytes, NveValueTag.array, NveValueTag.map]) {
      final v = ffi.Struct.create<NveValue>()..tag = tag.code;
      switch (tag) {
        case NveValueTag.str:
          v.data.strVal.len = 2;
        case NveValueTag.bytes:
          v.data.bytesVal.len = 2;
        case NveValueTag.array:
          v.data.arrayVal.len = 1;
        case NveValueTag.map:
          v.data.mapVal.len = 1;
        default:
          break;
      }
      c.eq(
        c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(v), 'null ${tag.name} len>0')?.kind,
        BinErrorKind.truncated,
        'null ${tag.name} truncated',
      );
    }

    // Unaligned non-empty array / map pointers rejected with ArgumentError.
    final unalignedAddr = arena.allocBytes(64, align: 8).address + 1;
    final unalignedArr = ffi.Struct.create<NveValue>()
      ..tag = NveValueTag.array.code
      ..data.arrayVal.ptr = ffi.Pointer<NveValue>.fromAddress(unalignedAddr)
      ..data.arrayVal.len = 1;
    c.throwsA<ArgumentError>(() => NveValueTree.toDynamicValue(unalignedArr), 'unaligned array pointer');

    final unalignedMap = ffi.Struct.create<NveValue>()
      ..tag = NveValueTag.map.code
      ..data.mapVal.ptr = ffi.Pointer<NveMapEntry>.fromAddress(unalignedAddr)
      ..data.mapVal.len = 1;
    c.throwsA<ArgumentError>(() => NveValueTree.toDynamicValue(unalignedMap), 'unaligned map pointer');

    // Invalid UTF-8 in NveStrSlice and NveMapEntry key.
    final invalidUtf8 = arena.allocBytes(2, align: 1);
    invalidUtf8.asTypedList(2).setAll(0, const <int>[0xC0, 0xAF]);
    final badUtf8Str = ffi.Struct.create<NveValue>()
      ..tag = NveValueTag.str.code
      ..data.strVal.ptr = invalidUtf8
      ..data.strVal.len = 2;
    c.eq(
      c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(badUtf8Str), 'invalid UTF-8 str')?.kind,
      BinErrorKind.invalidUtf8,
      'invalid UTF-8 str kind',
    );
    final badEntry = arena.allocStruct<NveMapEntry>(ffi.sizeOf<NveMapEntry>());
    badEntry.ref.key
      ..ptr = invalidUtf8
      ..len = 2;
    final badUtf8Map = ffi.Struct.create<NveValue>()
      ..tag = NveValueTag.map.code
      ..data.mapVal.ptr = badEntry
      ..data.mapVal.len = 1;
    c.eq(
      c.throwsA<BinCodecException>(() => NveValueTree.toDynamicValue(badUtf8Map), 'invalid UTF-8 map key')?.kind,
      BinErrorKind.invalidUtf8,
      'invalid UTF-8 map key kind',
    );
  });
}

void _testPropsLoneSurrogate(Check c) {
  final badVal = c.throwsA<BinCodecException>(
    () => HybridPropsWriter.withEncodedProps<void>(
      const _LoneSurrogateProp(key: 'title', value: '\uD800'),
      (_, __) {},
    ),
    'lone surrogate in String prop rejected',
  );
  c.eq(badVal?.kind, BinErrorKind.invalidUtf8, 'String prop lone surrogate error kind');

  final badKey = c.throwsA<BinCodecException>(
    () => HybridPropsWriter.withEncodedProps<void>(
      const _LoneSurrogateProp(key: 'bad-\uDC00', value: 'ok'),
      (_, __) {},
    ),
    'lone surrogate in prop map key rejected',
  );
  c.eq(badKey?.kind, BinErrorKind.invalidUtf8, 'prop key lone surrogate error kind');
}
