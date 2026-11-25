#import "NveAppleComponentHost.h"

<<<<<<< HEAD
#import <CoreFoundation/CoreFoundation.h>

#include "nve_value.hpp"

=======
#include <algorithm>
>>>>>>> 74aec62 (fix(hosts,fabric-bridge,headless): harden subview reconciliation, scroll/input/hit-test, atomic Android reload, EventRing decoders, and headless tree guards)
#include <cmath>
#include <memory>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace {

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
  void *_dylibHandle;
  uint32_t _surfaceId;
  std::unordered_map<uint32_t, NvePlatformView *> _nodeViews;
  std::unordered_map<uint32_t, uint32_t> _nodeViewTags;
  nve::BumpArena _propsArena;
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter
                    errorDomain:(NSString *)errorDomain {
  self = [super init];
  if (self) {
    _adapter = adapter;
    _errorDomain = [errorDomain copy];
    _dylibHandle = nullptr;
    _surfaceId = 1;
  }
  return self;
}

- (void)dealloc {
  // Destroy the bridge (which calls `surface->destroy`) before closing the dylib handle.
  _bridge.reset();
  nve::apple::close_dylib(_dylibHandle);
}

- (uint32_t)surfaceId {
  return _surfaceId;
}

- (uint32_t)nodeCount {
  return _bridge ? _bridge->node_count() : 0;
}

- (BOOL)_installModule:(NveModule)mod
           dylibHandle:(void *)dylibHandle
             surfaceId:(uint32_t)surfaceId
         restoredState:(const std::vector<uint8_t> &)savedState {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (!adapter || !mod.surface || !mod.surface->create) {
    nve::apple::close_dylib(dylibHandle);
    return NO;
  }

  _bridge.reset();
  nve::apple::close_dylib(_dylibHandle);
  [self clearNativeNodeViews];

  _dylibHandle = dylibHandle;
  _surfaceId = surfaceId;

  __weak NveAppleComponentHost *weakSelf = self;
  auto bridge = std::make_unique<nve::NveRnComponentBridge>(
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
  if (!bridge->is_valid()) {
    bridge.reset();
    nve::apple::close_dylib(_dylibHandle);
    return NO;
  }

  nve::apple::install_native_text_measurer(bridge->fabric_host());
  _bridge = std::move(bridge);
  if (!savedState.empty()) {
    _bridge->restore_state(
        NveBufferSlice{savedState.data(), savedState.size()}, /*auto_render=*/false);
  }
  [self renderFrame];
  return YES;
}

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface surfaceId:(uint32_t)surfaceId {
  const NveModule mod = nve_resolve_module(getInterface);
  return [self _installModule:mod dylibHandle:nullptr surfaceId:surfaceId restoredState:{}];
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
                restoredState:{}];
}

- (BOOL)reloadDylibAtPath:(NSString *)dylibPath
                    error:(NSError * _Nullable * _Nullable)error {
  // Validate the replacement dylib before tearing down the existing bridge so a failed reload
  // preserves the currently mounted component and its state.
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
                restoredState:saved];
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
    [self syncNativeViewsFromHost];
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

- (void)updateSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor {
  if (!_bridge) return;
  if (width > 0.0f && height > 0.0f) {
    _bridge->set_surface_size(width, height, scaleFactor);
    [self syncNativeViewsFromHost];
  }
}

