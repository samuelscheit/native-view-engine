#import "NveAppleComponentHost.h"

#import <CoreFoundation/CoreFoundation.h>

#include "nve_host_runtime.hpp"
#include "nve_snapshot.hpp"
#include "nve_value.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <memory>
#include <span>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace {

NSError *make_init_error(NSString *domain, NSString *description) {
  return [NSError errorWithDomain:domain
                             code:static_cast<NSInteger>(nve::apple::DylibErrorCode::MissingSurfaceInterface)
                         userInfo:@{NSLocalizedDescriptionKey : description}];
}

NveValue nve_value_from_objc_object(id _Nullable obj, nve::BumpArena &arena, uint32_t depth) {
  if (obj == nil || obj == [NSNull null] || depth > NVE_BIN_VALUE_MAX_DEPTH) {
    return nve::make_null_value();
  }
  if ([obj isKindOfClass:[NSNumber class]]) {
    NSNumber *num = (NSNumber *)obj;
    if (CFGetTypeID((__bridge CFTypeRef)num) == CFBooleanGetTypeID()) {
      return nve::make_bool_value(num.boolValue ? true : false);
    }
    if (CFNumberIsFloatType((__bridge CFNumberRef)num)) {
      const double d = num.doubleValue;
      return std::isfinite(d) ? nve::make_f64_value(d) : nve::make_null_value();
    }
    return nve::make_i64_value(static_cast<int64_t>(num.longLongValue));
  }
  if ([obj isKindOfClass:[NSString class]]) {
    NSString *str = (NSString *)obj;
    const char *utf8 = str.UTF8String;
    if (!utf8) {
      return nve::make_str_value(NveStrSlice{nullptr, 0});
    }
    return nve::make_str_value(arena.copy_str(std::string_view(utf8)));
  }
  if ([obj isKindOfClass:[NSData class]]) {
    NSData *data = (NSData *)obj;
    return nve::make_bytes_value(
        arena.copy_bytes(static_cast<const uint8_t *>(data.bytes), data.length));
  }
  if ([obj isKindOfClass:[NSArray class]]) {
    NSArray *arr = (NSArray *)obj;
    const size_t n = arr.count;
    NveValue *elems = n > 0 ? arena.alloc_array<NveValue>(n) : nullptr;
    for (size_t i = 0; i < n; ++i) {
      elems[i] = nve_value_from_objc_object(arr[i], arena, depth + 1u);
    }
    return nve::make_array_value(elems, n);
  }
  if ([obj isKindOfClass:[NSDictionary class]]) {
    NSDictionary *dict = (NSDictionary *)obj;
    const size_t n = dict.count;
    NveMapEntry *entries = n > 0 ? arena.alloc_array<NveMapEntry>(n) : nullptr;
    __block size_t idx = 0;
    [dict enumerateKeysAndObjectsUsingBlock:^(id key, id val, BOOL *stop) {
      (void)stop;
      if (idx >= n) return;
      NSString *keyStr = [key isKindOfClass:[NSString class]] ? (NSString *)key : [key description];
      const char *kutf8 = keyStr.UTF8String;
      entries[idx++] = NveMapEntry{
          kutf8 ? arena.copy_str(std::string_view(kutf8)) : NveStrSlice{nullptr, 0},
          nve_value_from_objc_object(val, arena, depth + 1u),
      };
    }];
    return nve::make_map_value(entries, idx);
  }
  return nve::make_null_value();
}

} // namespace

