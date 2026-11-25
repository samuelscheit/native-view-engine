#import "NveAppleSceneReconciler.h"

#include "nve_host_runtime.hpp"

#include <algorithm>
#include <cmath>
#include <span>
#include <unordered_map>
#include <unordered_set>
#include <vector>

@implementation NveAppleSceneReconciler {
  __weak id<NveAppleComponentViewAdapter> _adapter;

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
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter {
  self = [super init];
  if (self) {
    _adapter = adapter;
    _sceneInitialized = false;
    _lastSceneFrameId = 0;
    _lastReconciledDirtyCount = 0;
    _lastReconciledRemovedCount = 0;
  }
  return self;
}

- (uint32_t)lastReconciledDirtyCount {
  return _lastReconciledDirtyCount;
}

- (uint32_t)lastReconciledRemovedCount {
  return _lastReconciledRemovedCount;
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

- (void)syncFromFabricHost:(NveFabricHostHandle _Nullable)host hasMutations:(BOOL)hasMutations {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
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
  // `NVE_NODE_KIND_SKIA_CANVAS` draw-list updates flow directly through `NveSceneView::text_slab`
  // with zero `raw_props` snapshot overhead.
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
