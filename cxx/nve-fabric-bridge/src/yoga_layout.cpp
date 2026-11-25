#include "fabric_internal.hpp"

namespace nve {

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA

namespace {

YGFlexDirection to_yg_flex_direction(uint8_t d) noexcept {
  switch (d) {
    case 0: return YGFlexDirectionColumn;
    case 1: return YGFlexDirectionRow;
    case 2: return YGFlexDirectionColumnReverse;
    case 3: return YGFlexDirectionRowReverse;
    default: return YGFlexDirectionColumn;
  }
}

YGJustify to_yg_justify(uint8_t j) noexcept {
  switch (j) {
    case 0: return YGJustifyFlexStart;
    case 1: return YGJustifyCenter;
    case 2: return YGJustifyFlexEnd;
    case 3: return YGJustifySpaceBetween;
    case 4: return YGJustifySpaceAround;
    case 5: return YGJustifySpaceEvenly;
    default: return YGJustifyFlexStart;
  }
}

YGAlign to_yg_align(uint8_t a) noexcept {
  switch (a) {
    case 0: return YGAlignStretch;
    case 1: return YGAlignFlexStart;
    case 2: return YGAlignCenter;
    case 3: return YGAlignFlexEnd;
    case 4: return YGAlignBaseline;
    default: return YGAlignStretch;
  }
}

void apply_yg_width(YGNodeRef node, const Dimension& d) noexcept {
  switch (d.tag) {
    case 0: YGNodeStyleSetWidthAuto(node); break;
    case 1: YGNodeStyleSetWidth(node, d.value); break;
    case 2: YGNodeStyleSetWidthPercent(node, d.value); break;
    default: YGNodeStyleSetWidthAuto(node); break;
  }
}

void apply_yg_height(YGNodeRef node, const Dimension& d) noexcept {
  switch (d.tag) {
    case 0: YGNodeStyleSetHeightAuto(node); break;
    case 1: YGNodeStyleSetHeight(node, d.value); break;
    case 2: YGNodeStyleSetHeightPercent(node, d.value); break;
    default: YGNodeStyleSetHeightAuto(node); break;
  }
}

void apply_yg_min_width(YGNodeRef node, const Dimension& d) noexcept {
  switch (d.tag) {
    case 0: YGNodeStyleSetMinWidth(node, YGUndefined); break;
    case 1: YGNodeStyleSetMinWidth(node, d.value); break;
    case 2: YGNodeStyleSetMinWidthPercent(node, d.value); break;
    default: YGNodeStyleSetMinWidth(node, YGUndefined); break;
  }
}

void apply_yg_min_height(YGNodeRef node, const Dimension& d) noexcept {
  switch (d.tag) {
    case 0: YGNodeStyleSetMinHeight(node, YGUndefined); break;
    case 1: YGNodeStyleSetMinHeight(node, d.value); break;
    case 2: YGNodeStyleSetMinHeightPercent(node, d.value); break;
    default: YGNodeStyleSetMinHeight(node, YGUndefined); break;
  }
}

void apply_yg_max_width(YGNodeRef node, const Dimension& d) noexcept {
  switch (d.tag) {
    case 0: YGNodeStyleSetMaxWidth(node, YGUndefined); break;
    case 1: YGNodeStyleSetMaxWidth(node, d.value); break;
    case 2: YGNodeStyleSetMaxWidthPercent(node, d.value); break;
    default: YGNodeStyleSetMaxWidth(node, YGUndefined); break;
  }
}

void apply_yg_max_height(YGNodeRef node, const Dimension& d) noexcept {
  switch (d.tag) {
    case 0: YGNodeStyleSetMaxHeight(node, YGUndefined); break;
    case 1: YGNodeStyleSetMaxHeight(node, d.value); break;
    case 2: YGNodeStyleSetMaxHeightPercent(node, d.value); break;
    default: YGNodeStyleSetMaxHeight(node, YGUndefined); break;
  }
}

inline const ShadowNodeEntry* resolve_yg_entry(
    YGNodeConstRef node,
    const FabricHost** out_host = nullptr) noexcept {
  const auto* host = static_cast<const FabricHost*>(
      YGConfigGetContext(YGNodeGetConfig(const_cast<YGNodeRef>(node))));
  if (out_host != nullptr) {
    *out_host = host;
  }
  if (host == nullptr) {
    return nullptr;
  }
  const uint32_t node_id =
      static_cast<uint32_t>(reinterpret_cast<uintptr_t>(YGNodeGetContext(node)));
  return host->find_node(node_id);
}

float baseline_text_node(YGNodeConstRef node, float /*width*/, float height) {
  const ShadowNodeEntry* entry = resolve_yg_entry(node);
  if (!entry) {
    return (std::isfinite(height) && height >= 0.0f) ? height : 0.0f;
  }
  return compute_text_leaf_baseline(*entry, height);
}

YGSize measure_text_node(
    YGNodeConstRef node,
    float width,
    YGMeasureMode width_mode,
    float height,
    YGMeasureMode height_mode) {
  const FabricHost* host = nullptr;
  const ShadowNodeEntry* entry = resolve_yg_entry(node, &host);
  const float font_size =
      (entry && std::isfinite(entry->style.font_size) && entry->style.font_size > 0.0f)
          ? entry->style.font_size
          : 16.0f;
  const size_t len = entry ? entry->text.size() : 0;

  float out_w = 0.0f;
  float out_h = 0.0f;
  if (len > 0 && entry && host && host->text_measurer()) {
    const float max_w = (width_mode == YGMeasureModeUndefined) ? 0.0f : width;
    const NveIntrinsicSize measured = host->text_measurer()(entry->text, font_size, max_w);
    out_w = (std::isfinite(measured.width) && measured.width >= 0.0f) ? measured.width : 0.0f;
    out_h = (std::isfinite(measured.height) && measured.height >= 0.0f) ? measured.height : 0.0f;
    if (width_mode == YGMeasureModeExactly) {
      out_w = width;
    } else if (width_mode == YGMeasureModeAtMost) {
      out_w = std::min(out_w, width);
    }
    if (height_mode == YGMeasureModeExactly) {
      out_h = height;
    } else if (height_mode == YGMeasureModeAtMost) {
      out_h = std::min(out_h, height);
    }
    return YGSize{out_w, out_h};
  }

  const float char_width = font_size * 0.55f;
  const float line_height = font_size * 1.3f;
  const float intrinsic_w = len > 0 ? (static_cast<float>(len) * char_width + 4.0f) : 0.0f;
  const float intrinsic_h = len > 0 ? line_height : 0.0f;

  out_w = intrinsic_w;
  if (width_mode == YGMeasureModeExactly) {
    out_w = width;
  } else if (width_mode == YGMeasureModeAtMost) {
    out_w = std::min(intrinsic_w, width);
  }

  out_h = intrinsic_h;
  if (out_w > 0.0f && intrinsic_w > out_w && len > 0) {
    const float lines = std::ceil(intrinsic_w / out_w);
    out_h = lines * line_height;
  }
  if (height_mode == YGMeasureModeExactly) {
    out_h = height;
  } else if (height_mode == YGMeasureModeAtMost) {
    out_h = std::min(out_h, height);
  }
  return YGSize{out_w, out_h};
}

} // namespace

void apply_yg_align_self(YGNodeRef node, uint8_t align_self) noexcept {
  if (align_self <= 4u) {
    YGNodeStyleSetAlignSelf(node, to_yg_align(align_self));
  } else {
    YGNodeStyleSetAlignSelf(node, YGAlignAuto);
  }
}

void apply_yg_flex_basis(YGNodeRef node, const Dimension& d) noexcept {
  switch (d.tag) {
    case 1: YGNodeStyleSetFlexBasis(node, d.value); break;
    case 2: YGNodeStyleSetFlexBasisPercent(node, d.value); break;
    default: YGNodeStyleSetFlexBasisAuto(node); break;
  }
}

void apply_yg_layout_props(YGNodeRef node, const LayoutProps& l) noexcept {
  apply_yg_width(node, l.width);
  apply_yg_height(node, l.height);
  apply_yg_min_width(node, l.min_width);
  apply_yg_min_height(node, l.min_height);
  apply_yg_max_width(node, l.max_width);
  apply_yg_max_height(node, l.max_height);
  YGNodeStyleSetFlexDirection(node, to_yg_flex_direction(l.flex_direction));
  YGNodeStyleSetJustifyContent(node, to_yg_justify(l.justify_content));
  YGNodeStyleSetAlignItems(node, to_yg_align(l.align_items));
  YGNodeStyleSetFlexGrow(node, l.flex_grow);
  YGNodeStyleSetFlexShrink(node, l.flex_shrink);
  YGNodeStyleSetGap(node, YGGutterAll, l.gap);
  YGNodeStyleSetPadding(node, YGEdgeTop, l.padding.top);
  YGNodeStyleSetPadding(node, YGEdgeRight, l.padding.right);
  YGNodeStyleSetPadding(node, YGEdgeBottom, l.padding.bottom);
  YGNodeStyleSetPadding(node, YGEdgeLeft, l.padding.left);
  YGNodeStyleSetMargin(node, YGEdgeTop, l.margin.top);
  YGNodeStyleSetMargin(node, YGEdgeRight, l.margin.right);
  YGNodeStyleSetMargin(node, YGEdgeBottom, l.margin.bottom);
  YGNodeStyleSetMargin(node, YGEdgeLeft, l.margin.left);
}

void FabricHost::init_node_yoga(ShadowNodeEntry& entry) {
  entry.yg_node = YGNodeNewWithConfig(yg_config_);
  YGNodeSetContext(entry.yg_node, reinterpret_cast<void*>(static_cast<uintptr_t>(entry.node_id)));
  YGNodeStyleSetFlexShrink(entry.yg_node, entry.layout.flex_shrink);
  if (entry.kind == NVE_COMP_TEXT && entry.children.empty()) {
    YGNodeSetNodeType(entry.yg_node, YGNodeTypeText);
    YGNodeSetMeasureFunc(entry.yg_node, &measure_text_node);
    YGNodeSetBaselineFunc(entry.yg_node, &baseline_text_node);
  }
}

void FabricHost::sync_children_yoga(ShadowNodeEntry& parent) {
  if (!parent.yg_node) return;
  YGNodeRemoveAllChildren(parent.yg_node);
  if (!parent.children.empty()) {
    if (YGNodeHasMeasureFunc(parent.yg_node)) {
      YGNodeSetMeasureFunc(parent.yg_node, nullptr);
    }
    if (YGNodeHasBaselineFunc(parent.yg_node)) {
      YGNodeSetBaselineFunc(parent.yg_node, nullptr);
    }
  }
  for (uint32_t child_id : parent.children) {
    if (child_id == parent.node_id) continue;
    ShadowNodeEntry* child = nodes_.find(child_id);
    if (child == nullptr || !child->yg_node) continue;
    YGNodeRef child_yg = child->yg_node;
    if (YGNodeRef prev_owner = YGNodeGetOwner(child_yg)) {
      YGNodeRemoveChild(prev_owner, child_yg);
    }
    YGNodeInsertChild(parent.yg_node, child_yg, YGNodeGetChildCount(parent.yg_node));
  }
  if (parent.kind == NVE_COMP_TEXT && YGNodeGetChildCount(parent.yg_node) == 0) {
    YGNodeSetMeasureFunc(parent.yg_node, &measure_text_node);
    YGNodeSetBaselineFunc(parent.yg_node, &baseline_text_node);
  }
}

bool FabricHost::readback_single_yoga_node(ShadowNodeEntry& entry, uint64_t ts) {
  if (!entry.yg_node) return false;
  auto finite_or_zero = [](float v) noexcept -> float { return std::isfinite(v) ? v : 0.0f; };
  const LayoutRect next_rect{
      .x = finite_or_zero(YGNodeLayoutGetLeft(entry.yg_node)),
      .y = finite_or_zero(YGNodeLayoutGetTop(entry.yg_node)),
      .width = std::max(0.0f, finite_or_zero(YGNodeLayoutGetWidth(entry.yg_node))),
      .height = std::max(0.0f, finite_or_zero(YGNodeLayoutGetHeight(entry.yg_node))),
  };
  const bool size_same = entry.has_computed_layout &&
                         std::fabs(entry.computed_layout.width - next_rect.width) <= 1e-4f &&
                         std::fabs(entry.computed_layout.height - next_rect.height) <= 1e-4f;
  const bool changed = !entry.has_computed_layout || rect_changed(entry.computed_layout, next_rect);
  entry.computed_layout = next_rect;
  entry.has_computed_layout = true;
  const bool had_new = YGNodeGetHasNewLayout(entry.yg_node);
  if (had_new) {
    YGNodeSetHasNewLayout(entry.yg_node, false);
  }
  if (changed) {
    mark_node_scene_dirty(entry, kSceneDirtyLayout);
  }
  if ((entry.event_mask & kEventMaskLayout) != 0 && (changed || had_new)) {
    std::lock_guard<std::mutex> lock(event_mutex_);
    event_ring_.push_layout(
        config_.surface_id,
        entry.node_id,
        ts,
        next_rect.x,
        next_rect.y,
        next_rect.width,
        next_rect.height);
  }
  return size_same;
}

void FabricHost::readback_yoga_subtree(uint32_t node_id, uint64_t ts, uint32_t depth) {
  if (depth > 512u) return;
  ShadowNodeEntry* entry = nodes_.find(node_id);
  if (entry == nullptr || !entry->yg_node) return;
  const bool was_dirty = entry->subtree_layout_dirty;
  if (!force_full_layout_ && !was_dirty && entry->has_computed_layout &&
      !YGNodeGetHasNewLayout(entry->yg_node)) {
    return;
  }
  const bool size_same = readback_single_yoga_node(*entry, ts);
  entry->subtree_layout_dirty = false;
  if (!force_full_layout_ && !was_dirty && size_same) {
    return;
  }
  for (uint32_t cid : entry->children) {
    readback_yoga_subtree(cid, ts, depth + 1u);
  }
}

#else

void FabricHost::init_node_yoga(ShadowNodeEntry& entry) { (void)entry; }
void FabricHost::sync_children_yoga(ShadowNodeEntry& parent) { (void)parent; }

#endif // defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA

void FabricHost::set_surface_size(float width, float height, float scale_factor) {
  config_.width = width;
  config_.height = height;
  if (scale_factor > 0.0f) {
    if (std::fabs(config_.scale_factor - scale_factor) > 1e-5f) {
      force_full_layout_ = true;
    }
    config_.scale_factor = scale_factor;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    if (yg_config_) {
      YGConfigSetPointScaleFactor(yg_config_, scale_factor);
    }
#endif
  }
  mark_layout_dirty_upward(NVE_ROOT_NODE_ID);
  run_layout_pass();
  scene_pending_publish_ = true;
}

void FabricHost::set_text_measurer(TextMeasurerCallback measurer) {
  text_measurer_ = std::move(measurer);
  ++measure_epoch_;
  force_full_layout_ = true;
  layout_dirty_ = true;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  for (auto& entry : nodes_.entries()) {
    if (entry.yg_node && YGNodeHasMeasureFunc(entry.yg_node)) {
      YGNodeMarkDirty(entry.yg_node);
    }
  }
#endif
}

void FabricHost::run_layout_pass() {
  ShadowNodeEntry* root = nodes_.find(NVE_ROOT_NODE_ID);
  if (root == nullptr) return;
  propagate_text_styles();

  if (!layout_dirty_ && !force_full_layout_ && new_nodes_pending_layout_.empty()) {
    return;
  }

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  YGNodeRef root_yg = root->yg_node;
  if (!root_yg) return;
  const float yg_surf_w =
      (std::isfinite(config_.width) && config_.width >= 0.0f) ? config_.width : YGUndefined;
  const float yg_surf_h =
      (std::isfinite(config_.height) && config_.height >= 0.0f) ? config_.height : YGUndefined;
  if (root->layout.width.tag == 0) {
    YGNodeStyleSetWidth(root_yg, yg_surf_w);
  }
  if (root->layout.height.tag == 0) {
    YGNodeStyleSetHeight(root_yg, yg_surf_h);
  }
  YGNodeCalculateLayout(root_yg, yg_surf_w, yg_surf_h, YGDirectionLTR);

  const uint64_t ts = now_us();
  readback_yoga_subtree(NVE_ROOT_NODE_ID, ts, 0u);
  for (uint32_t nid : new_nodes_pending_layout_) {
    if (ShadowNodeEntry* entry = nodes_.find(nid)) {
      if (!entry->has_computed_layout) {
        (void)readback_single_yoga_node(*entry, ts);
        entry->subtree_layout_dirty = false;
      }
    }
  }
#else
  run_standalone_layout_pass(*root);
#endif
  new_nodes_pending_layout_.clear();
  layout_dirty_ = false;
  force_full_layout_ = false;
}

} // namespace nve
