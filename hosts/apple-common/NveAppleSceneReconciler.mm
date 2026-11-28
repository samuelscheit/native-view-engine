#import "NveAppleSceneReconciler.h"

#include <nve/host/retained_tree.hpp>

#include <algorithm>
#include <unordered_map>
#include <vector>

using nve::apple::NodeKind;
using nve::apple::NodeView;
using nve::host::NodeData;
using nve::host::NodeGeometry;

namespace {

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
    return nve::apple::decode_image_data([bytes subdataWithRange:NSMakeRange(5, len)]);
  }
  return nil;
}

CGRect cg_rect(const NveLayoutRect &r) { return CGRectMake(r.x, r.y, r.width, r.height); }

/// One realized node.
struct AppleView {
  NvePlatformView *view = nil;
  NodeKind kind = NodeKind::Container;
  int32_t z_index = 0;
  /// Decoded `nve.image` source and the resource id it came from.
  id image = nil;
  uint32_t image_resource = 0u;
};

/// `RetainedTree` adapter over the UIKit / AppKit view adapter: frames are parent-relative
/// (`NodeGeometry::local`), children follow scene order stably sorted by `z_index`, scroll content
/// spans the container and its children, and images are re-decoded only when their resource id or
/// bytes changed (`resource_unchanged`).
struct NveAppleAdapter {
  using Handle = AppleView;

  __weak id<NveAppleComponentViewAdapter> views;
  NveAppleResourceProvider resources;
  nve::host::RetainedTree<NveAppleAdapter> *tree = nullptr;
  /// The scene being synced (set for the duration of `RetainedTree::sync`).
  const nve::host::SceneReader *scene = nullptr;
  /// Node id of every realized view.
  std::unordered_map<const void *, uint32_t> ids;

  uint32_t tag(const NodeData &n) noexcept {
    // AppKit realizes secure inputs as another class (`NSSecureTextField`): toggling recreates.
    return static_cast<uint32_t>(nve::apple::node_kind(n)) | (n.input.secure ? 0x100u : 0u);
  }

  Handle create(const NodeData &n) {
    const NodeView v = nve::apple::node_view(n);
    AppleView h;
    h.view = [views nveCreateNativeViewForNode:v] ?: [[NvePlatformView alloc] initWithFrame:CGRectZero];
    h.kind = v.kind;
    ids[(__bridge const void *)h.view] = n.id;
    return h;
  }

  void update(Handle &h, const NodeData &n) {
    const NodeView v = nve::apple::node_view(n);
    if (v.image_resource == 0u) {
      h.image = nil;
    } else if (h.image == nil || !tree->resource_unchanged(*scene, v.image_resource, h.image_resource)) {
      h.image = resources ? image_from_source(resources(v.image_resource)) : nil;  // nil retries next update
    }
    h.image_resource = v.image_resource;
    [views nveUpdateNativeView:h.view forNode:v image:h.image];
    const bool restack = h.z_index != v.style.z_index;
    h.z_index = v.style.z_index;
    place(h, n);
    if (restack && h.view.superview != nil) restack_managed(h.view.superview);
  }

  void move(Handle &h, const NodeGeometry &g) { place(h, g); }

  void update_root(const NodeData &n) {
    NvePlatformView *root = [views nveHostRootView];
#if TARGET_OS_OSX
    root.wantsLayer = YES;
#endif
    nve::apple::apply_common_layer_style(root.layer, nve::apple::node_view(n).style);
  }

  void destroy(uint32_t node_id, Handle &h) {
    id<NveAppleComponentViewAdapter> adapter = views;
    if ([adapter respondsToSelector:@selector(nveWillDestroyNativeView:forNodeId:)]) {
      [adapter nveWillDestroyNativeView:h.view forNodeId:node_id];
    }
    ids.erase((__bridge const void *)h.view);
    [h.view removeFromSuperview];
    h = AppleView{};
  }

  void set_children(const NodeGeometry &, Handle *parent, std::span<Handle *const> kids) {
    NvePlatformView *container = parent != nullptr ? container_of(parent->view) : [views nveHostRootView];
    std::vector<const AppleView *> sorted(kids.begin(), kids.end());
    std::stable_sort(sorted.begin(), sorted.end(),
                     [](const AppleView *a, const AppleView *b) { return a->z_index < b->z_index; });
    for (const AppleView *k : sorted) {
      if (k->view.superview != container) [container addSubview:k->view];
    }
    order(container, sorted);
    if (parent != nullptr) size_scroll(*parent);
  }

