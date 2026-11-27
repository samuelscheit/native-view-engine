#import "NveAppleSceneReconciler.h"

#include <nve/generated/nve_tables.generated.h>

#include <algorithm>
#include <unordered_map>
#include <unordered_set>
#include <vector>

using nve::apple::NodeKind;
using nve::apple::NodeView;
using nve::apple::SceneTables;

namespace {

struct Mounted {
  NvePlatformView *view = nil;
  NodeKind kind = NodeKind::Container;
  bool secure = false;
  CGRect frame = CGRectZero;  // absolute
  /// Child views in the order last applied to the container (reorder only when it changes).
  std::vector<const void *> order;
};

/// Decodes `nve.res.image_src` (tag 0: URL, tag 1: encoded bytes) into a platform image.
id image_from_source(NSData *bytes) {
  if (bytes.length < 5u) return nil;
  const uint8_t *p = static_cast<const uint8_t *>(bytes.bytes);
  const uint32_t len = uint32_t{p[1]} | (uint32_t{p[2]} << 8u) | (uint32_t{p[3]} << 16u) | (uint32_t{p[4]} << 24u);
  if (static_cast<NSUInteger>(len) + 5u != bytes.length) return nil;
  if (p[0] == 0u) {
    return nve::apple::shared_image_cache().image_for_source(
        std::string_view(reinterpret_cast<const char *>(p + 5), len));
  }
  if (p[0] == 1u) {
    return [[NvePlatformImage alloc] initWithData:[bytes subdataWithRange:NSMakeRange(5, len)]];
  }
  return nil;
}

}  // namespace

