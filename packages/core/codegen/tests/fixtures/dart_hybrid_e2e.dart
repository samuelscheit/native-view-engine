import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:nve_dart_e2e/crypto_hybrid.generated.dart';
import 'package:nve_dart_e2e/multi_opt_shadow_hybrid.generated.dart';
import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

void expect(bool cond, String msg) {
  if (!cond) {
    throw StateError('ASSERTION FAILED: $msg');
  }
}

void expectEq(Object? actual, Object? expected, String msg) {
  if (actual != expected) {
    throw StateError('ASSERTION FAILED ($msg): expected $expected, got $actual');
  }
}

bool sameBytes(List<int> a, List<int> b) {
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

void main(List<String> args) {
  final lib = ffi.DynamicLibrary.open(args.first);

  // ---- 1. Crypto module ----
  final crypto = CryptoHybridClient.resolve(lib);
  expectEq(crypto.version(), '\uFEFFnve-dart-1.0', 'version preserves BOM');
  expectEq(crypto.add(19, 23), 42, 'add');

  final input = Uint8List.fromList(<int>[0x10, 0x20, 0x30]);
  final hashed = crypto.blake3(input);
  expect(sameBytes(hashed, <int>[0x4A, 0x7A, 0x6A]), 'blake3 xor');

  final hdr = crypto.hashWith(HashAlgorithm.sha512, input);
  expectEq(hdr.algorithm, HashAlgorithm.sha512, 'hdr.algorithm');
  expectEq(hdr.byteLen, 3, 'hdr.byteLen');
  expectEq(hdr.verified, true, 'hdr.verified');
  expectEq(hdr.anchor.x, 12.5, 'hdr.anchor.x');
  expectEq(hdr.anchor.y, -4.25, 'hdr.anchor.y');

  expectEq(crypto.findNonce('hello', 100), 105, 'findNonce some');
  expectEq(crypto.findNonce('hello', null), null, 'findNonce none');

  final payload = SignedPayload(
    header: const DigestHeader(
      algorithm: HashAlgorithm.sha256,
      byteLen: 8,
      verified: true,
      anchor: FramePoint(x: 1.0, y: 2.0),
    ),
    label: 'ok',
    digest: Uint8List.fromList(<int>[1, 2, 3]),
    tags: const <String>['alpha', 'beta'],
    nonce: 1337,
  );
  expectEq(crypto.verifyPayload(payload), true, 'verifyPayload ok');

  var caughtLabel = false;
  try {
    crypto.verifyPayload(
      SignedPayload(
        header: payload.header,
        label: '',
        digest: Uint8List(0),
        tags: const <String>[],
        nonce: null,
      ),
    );
  } on HybridException catch (e) {
    caughtLabel = e.status == HybridStatus.err && e.message == 'missing payload label';
  }
  expect(caughtLabel, 'verifyPayload err throws HybridException');

  expectEq(crypto.lookupPayload(0), null, 'lookupPayload none');
  final found = crypto.lookupPayload(7);
  expect(found != null, 'lookupPayload some');
  expectEq(found!.header.algorithm, HashAlgorithm.blake3, 'found.algorithm');
  expectEq(found.header.byteLen, 28, 'found.byteLen');
  expectEq(found.label, 'rec-7', 'found.label');
  expect(sameBytes(found.digest, <int>[0xDE, 0xAD, 0xBE, 0xEF]), 'found.digest');
  expectEq(found.tags.length, 2, 'found.tags len');
  expectEq(found.nonce, 4242, 'found.nonce');

  final batch = crypto.batchSign(const <String>['one', 'two'], HashAlgorithm.sha256);
  expectEq(batch.length, 2, 'batchSign len');
  expectEq(batch[0].label, 'one', 'batch[0].label');
  expectEq(batch[1].label, 'two', 'batch[1].label');

  // Exercise LiveCrypto::batch_sign's Err("empty batch") branch!
  var caughtEmptyBatch = false;
  try {
    crypto.batchSign(const <String>[], HashAlgorithm.sha256);
  } on HybridException catch (e) {
    caughtEmptyBatch = e.status == HybridStatus.err && e.message == 'empty batch';
  }
  expect(caughtEmptyBatch, 'batchSign empty list throws HybridException("empty batch")');
  crypto.resetCache();

  // ---- 2. HybridType::Value across method param & return (NO NVE_VAL_BYTES double-wrapping!) ----
  final minI64 = const Key64(0, 0x80000000).toSignedInt();
  final maxI64 = const Key64(0xFFFFFFFF, 0x7FFFFFFF).toSignedInt();
  final u64MaxBits = const Key64(0xFFFFFFFF, 0xFFFFFFFF).toSignedInt();
  final richMeta = DynamicValue.fromObject(<String, Object?>{
    'null': null,
    'flag': true,
    'min': minI64,
    'max': maxI64,
    'u64Max': u64MaxBits,
    'f': 2.5,
    'inf': double.infinity,
    'ninf': double.negativeInfinity,
    'text': '\uFEFFhéllo 🚀',
    'raw': Uint8List.fromList(<int>[0x00, 0xAA, 0xFF]),
    'emptyArr': <Object?>[],
    'emptyMap': <String, Object?>{},
    'arr': <Object?>[
      1,
      'x',
      null,
      <String, Object?>{'nested': false},
    ],
  });
  final echoed = crypto.inspectDynamic(richMeta);
  expectEq(echoed.tag, BinTag.map, 'inspectDynamic preserves MAP tag (not BYTES)');
  expectEq(echoed, richMeta, 'inspectDynamic byte-identical round-trip');
  final echoedNan = crypto.inspectDynamic(DynamicValue.fromObject(double.nan));
  expectEq(echoedNan.tag, BinTag.f64, 'inspectDynamic NaN tag');
  expect((echoedNan.toObject() as double).isNaN, 'inspectDynamic NaN preserved');
  expectEq(crypto.inspectDynamic(DynamicValue.nullValue), DynamicValue.nullValue, 'inspectDynamic null');

  // ---- 3. Cursor shadowing + multi-Option + nested Vec<Vec<i32>> ----
  final cursor = Cursor(
    mark: 1,
    w: Uint8List.fromList(<int>[1, 2, 3]),
    r: 'ab',
    count: 4,
    tag: 42,
    grid: const <List<int>>[
      <int>[1, 2, 3],
      <int>[4, 5, 6],
    ],
  );
  final t = crypto.transpose(cursor);
  expectEq(t.mark, 2, 'cursor.mark');
  expectEq(t.count, 8, 'cursor.count');
  expectEq(t.r, 'AB', 'cursor.r');
  expectEq(t.tag, -42, 'cursor.tag');
  expect(sameBytes(t.w, <int>[3, 2, 1]), 'cursor.w');
  expectEq(t.grid.length, 3, 'cursor.grid rows');
  expectEq(t.grid[0][0], 1, 'cursor.grid[0][0]');
  expectEq(t.grid[0][1], 4, 'cursor.grid[0][1]');
  expectEq(t.grid[2][1], 6, 'cursor.grid[2][1]');

  // Invalid UTF-8 (lone surrogate) rejected BEFORE crossing FFI.
  var caughtUtf8 = false;
  try {
    crypto.batchSign(const <String>['\uD800'], HashAlgorithm.sha256);
  } on BinCodecException catch (e) {
    caughtUtf8 = e.kind == BinErrorKind.invalidUtf8;
  }
  expect(caughtUtf8, 'lone surrogate rejected before FFI');

  // ---- 4. VideoSurface view: setProps with lossless HybridType::Value (NO double-wrapping!) ----
  final video = VideoSurfaceHybridClient.resolve(lib);
  video.setProps(
    VideoSurfaceProps(
      sourceUrl: 'https://nve.dev/clip.mp4',
      muted: true,
      quality: VideoQuality.uhd4k,
    ),
  );
  video.seek(9876);
  final frame1 = video.captureFrame(const FramePoint(x: 5.0, y: 9.0));
  expect(sameBytes(frame1, <int>[5, 9, 24]), 'captureFrame baseline');

  // Now set present Option<String>, Option<FramePoint>, Bytes, Vec<i32>, AND rich DynamicValue metadata.
  video.setProps(
    VideoSurfaceProps(
      sourceUrl: 'https://nve.dev/clip.mp4',
      muted: true,
      quality: VideoQuality.uhd4k,
      posterUrl: 'p.png',
      focusPoint: const FramePoint(x: 7.0, y: 0.0),
      drmKey: Uint8List.fromList(<int>[0xAA, 0xBB]),
      chapters: const <int>[10, 20, 30],
      metadata: richMeta,
    ),
  );
  final frame2 = video.captureFrame(const FramePoint(x: 5.0, y: 9.0));
  expect(sameBytes(frame2, <int>[5, 9, 24, 7, 5, 0xAA, 0xBB, 10, 20, 30]), 'captureFrame with all props');
  final gotMeta = video.currentMetadata();
  expectEq(gotMeta.tag, BinTag.map, 'VideoSurface.metadata arrives as MAP, never BYTES');
  expectEq(gotMeta, richMeta, 'VideoSurface.metadata byte-identical round-trip');

  // Verify scalar & array DynamicValue props too (never double-wrapped).
  for (final sample in <DynamicValue>[
    DynamicValue.nullValue,
    DynamicValue.fromObject(false),
    DynamicValue.fromObject(-999),
    DynamicValue.fromObject(3.14),
    DynamicValue.fromObject('hello 🚀'),
    DynamicValue.fromObject(Uint8List.fromList(<int>[7, 8, 9])),
    DynamicValue.fromObject(<Object?>[1, 'two', true]),
  ]) {
    video.setProps(VideoSurfaceProps(sourceUrl: 'x', muted: true, metadata: sample));
    final cur = video.currentMetadata();
    expectEq(cur.tag, sample.tag, 'metadata prop tag ${sample.tag}');
    expectEq(cur, sample, 'metadata prop value ${sample.tag}');
  }

  // Error message with UTF-8 emoji round-trips cleanly.
  var caughtEmoji = false;
  try {
    video.captureFrame(const FramePoint(x: -1.0, y: 0.0));
  } on HybridException catch (e) {
    caughtEmoji = e.status == HybridStatus.err && e.message == 'bad \u{1F600}';
  }
  expect(caughtEmoji, 'captureFrame emoji error');

  // ---- 5. Multi-Option + heavy shadowing module & view ----
  final multi = MultiOptShadowHybridClient.resolve(lib);
  expectEq(multi.podOnly(6, 7), 42, 'podOnly');

  final recIn = ShadowRecord(
    w: 'inner',
    r: 'opt',
    mark: 3,
    count: Uint8List.fromList(<int>[9]),
    value: richMeta,
  );
  final rSome = multi.collide(
    const RawPod(raw: 11),
    'h',
    55,
    'ar',
    const RawPod(raw: 22),
    const <int>[1, 2, 3],
    recIn,
    richMeta,
  );
  expectEq(rSome.w, 'h:11|fn=55|arena=ar|builder=22|props=3|rec=inner', 'multi-opt all some');
  expectEq(rSome.r, 'ar', 'multi-opt r');
  expectEq(rSome.mark, 22, 'multi-opt mark');
  expect(sameBytes(rSome.count, <int>[1, 2, 3]), 'multi-opt count');
  expectEq(rSome.value, richMeta, 'multi-opt DynamicValue');

  final rNone = multi.collide(
    const RawPod(raw: 7),
    'z',
    null,
    null,
    null,
    null,
    null,
    DynamicValue.nullValue,
  );
  expectEq(rNone.w, 'z:7', 'multi-opt all none');
  expectEq(rNone.r, null, 'multi-opt none r');
  expectEq(rNone.mark, null, 'multi-opt none mark');
  expect(rNone.value.isNull, 'multi-opt none DynamicValue');

  final shadowView = ShadowViewHybridClient.resolve(lib);
  shadowView.setProps(
    ShadowViewProps(
      builder: 'my-builder',
      optPod: const RawPod(raw: 88),
      optRec: recIn,
      meta: richMeta,
    ),
  );
  expectEq(shadowView.summary(), 'my-builder|88|inner', 'ShadowView summary some');
  expectEq(shadowView.currentMeta(), richMeta, 'ShadowView meta lossless');
  shadowView.setProps(ShadowViewProps(builder: 'empty', meta: DynamicValue.nullValue));
  expectEq(shadowView.summary(), 'empty|-1|none', 'ShadowView summary none');
  expect(shadowView.currentMeta().isNull, 'ShadowView meta null');

  // Lone surrogate in view String prop rejected BEFORE crossing FFI.
  var caughtPropUtf8 = false;
  try {
    video.setProps(VideoSurfaceProps(sourceUrl: 'bad-\uD800'));
  } on BinCodecException catch (e) {
    caughtPropUtf8 = e.kind == BinErrorKind.invalidUtf8;
  }
  expect(caughtPropUtf8, 'lone surrogate in String prop rejected before FFI');

  // ---- 6. Negative runtime FFI checks: resolve missing, fromVtable(nullptr), ABI/schema mismatch,
  //         and setProps non-OK status ----
  var caughtMissingIface = false;
  try {
    hybridResolveInterface(lib, 0xDEADBEEF, 'Missing');
  } on HybridException catch (e) {
    caughtMissingIface = e.status == HybridStatus.err && e.message.contains('Missing');
  }
  expect(caughtMissingIface, 'hybridResolveInterface missing interface throws HybridException');

  var caughtNullVtable = false;
  try {
    CryptoHybridClient.fromVtable(ffi.nullptr);
  } on HybridException catch (e) {
    caughtNullVtable = e.status == HybridStatus.nullHandle;
  }
  expect(caughtNullVtable, 'fromVtable(nullptr) throws HybridException(nullHandle)');

  HybridArena.using<void>((arena) {
    final fakeCryptoVt = arena.allocStruct<NveHybridCryptoVTable>(ffi.sizeOf<NveHybridCryptoVTable>());
    fakeCryptoVt.ref
      ..abiVersion = CryptoHybridClient.abiVersion + 1
      ..interfaceId = CryptoHybridClient.interfaceId
      ..schemaHash = CryptoHybridClient.schemaHash.toSignedInt();
    var caughtAbi = false;
    try {
      CryptoHybridClient.fromVtable(fakeCryptoVt);
    } on HybridException catch (e) {
      caughtAbi = e.status == HybridStatus.schemaMismatch && e.message.contains('Crypto');
    }
    expect(caughtAbi, 'fromVtable ABI mismatch throws HybridException(schemaMismatch)');

    fakeCryptoVt.ref
      ..abiVersion = CryptoHybridClient.abiVersion
      ..schemaHash = const Key64(1, 2).toSignedInt();
    var caughtSchema = false;
    try {
      CryptoHybridClient.fromVtable(fakeCryptoVt);
    } on HybridException catch (e) {
      caughtSchema = e.status == HybridStatus.schemaMismatch && e.message.contains('Crypto');
    }
    expect(caughtSchema, 'fromVtable schema mismatch throws HybridException(schemaMismatch)');

    // Synthetic VideoSurface vtable whose setPropsDirect returns HybridStatus.invalidArg.
    final dummyHandle = arena.allocBytes(8).cast<ffi.Void>();
    final createCb = ffi.NativeCallable<HybridCreateNative>.isolateLocal(
      () => dummyHandle,
    );
    final destroyCb = ffi.NativeCallable<HybridDestroyNative>.isolateLocal(
      (ffi.Pointer<ffi.Void> _) {},
    );
    final setPropsFailCb = ffi.NativeCallable<HybridSetPropsDirectNative>.isolateLocal(
      (ffi.Pointer<ffi.Void> _, ffi.Pointer<NveMapEntry> __, int ___) => HybridStatus.invalidArg.code,
      exceptionalReturn: -1,
    );
    try {
      final realVideoVt = hybridResolveInterface(lib, VideoSurfaceHybridClient.interfaceId, 'VideoSurface')
          .cast<NveHybridVideoSurfaceVTable>();
      final fakeVideoVt =
          arena.allocStruct<NveHybridVideoSurfaceVTable>(ffi.sizeOf<NveHybridVideoSurfaceVTable>());
      fakeVideoVt.ref
        ..abiVersion = VideoSurfaceHybridClient.abiVersion
        ..interfaceId = VideoSurfaceHybridClient.interfaceId
        ..schemaHash = VideoSurfaceHybridClient.schemaHash.toSignedInt()
        ..create = createCb.nativeFunction
        ..destroy = destroyCb.nativeFunction
        ..allocBytes = realVideoVt.ref.allocBytes
        ..freeBytes = realVideoVt.ref.freeBytes
        ..setPropsDirect = setPropsFailCb.nativeFunction
        ..seek = realVideoVt.ref.seek
        ..captureFrame = realVideoVt.ref.captureFrame
        ..currentMetadata = realVideoVt.ref.currentMetadata;
      final failingVideo = VideoSurfaceHybridClient.fromVtable(fakeVideoVt);
      var caughtSetProps = false;
      try {
        failingVideo.setProps(VideoSurfaceProps(sourceUrl: 'x'));
      } on HybridException catch (e) {
        caughtSetProps = e.status == HybridStatus.invalidArg;
      }
      expect(caughtSetProps, 'setProps non-OK status throws HybridException(invalidArg)');
      failingVideo.dispose();
    } finally {
      createCb.close();
      destroyCb.close();
      setPropsFailCb.close();
    }
  });

  // ---- 7. Dispose & use-after-dispose ----
  crypto.dispose();
  crypto.dispose(); // idempotent
  var caughtDisposed = false;
  try {
    crypto.add(1, 2);
  } on HybridException catch (e) {
    caughtDisposed = e.status == HybridStatus.nullHandle;
  }
  expect(caughtDisposed, 'use-after-dispose throws HybridException(nullHandle)');

  video.dispose();
  multi.dispose();
  shadowView.dispose();
}
