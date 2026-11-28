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
  float row_gap;
  float column_gap;
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

struct NveYogaLayoutExt {
  uint8_t display;         // 0=flex, 1=none
  uint8_t flex_basis_tag;  // 0=auto, 1=px, 2=pct
  float flex_basis_val;
  uint8_t has_aspect_ratio;
  float aspect_ratio;
  uint8_t position_mode;   // 0=relative, 1=absolute, 2=sticky(normal flow)
  uint8_t top_tag;
  float top_val;
  uint8_t right_tag;
  float right_val;
  uint8_t bottom_tag;
  float bottom_val;
  uint8_t left_tag;
  float left_val;
  uint8_t direction;       // 0=inherit, 1=ltr, 2=rtl
  uint8_t has_pad_start;
  float pad_start;
  uint8_t has_pad_end;
  float pad_end;
  uint8_t has_mar_start;
  float mar_start;
  uint8_t has_mar_end;
  float mar_end;
  uint8_t flex_wrap;      // 0=nowrap, 1=wrap, 2=wrap-reverse
  uint8_t align_content;  // 0=stretch, 1=flex-start, 2=center, 3=flex-end, 4=space-between, 5=space-around, 6=space-evenly
  uint8_t align_self;     // 0=auto, 1=stretch, 2=flex-start, 3=center, 4=flex-end, 5=baseline
  uint8_t box_sizing;     // 0=border-box, 1=content-box
  uint8_t overflow;       // 0=visible, 1=hidden, 2=scroll
  float border_width;
};

// Compile-time ABI layout locks mirroring `packages/modules/layout-yoga/src/ffi.rs`.
static_assert(sizeof(NveYogaMeasureHook) == 2u * sizeof(void*), "NveYogaMeasureHook size");
static_assert(offsetof(NveYogaMeasureHook, callback) == 0u, "NveYogaMeasureHook::callback");
static_assert(offsetof(NveYogaMeasureHook, user_data) == sizeof(void*), "NveYogaMeasureHook::user_data");

static_assert(sizeof(NveYogaRect) == 16u, "NveYogaRect size");
static_assert(alignof(NveYogaRect) == 4u, "NveYogaRect align");
static_assert(offsetof(NveYogaRect, x) == 0u, "NveYogaRect::x");
static_assert(offsetof(NveYogaRect, y) == 4u, "NveYogaRect::y");
static_assert(offsetof(NveYogaRect, width) == 8u, "NveYogaRect::width");
static_assert(offsetof(NveYogaRect, height) == 12u, "NveYogaRect::height");

static_assert(sizeof(NveYogaLayoutProps) == 100u, "NveYogaLayoutProps size");
static_assert(alignof(NveYogaLayoutProps) == 4u, "NveYogaLayoutProps align");
static_assert(offsetof(NveYogaLayoutProps, width_tag) == 0u, "width_tag");
static_assert(offsetof(NveYogaLayoutProps, width_val) == 4u, "width_val");
static_assert(offsetof(NveYogaLayoutProps, height_tag) == 8u, "height_tag");
static_assert(offsetof(NveYogaLayoutProps, height_val) == 12u, "height_val");
static_assert(offsetof(NveYogaLayoutProps, min_width_tag) == 16u, "min_width_tag");
static_assert(offsetof(NveYogaLayoutProps, min_width_val) == 20u, "min_width_val");
static_assert(offsetof(NveYogaLayoutProps, min_height_tag) == 24u, "min_height_tag");
static_assert(offsetof(NveYogaLayoutProps, min_height_val) == 28u, "min_height_val");
static_assert(offsetof(NveYogaLayoutProps, max_width_tag) == 32u, "max_width_tag");
static_assert(offsetof(NveYogaLayoutProps, max_width_val) == 36u, "max_width_val");
static_assert(offsetof(NveYogaLayoutProps, max_height_tag) == 40u, "max_height_tag");
static_assert(offsetof(NveYogaLayoutProps, max_height_val) == 44u, "max_height_val");
static_assert(offsetof(NveYogaLayoutProps, flex_direction) == 48u, "flex_direction");
static_assert(offsetof(NveYogaLayoutProps, justify_content) == 49u, "justify_content");
static_assert(offsetof(NveYogaLayoutProps, align_items) == 50u, "align_items");
static_assert(offsetof(NveYogaLayoutProps, _reserved) == 51u, "_reserved");
static_assert(offsetof(NveYogaLayoutProps, flex_grow) == 52u, "flex_grow");
static_assert(offsetof(NveYogaLayoutProps, flex_shrink) == 56u, "flex_shrink");
static_assert(offsetof(NveYogaLayoutProps, row_gap) == 60u, "row_gap");
static_assert(offsetof(NveYogaLayoutProps, column_gap) == 64u, "column_gap");
static_assert(offsetof(NveYogaLayoutProps, pad_top) == 68u, "pad_top");
static_assert(offsetof(NveYogaLayoutProps, pad_right) == 72u, "pad_right");
static_assert(offsetof(NveYogaLayoutProps, pad_bottom) == 76u, "pad_bottom");
static_assert(offsetof(NveYogaLayoutProps, pad_left) == 80u, "pad_left");
static_assert(offsetof(NveYogaLayoutProps, margin_top) == 84u, "margin_top");
static_assert(offsetof(NveYogaLayoutProps, margin_right) == 88u, "margin_right");
static_assert(offsetof(NveYogaLayoutProps, margin_bottom) == 92u, "margin_bottom");
static_assert(offsetof(NveYogaLayoutProps, margin_left) == 96u, "margin_left");