@implementation NveAppleComponentHost {
  __weak id<NveAppleComponentViewAdapter> _adapter;
  NSString *_errorDomain;
  std::unique_ptr<nve::NveRnComponentBridge> _bridge;
  NveFabricHostHandle _frame0Host;
  std::vector<uint8_t> _pendingSnapshotState;
  void *_dylibHandle;
  uint32_t _surfaceId;

  // Persistent retained-view tables (reused across frames; zero per-frame hash-table allocations).
  std::unordered_map<uint32_t, NvePlatformView *> _nodeViews;
  std::unordered_map<uint32_t, uint32_t> _nodeViewTags;
  std::unordered_set<const void *> _managedViewPtrs;
  std::unordered_map<uint32_t, nve::RawPropMap> _rawPropsCache;
  std::unordered_set<uint32_t> _scrollViewNodeIds;
  std::unordered_map<uint32_t, std::vector<uint32_t>> _scrollViewChildren;

  // Reusable scratch buffers for O(changed) child reconciliation and full-sync passes.
  std::vector<NvePlatformView *> _scratchDesiredChildren;
  std::unordered_set<const void *> _scratchDesiredPtrs;
  std::vector<const void *> _scratchCurrentOrder;
  std::unordered_set<uint32_t> _scratchLiveIds;
  std::unordered_set<uint32_t> _scratchMountedChildIds;
  std::vector<uint32_t> _scratchExtraDirtyIds;

  bool _sceneInitialized;
  uint64_t _lastSceneFrameId;
  uint32_t _lastReconciledDirtyCount;
  uint32_t _lastReconciledRemovedCount;
  nve::BumpArena _propsArena;
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                    errorDomain:(NSString *)errorDomain {
  self = [super init];
  if (self) {
    _adapter = adapter;
    _errorDomain = [errorDomain copy];
    _frame0Host = nullptr;
    _dylibHandle = nullptr;
    _surfaceId = 1;
    _sceneInitialized = false;
    _lastSceneFrameId = 0;
    _lastReconciledDirtyCount = 0;
    _lastReconciledRemovedCount = 0;
  }
  return self;
}

- (void)dealloc {
  // Destroy the bridge (which calls `surface->destroy`) before closing the dylib handle.
  _bridge.reset();
  if (_frame0Host != nullptr) {
    nve_fabric_host_destroy(_frame0Host);
    _frame0Host = nullptr;
  }
  nve::apple::close_dylib(_dylibHandle);
}

- (uint32_t)surfaceId {
  return _surfaceId;
}

- (uint32_t)lastReconciledDirtyCount {
  return _lastReconciledDirtyCount;
}

- (uint32_t)lastReconciledRemovedCount {
  return _lastReconciledRemovedCount;
}

- (NveFabricHostHandle)_activeFabricHost {
  if (_bridge) {
    return _bridge->fabric_host();
  }
  return _frame0Host;
}

- (uint32_t)nodeCount {
  NveFabricHostHandle host = [self _activeFabricHost];
  return host ? nve_fabric_host_node_count(host) : 0;
}

- (BOOL)hasReplayedFrame0 {
  return _frame0Host != nullptr ? YES : NO;
}

- (BOOL)_installModule:(NveModule)mod
           dylibHandle:(void *)dylibHandle
             surfaceId:(uint32_t)surfaceId
         restoredState:(const std::vector<uint8_t> &)savedState
                 error:(NSError * _Nullable * _Nullable)error {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (!adapter || !mod.surface || !mod.surface->create) {
    nve::apple::close_dylib(dylibHandle);
    if (error) {
      *error = make_init_error(_errorDomain, @"Missing NVE_IFACE_SURFACE interface");
    }
    return NO;
  }

  const std::vector<uint8_t> &effectiveState =
      !savedState.empty() ? savedState : _pendingSnapshotState;

  // Transactional hot-swap: construct, restore state, and render the replacement bridge BEFORE
  // tearing down the currently running bridge, dylib, or mounted native views.
  __weak NveAppleComponentHost *weakSelf = self;
  auto nextBridge = std::make_unique<nve::NveRnComponentBridge>(
      mod,
      [adapter nveSurfaceConfigForSurfaceId:surfaceId],
      [weakSelf](const nve::DecodedEvent &ev) {
        __strong NveAppleComponentHost *strongSelf = weakSelf;
        if (!strongSelf) return;
        id<NveAppleComponentViewAdapter> strongAdapter = strongSelf->_adapter;
        if (!strongAdapter) return;
        NSString *eventName = [NSString stringWithUTF8String:ev.name.c_str()] ?: @"onCustom";
        NSDictionary<NSString *, id> *payload = nve::apple::payload_dictionary_from_event(ev);
        [strongAdapter nveEmitEvent:eventName payload:payload];
      });
  if (!nextBridge->is_valid()) {
    nextBridge.reset();
    nve::apple::close_dylib(dylibHandle);
    if (error) {
      *error = make_init_error(_errorDomain, @"Failed to initialize NVE surface instance");
    }
    return NO;
  }

  nve::apple::install_native_text_measurer(nextBridge->fabric_host());
  if (!effectiveState.empty()) {
    nextBridge->restore_state(
        NveBufferSlice{effectiveState.data(), effectiveState.size()}, /*auto_render=*/false);
  }

  NveFrameStats initialStats{};
  const int32_t rc = nextBridge->render(&initialStats);
  if (rc != 0) {
    nextBridge.reset();
    nve::apple::close_dylib(dylibHandle);
    if (error) {
      *error = make_init_error(
          _errorDomain,
          [NSString stringWithFormat:@"Initial frame render failed (rc=%d)", rc]);
    }
    return NO;
  }

  // Replacement is live and verified — now atomically tear down old state and commit.
  _bridge.reset();
  if (_frame0Host != nullptr) {
    nve_fabric_host_destroy(_frame0Host);
    _frame0Host = nullptr;
  }
  _pendingSnapshotState.clear();
  nve::apple::close_dylib(_dylibHandle);
  [self clearNativeNodeViews];

  _dylibHandle = dylibHandle;
  _surfaceId = surfaceId;
  _bridge = std::move(nextBridge);
  [self _syncNativeViewsFromHostWithMutations:YES];
  [self drainEmittedEvents];
  return YES;
}

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  const NveModule mod = nve_resolve_module(getInterface);
  return [self _installModule:mod
                  dylibHandle:nullptr
                    surfaceId:surfaceId
                restoredState:{}
                        error:nil];
}

