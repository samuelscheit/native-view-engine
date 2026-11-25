#include <yoga/Yoga.h>

#include <cmath>
#include <cstddef>
#include <cstdint>

extern "C" {

typedef void (*NveYogaMeasureCb)(
    void* user_data,
    float width,
    uint8_t width_mode,
    float height,
    uint8_t height_mode,
    float* out_width,
    float* out_height);

struct NveYogaMeasureHook {
  NveYogaMeasureCb callback;
  void* user_data;
};

struct NveYogaLayoutProps {
  uint8_t width_tag;
  float width_val;
  uint8_t height_tag;
  float height_val;
  uint8_t min_width_tag;
  float min_width_val;
  uint8_t min_height_tag;
  float min_height_val;
  uint8_t max_width_tag;
  float max_width_val;
  uint8_t max_height_tag;
  float max_height_val;
  uint8_t flex_direction;
  uint8_t justify_content;
  uint8_t align_items;
  uint8_t _reserved;
  float flex_grow;
  float flex_shrink;
  float gap;
  float pad_top;
  float pad_right;
  float pad_bottom;
  float pad_left;
  float margin_top;
  float margin_right;
  float margin_bottom;
  float margin_left;
};

struct NveYogaRect {
  float x;
  float y;
  float width;
  float height;
};

static uint8_t encode_measure_mode(YGMeasureMode mode) noexcept {
  switch (mode) {
    case YGMeasureModeUndefined:
      return 0;
    case YGMeasureModeExactly:
      return 1;
    case YGMeasureModeAtMost:
      return 2;
    default:
      return 0;
  }
}

static YGSize nve_yoga_measure_trampoline(
    YGNodeConstRef node,
    float width,
    YGMeasureMode width_mode,
    float height,
    YGMeasureMode height_mode) {
  auto* hook = static_cast<NveYogaMeasureHook*>(YGNodeGetContext(node));
  if (!hook || !hook->callback) {
    return YGSize{0.0f, 0.0f};
  }
  float out_w = 0.0f;
  float out_h = 0.0f;
  hook->callback(
      hook->user_data,
      width,
      encode_measure_mode(width_mode),
      height,
      encode_measure_mode(height_mode),
      &out_w,
      &out_h);
  if (!std::isfinite(out_w) || out_w < 0.0f) {
    out_w = 0.0f;
  }
  if (!std::isfinite(out_h) || out_h < 0.0f) {
    out_h = 0.0f;
  }
  return YGSize{out_w, out_h};
}

static YGFlexDirection map_flex_direction(uint8_t d) noexcept {
  switch (d) {
    case 0:
      return YGFlexDirectionColumn;
    case 1:
      return YGFlexDirectionRow;
    case 2:
      return YGFlexDirectionColumnReverse;
    case 3:
      return YGFlexDirectionRowReverse;
    default:
      return YGFlexDirectionColumn;
  }
}

static YGJustify map_justify(uint8_t j) noexcept {
  switch (j) {
    case 0:
      return YGJustifyFlexStart;
    case 1:
      return YGJustifyCenter;
    case 2:
      return YGJustifyFlexEnd;
    case 3:
      return YGJustifySpaceBetween;
    case 4:
      return YGJustifySpaceAround;
    case 5:
      return YGJustifySpaceEvenly;
    default:
      return YGJustifyFlexStart;
  }
}

static YGAlign map_align(uint8_t a) noexcept {
  switch (a) {
    case 0:
      return YGAlignStretch;
    case 1:
      return YGAlignFlexStart;
    case 2:
      return YGAlignCenter;
    case 3:
      return YGAlignFlexEnd;
    case 4:
      return YGAlignBaseline;
    default:
      return YGAlignStretch;
  }
}

YGConfigRef nve_yg_config_new(void) {
  return YGConfigNew();
}

void nve_yg_config_free(YGConfigRef config) {
  if (config) {
    YGConfigFree(config);
  }
}

void nve_yg_config_set_point_scale_factor(YGConfigRef config, float scale) {
  if (config && std::isfinite(scale) && scale > 0.0f) {
    YGConfigSetPointScaleFactor(config, scale);
  }
}

YGNodeRef nve_yg_node_new_with_config(YGConfigConstRef config) {
  return YGNodeNewWithConfig(config);
}

void nve_yg_node_detach_and_clear(YGNodeRef node) {
  if (!node) {
    return;
  }
  if (YGNodeRef owner = YGNodeGetOwner(node)) {
    YGNodeRemoveChild(owner, node);
  }
  YGNodeRemoveAllChildren(node);
  YGNodeSetMeasureFunc(node, nullptr);
  YGNodeSetContext(node, nullptr);
}

void nve_yg_node_free(YGNodeRef node) {
  if (!node) {
    return;
  }
  nve_yg_node_detach_and_clear(node);
  YGNodeFree(node);
}

void nve_yg_node_set_is_text(YGNodeRef node, bool is_text) {
  if (!node) {
    return;
  }
  YGNodeSetNodeType(node, is_text ? YGNodeTypeText : YGNodeTypeDefault);
}

void nve_yg_node_set_measure_hook(YGNodeRef node, NveYogaMeasureHook* hook) {
  if (!node) {
    return;
  }
  if (!hook) {
    YGNodeSetMeasureFunc(node, nullptr);
    YGNodeSetContext(node, nullptr);
    return;
  }
  // Yoga requires nodes with a measure function to have no children.
  if (YGNodeGetChildCount(node) > 0) {
    return;
  }
  YGNodeSetContext(node, hook);
  YGNodeSetMeasureFunc(node, nve_yoga_measure_trampoline);
}

void nve_yg_node_mark_dirty(YGNodeRef node) {
  if (node && YGNodeHasMeasureFunc(node)) {
    YGNodeMarkDirty(node);
  }
}

void nve_yg_node_set_children(YGNodeRef parent, const YGNodeRef* children, size_t count) {
  if (!parent) {
    return;
  }
  if (count > 0 && YGNodeHasMeasureFunc(parent)) {
    YGNodeSetMeasureFunc(parent, nullptr);
    YGNodeSetContext(parent, nullptr);
  }
  for (size_t i = 0; i < count; ++i) {
    YGNodeRef child = children[i];
    if (!child) {
      continue;
    }
    if (YGNodeRef old_owner = YGNodeGetOwner(child)) {
      if (old_owner != parent) {
        YGNodeRemoveChild(old_owner, child);
      }
    }
  }
  YGNodeSetChildren(parent, children, count);
}

void nve_yg_node_apply_layout(YGNodeRef node, const NveYogaLayoutProps* props) {
  if (!node || !props) {
    return;
  }
  switch (props->width_tag) {
    case 1:
      YGNodeStyleSetWidth(node, props->width_val);
      break;
    case 2:
      YGNodeStyleSetWidthPercent(node, props->width_val);
      break;
    default:
      YGNodeStyleSetWidthAuto(node);
      break;
  }
  switch (props->height_tag) {
    case 1:
      YGNodeStyleSetHeight(node, props->height_val);
      break;
    case 2:
      YGNodeStyleSetHeightPercent(node, props->height_val);
      break;
    default:
      YGNodeStyleSetHeightAuto(node);
      break;
  }
  switch (props->min_width_tag) {
    case 1:
      YGNodeStyleSetMinWidth(node, props->min_width_val);
      break;
    case 2:
      YGNodeStyleSetMinWidthPercent(node, props->min_width_val);
      break;
    default:
      YGNodeStyleSetMinWidth(node, YGUndefined);
      break;
  }
  switch (props->min_height_tag) {
    case 1:
      YGNodeStyleSetMinHeight(node, props->min_height_val);
      break;
    case 2:
      YGNodeStyleSetMinHeightPercent(node, props->min_height_val);
      break;
    default:
      YGNodeStyleSetMinHeight(node, YGUndefined);
      break;
  }
  switch (props->max_width_tag) {
    case 1:
      YGNodeStyleSetMaxWidth(node, props->max_width_val);
      break;
    case 2:
      YGNodeStyleSetMaxWidthPercent(node, props->max_width_val);
      break;
    default:
      YGNodeStyleSetMaxWidth(node, YGUndefined);
      break;
  }
  switch (props->max_height_tag) {
    case 1:
      YGNodeStyleSetMaxHeight(node, props->max_height_val);
      break;
    case 2:
      YGNodeStyleSetMaxHeightPercent(node, props->max_height_val);
      break;
    default:
      YGNodeStyleSetMaxHeight(node, YGUndefined);
      break;
  }

  YGNodeStyleSetBoxSizing(node, YGBoxSizingBorderBox);
  YGNodeStyleSetFlexDirection(node, map_flex_direction(props->flex_direction));
  YGNodeStyleSetJustifyContent(node, map_justify(props->justify_content));
  YGNodeStyleSetAlignItems(node, map_align(props->align_items));
  YGNodeStyleSetFlexGrow(node, props->flex_grow);
  YGNodeStyleSetFlexShrink(node, props->flex_shrink);
  YGNodeStyleSetGap(node, YGGutterAll, props->gap);

  YGNodeStyleSetPadding(node, YGEdgeTop, props->pad_top);
  YGNodeStyleSetPadding(node, YGEdgeRight, props->pad_right);
  YGNodeStyleSetPadding(node, YGEdgeBottom, props->pad_bottom);
  YGNodeStyleSetPadding(node, YGEdgeLeft, props->pad_left);

  YGNodeStyleSetMargin(node, YGEdgeTop, props->margin_top);
  YGNodeStyleSetMargin(node, YGEdgeRight, props->margin_right);
  YGNodeStyleSetMargin(node, YGEdgeBottom, props->margin_bottom);
  YGNodeStyleSetMargin(node, YGEdgeLeft, props->margin_left);
}

void nve_yg_node_set_fixed_size(YGNodeRef node, float width, float height) {
  if (!node) {
    return;
  }
  YGNodeStyleSetWidth(node, width);
  YGNodeStyleSetHeight(node, height);
}

void nve_yg_node_calculate_layout(YGNodeRef root, float avail_width, float avail_height) {
  if (!root) {
    return;
  }
  YGNodeCalculateLayout(root, avail_width, avail_height, YGDirectionLTR);
}

NveYogaRect nve_yg_node_get_layout_rect(YGNodeConstRef node) {
  if (!node) {
    return NveYogaRect{0.0f, 0.0f, 0.0f, 0.0f};
  }
  auto sanitize = [](float v) noexcept -> float { return std::isfinite(v) ? v : 0.0f; };
  return NveYogaRect{
      sanitize(YGNodeLayoutGetLeft(node)),
      sanitize(YGNodeLayoutGetTop(node)),
      sanitize(YGNodeLayoutGetWidth(node)),
      sanitize(YGNodeLayoutGetHeight(node)),
  };
}

} // extern "C"
