import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/src/nve_bin_value.dart';
import 'package:nve_flutter/src/nve_scene_abi_gen.dart';

class _NativeAlloc {
  static final ffi.DynamicLibrary _libc = Platform.isWindows
      ? ffi.DynamicLibrary.open('ucrtbase.dll')
      : ffi.DynamicLibrary.process();

  static final ffi.Pointer<ffi.Void> Function(int) _calloc = _libc.lookupFunction<
      ffi.Pointer<ffi.Void> Function(ffi.Size, ffi.Size),
      ffi.Pointer<ffi.Void> Function(int, int)>('calloc').letCalloc();

  static final void Function(ffi.Pointer<ffi.Void>) _free = _libc.lookupFunction<
      ffi.Void Function(ffi.Pointer<ffi.Void>),
      void Function(ffi.Pointer<ffi.Void>)>('free');

  final List<ffi.Pointer<ffi.Void>> _ptrs = <ffi.Pointer<ffi.Void>>[];

  ffi.Pointer<T> allocate<T extends ffi.NativeType>(int byteCount) {
    final ffi.Pointer<ffi.Void> ptr = _calloc(byteCount == 0 ? 1 : byteCount);
    if (ptr == ffi.nullptr) {
      throw StateError('calloc($byteCount) failed');
    }
    _ptrs.add(ptr);
    return ptr.cast<T>();
  }

  void dispose() {
    for (final ffi.Pointer<ffi.Void> ptr in _ptrs) {
      _free(ptr);
    }
    _ptrs.clear();
  }
}

extension on ffi.Pointer<ffi.Void> Function(int, int) {
  ffi.Pointer<ffi.Void> Function(int) letCalloc() {
    return (int bytes) => this(1, bytes);
  }
}