- (BOOL)loadDylibAtPath:(NSString *)dylibPath
              surfaceId:(uint32_t)surfaceId
                  error:(NSError * _Nullable * _Nullable)error {
  nve::apple::LoadedDylib loaded{};
  if (!nve::apple::open_nve_dylib(dylibPath, _errorDomain, &loaded, error)) {
    return NO;
  }
  return [self _installModule:loaded.module
                  dylibHandle:loaded.handle
                    surfaceId:surfaceId
                restoredState:{}
                        error:error];
}

- (BOOL)reloadDylibAtPath:(NSString *)dylibPath
                    error:(NSError * _Nullable * _Nullable)error {
  // Snapshot current state, validate + open + initialize replacement BEFORE tearing down old app.
  nve::apple::LoadedDylib loaded{};
  if (!nve::apple::open_nve_dylib(dylibPath, _errorDomain, &loaded, error)) {
    return NO;
  }
  std::vector<uint8_t> saved;
  if (_bridge) {
    saved = _bridge->snapshot_state();
  }
  return [self _installModule:loaded.module
                  dylibHandle:loaded.handle
                    surfaceId:_surfaceId
                restoredState:saved
                        error:error];
}

- (BOOL)replayFrame0SnapshotAtPath:(NSString *)snapshotPath
             expectedSchemaVersion:(uint32_t)expectedSchemaVersion
                     restoreTimeUs:(uint64_t * _Nullable)outRestoreUs {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (!adapter || snapshotPath.length == 0) {
    return NO;
  }
  const char *utf8 = snapshotPath.UTF8String;
  if (utf8 == nullptr) {
    return NO;
  }

  using Clock = std::chrono::steady_clock;
  const auto t0 = Clock::now();

  nve::MmapSnapshot snap;
  if (!nve::MmapSnapshot::open_with_schema(
          std::string_view(utf8), expectedSchemaVersion, snap, /*verify_checksum=*/true)) {
    return NO;
  }

  const NveBufferSlice frame0 = snap.frame0_slice();
  if (frame0.ptr == nullptr || frame0.len == 0u) {
    return NO;
  }
  const NveBufferSlice state = snap.state_slice();

  if (_bridge) {
    if (nve_fabric_host_submit_frame(_bridge->fabric_host(), frame0, nullptr) != 0) {
      return NO;
    }
    if (state.ptr != nullptr && state.len > 0u) {
      _bridge->restore_state(state, /*auto_render=*/false);
    }
  } else {
    const NveSurfaceConfig cfg = [adapter nveSurfaceConfigForSurfaceId:_surfaceId];
    NveFabricHostHandle nextHost = nve_fabric_host_create(cfg);
    if (nextHost == nullptr) {
      return NO;
    }
    nve::apple::install_native_text_measurer(nextHost);
    if (nve_fabric_host_submit_frame(nextHost, frame0, nullptr) != 0) {
      nve_fabric_host_destroy(nextHost);
      return NO;
    }
    if (_frame0Host != nullptr) {
      nve_fabric_host_destroy(_frame0Host);
    }
    _frame0Host = nextHost;
    if (state.ptr != nullptr && state.len > 0u) {
      _pendingSnapshotState.assign(state.ptr, state.ptr + state.len);
    } else {
      _pendingSnapshotState.clear();
    }
  }

  [self clearNativeNodeViews];
  [self _syncNativeViewsFromHostWithMutations:YES];

  const auto t1 = Clock::now();
  if (outRestoreUs != nullptr) {
    *outRestoreUs = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count());
  }
  return YES;
}

- (BOOL)saveSnapshotAtPath:(NSString *)snapshotPath
             schemaVersion:(uint32_t)schemaVersion {
  if (snapshotPath.length == 0) {
    return NO;
  }
  const char *utf8 = snapshotPath.UTF8String;
  if (utf8 == nullptr) {
    return NO;
  }
  NveFabricHostHandle host = [self _activeFabricHost];
  if (host == nullptr) {
    return NO;
  }
  const uint64_t frameId = (_bridge && _bridge->frame_count() > 0u) ? _bridge->frame_count() : 1u;
  const std::vector<uint8_t> frame0 = nve::fabric_host_serialize_frame0(host, frameId);
  if (frame0.empty()) {
    return NO;
  }
  const std::vector<uint8_t> state = _bridge ? _bridge->snapshot_state() : _pendingSnapshotState;
  return nve::write_snapshot_atomic(
             std::string_view(utf8),
             schemaVersion,
             NveBufferSlice{frame0.data(), frame0.size()},
             NveBufferSlice{state.empty() ? nullptr : state.data(), state.size()})
             ? YES
             : NO;
}

- (BOOL)applyPropsJson:(NSString *)propsJson {
  if (!_bridge) return NO;
  NSData *utf8 = [propsJson dataUsingEncoding:NSUTF8StringEncoding];
  if (!utf8) return NO;
  const std::string_view sv(reinterpret_cast<const char *>(utf8.bytes), utf8.length);
  const bool dirty = _bridge->update_props_json(sv, /*auto_render=*/false);
  if (dirty) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
  return dirty ? YES : NO;
}

