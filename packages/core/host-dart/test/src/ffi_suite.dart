// VM-only (imports `dart:ffi`): the generated `ffi.Struct`s have exactly the generated sizes of the
// running data model (64-bit, ILP32, or i386).
import 'dart:ffi' as ffi;

import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'check.dart';

void runFfi(Check c) {
  final abi = ffi.Abi.current();
  final model = ffi.sizeOf<ffi.Pointer<ffi.Void>>() == 8
      ? 0
      : (abi == ffi.Abi.linuxIA32 || abi == ffi.Abi.androidIA32 || abi == ffi.Abi.windowsIA32 ? 2 : 1);
  final measured = <String, int>{
    'NveBufferSlice': ffi.sizeOf<NveBufferSlice>(),
    'NveStrSlice': ffi.sizeOf<NveStrSlice>(),
    'NveValueArray': ffi.sizeOf<NveValueArray>(),
    'NveValueMap': ffi.sizeOf<NveValueMap>(),
    'NveValueData': ffi.sizeOf<NveValueData>(),
    'NveValue': ffi.sizeOf<NveValue>(),
    'NveMapEntry': ffi.sizeOf<NveMapEntry>(),
    'NveDirectEvent': ffi.sizeOf<NveDirectEvent>(),
    'NveEventBatch': ffi.sizeOf<NveEventBatch>(),
    'NvePropEntry': ffi.sizeOf<NvePropEntry>(),
    'NvePropSlice': ffi.sizeOf<NvePropSlice>(),
    'NveCommandSlice': ffi.sizeOf<NveCommandSlice>(),
    'NveSurfaceConfig': ffi.sizeOf<NveSurfaceConfig>(),
    'NveFrameStats': ffi.sizeOf<NveFrameStats>(),
    'NveSizeConstraints': ffi.sizeOf<NveSizeConstraints>(),
    'NveIntrinsicSize': ffi.sizeOf<NveIntrinsicSize>(),
    'NveHostCallbacks': ffi.sizeOf<NveHostCallbacks>(),
    'NveRenderNode': ffi.sizeOf<NveRenderNode>(),
    'NveTableView': ffi.sizeOf<NveTableView>(),
    'NveSceneView': ffi.sizeOf<NveSceneView>(),
    'NveNodeMeta': ffi.sizeOf<NveNodeMeta>(),
  };
  // A record added to the ABI must be measured here too.
  c.eq(measured.length, nveAbiRecordSizes.length, 'every ABI record is measured');
  c.that(measured.keys.toSet().containsAll(nveAbiRecordSizes.keys), 'measured names match the generated records');
  for (final MapEntry(:key, :value) in measured.entries) {
    c.eq(value, nveAbiRecordSizes[key]?[model], '$key ffi.sizeOf (model $model)');
  }
}
