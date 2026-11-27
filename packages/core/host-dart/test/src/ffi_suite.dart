// VM-only (imports `dart:ffi`): the generated `ffi.Struct`s have exactly the generated 64-bit sizes.
import 'dart:ffi' as ffi;

import 'package:nve_host/nve_host.dart';
import 'package:nve_host/nve_host_ffi.dart';

import 'check.dart';

void runFfi(Check c) {
  if (ffi.sizeOf<ffi.Pointer<ffi.Void>>() != 8) {
    return; // the generated sizes below are the 64-bit ones
  }
  c.eq(ffi.sizeOf<NveRenderNode>(), NveRenderNodeLayout.size64, 'NveRenderNode');
  c.eq(ffi.sizeOf<NveNodeMeta>(), NveNodeMetaLayout.size64, 'NveNodeMeta');
  c.eq(ffi.sizeOf<NveSceneView>(), NveSceneViewLayout.size64, 'NveSceneView');
  c.eq(ffi.sizeOf<NveTableView>(), NveTableViewLayout.size64, 'NveTableView');
  c.eq(ffi.sizeOf<NveDirectEvent>(), NveDirectEventLayout.size64, 'NveDirectEvent');
  c.eq(ffi.sizeOf<NveValue>(), NveValueLayout.size64, 'NveValue');
  c.eq(ffi.sizeOf<NveMapEntry>(), NveMapEntryLayout.size64, 'NveMapEntry');
  c.eq(ffi.sizeOf<NveHostCallbacks>(), NveHostCallbacksLayout.size64, 'NveHostCallbacks');
  c.eq(ffi.sizeOf<NveFrameStats>(), NveFrameStatsLayout.size64, 'NveFrameStats');
}