- (BOOL)applyPropsDictionary:(NSDictionary<NSString *, id> *)props {
  if (!_bridge || !props) return NO;
  _propsArena.reset();
  const NveValue mapVal = nve_value_from_objc_object(props, _propsArena, 0u);
  const NveMapEntry *entries = (mapVal.tag == NVE_VAL_MAP) ? mapVal.data.map_val.ptr : nullptr;
  const uint32_t count =
      (mapVal.tag == NVE_VAL_MAP) ? static_cast<uint32_t>(mapVal.data.map_val.len) : 0u;
  const bool dirty = _bridge->update_props_direct(entries, count, /*auto_render=*/false);
  if (dirty) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
  return dirty ? YES : NO;
}

- (void)renderFrame {
  if (!_bridge) return;
  NveFrameStats stats{};
  const int32_t rc = _bridge->render(&stats);
  if (rc == 0) {
    [self _syncNativeViewsFromHostWithMutations:(stats.mutation_count > 0u ? YES : NO)];
  }
  [self drainEmittedEvents];
}

- (void)drainEmittedEvents {
  if (!_bridge) return;
  (void)_bridge->drain_emitted_events();
}

- (void)_dispatchInboundEventsAndRender {
  const NveBufferSlice inbound = nve_fabric_host_drain_events(_bridge->fabric_host());
  const uint32_t dirty = _bridge->dispatch_native_events(inbound, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
}

- (void)dispatchPressOnNodeId:(uint32_t)nodeId x:(float)x y:(float)y {
  if (!_bridge) return;
  nve::fabric_host_enqueue_press(
      _bridge->fabric_host(), nodeId, nve::apple::current_timestamp_us(), x, y);
  [self _dispatchInboundEventsAndRender];
}

- (void)dispatchTextChangeOnNodeId:(uint32_t)nodeId text:(NSString *)text {
  if (!_bridge) return;
  NSData *utf8 = [(text ?: @"") dataUsingEncoding:NSUTF8StringEncoding];
  const std::string_view sv =
      utf8 ? std::string_view(reinterpret_cast<const char *>(utf8.bytes), utf8.length)
           : std::string_view{};
  nve::fabric_host_enqueue_text_change(
      _bridge->fabric_host(), nodeId, nve::apple::current_timestamp_us(), sv);
  [self _dispatchInboundEventsAndRender];
}

- (void)dispatchSubmitOnNodeId:(uint32_t)nodeId text:(NSString *)text {
  if (!_bridge) return;
  NSData *utf8 = [(text ?: @"") dataUsingEncoding:NSUTF8StringEncoding];
  const std::string_view sv =
      utf8 ? std::string_view(reinterpret_cast<const char *>(utf8.bytes), utf8.length)
           : std::string_view{};
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(nve::EventKind::Custom);
  ev.payload_tag = NVE_EVENT_PAYLOAD_CUSTOM;
  ev.name = nve::make_str_slice("onSubmit");
  ev.text = nve::make_str_slice(sv);
  ev.value = nve::make_str_value(sv);
  const uint32_t dirty =
      _bridge->dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
}

- (void)dispatchFocusOnNodeId:(uint32_t)nodeId focused:(BOOL)focused {
  if (!_bridge) return;
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(focused ? nve::EventKind::Focus : nve::EventKind::Blur);
  ev.payload_tag = NVE_EVENT_PAYLOAD_EMPTY;
  ev.value = nve::make_null_value();
  const uint32_t dirty =
      _bridge->dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
}

- (void)dispatchScrollOnNodeId:(uint32_t)nodeId
                             x:(float)x
                             y:(float)y
                  contentWidth:(float)contentWidth
                 contentHeight:(float)contentHeight {
  if (!_bridge || nodeId == 0u) return;
  NveDirectEvent ev{};
  ev.surface_id = _surfaceId;
  ev.node_id = nodeId;
  ev.timestamp_us = nve::apple::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(nve::EventKind::Scroll);
  ev.payload_tag = NVE_EVENT_PAYLOAD_SCROLL;
  ev.x = std::isfinite(x) ? x : 0.0f;
  ev.y = std::isfinite(y) ? y : 0.0f;
  ev.width = (std::isfinite(contentWidth) && contentWidth >= 0.0f) ? contentWidth : 0.0f;
  ev.height = (std::isfinite(contentHeight) && contentHeight >= 0.0f) ? contentHeight : 0.0f;
  ev.value = nve::make_null_value();
  const uint32_t dirty =
      _bridge->dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/false);
  if (dirty != 0) {
    [self renderFrame];
  } else {
    [self drainEmittedEvents];
  }
}

- (void)updateSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor {
  NveFabricHostHandle host = [self _activeFabricHost];
  if (!host) return;
  if (width > 0.0f && height > 0.0f) {
    if (_bridge) {
      _bridge->set_surface_size(width, height, scaleFactor);
    } else {
      nve::fabric_host_set_surface_size(host, width, height, scaleFactor);
    }
    [self _syncNativeViewsFromHostWithMutations:NO];
  }
}