- (BOOL)getRootLayoutSize:(CGSize *)outSize {
  if (!_bridge || !outSize) return NO;
  const auto rootRect = _bridge->node_layout(NVE_ROOT_NODE_ID);
  if (rootRect.has_value() && rootRect->width > 0.0f && rootRect->height > 0.0f) {
    *outSize = CGSizeMake(rootRect->width, rootRect->height);
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

- (void)clearNativeNodeViews {
  for (auto &[_, view] : _nodeViews) {
    [view removeFromSuperview];
  }
  _nodeViews.clear();
  _nodeViewTags.clear();
}

- (uint32_t)_viewTagForSnapshot:(const nve::RenderedNodeSnapshot &)snap {
  // Distinguish leaf text nodes (UILabel / NSTextField) from wrapper text containers.
  const uint32_t isContainerText =
      (snap.kind == NVE_COMP_TEXT && !snap.children.empty()) ? 0x10000u : 0u;
  return static_cast<uint32_t>(snap.kind) | isContainerText;
}

- (void)syncNativeViewsFromHost {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (!_bridge || !adapter) return;
  const auto snapshots = nve::fabric_host_snapshot_nodes(_bridge->fabric_host());

  // Pass 1: Prune removed nodes.
  std::unordered_set<uint32_t> liveIds;
  liveIds.reserve(snapshots.size());
  for (const auto &snap : snapshots) {
    liveIds.insert(snap.node_id);
  }
  for (auto it = _nodeViews.begin(); it != _nodeViews.end();) {
    if (liveIds.find(it->first) == liveIds.end()) {
      [it->second removeFromSuperview];
      _nodeViewTags.erase(it->first);
      it = _nodeViews.erase(it);
    } else {
      ++it;
    }
  }

  // Pass 2: Create or update mounted native views and apply layout + style properties.
  for (const auto &snap : snapshots) {
    if (snap.node_id == NVE_ROOT_NODE_ID) {
      [adapter nveApplyRootBackgroundRgba:snap.background_rgba];
      continue;
    }

    NvePlatformView *view = nil;
    const uint32_t viewTag = [self _viewTagForSnapshot:snap];
    const auto existingIt = _nodeViews.find(snap.node_id);
    if (existingIt != _nodeViews.end() && _nodeViewTags[snap.node_id] == viewTag) {
      view = existingIt->second;
    } else {
      if (existingIt != _nodeViews.end()) {
        [existingIt->second removeFromSuperview];
      }
      view = [adapter nveCreateNativeViewForSnapshot:snap];
      _nodeViews[snap.node_id] = view;
      _nodeViewTags[snap.node_id] = viewTag;
    }

    const CGFloat fx = std::isfinite(snap.layout.x) ? static_cast<CGFloat>(snap.layout.x) : 0.0;
    const CGFloat fy = std::isfinite(snap.layout.y) ? static_cast<CGFloat>(snap.layout.y) : 0.0;
    const CGFloat fw =
        (std::isfinite(snap.layout.width) && snap.layout.width > 0.0f)
            ? static_cast<CGFloat>(snap.layout.width)
            : 0.0;
    const CGFloat fh =
        (std::isfinite(snap.layout.height) && snap.layout.height > 0.0f)
            ? static_cast<CGFloat>(snap.layout.height)
            : 0.0;
    view.frame = CGRectMake(fx, fy, fw, fh);
    const BOOL interactive =
        (snap.kind == NVE_COMP_PRESSABLE || (snap.event_mask & nve::apple::kEventMaskPress) != 0);
    [adapter nveUpdateNativeView:view forSnapshot:snap interactive:interactive];
    nve::apple::apply_common_layer_style(view.layer, snap);
  }

  // Pass 3: Reconcile parent-child view hierarchy, exact child order, detached orphans, and ScrollView content sizes.
  const BOOL hasCustomContainer =
      [adapter respondsToSelector:@selector(nveChildContainerForParentView:)];
  const BOOL hasScrollSizeHook =
      [adapter respondsToSelector:@selector(nveUpdateScrollView:contentSize:)];
  NvePlatformView *rootView = [adapter nveHostRootView];

  std::unordered_map<uint32_t, const nve::RenderedNodeSnapshot *> snapById;
  snapById.reserve(snapshots.size());
  std::unordered_set<const void *> managedViewPtrs;
  managedViewPtrs.reserve(_nodeViews.size());
  for (const auto &snap : snapshots) {
    snapById.emplace(snap.node_id, &snap);
  }
  for (const auto &[_, v] : _nodeViews) {
    managedViewPtrs.insert((__bridge const void *)v);
  }

  std::unordered_set<uint32_t> mountedChildIds;
  mountedChildIds.reserve(_nodeViews.size());

  for (const auto &snap : snapshots) {
    NvePlatformView *rawParent = (snap.node_id == NVE_ROOT_NODE_ID) ? rootView : nil;
    if (!rawParent) {
      const auto pit = _nodeViews.find(snap.node_id);
      if (pit != _nodeViews.end()) {
        rawParent = pit->second;
      }
    }
    if (!rawParent) continue;

    NvePlatformView *parentView =
        hasCustomContainer ? [adapter nveChildContainerForParentView:rawParent] : rawParent;
    if (!parentView) continue;

    std::vector<NvePlatformView *> desiredChildren;
    desiredChildren.reserve(snap.children.size());
    std::unordered_set<const void *> desiredPtrs;
    desiredPtrs.reserve(snap.children.size());
    CGFloat contentMaxX = 0.0;
    CGFloat contentMaxY = 0.0;

    for (uint32_t childId : snap.children) {
      const auto cit = _nodeViews.find(childId);
      if (cit == _nodeViews.end()) continue;
      NvePlatformView *childView = cit->second;
      desiredChildren.push_back(childView);
      desiredPtrs.insert((__bridge const void *)childView);
      mountedChildIds.insert(childId);

      const auto sit = snapById.find(childId);
      if (sit != snapById.end()) {
        const auto &cl = sit->second->layout;
        contentMaxX = std::max(contentMaxX, static_cast<CGFloat>(cl.x + cl.width));
        contentMaxY = std::max(contentMaxY, static_cast<CGFloat>(cl.y + cl.height));
      }
    }

    // Remove any managed subview that is no longer a child of this parent.
    NSArray<NvePlatformView *> *existingSubviews = [parentView.subviews copy];
    for (NvePlatformView *sub in existingSubviews) {
      const void *ptr = (__bridge const void *)sub;
      if (managedViewPtrs.find(ptr) != managedViewPtrs.end() &&
          desiredPtrs.find(ptr) == desiredPtrs.end()) {
        [sub removeFromSuperview];
      }
    }

    // Check whether existing managed subview order already matches `desiredChildren`.
    std::vector<const void *> currentManagedOrder;
    currentManagedOrder.reserve(parentView.subviews.count);
    for (NvePlatformView *sub in parentView.subviews) {
      const void *ptr = (__bridge const void *)sub;
      if (managedViewPtrs.find(ptr) != managedViewPtrs.end()) {
        currentManagedOrder.push_back(ptr);
      }
    }
    bool orderMatches = (currentManagedOrder.size() == desiredChildren.size());
    if (orderMatches) {
      for (size_t i = 0; i < desiredChildren.size(); ++i) {
        if (currentManagedOrder[i] != (__bridge const void *)desiredChildren[i]) {
          orderMatches = false;
          break;
        }
      }
    }

    if (!orderMatches) {
#if TARGET_OS_OSX
      NvePlatformView *prevView = nil;
      for (NvePlatformView *childView : desiredChildren) {
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
      for (NvePlatformView *childView : desiredChildren) {
        if (childView.superview != parentView) {
          [parentView addSubview:childView];
        } else {
          [parentView bringSubviewToFront:childView];
        }
      }
#endif
    }

    if (snap.kind == NVE_COMP_SCROLL_VIEW && hasScrollSizeHook) {
      const CGSize contentSize = CGSizeMake(
          std::max(static_cast<CGFloat>(snap.layout.width), contentMaxX),
          std::max(static_cast<CGFloat>(snap.layout.height), contentMaxY));
      [adapter nveUpdateScrollView:rawParent contentSize:contentSize];
    }
  }

  // Detach any live views that are no longer referenced by any parent's children list.
  for (const auto &[nid, view] : _nodeViews) {
    if (mountedChildIds.find(nid) == mountedChildIds.end() && view.superview != nil) {
      [view removeFromSuperview];
    }
  }
}

@end