void main() {
  group('NveSceneView dart:ffi ABI', () {
    test('NveRenderNode is exactly 64 bytes matching schema stride', () {
      expect(ffi.sizeOf<NveRenderNode>(), nveRenderNodeStride);
      expect(ffi.sizeOf<NveRenderNode>(), 64);
      expect(nveNodeOffsetZIndex, 60);
    });

    test('NveSceneReader reads nodes, dirty indices, text slab, and children zero-copy', () {
      final _NativeAlloc alloc = _NativeAlloc();
      addTearDown(alloc.dispose);

      final Uint8List textUtf8 = Uint8List.fromList(utf8.encode('Hello 🚀Flutter'));
      final ffi.Pointer<ffi.Uint8> textPtr = alloc.allocate<ffi.Uint8>(textUtf8.length);
      textPtr.asTypedList(textUtf8.length).setAll(0, textUtf8);

      final ffi.Pointer<ffi.Uint32> childrenPtr = alloc.allocate<ffi.Uint32>(2 * 4);
      childrenPtr.asTypedList(2).setAll(0, <int>[2, 3]);

      final ffi.Pointer<ffi.Uint32> dirtyPtr = alloc.allocate<ffi.Uint32>(4);
      dirtyPtr[0] = 1;

      final ffi.Pointer<ffi.Uint32> removedPtr = alloc.allocate<ffi.Uint32>(4);
      removedPtr[0] = 77;

      final ffi.Pointer<NveRenderNode> nodesPtr =
          alloc.allocate<NveRenderNode>(2 * ffi.sizeOf<NveRenderNode>());

      final NveRenderNode root = nodesPtr[0];
      root.nodeId = 1;
      root.kind = nveNodeKindRoot;
      root.flags = 0;
      root.opacityU8 = 255;
      root.width = 412.0;
      root.height = 915.0;
      root.bgRgba = 0xFF0F172A;
      root.childCount = 2;
      root.childOffset = 0;

      final int helloLen = utf8.encode('Hello 🚀').length;
      final NveRenderNode label = nodesPtr[1];
      label.nodeId = 2;
      label.kind = nveNodeKindText;
      label.flags = nveNodeLeafText | nveNodeInteractive;
      label.opacityU8 = 200;
      label.x = 12.5;
      label.y = 24.0;
      label.width = 180.0;
      label.height = 36.0;
      label.textRgba = 0xFFFFFFFF;
      label.borderRadius = 8.0;
      label.fontSize = 18.0;
      label.textOffset = 0;
      label.textLen = helloLen;
      label.zIndex = 4;

      final ffi.Pointer<NveSceneView> viewPtr =
          alloc.allocate<NveSceneView>(ffi.sizeOf<NveSceneView>());
      viewPtr.ref
        ..schemaHash = nveSceneSchemaHash
        ..frameId = 42
        ..nodeCount = 2
        ..dirtyCount = 1
        ..removedCount = 1
        ..textSlabLen = textUtf8.length
        ..childrenSlabLen = 2
        ..pad = 0
        ..nodes = nodesPtr
        ..textSlab = textPtr
        ..childrenSlab = childrenPtr
        ..dirtyIndices = dirtyPtr
        ..removedIds = removedPtr;

      final NveSceneReader reader = NveSceneReader.fromPointer(viewPtr);
      expect(reader.frameId, 42);
      expect(reader.nodeCount, 2);
      expect(reader.dirtyIndices, <int>[1]);
      expect(reader.removedIds, <int>[77]);
      expect(reader.childrenFor(reader.nodeAt(0)), <int>[2, 3]);
      expect(reader.textFor(reader.nodeAt(0)), '');

      final List<int> dirtyIds = <int>[];
      reader.forEachDirty((int idx, NveRenderNode node) {
        expect(idx, 1);
        dirtyIds.add(node.nodeId);
        expect(node.flags & nveNodeLeafText, isNonZero);
        expect(node.flags & nveNodeInteractive, isNonZero);
        expect(node.x, closeTo(12.5, 0.001));
        expect(node.zIndex, 4);
        expect(reader.textFor(node), 'Hello 🚀');
      });
      expect(dirtyIds, <int>[2]);
    });

    test('NveSceneReader rejects schema mismatch and null buffer pointers', () {
      final _NativeAlloc alloc = _NativeAlloc();
      addTearDown(alloc.dispose);

      final ffi.Pointer<NveSceneView> viewPtr =
          alloc.allocate<NveSceneView>(ffi.sizeOf<NveSceneView>());
      viewPtr.ref.schemaHash = nveSceneSchemaHash ^ 1;
      expect(
        () => NveSceneReader.fromPointer(viewPtr),
        throwsA(isA<NveSceneAbiException>()),
      );

      viewPtr.ref
        ..schemaHash = nveSceneSchemaHash
        ..nodeCount = 1
        ..nodes = ffi.nullptr;
      expect(
        () => NveSceneReader.fromPointer(viewPtr),
        throwsA(isA<NveSceneAbiException>()),
      );
    });
  });

  group('NveBinValueView zero-copy binary decoder', () {
    test('round-trips all wire tags 0..7 including nested containers', () {
      final Uint8List encoded = NveBinValueEncoder.encode(<String, Object?>{
        'null': null,
        'flag': true,
        'i64': -9007199254740991,
        'f64': 3.141592653589793,
        'str': 'Hello NVE 🚀',
        'bytes': Uint8List.fromList(<int>[0xDE, 0xAD, 0xBE, 0xEF]),
        'arr': <Object?>[
          42,
          <String, Object?>{'inner': 'ok'},
          false,
        ],
      });

      final NveBinValueView root = NveBinValueView.parse(encoded);
      expect(root.tag, NveBinValueTag.map);
      expect(root.mapLength, 7);
      expect(root.get('null')?.isNull, isTrue);
      expect(root.get('flag')?.asBool(), isTrue);
      expect(root.get('i64')?.asInt64(), -9007199254740991);
      expect(root.get('f64')?.asFloat64(), closeTo(3.141592653589793, 1e-12));
      expect(root.get('str')?.asString(), 'Hello NVE 🚀');
      expect(root.get('bytes')?.asBytes(), <int>[0xDE, 0xAD, 0xBE, 0xEF]);
      expect(root.get('missing'), isNull);

      final NveBinValueView? arr = root.get('arr');
      expect(arr?.arrayLength, 3);
      expect(arr?.arrayAt(0)?.asInt64(), 42);
      expect(arr?.arrayAt(1)?.get('inner')?.asString(), 'ok');
      expect(arr?.arrayAt(2)?.asBool(), isFalse);
      expect(arr?.arrayAt(3), isNull);
    });

    test('enforces depth-64 guard, bounds checks, strict UTF-8, and trailing bytes', () {
      Object? depth64;
      for (int i = 0; i < nveBinValueMaxDepth; i++) {
        depth64 = <Object?>[depth64];
      }
      final Uint8List encoded64 = NveBinValueEncoder.encode(depth64);
      expect(NveBinValueView.parse(encoded64).tag, NveBinValueTag.array);

      final Object depth65 = <Object?>[depth64];
      expect(
        () => NveBinValueEncoder.encode(depth65),
        throwsA(
          isA<NveBinValueException>().having(
            (NveBinValueException e) => e.kind,
            'kind',
            NveBinValueErrorKind.maxDepthExceeded,
          ),
        ),
      );

      // Hand-craft a 65-deep wire array to test parser depth guard directly.
      final BytesBuilder deepWire = BytesBuilder(copy: false);
      for (int level = 0; level < 65; level++) {
        final int remaining = 64 - level;
        final int payloadLen = 4 + remaining * 9 + 1;
        deepWire.addByte(NveBinValueTag.array.wireCode);
        final ByteData hdr = ByteData(8)
          ..setUint32(0, payloadLen, Endian.little)
          ..setUint32(4, 1, Endian.little);
        deepWire.add(hdr.buffer.asUint8List());
      }
      deepWire.addByte(NveBinValueTag.nullValue.wireCode);
      expect(
        () => NveBinValueView.parse(deepWire.takeBytes()),
        throwsA(
          isA<NveBinValueException>().having(
            (NveBinValueException e) => e.kind,
            'kind',
            NveBinValueErrorKind.maxDepthExceeded,
          ),
        ),
      );

      // Every strict prefix of a valid payload must fail with truncated.
      final Uint8List valid = NveBinValueEncoder.encode(<String, Object?>{'k': 'value'});
      for (int cut = 0; cut < valid.length; cut++) {
        expect(
          () => NveBinValueView.parse(Uint8List.sublistView(valid, 0, cut)),
          throwsA(
            isA<NveBinValueException>().having(
              (NveBinValueException e) => e.kind,
              'kind',
              NveBinValueErrorKind.truncated,
            ),
          ),
        );
      }

      // Trailing byte rejected.
      final Uint8List trailing = Uint8List.fromList(<int>[...valid, 0]);
      expect(
        () => NveBinValueView.parse(trailing),
        throwsA(
          isA<NveBinValueException>().having(
            (NveBinValueException e) => e.kind,
            'kind',
            NveBinValueErrorKind.trailingBytes,
          ),
        ),
      );

      // Invalid tag, invalid bool, invalid UTF-8 rejected.
      expect(
        () => NveBinValueView.parse(Uint8List.fromList(<int>[9])),
        throwsA(
          isA<NveBinValueException>().having(
            (NveBinValueException e) => e.kind,
            'kind',
            NveBinValueErrorKind.invalidTag,
          ),
        ),
      );
      expect(
        () => NveBinValueView.parse(Uint8List.fromList(<int>[1, 2])),
        throwsA(
          isA<NveBinValueException>().having(
            (NveBinValueException e) => e.kind,
            'kind',
            NveBinValueErrorKind.invalidBool,
          ),
        ),
      );
      expect(
        () => NveBinValueView.parse(Uint8List.fromList(<int>[4, 2, 0, 0, 0, 0xFF, 0xFE])),
        throwsA(
          isA<NveBinValueException>().having(
            (NveBinValueException e) => e.kind,
            'kind',
            NveBinValueErrorKind.invalidUtf8,
          ),
        ),
      );
    });
  });
}