- (BOOL)getRootLayoutSize:(CGSize *)outSize {
  NveFabricHostHandle host = [self _activeFabricHost];
  if (!host || !outSize) return NO;
  nve::LayoutRect rootRect{};
  if (nve::fabric_host_get_node_layout(host, NVE_ROOT_NODE_ID, &rootRect) &&
      rootRect.width > 0.0f && rootRect.height > 0.0f) {
    *outSize = CGSizeMake(rootRect.width, rootRect.height);
    return YES;
  }
  return NO;
}

- (nullable NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId {
  if (nodeId == NVE_ROOT_NODE_ID) {
    return [_adapter nveHostRootView];
  }
  const auto it = _nodeViews.find(nodeId);
  return (it != _nodeViews.end()) ? it->second : nil;
}

- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view {
  for (const auto &[nid, v] : _nodeViews) {
    if (v == view) {
      return nid;
    }
  }
  return 0;
}

- (void)_destroyManagedViewForNodeId:(uint32_t)nodeId
                                view:(NvePlatformView *)view
                             adapter:(id<NveAppleComponentViewAdapter> _Nullable)adapter
                      hasDestroyHook:(BOOL)hasDestroyHook {
  if (hasDestroyHook && adapter != nil && view != nil) {
    [adapter nveWillDestroyNativeView:view forNodeId:nodeId];
  }
  if (view != nil) {
    _managedViewPtrs.erase((__bridge const void *)view);
    [view removeFromSuperview];
  }
  _nodeViewTags.erase(nodeId);
  _scrollViewNodeIds.erase(nodeId);
  _scrollViewChildren.erase(nodeId);
}

- (void)clearNativeNodeViews {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  const BOOL hasDestroyHook =
      adapter != nil && [adapter respondsToSelector:@selector(nveWillDestroyNativeView:forNodeId:)];
  for (auto &[nid, view] : _nodeViews) {
    if (hasDestroyHook) {
      [adapter nveWillDestroyNativeView:view forNodeId:nid];
    }
    [view removeFromSuperview];
  }
  _nodeViews.clear();
  _nodeViewTags.clear();
  _managedViewPtrs.clear();
  _rawPropsCache.clear();
  _scrollViewNodeIds.clear();
  _scrollViewChildren.clear();
  _sceneInitialized = false;
  _lastSceneFrameId = 0;
}

- (uint32_t)_viewTagForNode:(const NveRenderNode &)node
                  semantics:(const nve::apple::NodeSemantics &)semantics {
  // Distinguish leaf text nodes (UILabel / NSTextField) from wrapper text containers,
  // and secure text fields (NSSecureTextField) from plain text fields.
  const uint32_t isContainerText =
      (node.kind == NVE_NODE_KIND_TEXT && (node.flags & NVE_NODE_LEAF_TEXT) == 0u) ? 0x10000u : 0u;
  const uint32_t isSecureInput =
      (node.kind == NVE_NODE_KIND_TEXT_INPUT && semantics.secure_text_entry) ? 0x20000u : 0u;
  return static_cast<uint32_t>(node.kind) | isContainerText | isSecureInput;
}

- (void)_refreshRawPropsCacheFromHost:(NveFabricHostHandle)host
                    collectChangedIds:(std::vector<uint32_t> * _Nullable)outChanged {
  const auto snapshots = nve::fabric_host_snapshot_nodes(host);
  _scratchLiveIds.clear();
  for (const auto &snap : snapshots) {
    if (snap.raw_props.empty()) {
      auto oldIt = _rawPropsCache.find(snap.node_id);
      if (oldIt != _rawPropsCache.end()) {
        _rawPropsCache.erase(oldIt);
        if (outChanged != nullptr) {
          outChanged->push_back(snap.node_id);
        }
      }
      continue;
    }
    _scratchLiveIds.insert(snap.node_id);
    auto oldIt = _rawPropsCache.find(snap.node_id);
    if (oldIt == _rawPropsCache.end() || oldIt->second != snap.raw_props) {
      _rawPropsCache[snap.node_id] = snap.raw_props;
      if (outChanged != nullptr) {
        outChanged->push_back(snap.node_id);
      }
    }
  }
  for (auto it = _rawPropsCache.begin(); it != _rawPropsCache.end();) {
    if (_scratchLiveIds.find(it->first) == _scratchLiveIds.end()) {
      it = _rawPropsCache.erase(it);
    } else {
      ++it;
    }
  }
}

- (void)_upsertNode:(const NveRenderNode &)node
          sceneView:(const NveSceneView &)view
            adapter:(id<NveAppleComponentViewAdapter>)adapter
     hasDestroyHook:(BOOL)hasDestroyHook {
  const std::string_view text = nve::scene_node_text(view, node);
  const auto rpIt = _rawPropsCache.find(node.node_id);
  const nve::RawPropMap *rawPropsPtr = (rpIt != _rawPropsCache.end()) ? &rpIt->second : nullptr;
  const nve::apple::NodeSemantics sem =
      nve::apple::extract_node_semantics(node, text, rawPropsPtr);

  if (node.node_id == NVE_ROOT_NODE_ID) {
    [adapter nveApplyRootBackgroundRgba:node.bg_rgba];
    nve::apple::apply_node_accessibility([adapter nveHostRootView], node, sem);
    return;
  }

  NvePlatformView *nativeView = nil;
  const uint32_t viewTag = [self _viewTagForNode:node semantics:sem];
  const auto existingIt = _nodeViews.find(node.node_id);
  const auto tagIt = _nodeViewTags.find(node.node_id);
  if (existingIt != _nodeViews.end() && tagIt != _nodeViewTags.end() && tagIt->second == viewTag) {
    nativeView = existingIt->second;
  } else {
    if (existingIt != _nodeViews.end()) {
      [self _destroyManagedViewForNodeId:node.node_id
                                    view:existingIt->second
                                 adapter:adapter
                          hasDestroyHook:hasDestroyHook];
      _nodeViews.erase(existingIt);
    }
    nativeView = [adapter nveCreateNativeViewForNode:node semantics:sem];
    _nodeViews[node.node_id] = nativeView;
    _nodeViewTags[node.node_id] = viewTag;
    _managedViewPtrs.insert((__bridge const void *)nativeView);
    if (node.kind == NVE_NODE_KIND_SCROLL_VIEW) {
      _scrollViewNodeIds.insert(node.node_id);
    }
  }

  const CGFloat fx = std::isfinite(node.x) ? static_cast<CGFloat>(node.x) : 0.0;
  const CGFloat fy = std::isfinite(node.y) ? static_cast<CGFloat>(node.y) : 0.0;
  const CGFloat fw =
      (std::isfinite(node.width) && node.width > 0.0f) ? static_cast<CGFloat>(node.width) : 0.0;
  const CGFloat fh =
      (std::isfinite(node.height) && node.height > 0.0f) ? static_cast<CGFloat>(node.height) : 0.0;
  nativeView.frame = CGRectMake(fx, fy, fw, fh);

  const BOOL interactive =
      !sem.disabled &&
      node.kind != NVE_NODE_KIND_TEXT_INPUT &&
      node.kind != NVE_NODE_KIND_SCROLL_VIEW &&
      (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u);
  [adapter nveUpdateNativeView:nativeView
                       forNode:node
                          text:text
                     semantics:sem
                   interactive:interactive];
  if (node.kind == NVE_NODE_KIND_TEXT_INPUT) {
    nve::apple::apply_text_input_props(nativeView, text, sem);
  }
  nve::apple::apply_node_accessibility(nativeView, node, sem);
  nve::apple::apply_common_layer_style(nativeView.layer, node);
}

- (void)_reconcileChildrenForNode:(const NveRenderNode &)node
                        sceneView:(const NveSceneView &)view
                         rootView:(NvePlatformView *)rootView
                          adapter:(id<NveAppleComponentViewAdapter>)adapter
               hasCustomContainer:(BOOL)hasCustomContainer
              trackMountedChildren:(BOOL)trackMounted {
  NvePlatformView *rawParent = (node.node_id == NVE_ROOT_NODE_ID) ? rootView : nil;
  if (!rawParent) {
    const auto pit = _nodeViews.find(node.node_id);
    if (pit != _nodeViews.end()) {
      rawParent = pit->second;
    }
  }
  if (!rawParent) return;

  NvePlatformView *parentView =
      hasCustomContainer ? [adapter nveChildContainerForParentView:rawParent] : rawParent;
  if (!parentView) return;

  const std::span<const uint32_t> childIds = nve::scene_node_children(view, node);
  if (node.kind == NVE_NODE_KIND_SCROLL_VIEW) {
    auto &cachedKids = _scrollViewChildren[node.node_id];
    cachedKids.assign(childIds.begin(), childIds.end());
  }

  _scratchDesiredChildren.clear();
  _scratchDesiredPtrs.clear();
  for (uint32_t childId : childIds) {
    const auto cit = _nodeViews.find(childId);
    if (cit == _nodeViews.end()) continue;
    NvePlatformView *childView = cit->second;
    _scratchDesiredChildren.push_back(childView);
    _scratchDesiredPtrs.insert((__bridge const void *)childView);
    if (trackMounted) {
      _scratchMountedChildIds.insert(childId);
    }
  }

  // Remove any managed subview that is no longer a child of this parent.
  NSArray<NvePlatformView *> *existingSubviews = [parentView.subviews copy];
  for (NvePlatformView *sub in existingSubviews) {
    const void *ptr = (__bridge const void *)sub;
    if (_managedViewPtrs.find(ptr) != _managedViewPtrs.end() &&
        _scratchDesiredPtrs.find(ptr) == _scratchDesiredPtrs.end()) {
      [sub removeFromSuperview];
    }
  }

  // Check whether existing managed subview order already matches `_scratchDesiredChildren`.
  _scratchCurrentOrder.clear();
  for (NvePlatformView *sub in parentView.subviews) {
    const void *ptr = (__bridge const void *)sub;
    if (_managedViewPtrs.find(ptr) != _managedViewPtrs.end()) {
      _scratchCurrentOrder.push_back(ptr);
    }
  }

  bool orderMatches = (_scratchCurrentOrder.size() == _scratchDesiredChildren.size());
  if (orderMatches) {
    for (size_t i = 0; i < _scratchDesiredChildren.size(); ++i) {
      if (_scratchCurrentOrder[i] != (__bridge const void *)_scratchDesiredChildren[i]) {
        orderMatches = false;
        break;
      }
    }
  }

  if (!orderMatches) {
#if TARGET_OS_OSX
    NvePlatformView *prevView = nil;
    for (NvePlatformView *childView : _scratchDesiredChildren) {
      if (childView.superview != parentView) {
        if (prevView != nil) {
          [parentView addSubview:childView positioned:NSWindowAbove relativeTo:prevView];
        } else {
          [parentView addSubview:childView];
        }
      } else {
        // Re-insert in exact order relative to previous managed sibling.
        [childView removeFromSuperviewWithoutNeedingDisplay];
        if (prevView != nil) {
          [parentView addSubview:childView positioned:NSWindowAbove relativeTo:prevView];
        } else {
          [parentView addSubview:childView];
        }
      }
      prevView = childView;
    }
#else
    for (NvePlatformView *childView : _scratchDesiredChildren) {
      if (childView.superview != parentView) {
        [parentView addSubview:childView];
      } else {
        [parentView bringSubviewToFront:childView];
      }
    }
#endif
  }
}

- (void)_refreshScrollViewContentSizesWithAdapter:(id<NveAppleComponentViewAdapter>)adapter {
  if (_scrollViewNodeIds.empty() ||
      ![adapter respondsToSelector:@selector(nveUpdateScrollView:contentSize:)]) {
    return;
  }
  for (uint32_t scrollId : _scrollViewNodeIds) {
    const auto svIt = _nodeViews.find(scrollId);
    if (svIt == _nodeViews.end()) continue;
    NvePlatformView *scrollView = svIt->second;
    CGFloat contentMaxX = scrollView.frame.size.width;
    CGFloat contentMaxY = scrollView.frame.size.height;
    const auto kidsIt = _scrollViewChildren.find(scrollId);
    if (kidsIt != _scrollViewChildren.end()) {
      for (uint32_t childId : kidsIt->second) {
        const auto cit = _nodeViews.find(childId);
        if (cit == _nodeViews.end()) continue;
        const CGRect cf = cit->second.frame;
        contentMaxX = std::max(contentMaxX, CGRectGetMaxX(cf));
        contentMaxY = std::max(contentMaxY, CGRectGetMaxY(cf));
      }
    }
    [adapter nveUpdateScrollView:scrollView contentSize:CGSizeMake(contentMaxX, contentMaxY)];
  }
}

- (void)syncNativeViewsFromHost {
  [self _syncNativeViewsFromHostWithMutations:NO];
}

- (void)_syncNativeViewsFromHostWithMutations:(BOOL)hasMutations {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  NveFabricHostHandle host = [self _activeFabricHost];
  if (!host || !adapter) return;

  const NveSceneView view = nve::fabric_host_scene_view(host);
  if (view.schema_hash != NVE_SCENE_SCHEMA_HASH) return;
  if ((view.node_count > 0u && view.nodes == nullptr) ||
      (view.dirty_count > 0u && view.dirty_indices == nullptr) ||
      (view.removed_count > 0u && view.removed_ids == nullptr)) {
    return;
  }

  const BOOL hasDestroyHook =
      [adapter respondsToSelector:@selector(nveWillDestroyNativeView:forNodeId:)];
  const BOOL hasCustomContainer =
      [adapter respondsToSelector:@selector(nveChildContainerForParentView:)];
  NvePlatformView *rootView = [adapter nveHostRootView];

  const bool fullSync = (!_sceneInitialized || view.frame_id < _lastSceneFrameId);
  _lastReconciledDirtyCount = 0u;
  _lastReconciledRemovedCount = 0u;

  if (fullSync) {
    [self _refreshRawPropsCacheFromHost:host collectChangedIds:nullptr];

    // Pass 1: Prune views not present in the scene.
    _scratchLiveIds.clear();
    for (uint32_t i = 0u; i < view.node_count; ++i) {
      _scratchLiveIds.insert(view.nodes[i].node_id);
    }
    for (auto it = _nodeViews.begin(); it != _nodeViews.end();) {
      if (_scratchLiveIds.find(it->first) == _scratchLiveIds.end()) {
        [self _destroyManagedViewForNodeId:it->first
                                      view:it->second
                                   adapter:adapter
                            hasDestroyHook:hasDestroyHook];
        it = _nodeViews.erase(it);
        ++_lastReconciledRemovedCount;
      } else {
        ++it;
      }
    }

    // Pass 2: Create/update all scene nodes.
    for (uint32_t i = 0u; i < view.node_count; ++i) {
      [self _upsertNode:view.nodes[i]
              sceneView:view
                adapter:adapter
         hasDestroyHook:hasDestroyHook];
      ++_lastReconciledDirtyCount;
    }

    // Pass 3: Reconcile children & detach any unreferenced live views.
    _scratchMountedChildIds.clear();
    for (uint32_t i = 0u; i < view.node_count; ++i) {
      [self _reconcileChildrenForNode:view.nodes[i]
                            sceneView:view
                             rootView:rootView
                              adapter:adapter
                   hasCustomContainer:hasCustomContainer
                 trackMountedChildren:YES];
    }
    for (const auto &[nid, v] : _nodeViews) {
      if (_scratchMountedChildIds.find(nid) == _scratchMountedChildIds.end() &&
          v.superview != nil) {
        [v removeFromSuperview];
      }
    }

    [self _refreshScrollViewContentSizesWithAdapter:adapter];
    _sceneInitialized = true;
    _lastSceneFrameId = view.frame_id;
    return;
  }

  if (view.dirty_count == 0u && view.removed_count == 0u && !hasMutations) {
    return;
  }

  // Incremental O(removed + dirty) steady-state reconciliation.
  // Pass 1: O(removed) — destroy explicitly removed nodes.
  for (uint32_t i = 0u; i < view.removed_count; ++i) {
    const uint32_t rid = view.removed_ids[i];
    _rawPropsCache.erase(rid);
    const auto it = _nodeViews.find(rid);
    if (it != _nodeViews.end()) {
      [self _destroyManagedViewForNodeId:rid
                                    view:it->second
                                 adapter:adapter
                          hasDestroyHook:hasDestroyHook];
      _nodeViews.erase(it);
      ++_lastReconciledRemovedCount;
    }
  }

  // Only read `raw_props` when a dirty node is newly created, is an Image/TextInput, already carries
  // raw props, or mutations ran with zero scene-table changes (pure SET_RAW_PROP frame).
  bool needRawPropsRefresh = (hasMutations && view.dirty_count == 0u && view.removed_count == 0u);
  if (!needRawPropsRefresh) {
    for (uint32_t i = 0u; i < view.dirty_count; ++i) {
      const uint32_t idx = view.dirty_indices[i];
      if (idx >= view.node_count) continue;
      const NveRenderNode &node = view.nodes[idx];
      if (node.kind == NVE_NODE_KIND_IMAGE || node.kind == NVE_NODE_KIND_TEXT_INPUT ||
          (node.node_id != NVE_ROOT_NODE_ID && _nodeViews.find(node.node_id) == _nodeViews.end()) ||
          _rawPropsCache.find(node.node_id) != _rawPropsCache.end()) {
        needRawPropsRefresh = true;
        break;
      }
    }
  }

  _scratchExtraDirtyIds.clear();
  if (needRawPropsRefresh) {
    [self _refreshRawPropsCacheFromHost:host collectChangedIds:&_scratchExtraDirtyIds];
  }

  // Pass 2: O(dirty) — create/update only dirty nodes directly from `NveSceneView`.
  _scratchLiveIds.clear();
  for (uint32_t i = 0u; i < view.dirty_count; ++i) {
    const uint32_t idx = view.dirty_indices[i];
    if (idx >= view.node_count) continue;
    const NveRenderNode &node = view.nodes[idx];
    _scratchLiveIds.insert(node.node_id);
    [self _upsertNode:node sceneView:view adapter:adapter hasDestroyHook:hasDestroyHook];
    ++_lastReconciledDirtyCount;
  }

  // Apply any nodes whose `raw_props` changed without mutating the 64-byte `NveRenderNode`.
  if (!_scratchExtraDirtyIds.empty()) {
    for (uint32_t nid : _scratchExtraDirtyIds) {
      if (_scratchLiveIds.find(nid) != _scratchLiveIds.end()) continue;
      for (uint32_t i = 0u; i < view.node_count; ++i) {
        if (view.nodes[i].node_id == nid) {
          [self _upsertNode:view.nodes[i]
                  sceneView:view
                    adapter:adapter
             hasDestroyHook:hasDestroyHook];
          ++_lastReconciledDirtyCount;
          break;
        }
      }
    }
  }

  // Pass 3: O(dirty) — reconcile child order only for dirty containers.
  for (uint32_t i = 0u; i < view.dirty_count; ++i) {
    const uint32_t idx = view.dirty_indices[i];
    if (idx >= view.node_count) continue;
    [self _reconcileChildrenForNode:view.nodes[idx]
                          sceneView:view
                           rootView:rootView
                            adapter:adapter
                 hasCustomContainer:hasCustomContainer
               trackMountedChildren:NO];
  }

  if (view.dirty_count > 0u || view.removed_count > 0u) {
    [self _refreshScrollViewContentSizesWithAdapter:adapter];
  }
  _lastSceneFrameId = view.frame_id;
}

@end