static_assert(sizeof(NveYogaLayoutExt) == 92u, "NveYogaLayoutExt size");
static_assert(alignof(NveYogaLayoutExt) == 4u, "NveYogaLayoutExt align");
static_assert(offsetof(NveYogaLayoutExt, display) == 0u, "display");
static_assert(offsetof(NveYogaLayoutExt, flex_basis_tag) == 1u, "flex_basis_tag");
static_assert(offsetof(NveYogaLayoutExt, flex_basis_val) == 4u, "flex_basis_val");
static_assert(offsetof(NveYogaLayoutExt, has_aspect_ratio) == 8u, "has_aspect_ratio");
static_assert(offsetof(NveYogaLayoutExt, aspect_ratio) == 12u, "aspect_ratio");
static_assert(offsetof(NveYogaLayoutExt, position_mode) == 16u, "position_mode");
static_assert(offsetof(NveYogaLayoutExt, top_tag) == 17u, "top_tag");
static_assert(offsetof(NveYogaLayoutExt, top_val) == 20u, "top_val");
static_assert(offsetof(NveYogaLayoutExt, right_tag) == 24u, "right_tag");
static_assert(offsetof(NveYogaLayoutExt, right_val) == 28u, "right_val");
static_assert(offsetof(NveYogaLayoutExt, bottom_tag) == 32u, "bottom_tag");
static_assert(offsetof(NveYogaLayoutExt, bottom_val) == 36u, "bottom_val");
static_assert(offsetof(NveYogaLayoutExt, left_tag) == 40u, "left_tag");
static_assert(offsetof(NveYogaLayoutExt, left_val) == 44u, "left_val");
static_assert(offsetof(NveYogaLayoutExt, direction) == 48u, "direction");
static_assert(offsetof(NveYogaLayoutExt, has_pad_start) == 49u, "has_pad_start");
static_assert(offsetof(NveYogaLayoutExt, pad_start) == 52u, "pad_start");
static_assert(offsetof(NveYogaLayoutExt, has_pad_end) == 56u, "has_pad_end");
static_assert(offsetof(NveYogaLayoutExt, pad_end) == 60u, "pad_end");
static_assert(offsetof(NveYogaLayoutExt, has_mar_start) == 64u, "has_mar_start");
static_assert(offsetof(NveYogaLayoutExt, mar_start) == 68u, "mar_start");
static_assert(offsetof(NveYogaLayoutExt, has_mar_end) == 72u, "has_mar_end");
static_assert(offsetof(NveYogaLayoutExt, mar_end) == 76u, "mar_end");
static_assert(offsetof(NveYogaLayoutExt, flex_wrap) == 80u, "flex_wrap");
static_assert(offsetof(NveYogaLayoutExt, align_content) == 81u, "align_content");
static_assert(offsetof(NveYogaLayoutExt, align_self) == 82u, "align_self");
static_assert(offsetof(NveYogaLayoutExt, box_sizing) == 83u, "box_sizing");
static_assert(offsetof(NveYogaLayoutExt, overflow) == 84u, "overflow");
static_assert(offsetof(NveYogaLayoutExt, border_width) == 88u, "border_width");