@implementation NveAppleSceneReconciler {
  __weak id<NveAppleComponentViewAdapter> _adapter;
  NveAppleResourceProvider _resources;
  std::unordered_map<uint32_t, Mounted> _views;
  std::unordered_map<uint64_t, std::vector<int32_t>> _sparse;
  NSMutableDictionary<NSNumber *, id> *_images;
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter resources:(NveAppleResourceProvider)resources {
  if ((self = [super init])) {
    _adapter = adapter;
    _resources = [resources copy];
    _images = [NSMutableDictionary dictionary];
  }
  return self;
}

- (NSUInteger)mountedViewCount {
  return _views.size();
}

- (NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId {
  const auto it = _views.find(nodeId);
  return it == _views.end() ? nil : it->second.view;
}

- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view {
  for (const auto &[id, m] : _views) {
    if (m.view == view) return id;
  }
  return 0u;
}

- (void)_destroy:(uint32_t)nodeId {
  const auto it = _views.find(nodeId);
  if (it == _views.end()) return;
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if ([adapter respondsToSelector:@selector(nveWillDestroyNativeView:forNodeId:)]) {
    [adapter nveWillDestroyNativeView:it->second.view forNodeId:nodeId];
  }
  [it->second.view removeFromSuperview];
  _views.erase(it);
}

- (void)clearNativeNodeViews {
  std::vector<uint32_t> ids;
  ids.reserve(_views.size());
  for (const auto &[id, m] : _views) ids.push_back(id);
  for (const uint32_t id : ids) [self _destroy:id];
  [_images removeAllObjects];
}

- (id)_imageForResource:(uint32_t)resourceId {
  if (resourceId == 0u) return nil;
  id cached = _images[@(resourceId)];
  if (cached != nil) return cached;
  id image = _resources ? image_from_source(_resources(resourceId)) : nil;
  if (image != nil) _images[@(resourceId)] = image;  // failures retry: the payload may still arrive
  return image;
}

/// Creates or updates the view of `index`; returns its mount record.
- (Mounted *)_realize:(const SceneTables &)tables index:(uint32_t)index {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  const NodeView node = tables.describe(index);
  auto it = _views.find(node.node_id);
  if (it != _views.end() && (it->second.kind != node.kind || it->second.secure != node.semantics.secure_text_entry)) {
    [self _destroy:node.node_id];
    it = _views.end();
  }
  if (it == _views.end()) {
    NvePlatformView *view = [adapter nveCreateNativeViewForNode:node];
    if (view == nil) return nullptr;
    it = _views.emplace(node.node_id, Mounted{view, node.kind, node.semantics.secure_text_entry, CGRectZero, {}}).first;
  }
  [adapter nveUpdateNativeView:it->second.view forNode:node image:[self _imageForResource:node.image_resource]];
  return &it->second;
}

- (NvePlatformView *)_containerFor:(NvePlatformView *)view {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if ([adapter respondsToSelector:@selector(nveChildContainerForParentView:)]) {
    return [adapter nveChildContainerForParentView:view] ?: view;
  }
  return view;
}

/// Walks the live tree: frames (parent-relative), subview order, scroll content sizes.
- (void)_layout:(const SceneTables &)tables index:(uint32_t)index parent:(const Mounted *)parent into:(NvePlatformView *)container {
  const uint32_t node_id = tables.node_id_at(index);
  auto it = _views.find(node_id);
  if (it == _views.end()) {
    if ([self _realize:tables index:index] == nullptr) return;
    it = _views.find(node_id);
  }
  Mounted &m = it->second;
  const NveLayoutRect *r = tables.scene().rect(index);
  m.frame = r != nullptr ? CGRectMake(r->x, r->y, r->width, r->height) : CGRectZero;
  CGRect local = m.frame;
  if (parent != nullptr) local = CGRectOffset(local, -parent->frame.origin.x, -parent->frame.origin.y);
  if (!CGRectEqualToRect(m.view.frame, local)) m.view.frame = local;
  if (m.view.superview != container) [container addSubview:m.view];

  const std::vector<uint32_t> kids = tables.paint_ordered_children(index);
  NvePlatformView *childContainer = [self _containerFor:m.view];
  CGSize content = m.frame.size;
  std::vector<const void *> order;
  order.reserve(kids.size());
  for (const uint32_t c : kids) {
    [self _layout:tables index:c parent:&m into:childContainer];
    const auto kid = _views.find(tables.node_id_at(c));
    if (kid == _views.end()) continue;
    NvePlatformView *kv = kid->second.view;
    order.push_back((__bridge const void *)kv);
    content.width = std::max(content.width, CGRectGetMaxX(kv.frame));
    content.height = std::max(content.height, CGRectGetMaxY(kv.frame));
  }
  // Child `_layout`s only insert / replace *other* entries; `unordered_map` references survive rehash.
  if (order != m.order) {
    NvePlatformView *previous = nil;
    for (const void *p : order) {
      NvePlatformView *kv = (__bridge NvePlatformView *)p;
#if TARGET_OS_OSX
      [childContainer addSubview:kv positioned:NSWindowAbove relativeTo:previous];
#else
      if (previous == nil) [childContainer sendSubviewToBack:kv]; else [childContainer insertSubview:kv aboveSubview:previous];
#endif
      previous = kv;
    }
    m.order = std::move(order);
  }
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (m.kind == NodeKind::Scroll && [adapter respondsToSelector:@selector(nveUpdateScrollView:contentSize:)]) {
    [adapter nveUpdateScrollView:m.view contentSize:content];
  }
}

- (void)syncFromScene:(const nve::host::RuntimeSceneReader &)scene {
  id<NveAppleComponentViewAdapter> adapter = _adapter;
  if (adapter == nil) return;
  const SceneTables tables(scene, _sparse);
  const auto image = scene.find_table(nve::host::Key64{NVE_TABLE_IMAGE_KEY});
  if ((image && !image->dirty_rows().empty()) || !scene.removed_ids().empty()) [_images removeAllObjects];

  for (const uint32_t raw : scene.removed_ids()) [self _destroy:raw];
  const bool initial = _views.empty();
  if (!initial) {
    // Inherited typography: a dirty ancestor that sets it re-describes its whole subtree.
    std::vector<bool> seen(scene.node_capacity(), false);
    std::vector<uint32_t> cascade;
    for (const uint32_t index : scene.dirty_indices()) {
      if (index >= seen.size() || seen[index] || !scene.is_live(index)) continue;
      seen[index] = true;
      [self _realize:tables index:index];
      if (tables.sets_typography(index)) cascade.push_back(index);
    }
    while (!cascade.empty()) {
      const uint32_t parent = cascade.back();
      cascade.pop_back();
      for (const uint32_t c : scene.node_children(parent).value_or(std::span<const uint32_t>())) {
        if (c >= seen.size() || seen[c] || !scene.is_live(c)) continue;
        seen[c] = true;
        [self _realize:tables index:c];
        cascade.push_back(c);
      }
    }
  }
  // Drop views whose node died without a removal record (e.g. after a keyframe reset).
  std::vector<uint32_t> stale;
  for (const auto &[id, m] : _views) {
    if (!tables.index_of(id)) stale.push_back(id);
  }
  for (const uint32_t id : stale) [self _destroy:id];

  if (const auto root = tables.index_of(1u)) {
    [self _layout:tables index:*root parent:nullptr into:[adapter nveHostRootView]];
  }
  _lastReconciledDirtyCount = initial ? static_cast<uint32_t>(_views.size()) : static_cast<uint32_t>(scene.dirty_indices().size());
  _lastReconciledRemovedCount = static_cast<uint32_t>(scene.removed_ids().size() + stale.size());
}

@end