 private:
  NvePlatformView *container_of(NvePlatformView *view) {
    id<NveAppleComponentViewAdapter> adapter = views;
    if ([adapter respondsToSelector:@selector(nveChildContainerForParentView:)]) {
      return [adapter nveChildContainerForParentView:view] ?: view;
    }
    return view;
  }

  /// Frame relative to the parent's content; a moved / resized child re-sizes its scroll parent.
  void place(AppleView &h, const NodeGeometry &g) {
    const CGRect local = cg_rect(g.local);
    if (!CGRectEqualToRect(h.view.frame, local)) h.view.frame = local;
    size_scroll(h);
    if (g.parent != 0u) {
      if (AppleView *parent = tree->find(g.parent)) size_scroll(*parent);
    }
  }

  void size_scroll(AppleView &h) {
    if (h.kind != NodeKind::Scroll) return;
    id<NveAppleComponentViewAdapter> adapter = views;
    if (![adapter respondsToSelector:@selector(nveUpdateScrollView:contentSize:)]) return;
    CGSize content = h.view.frame.size;
    for (NvePlatformView *kv in container_of(h.view).subviews) {
      if (ids.count((__bridge const void *)kv) == 0u) continue;
      content.width = std::max(content.width, CGRectGetMaxX(kv.frame));
      content.height = std::max(content.height, CGRectGetMaxY(kv.frame));
    }
    [adapter nveUpdateScrollView:h.view contentSize:content];
  }

  /// Re-sorts `container`'s realized subviews by their current `z_index` (stable: equal z keep the
  /// scene order the last `set_children` applied).
  void restack_managed(NvePlatformView *container) {
    std::vector<const AppleView *> sorted;
    for (NvePlatformView *kv in container.subviews) {
      const auto it = ids.find((__bridge const void *)kv);
      if (it == ids.end()) continue;
      if (const AppleView *h = tree->find(it->second)) sorted.push_back(h);
    }
    std::stable_sort(sorted.begin(), sorted.end(),
                     [](const AppleView *a, const AppleView *b) { return a->z_index < b->z_index; });
    order(container, sorted);
  }

  /// Stacks `sorted` bottom to top in `container`, touching UIKit / AppKit only when the
  /// realized subviews are out of order.
  void order(NvePlatformView *container, const std::vector<const AppleView *> &sorted) {
    size_t k = 0u;
    bool same = true;
    for (NvePlatformView *kv in container.subviews) {
      if (ids.count((__bridge const void *)kv) == 0u) continue;
      if (k >= sorted.size() || sorted[k]->view != kv) {
        same = false;
        break;
      }
      ++k;
    }
    if (same && k == sorted.size()) return;
    NvePlatformView *previous = nil;
    for (const AppleView *h : sorted) {
#if TARGET_OS_OSX
      [container addSubview:h->view positioned:NSWindowAbove relativeTo:previous];
#else
      if (previous == nil) [container sendSubviewToBack:h->view]; else [container insertSubview:h->view aboveSubview:previous];
#endif
      previous = h->view;
    }
  }
};

}  // namespace

@implementation NveAppleSceneReconciler {
  NveAppleAdapter _adapter;
  nve::host::RetainedTree<NveAppleAdapter> _tree;
  nve::host::SceneTables _tables;
}

- (instancetype)initWithAdapter:(id<NveAppleComponentViewAdapter>)adapter resources:(NveAppleResourceProvider)resources {
  if ((self = [super init])) {
    _adapter.views = adapter;
    _adapter.resources = [resources copy];
    _adapter.tree = &_tree;
  }
  return self;
}

- (NSUInteger)mountedViewCount {
  return _tree.size();
}

- (NvePlatformView *)nativeViewForNodeId:(uint32_t)nodeId {
  const AppleView *h = _tree.find(nodeId);
  return h != nullptr ? h->view : nil;
}

- (uint32_t)nodeIdForNativeView:(NvePlatformView *)view {
  const auto it = _adapter.ids.find((__bridge const void *)view);
  return it != _adapter.ids.end() ? it->second : 0u;
}

- (void)clearNativeNodeViews {
  _tree.clear(_adapter);
}

- (void)syncFromScene:(const nve::host::SceneReader &)scene {
  if (_adapter.views == nil) return;
  _adapter.scene = &scene;
  const size_t updated = _tree.sync(scene, _tables, _adapter);
  _adapter.scene = nullptr;
  _lastReconciledDirtyCount = static_cast<uint32_t>(updated);
  _lastReconciledMovedCount = static_cast<uint32_t>(_tree.moved());
  _lastReconciledRemovedCount = static_cast<uint32_t>(scene.removed_ids().size());
}

@end