static inline float sanitize_finite(float v) noexcept {
  return std::isfinite(v) ? v : 0.0f;
}

static inline float sanitize_non_neg(float v) noexcept {
  return (std::isfinite(v) && v >= 0.0f) ? v : 0.0f;
}

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
    YGMeasureMode height_mode) noexcept {
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
  return YGSize{sanitize_non_neg(out_w), sanitize_non_neg(out_h)};
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

YGConfigRef nve_yg_config_new(void) noexcept {
  YGConfigRef cfg = YGConfigNew();
  if (cfg) {
    YGConfigSetErrata(cfg, YGErrataNone);
  }
  return cfg;
}

void nve_yg_config_free(YGConfigRef config) noexcept {
  if (config) {
    YGConfigFree(config);
  }
}

void nve_yg_config_set_point_scale_factor(YGConfigRef config, float scale) noexcept {
  if (config && std::isfinite(scale) && scale > 0.0f) {
    YGConfigSetPointScaleFactor(config, scale);
  }
}

YGNodeRef nve_yg_node_new_with_config(YGConfigConstRef config) noexcept {
  return YGNodeNewWithConfig(config);
}

void nve_yg_node_detach_and_clear(YGNodeRef node) noexcept {
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

void nve_yg_node_free(YGNodeRef node) noexcept {
  if (!node) {
    return;
  }
  nve_yg_node_detach_and_clear(node);
  YGNodeFree(node);
}

bool nve_yg_node_set_measure_hook(YGNodeRef node, NveYogaMeasureHook* hook) noexcept {
  if (!node) {
    return false;
  }
  if (!hook) {
    YGNodeSetMeasureFunc(node, nullptr);
    YGNodeSetContext(node, nullptr);
    YGNodeSetNodeType(node, YGNodeTypeDefault);
    return false;
  }
  // Yoga requires nodes with a measure function to have no children.
  if (YGNodeGetChildCount(node) > 0) {
    return false;
  }
  YGNodeSetContext(node, hook);
  YGNodeSetMeasureFunc(node, nve_yoga_measure_trampoline);
  // Keep `YGNodeTypeDefault` rounding so cumulative edge rounding matches Taffy.
  YGNodeSetNodeType(node, YGNodeTypeDefault);
  return true;
}

void nve_yg_node_mark_dirty(YGNodeRef node) noexcept {
  if (node && YGNodeHasMeasureFunc(node)) {
    YGNodeMarkDirty(node);
  }
}

void nve_yg_node_set_children(YGNodeRef parent, const YGNodeRef* children, size_t count) noexcept {
  if (!parent) {
    return;
  }
  if (count == 0) {
    YGNodeRemoveAllChildren(parent);
    return;
  }
  if (!children) {
    return;
  }
  // Reject arrays containing null handles before touching Yoga (YGNodeSetChildren dereferences).
  for (size_t i = 0; i < count; ++i) {
    if (!children[i]) {
      return;
    }
  }
  if (YGNodeHasMeasureFunc(parent)) {
    YGNodeSetMeasureFunc(parent, nullptr);
    YGNodeSetContext(parent, nullptr);
  }
  for (size_t i = 0; i < count; ++i) {
    YGNodeRef child = children[i];
    if (YGNodeRef old_owner = YGNodeGetOwner(child)) {
      if (old_owner != parent) {
        YGNodeRemoveChild(old_owner, child);
      }
    }
  }
  YGNodeSetChildren(parent, children, count);
}

void nve_yg_node_apply_layout(YGNodeRef node, const NveYogaLayoutProps* props) noexcept {
  if (!node || !props) {
    return;
  }
  if (props->width_tag == 1 && std::isfinite(props->width_val) && props->width_val >= 0.0f) {
    YGNodeStyleSetWidth(node, props->width_val);
  } else if (props->width_tag == 2 && std::isfinite(props->width_val) && props->width_val >= 0.0f) {
    YGNodeStyleSetWidthPercent(node, props->width_val);
  } else {
    YGNodeStyleSetWidthAuto(node);
  }

  if (props->height_tag == 1 && std::isfinite(props->height_val) && props->height_val >= 0.0f) {
    YGNodeStyleSetHeight(node, props->height_val);
  } else if (props->height_tag == 2 && std::isfinite(props->height_val) && props->height_val >= 0.0f) {
    YGNodeStyleSetHeightPercent(node, props->height_val);
  } else {
    YGNodeStyleSetHeightAuto(node);
  }

  if (props->min_width_tag == 1 && std::isfinite(props->min_width_val) && props->min_width_val >= 0.0f) {
    YGNodeStyleSetMinWidth(node, props->min_width_val);
  } else if (props->min_width_tag == 2 && std::isfinite(props->min_width_val) && props->min_width_val >= 0.0f) {
    YGNodeStyleSetMinWidthPercent(node, props->min_width_val);
  } else {
    YGNodeStyleSetMinWidth(node, YGUndefined);
  }

  if (props->min_height_tag == 1 && std::isfinite(props->min_height_val) && props->min_height_val >= 0.0f) {
    YGNodeStyleSetMinHeight(node, props->min_height_val);
  } else if (props->min_height_tag == 2 && std::isfinite(props->min_height_val) && props->min_height_val >= 0.0f) {
    YGNodeStyleSetMinHeightPercent(node, props->min_height_val);
  } else {
    YGNodeStyleSetMinHeight(node, YGUndefined);
  }

  if (props->max_width_tag == 1 && std::isfinite(props->max_width_val) && props->max_width_val >= 0.0f) {
    YGNodeStyleSetMaxWidth(node, props->max_width_val);
  } else if (props->max_width_tag == 2 && std::isfinite(props->max_width_val) && props->max_width_val >= 0.0f) {
    YGNodeStyleSetMaxWidthPercent(node, props->max_width_val);
  } else {
    YGNodeStyleSetMaxWidth(node, YGUndefined);
  }

  if (props->max_height_tag == 1 && std::isfinite(props->max_height_val) && props->max_height_val >= 0.0f) {
    YGNodeStyleSetMaxHeight(node, props->max_height_val);
  } else if (props->max_height_tag == 2 && std::isfinite(props->max_height_val) && props->max_height_val >= 0.0f) {
    YGNodeStyleSetMaxHeightPercent(node, props->max_height_val);
  } else {
    YGNodeStyleSetMaxHeight(node, YGUndefined);
  }

  YGNodeStyleSetFlexDirection(node, map_flex_direction(props->flex_direction));
  YGNodeStyleSetJustifyContent(node, map_justify(props->justify_content));
  YGNodeStyleSetAlignItems(node, map_align(props->align_items));
  YGNodeStyleSetFlexGrow(node, sanitize_non_neg(props->flex_grow));
  YGNodeStyleSetFlexShrink(node, sanitize_non_neg(props->flex_shrink));
  YGNodeStyleSetGap(node, YGGutterRow, sanitize_non_neg(props->row_gap));
  YGNodeStyleSetGap(node, YGGutterColumn, sanitize_non_neg(props->column_gap));

  YGNodeStyleSetPadding(node, YGEdgeTop, sanitize_non_neg(props->pad_top));
  YGNodeStyleSetPadding(node, YGEdgeRight, sanitize_non_neg(props->pad_right));
  YGNodeStyleSetPadding(node, YGEdgeBottom, sanitize_non_neg(props->pad_bottom));
  YGNodeStyleSetPadding(node, YGEdgeLeft, sanitize_non_neg(props->pad_left));

  YGNodeStyleSetMargin(node, YGEdgeTop, sanitize_finite(props->margin_top));
  YGNodeStyleSetMargin(node, YGEdgeRight, sanitize_finite(props->margin_right));
  YGNodeStyleSetMargin(node, YGEdgeBottom, sanitize_finite(props->margin_bottom));
  YGNodeStyleSetMargin(node, YGEdgeLeft, sanitize_finite(props->margin_left));
}

static void apply_position_edge(YGNodeRef node, YGEdge edge, uint8_t tag, float val) noexcept {
  if (tag == 1 && std::isfinite(val)) {
    YGNodeStyleSetPosition(node, edge, val);
  } else if (tag == 2 && std::isfinite(val)) {
    YGNodeStyleSetPositionPercent(node, edge, val);
  } else {
    YGNodeStyleSetPosition(node, edge, YGUndefined);
  }
}

void nve_yg_node_apply_layout_ext(YGNodeRef node, const NveYogaLayoutExt* ext) noexcept {
  if (!node) {
    return;
  }
  if (!ext) {
    YGNodeStyleSetDisplay(node, YGDisplayFlex);
    YGNodeStyleSetFlexBasisAuto(node);
    YGNodeStyleSetAspectRatio(node, YGUndefined);
    YGNodeStyleSetPositionType(node, YGPositionTypeRelative);
    YGNodeStyleSetPosition(node, YGEdgeTop, YGUndefined);
    YGNodeStyleSetPosition(node, YGEdgeRight, YGUndefined);
    YGNodeStyleSetPosition(node, YGEdgeBottom, YGUndefined);
    YGNodeStyleSetPosition(node, YGEdgeLeft, YGUndefined);
    YGNodeStyleSetDirection(node, YGDirectionInherit);
    YGNodeStyleSetPadding(node, YGEdgeStart, YGUndefined);
    YGNodeStyleSetPadding(node, YGEdgeEnd, YGUndefined);
    YGNodeStyleSetMargin(node, YGEdgeStart, YGUndefined);
    YGNodeStyleSetMargin(node, YGEdgeEnd, YGUndefined);
    YGNodeStyleSetFlexWrap(node, YGWrapNoWrap);
    YGNodeStyleSetAlignContent(node, YGAlignStretch);
    YGNodeStyleSetAlignSelf(node, YGAlignAuto);
    YGNodeStyleSetBoxSizing(node, YGBoxSizingBorderBox);
    YGNodeStyleSetOverflow(node, YGOverflowVisible);
    YGNodeStyleSetBorder(node, YGEdgeAll, 0.0f);
    return;
  }
  YGNodeStyleSetDisplay(node, ext->display == 1 ? YGDisplayNone : YGDisplayFlex);
  if (ext->flex_basis_tag == 1 && std::isfinite(ext->flex_basis_val) && ext->flex_basis_val >= 0.0f) {
    YGNodeStyleSetFlexBasis(node, ext->flex_basis_val);
  } else if (ext->flex_basis_tag == 2 && std::isfinite(ext->flex_basis_val) && ext->flex_basis_val >= 0.0f) {
    YGNodeStyleSetFlexBasisPercent(node, ext->flex_basis_val);
  } else {
    YGNodeStyleSetFlexBasisAuto(node);
  }
  if (ext->has_aspect_ratio && std::isfinite(ext->aspect_ratio) && ext->aspect_ratio > 0.0f) {
    YGNodeStyleSetAspectRatio(node, ext->aspect_ratio);
  } else {
    YGNodeStyleSetAspectRatio(node, YGUndefined);
  }
  if (ext->position_mode == 1) {
    YGNodeStyleSetPositionType(node, YGPositionTypeAbsolute);
  } else {
    YGNodeStyleSetPositionType(node, YGPositionTypeRelative);
  }
  if (ext->position_mode != 2) {
    apply_position_edge(node, YGEdgeTop, ext->top_tag, ext->top_val);
    apply_position_edge(node, YGEdgeRight, ext->right_tag, ext->right_val);
    apply_position_edge(node, YGEdgeBottom, ext->bottom_tag, ext->bottom_val);
    apply_position_edge(node, YGEdgeLeft, ext->left_tag, ext->left_val);
  } else {
    YGNodeStyleSetPosition(node, YGEdgeTop, YGUndefined);
    YGNodeStyleSetPosition(node, YGEdgeRight, YGUndefined);
    YGNodeStyleSetPosition(node, YGEdgeBottom, YGUndefined);
    YGNodeStyleSetPosition(node, YGEdgeLeft, YGUndefined);
  }
  switch (ext->direction) {
    case 1:
      YGNodeStyleSetDirection(node, YGDirectionLTR);
      break;
    case 2:
      YGNodeStyleSetDirection(node, YGDirectionRTL);
      break;
    default:
      YGNodeStyleSetDirection(node, YGDirectionInherit);
      break;
  }
  YGNodeStyleSetPadding(
      node, YGEdgeStart, ext->has_pad_start ? sanitize_non_neg(ext->pad_start) : YGUndefined);
  YGNodeStyleSetPadding(
      node, YGEdgeEnd, ext->has_pad_end ? sanitize_non_neg(ext->pad_end) : YGUndefined);
  YGNodeStyleSetMargin(
      node, YGEdgeStart, ext->has_mar_start ? sanitize_finite(ext->mar_start) : YGUndefined);
  YGNodeStyleSetMargin(
      node, YGEdgeEnd, ext->has_mar_end ? sanitize_finite(ext->mar_end) : YGUndefined);
  switch (ext->flex_wrap) {
    case 1:
      YGNodeStyleSetFlexWrap(node, YGWrapWrap);
      break;
    case 2:
      YGNodeStyleSetFlexWrap(node, YGWrapWrapReverse);
      break;
    default:
      YGNodeStyleSetFlexWrap(node, YGWrapNoWrap);
      break;
  }
  switch (ext->align_content) {
    case 1:
      YGNodeStyleSetAlignContent(node, YGAlignFlexStart);
      break;
    case 2:
      YGNodeStyleSetAlignContent(node, YGAlignCenter);
      break;
    case 3:
      YGNodeStyleSetAlignContent(node, YGAlignFlexEnd);
      break;
    case 4:
      YGNodeStyleSetAlignContent(node, YGAlignSpaceBetween);
      break;
    case 5:
      YGNodeStyleSetAlignContent(node, YGAlignSpaceAround);
      break;
    case 6:
      YGNodeStyleSetAlignContent(node, YGAlignSpaceEvenly);
      break;
    default:
      YGNodeStyleSetAlignContent(node, YGAlignStretch);
      break;
  }
  switch (ext->align_self) {
    case 1:
      YGNodeStyleSetAlignSelf(node, YGAlignStretch);
      break;
    case 2:
      YGNodeStyleSetAlignSelf(node, YGAlignFlexStart);
      break;
    case 3:
      YGNodeStyleSetAlignSelf(node, YGAlignCenter);
      break;
    case 4:
      YGNodeStyleSetAlignSelf(node, YGAlignFlexEnd);
      break;
    case 5:
      YGNodeStyleSetAlignSelf(node, YGAlignBaseline);
      break;
    default:
      YGNodeStyleSetAlignSelf(node, YGAlignAuto);
      break;
  }
  YGNodeStyleSetBoxSizing(
      node, ext->box_sizing == 1 ? YGBoxSizingContentBox : YGBoxSizingBorderBox);
  switch (ext->overflow) {
    case 1:
      YGNodeStyleSetOverflow(node, YGOverflowHidden);
      break;
    case 2:
      YGNodeStyleSetOverflow(node, YGOverflowScroll);
      break;
    default:
      YGNodeStyleSetOverflow(node, YGOverflowVisible);
      break;
  }
  YGNodeStyleSetBorder(node, YGEdgeAll, sanitize_non_neg(ext->border_width));
}

void nve_yg_node_set_fixed_size(YGNodeRef node, float width, float height) noexcept {
  if (!node) {
    return;
  }
  YGNodeStyleSetWidth(node, sanitize_non_neg(width));
  YGNodeStyleSetHeight(node, sanitize_non_neg(height));
}

void nve_yg_node_calculate_layout(YGNodeRef root, float avail_width, float avail_height) noexcept {
  if (!root) {
    return;
  }
  YGNodeCalculateLayout(
      root, sanitize_non_neg(avail_width), sanitize_non_neg(avail_height), YGDirectionLTR);
}

NveYogaRect nve_yg_node_measure_layout(
    YGNodeRef root,
    float min_w,
    float max_w,
    float min_h,
    float max_h,
    uint8_t ideal) noexcept {
  if (!root) {
    return NveYogaRect{0.0f, 0.0f, 0.0f, 0.0f};
  }
  if (ideal != 0u) {
    YGNodeCalculateLayout(root, YGUndefined, YGUndefined, YGDirectionLTR);
  } else {
    float avail_w = YGUndefined;
    float avail_h = YGUndefined;
    const bool finite_max_w = std::isfinite(max_w);
    const bool finite_max_h = std::isfinite(max_h);
    if (finite_max_w && std::fabs(max_w - min_w) <= 1e-6f) {
      YGNodeStyleSetWidth(root, min_w);
      YGNodeStyleSetMinWidth(root, min_w);
      YGNodeStyleSetMaxWidth(root, max_w);
      avail_w = max_w;
    } else {
      if (min_w > 0.0f) {
        const YGValue cur = YGNodeStyleGetMinWidth(root);
        const float eff =
            (cur.unit == YGUnitPoint && std::isfinite(cur.value)) ? std::fmax(cur.value, min_w) : min_w;
        YGNodeStyleSetMinWidth(root, eff);
      }
      if (finite_max_w) {
        const YGValue cur = YGNodeStyleGetMaxWidth(root);
        const float eff =
            (cur.unit == YGUnitPoint && std::isfinite(cur.value)) ? std::fmin(cur.value, max_w) : max_w;
        YGNodeStyleSetMaxWidth(root, std::fmax(eff, min_w));
        avail_w = max_w;
      }
    }
    if (finite_max_h && std::fabs(max_h - min_h) <= 1e-6f) {
      YGNodeStyleSetHeight(root, min_h);
      YGNodeStyleSetMinHeight(root, min_h);
      YGNodeStyleSetMaxHeight(root, max_h);
      avail_h = max_h;
    } else {
      if (min_h > 0.0f) {
        const YGValue cur = YGNodeStyleGetMinHeight(root);
        const float eff =
            (cur.unit == YGUnitPoint && std::isfinite(cur.value)) ? std::fmax(cur.value, min_h) : min_h;
        YGNodeStyleSetMinHeight(root, eff);
      }
      if (finite_max_h) {
        const YGValue cur = YGNodeStyleGetMaxHeight(root);
        const float eff =
            (cur.unit == YGUnitPoint && std::isfinite(cur.value)) ? std::fmin(cur.value, max_h) : max_h;
        YGNodeStyleSetMaxHeight(root, std::fmax(eff, min_h));
        avail_h = max_h;
      }
    }
    YGNodeCalculateLayout(root, avail_w, avail_h, YGDirectionLTR);
    const float computed_w = YGNodeLayoutGetWidth(root);
    if (min_w > 0.0f && std::isfinite(computed_w) && computed_w < min_w - 1e-5f) {
      YGNodeStyleSetWidth(root, min_w);
      YGNodeCalculateLayout(root, min_w, avail_h, YGDirectionLTR);
    }
  }
  return nve_yg_node_get_layout_rect(root);
}

NveYogaRect nve_yg_node_get_layout_rect(YGNodeConstRef node) noexcept {
  if (!node) {
    return NveYogaRect{0.0f, 0.0f, 0.0f, 0.0f};
  }
  return NveYogaRect{
      sanitize_finite(YGNodeLayoutGetLeft(node)),
      sanitize_finite(YGNodeLayoutGetTop(node)),
      sanitize_finite(YGNodeLayoutGetWidth(node)),
      sanitize_finite(YGNodeLayoutGetHeight(node)),
  };
}

} // extern "C"
