#include "nve_abi.h"
#include "nve_rn_component.hpp"
#include "command_decoder.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
#include <yoga/Yoga.h>
#endif

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
#include <react/renderer/core/EventListener.h>
#include <react/renderer/core/EventPayload.h>
#include <react/renderer/core/EventPayloadType.h>
#include <react/renderer/core/RawEvent.h>
#include <react/renderer/core/ShadowNodeFamily.h>
#include <react/renderer/mounting/ShadowTree.h>
#include <react/renderer/uimanager/UIManager.h>
#endif

namespace nve {

namespace {

constexpr uint64_t kEventMaskLayout = 1ULL << 6;

uint64_t now_us() {
  using Clock = std::chrono::steady_clock;
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::microseconds>(Clock::now().time_since_epoch())
          .count());
}

bool rect_changed(const LayoutRect& a, const LayoutRect& b) noexcept {
  constexpr float kEps = 1e-3f;
  return std::fabs(a.x - b.x) > kEps || std::fabs(a.y - b.y) > kEps ||
         std::fabs(a.width - b.width) > kEps || std::fabs(a.height - b.height) > kEps;
}

constexpr uint8_t kAlignSelfAuto = 0xFFu;

#if !(defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA)
uint8_t resolve_child_align(uint8_t parent_align_items, uint8_t child_align_self, bool is_row) noexcept {
  const uint8_t align = (child_align_self <= 4u) ? child_align_self : parent_align_items;
  if (!is_row && align == 4u /* Baseline */) {
    return 1u /* FlexStart */;
  }
  return align;
}
#endif

uint8_t parse_align_self_prop(const RawPropValue& val) noexcept {
  if (std::holds_alternative<std::nullptr_t>(val)) {
    return kAlignSelfAuto;
  }
  if (const auto* i = std::get_if<int64_t>(&val)) {
    if (*i >= 0 && *i <= 4) {
      return static_cast<uint8_t>(*i);
    }
    return kAlignSelfAuto;
  }
  if (const auto* s = std::get_if<std::string>(&val)) {
    if (*s == "stretch") return 0u;
    if (*s == "flex-start" || *s == "flexStart" || *s == "start") return 1u;
    if (*s == "center") return 2u;
    if (*s == "flex-end" || *s == "flexEnd" || *s == "end") return 3u;
    if (*s == "baseline") return 4u;
    return kAlignSelfAuto;
  }
  return kAlignSelfAuto;
}

Dimension parse_flex_basis_prop(const RawPropValue& val) noexcept {
  if (std::holds_alternative<std::nullptr_t>(val)) {
    return Dimension{0u, 0.0f};
  }
  if (const auto* i = std::get_if<int64_t>(&val)) {
    const float v = static_cast<float>(*i);
    if (is_valid_scalar(v) && v >= 0.0f) {
      return Dimension{1u, v};
    }
    return Dimension{0u, 0.0f};
  }
  if (const auto* d = std::get_if<double>(&val)) {
    const float v = static_cast<float>(*d);
    if (is_valid_scalar(v) && v >= 0.0f) {
      return Dimension{1u, v};
    }
    return Dimension{0u, 0.0f};
  }
  if (const auto* s = std::get_if<std::string>(&val)) {
    if (s->empty() || *s == "auto") {
      return Dimension{0u, 0.0f};
    }
    const bool is_percent = (s->back() == '%');
    const std::string num_str = is_percent ? s->substr(0, s->size() - 1) : *s;
    if (num_str.empty()) {
      return Dimension{0u, 0.0f};
    }
    char* end_ptr = nullptr;
    const float v = std::strtof(num_str.c_str(), &end_ptr);
    if (end_ptr != nullptr && *end_ptr == '\0' && is_valid_scalar(v) && v >= 0.0f) {
      return Dimension{is_percent ? uint8_t{2u} : uint8_t{1u}, v};
    }
  }
  return Dimension{0u, 0.0f};
}

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA

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

#endif // defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA

} // namespace

struct ShadowNodeEntry {
  uint32_t node_id{};
  uint16_t kind{};
  std::string custom_name;
  std::string text;
  LayoutProps layout{};
  StyleProps style{};
  bool has_explicit_style{false};
  LayoutRect computed_layout{};
  bool has_computed_layout{false};
  bool raw_props_dirty{false};
  RawPropMap raw_props;
  uint64_t event_mask{0};
  std::vector<uint32_t> children;
  bool has_parent{false};
  uint32_t parent_id{0};
  uint64_t visit_gen{0};
  const TextMeasurerCallback* text_measurer{nullptr};
  uint8_t align_self{kAlignSelfAuto};
  Dimension flex_basis{0u, 0.0f};
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  YGNodeRef yg_node{nullptr};
#else
  uint64_t measure_gen{0};
  float cached_avail_w{-2.0f};
  float cached_avail_h{-2.0f};
  uint8_t cached_w_mode{0};
  uint8_t cached_h_mode{0};
  NveIntrinsicSize cached_measure{};
  uint64_t baseline_gen{0};
  float cached_baseline_w{-2.0f};
  float cached_baseline_h{-2.0f};
  float cached_baseline{0.0f};
#endif
};

static float compute_text_leaf_baseline(const ShadowNodeEntry& entry, float height) noexcept {
  if (entry.text.empty()) {
    return (std::isfinite(height) && height >= 0.0f) ? height : 0.0f;
  }
  const float font_size =
      (std::isfinite(entry.style.font_size) && entry.style.font_size > 0.0f)
          ? entry.style.font_size
          : 16.0f;
  const float pad_t = std::max(0.0f, entry.layout.padding.top);
  const float border_t =
      (std::isfinite(entry.style.border_width) && entry.style.border_width > 0.0f)
          ? entry.style.border_width
          : 0.0f;
  return pad_t + border_t + font_size * 0.8f;
}

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
static float baseline_text_node(YGNodeConstRef node, float /*width*/, float height) {
  const auto* entry = static_cast<const ShadowNodeEntry*>(YGNodeGetContext(node));
  if (!entry) {
    return (std::isfinite(height) && height >= 0.0f) ? height : 0.0f;
  }
  return compute_text_leaf_baseline(*entry, height);
}

static YGSize measure_text_node(
    YGNodeConstRef node,
    float width,
    YGMeasureMode width_mode,
    float height,
    YGMeasureMode height_mode) {
  const auto* entry = static_cast<const ShadowNodeEntry*>(YGNodeGetContext(node));
  const float font_size =
      (entry && std::isfinite(entry->style.font_size) && entry->style.font_size > 0.0f)
          ? entry->style.font_size
          : 16.0f;
  const size_t len = entry ? entry->text.size() : 0;

  float out_w = 0.0f;
  float out_h = 0.0f;
  if (len > 0 && entry && entry->text_measurer && *entry->text_measurer) {
    const float max_w = (width_mode == YGMeasureModeUndefined) ? 0.0f : width;
    const NveIntrinsicSize measured = (*entry->text_measurer)(entry->text, font_size, max_w);
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
  float intrinsic_w = len > 0 ? (static_cast<float>(len) * char_width + 4.0f) : 0.0f;
  float intrinsic_h = len > 0 ? line_height : 0.0f;

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
#endif

class FabricHost {
 public:
  explicit FabricHost(NveSurfaceConfig config) : config_(config) {
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    yg_config_ = YGConfigNew();
    if (config_.scale_factor > 0.0f) {
      YGConfigSetPointScaleFactor(yg_config_, config_.scale_factor);
    }
#endif
    ShadowNodeEntry root{};
    root.node_id = NVE_ROOT_NODE_ID;
    root.kind = NVE_COMP_ROOT;
    auto [it, _] = nodes_.emplace(NVE_ROOT_NODE_ID, std::move(root));
    init_node_yoga(it->second);
    event_ring_.clear();
  }

  ~FabricHost() {
#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
    detach_rn_uimanager();
#endif
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    for (auto& [_, entry] : nodes_) {
      if (entry.yg_node) {
        YGNodeRemoveAllChildren(entry.yg_node);
      }
    }
    for (auto& [_, entry] : nodes_) {
      if (entry.yg_node) {
        YGNodeFree(entry.yg_node);
        entry.yg_node = nullptr;
      }
    }
    if (yg_config_) {
      YGConfigFree(yg_config_);
      yg_config_ = nullptr;
    }
#endif
  }

  void set_surface_size(float width, float height, float scale_factor) {
    config_.width = width;
    config_.height = height;
    if (scale_factor > 0.0f) {
      config_.scale_factor = scale_factor;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
      if (yg_config_) {
        YGConfigSetPointScaleFactor(yg_config_, scale_factor);
      }
#endif
    }
    run_layout_pass();
  }

  void set_text_measurer(TextMeasurerCallback measurer) {
    text_measurer_ = std::move(measurer);
    for (auto& [_, entry] : nodes_) {
      entry.text_measurer = &text_measurer_;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
      if (entry.yg_node && YGNodeHasMeasureFunc(entry.yg_node)) {
        YGNodeMarkDirty(entry.yg_node);
      }
#endif
    }
  }

  void register_custom_component(CustomComponentDescriptor descriptor) {
    if (descriptor.component_name.empty()) return;
    custom_components_[descriptor.component_name] = std::move(descriptor);
  }

  void enqueue_press(uint32_t node_id, uint64_t ts_us, float x, float y) {
    std::lock_guard<std::mutex> lock(event_mutex_);
    event_ring_.push_press(config_.surface_id, node_id, ts_us, x, y);
  }

  void enqueue_text_change(uint32_t node_id, uint64_t ts_us, std::string_view text) {
    std::lock_guard<std::mutex> lock(event_mutex_);
    event_ring_.push_text_change(config_.surface_id, node_id, ts_us, text);
  }

  void enqueue_custom_event(
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const uint8_t* data,
      size_t len) {
    std::lock_guard<std::mutex> lock(event_mutex_);
    event_ring_.push_custom(config_.surface_id, node_id, ts_us, name, data, len);
  }

  bool get_node_layout(uint32_t node_id, LayoutRect* out_rect) const {
    if (!out_rect) return false;
    auto it = nodes_.find(node_id);
    if (it == nodes_.end() || !it->second.has_computed_layout) {
      return false;
    }
    *out_rect = it->second.computed_layout;
    return true;
  }

  std::vector<RenderedNodeSnapshot> snapshot_nodes() const {
    std::vector<RenderedNodeSnapshot> out;
    out.reserve(nodes_.size());
    for (const auto& [_, entry] : nodes_) {
      RenderedNodeSnapshot snap{};
      snap.node_id = entry.node_id;
      snap.kind = entry.kind;
      snap.custom_name = entry.custom_name;
      snap.text = entry.text;
      snap.layout = entry.computed_layout;
      snap.background_rgba = entry.style.background_rgba;
      snap.text_color_rgba = entry.style.text_color_rgba;
      snap.border_color_rgba = entry.style.border_color_rgba;
      snap.border_width = entry.style.border_width;
      snap.border_radius = entry.style.border_radius;
      snap.opacity = entry.style.opacity;
      snap.font_size = entry.style.font_size;
      snap.z_index = entry.style.z_index;
      snap.event_mask = entry.event_mask;
      snap.children = entry.children;
      out.push_back(std::move(snap));
    }
    return out;
  }

  std::vector<uint8_t> serialize_frame0(uint64_t frame_id) const {
    std::vector<uint32_t> ids;
    ids.reserve(nodes_.size());
    for (const auto& [id, _] : nodes_) {
      ids.push_back(id);
    }
    std::sort(ids.begin(), ids.end());

    CommandBufferEncoder enc(config_.surface_id, frame_id);
    enc.reserve(ids.size() * 160u + 64u);

    // 1. Create all non-root nodes in deterministic ID order.
    for (uint32_t id : ids) {
      if (id == NVE_ROOT_NODE_ID) {
        continue;
      }
      const ShadowNodeEntry& entry = nodes_.at(id);
      enc.create_node(entry.node_id, entry.kind, entry.custom_name);
    }

    // 2. Emit hierarchy (`SET_CHILDREN`) for root and all parent nodes.
    for (uint32_t id : ids) {
      const ShadowNodeEntry& entry = nodes_.at(id);
      if (!entry.children.empty()) {
        enc.set_children(entry.node_id, entry.children);
      }
    }

    // 3. Emit layout, explicit style, text, event masks, and raw props.
    for (uint32_t id : ids) {
      const ShadowNodeEntry& entry = nodes_.at(id);
      if (id != NVE_ROOT_NODE_ID) {
        enc.set_layout(entry.node_id, entry.layout);
      }
      if (entry.has_explicit_style) {
        enc.set_style(entry.node_id, entry.style);
      }
      if (!entry.text.empty()) {
        enc.set_text(entry.node_id, entry.text);
      }
      if (entry.event_mask != 0u) {
        enc.set_event_mask(entry.node_id, entry.event_mask);
      }
      if (!entry.raw_props.empty()) {
        std::vector<std::string_view> keys;
        keys.reserve(entry.raw_props.size());
        for (const auto& [k, _] : entry.raw_props) {
          keys.push_back(k);
        }
        std::sort(keys.begin(), keys.end());
        for (std::string_view key : keys) {
          const RawPropValue& val = entry.raw_props.at(std::string(key));
          std::visit(
              [&](const auto& v) {
                using T = std::decay_t<decltype(v)>;
                if constexpr (std::is_same_v<T, std::nullptr_t>) {
                  enc.set_raw_prop_null(entry.node_id, key);
                } else if constexpr (std::is_same_v<T, bool>) {
                  enc.set_raw_prop_bool(entry.node_id, key, v);
                } else if constexpr (std::is_same_v<T, int64_t>) {
                  enc.set_raw_prop_i64(entry.node_id, key, v);
                } else if constexpr (std::is_same_v<T, double>) {
                  enc.set_raw_prop_f64(entry.node_id, key, v);
                } else if constexpr (std::is_same_v<T, std::string>) {
                  enc.set_raw_prop_string(entry.node_id, key, v);
                } else if constexpr (std::is_same_v<T, std::vector<uint8_t>>) {
                  enc.set_raw_prop_bytes(entry.node_id, key, v);
                }
              },
              val);
        }
      }
    }

    enc.commit(true);
    (void)enc.finish();
    return std::move(enc.raw());
  }

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
  void attach_rn_uimanager(std::shared_ptr<facebook::react::UIManager> ui_manager) {
    detach_rn_uimanager();
    rn_ui_manager_ = std::move(ui_manager);
    if (!rn_ui_manager_) return;

    rn_event_listener_ = std::make_shared<const facebook::react::EventListener>(
        [this](const facebook::react::RawEvent& raw_event) -> bool {
          return this->intercept_rn_raw_event(raw_event);
        });
    rn_ui_manager_->addEventListener(rn_event_listener_);
  }

  void detach_rn_uimanager() {
    if (rn_ui_manager_ && rn_event_listener_) {
      rn_ui_manager_->removeEventListener(rn_event_listener_);
    }
    rn_event_listener_.reset();
    rn_ui_manager_.reset();
  }
#endif

  int32_t submit_frame(NveBufferSlice slice, NveFrameStats* out_stats) {
    using Clock = std::chrono::steady_clock;
    const auto t0 = Clock::now();

    if (slice.ptr == nullptr && slice.len > 0) {
      return -1;
    }

    BinaryReader r(slice.ptr, slice.len);
    Header h{};
    if (!r.read(h.magic) || !r.read(h.version) || !r.read(h.flags) || !r.read(h.surface_id) ||
        !r.read(h.frame_id) || !r.read(h.op_count)) {
      return -1;
    }
    if (h.magic != NVE_ABI_MAGIC || h.version != NVE_ABI_VERSION) {
      return -2;
    }

    uint32_t mutations = 0;
    bool committed = false;
    bool sync_mount = false;

    for (uint32_t i = 0; i < h.op_count; ++i) {
      uint8_t op = 0;
      if (!r.read(op)) return -3;
      switch (static_cast<NveOpcode>(op)) {
        case NVE_OP_CREATE_NODE: {
          uint32_t id = 0;
          uint16_t kind = 0;
          uint16_t name_len = 0;
          std::string_view name;
          if (!r.read(id) || !r.read(kind) || !r.read(name_len) || !r.read_string(name_len, name)) {
            return -4;
          }
          if (!is_valid_component_kind(kind)) {
            return -6;
          }
          destroy_node_if_exists(id);
          ShadowNodeEntry entry{};
          entry.node_id = id;
          entry.kind = kind;
          entry.custom_name = std::string(name);
          auto [it, _] = nodes_.emplace(id, std::move(entry));
          init_node_yoga(it->second);
          ++mutations;
          break;
        }
        case NVE_OP_REMOVE_NODE: {
          uint32_t id = 0;
          if (!r.read(id)) return -4;
          if (id == NVE_ROOT_NODE_ID) {
            return -5;
          }
          destroy_node_if_exists(id);
          ++mutations;
          break;
        }
        case NVE_OP_SET_CHILDREN: {
          uint32_t parent_id = 0;
          uint32_t count = 0;
          if (!r.read(parent_id) || !r.read(count)) return -4;
          if (static_cast<size_t>(count) > r.remaining() / sizeof(uint32_t)) return -4;
          std::vector<uint32_t> children(count);
          for (uint32_t c = 0; c < count; ++c) {
            if (!r.read(children[c])) return -4;
          }
          auto it = nodes_.find(parent_id);
          if (it == nodes_.end()) return -5;

          const uint64_t ancestor_gen = ++gen_counter_;
          uint32_t walk_id = parent_id;
          bool walking = true;
          for (size_t hops = 0; walking && hops <= nodes_.size(); ++hops) {
            auto wit = nodes_.find(walk_id);
            if (wit == nodes_.end()) break;
            wit->second.visit_gen = ancestor_gen;
            walking = wit->second.has_parent;
            walk_id = wit->second.parent_id;
          }

          const uint64_t dup_gen = ++gen_counter_;
          for (uint32_t cid : children) {
            if (cid == parent_id || cid == NVE_ROOT_NODE_ID) {
              return -6;
            }
            auto cit = nodes_.find(cid);
            if (cit != nodes_.end()) {
              if (cit->second.visit_gen == ancestor_gen || cit->second.visit_gen == dup_gen) {
                return -6;
              }
              cit->second.visit_gen = dup_gen;
            }
          }

          for (uint32_t old_cid : it->second.children) {
            auto oit = nodes_.find(old_cid);
            if (oit != nodes_.end() && oit->second.has_parent && oit->second.parent_id == parent_id) {
              oit->second.has_parent = false;
              oit->second.parent_id = 0;
            }
          }
          for (uint32_t cid : children) {
            auto cit = nodes_.find(cid);
            if (cit != nodes_.end()) {
              if (cit->second.has_parent && cit->second.parent_id != parent_id) {
                auto prev_pit = nodes_.find(cit->second.parent_id);
                if (prev_pit != nodes_.end()) {
                  std::erase(prev_pit->second.children, cid);
                }
              }
              cit->second.has_parent = true;
              cit->second.parent_id = parent_id;
            }
          }

          it->second.children = std::move(children);
          sync_children_yoga(it->second);
          ++mutations;
          break;
        }
        case NVE_OP_SET_TEXT: {
          uint32_t id = 0;
          uint32_t len = 0;
          std::string_view text;
          if (!r.read(id) || !r.read(len) || !r.read_string(len, text)) return -4;
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          it->second.text = std::string(text);
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
          if (it->second.yg_node && YGNodeHasMeasureFunc(it->second.yg_node)) {
            YGNodeMarkDirty(it->second.yg_node);
          }
#endif
          ++mutations;
          break;
        }
        case NVE_OP_SET_LAYOUT: {
          uint32_t id = 0;
          LayoutProps layout{};
          if (!r.read(id) || !r.read_layout(layout)) return -4;
          if (!is_valid_layout(layout)) return -6;
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          it->second.layout = layout;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
          if (it->second.yg_node) {
            apply_yg_layout_props(it->second.yg_node, layout);
          }
#endif
          ++mutations;
          break;
        }
        case NVE_OP_SET_STYLE: {
          uint32_t id = 0;
          StyleProps style{};
          if (!r.read(id) || !r.read_style(style)) return -4;
          if (!is_valid_style(style)) return -6;
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          it->second.style = style;
          it->second.has_explicit_style = true;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
          if (it->second.yg_node && YGNodeHasMeasureFunc(it->second.yg_node)) {
            YGNodeMarkDirty(it->second.yg_node);
          }
#endif
          ++mutations;
          break;
        }
        case NVE_OP_SET_RAW_PROP: {
          uint32_t id = 0;
          uint16_t key_len = 0;
          std::string_view key;
          uint8_t tag = 0;
          if (!r.read(id) || !r.read(key_len) || !r.read_string(key_len, key) || !r.read(tag)) {
            return -4;
          }
          RawPropValue prop_val{nullptr};
          switch (tag) {
            case 0:
              prop_val = nullptr;
              break;
            case 1: {
              uint8_t b = 0;
              if (!r.read(b)) return -4;
              prop_val = (b != 0);
              break;
            }
            case 2: {
              int64_t v = 0;
              if (!r.read(v)) return -4;
              prop_val = v;
              break;
            }
            case 3: {
              double v = 0.0;
              if (!r.read(v)) return -4;
              prop_val = v;
              break;
            }
            case 4: {
              uint32_t vlen = 0;
              std::string_view sv;
              if (!r.read(vlen) || !r.read_string(vlen, sv)) return -4;
              prop_val = std::string(sv);
              break;
            }
            case 5: {
              uint32_t vlen = 0;
              std::vector<uint8_t> bytes;
              if (!r.read(vlen) || !r.read_bytes(vlen, bytes)) return -4;
              prop_val = std::move(bytes);
              break;
            }
            default:
              return -6;
          }
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          if (key == "alignSelf" || key == "align_self") {
            it->second.align_self = parse_align_self_prop(prop_val);
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
            if (it->second.yg_node) {
              apply_yg_align_self(it->second.yg_node, it->second.align_self);
            }
#endif
          } else if (key == "flexBasis" || key == "flex_basis") {
            it->second.flex_basis = parse_flex_basis_prop(prop_val);
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
            if (it->second.yg_node) {
              apply_yg_flex_basis(it->second.yg_node, it->second.flex_basis);
            }
#endif
          }
          it->second.raw_props[std::string(key)] = std::move(prop_val);
          it->second.raw_props_dirty = true;
          ++mutations;
          break;
        }
        case NVE_OP_SET_EVENT_MASK: {
          uint32_t id = 0;
          uint64_t mask = 0;
          if (!r.read(id) || !r.read(mask)) return -4;
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          it->second.event_mask = mask;
          ++mutations;
          break;
        }
        case NVE_OP_DISPATCH_COMMAND: {
          uint32_t id = 0;
          uint16_t nlen = 0;
          uint32_t plen = 0;
          std::string_view name;
          std::vector<uint8_t> payload;
          if (!r.read(id) || !r.read(nlen) || !r.read_string(nlen, name) || !r.read(plen) ||
              !r.read_bytes(plen, payload)) {
            return -4;
          }
          auto it = nodes_.find(id);
          if (it == nodes_.end()) return -5;
          auto comp_it = custom_components_.find(it->second.custom_name);
          if (comp_it != custom_components_.end() && comp_it->second.on_dispatch_command) {
            comp_it->second.on_dispatch_command(
                id, name, NveBufferSlice{payload.data(), payload.size()});
          }
          break;
        }
        case NVE_OP_COMMIT: {
          uint8_t sync_flag = 0;
          if (!r.read(sync_flag)) return -4;
          sync_mount = (sync_flag != 0);
          committed = true;
          break;
        }
        default:
          return -7;
      }
    }

    if (r.remaining() != 0) {
      return -8;
    }

    const auto t_decode = Clock::now();
    const auto decode_us =
        static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(t_decode - t0).count());

    uint64_t layout_us = 0;
    uint64_t diff_us = 0;
    uint64_t mount_us = 0;

    if (committed) {
      const auto t_layout_start = Clock::now();
      run_layout_pass();
      const auto t_layout_end = Clock::now();
      layout_us = static_cast<uint64_t>(
          std::chrono::duration_cast<std::chrono::microseconds>(t_layout_end - t_layout_start).count());

      flush_custom_component_props();

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
      const auto t_mount_start = Clock::now();
      commit_rn_shadow_tree(sync_mount);
      const auto t_mount_end = Clock::now();
      mount_us = static_cast<uint64_t>(
          std::chrono::duration_cast<std::chrono::microseconds>(t_mount_end - t_mount_start).count());
#else
      (void)sync_mount;
#endif
    }

    if (out_stats != nullptr) {
      out_stats->op_count = h.op_count;
      out_stats->mutation_count = mutations;
      out_stats->decode_us = decode_us;
      out_stats->layout_us = layout_us;
      out_stats->diff_us = diff_us;
      out_stats->mount_us = mount_us;
    }
    return 0;
  }

  NveBufferSlice drain_events() {
    std::lock_guard<std::mutex> lock(event_mutex_);
    drained_snapshot_ = event_ring_.finalize();
    event_ring_.clear();
    return NveBufferSlice{drained_snapshot_.data(), drained_snapshot_.size()};
  }

  uint32_t node_count() const noexcept { return static_cast<uint32_t>(nodes_.size()); }

 private:
  void init_node_yoga(ShadowNodeEntry& entry) {
    entry.text_measurer = &text_measurer_;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    entry.yg_node = YGNodeNewWithConfig(yg_config_);
    YGNodeSetContext(entry.yg_node, &entry);
    apply_yg_layout_props(entry.yg_node, entry.layout);
    apply_yg_align_self(entry.yg_node, entry.align_self);
    apply_yg_flex_basis(entry.yg_node, entry.flex_basis);
    if (entry.kind == NVE_COMP_TEXT && entry.children.empty()) {
      YGNodeSetNodeType(entry.yg_node, YGNodeTypeText);
      YGNodeSetMeasureFunc(entry.yg_node, &measure_text_node);
      YGNodeSetBaselineFunc(entry.yg_node, &baseline_text_node);
    }
#else
    (void)entry;
#endif
  }

  void destroy_node_if_exists(uint32_t id) {
    auto it = nodes_.find(id);
    if (it == nodes_.end()) return;
    if (it->second.has_parent) {
      auto pit = nodes_.find(it->second.parent_id);
      if (pit != nodes_.end()) {
        std::erase(pit->second.children, id);
      }
    }
    for (uint32_t cid : it->second.children) {
      auto cit = nodes_.find(cid);
      if (cit != nodes_.end() && cit->second.has_parent && cit->second.parent_id == id) {
        cit->second.has_parent = false;
        cit->second.parent_id = 0;
      }
    }
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    if (YGNodeRef yg = it->second.yg_node) {
      if (YGNodeRef owner = YGNodeGetOwner(yg)) {
        YGNodeRemoveChild(owner, yg);
      }
      YGNodeRemoveAllChildren(yg);
      YGNodeFree(yg);
      it->second.yg_node = nullptr;
    }
#endif
    nodes_.erase(it);
  }

  void sync_children_yoga(ShadowNodeEntry& parent) {
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
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
      auto cit = nodes_.find(child_id);
      if (cit == nodes_.end() || !cit->second.yg_node) continue;
      YGNodeRef child_yg = cit->second.yg_node;
      if (YGNodeRef prev_owner = YGNodeGetOwner(child_yg)) {
        YGNodeRemoveChild(prev_owner, child_yg);
      }
      YGNodeInsertChild(parent.yg_node, child_yg, YGNodeGetChildCount(parent.yg_node));
    }
    if (parent.kind == NVE_COMP_TEXT && YGNodeGetChildCount(parent.yg_node) == 0) {
      YGNodeSetMeasureFunc(parent.yg_node, &measure_text_node);
      YGNodeSetBaselineFunc(parent.yg_node, &baseline_text_node);
    }
#else
    (void)parent;
#endif
  }

  void propagate_text_styles() {
    struct Frame {
      uint32_t node_id;
      uint32_t inherited_color;
      float inherited_font_size;
      bool has_inherited;
    };
    const uint64_t pass_gen = ++gen_counter_;
    std::vector<Frame> stack;
    stack.reserve(64);
    stack.push_back(Frame{NVE_ROOT_NODE_ID, 0xFF000000u, 16.0f, false});

    while (!stack.empty()) {
      const Frame f = stack.back();
      stack.pop_back();

      auto it = nodes_.find(f.node_id);
      if (it == nodes_.end()) continue;
      auto& entry = it->second;
      if (entry.visit_gen == pass_gen) continue;
      entry.visit_gen = pass_gen;

      uint32_t next_color = f.inherited_color;
      float next_font_size = f.inherited_font_size;
      bool next_has_inherited = f.has_inherited;

      if (entry.kind == NVE_COMP_TEXT) {
        if (!entry.has_explicit_style) {
          const uint32_t target_color = f.has_inherited ? f.inherited_color : 0xFF000000u;
          const float target_font_size = f.has_inherited ? f.inherited_font_size : 16.0f;
          if (entry.style.text_color_rgba != target_color ||
              std::fabs(entry.style.font_size - target_font_size) > 1e-3f) {
            entry.style.text_color_rgba = target_color;
            entry.style.font_size = target_font_size;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
            if (entry.yg_node && YGNodeHasMeasureFunc(entry.yg_node)) {
              YGNodeMarkDirty(entry.yg_node);
            }
#endif
          }
        }
        next_color = entry.style.text_color_rgba;
        next_font_size = entry.style.font_size;
        next_has_inherited = true;
      }

      for (auto rit = entry.children.rbegin(); rit != entry.children.rend(); ++rit) {
        stack.push_back(Frame{*rit, next_color, next_font_size, next_has_inherited});
      }
    }
  }

  void run_layout_pass() {
    auto root_it = nodes_.find(NVE_ROOT_NODE_ID);
    if (root_it == nodes_.end()) return;
    propagate_text_styles();

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
    YGNodeRef root_yg = root_it->second.yg_node;
    if (!root_yg) return;
    const float yg_surf_w =
        (std::isfinite(config_.width) && config_.width >= 0.0f) ? config_.width : YGUndefined;
    const float yg_surf_h =
        (std::isfinite(config_.height) && config_.height >= 0.0f) ? config_.height : YGUndefined;
    if (root_it->second.layout.width.tag == 0) {
      YGNodeStyleSetWidth(root_yg, yg_surf_w);
    }
    if (root_it->second.layout.height.tag == 0) {
      YGNodeStyleSetHeight(root_yg, yg_surf_h);
    }
    YGNodeCalculateLayout(root_yg, yg_surf_w, yg_surf_h, YGDirectionLTR);

    const uint64_t ts = now_us();
    auto finite_or_zero = [](float v) noexcept -> float { return std::isfinite(v) ? v : 0.0f; };
    for (auto& [_, entry] : nodes_) {
      if (!entry.yg_node) continue;
      const LayoutRect next_rect{
          .x = finite_or_zero(YGNodeLayoutGetLeft(entry.yg_node)),
          .y = finite_or_zero(YGNodeLayoutGetTop(entry.yg_node)),
          .width = std::max(0.0f, finite_or_zero(YGNodeLayoutGetWidth(entry.yg_node))),
          .height = std::max(0.0f, finite_or_zero(YGNodeLayoutGetHeight(entry.yg_node))),
      };
      const bool changed = !entry.has_computed_layout || rect_changed(entry.computed_layout, next_rect);
      entry.computed_layout = next_rect;
      entry.has_computed_layout = true;
      const bool had_new = YGNodeGetHasNewLayout(entry.yg_node);
      if (had_new) {
        YGNodeSetHasNewLayout(entry.yg_node, false);
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
    }
#else
    const uint64_t ts = now_us();
    const uint64_t measure_gen = ++gen_counter_;
    const bool surf_w_ok = std::isfinite(config_.width) && config_.width >= 0.0f;
    const bool surf_h_ok = std::isfinite(config_.height) && config_.height >= 0.0f;
    const float surf_w_base = surf_w_ok ? config_.width : kUnconstrained;
    const float surf_h_base = surf_h_ok ? config_.height : kUnconstrained;
    float root_w = resolve_definite_dim(root_it->second.layout.width, surf_w_base);
    float root_h = resolve_definite_dim(root_it->second.layout.height, surf_h_base);
    if (root_w < 0.0f && surf_w_ok) {
      root_w = config_.width;
    }
    if (root_h < 0.0f && surf_h_ok) {
      root_h = config_.height;
    }
    if (root_w < 0.0f || root_h < 0.0f) {
      const NveIntrinsicSize root_measured = measure_intrinsic_size(
          NVE_ROOT_NODE_ID,
          (root_w >= 0.0f) ? root_w : 0.0f,
          (root_w >= 0.0f) ? MeasureMode::Exact : MeasureMode::Unconstrained,
          (root_h >= 0.0f) ? root_h : 0.0f,
          (root_h >= 0.0f) ? MeasureMode::Exact : MeasureMode::Unconstrained,
          measure_gen,
          0);
      if (root_w < 0.0f) root_w = root_measured.width;
      if (root_h < 0.0f) root_h = root_measured.height;
    }
    compute_fallback_layout(NVE_ROOT_NODE_ID, 0.0f, 0.0f, root_w, root_h, ts, measure_gen);
#endif
  }

#if !(defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA)
  static constexpr float kUnconstrained = -1.0f;

  enum class MeasureMode : uint8_t {
    Unconstrained = 0,
    AtMost = 1,
    Exact = 2,
  };

  struct FlexLineResult {
    std::vector<uint32_t> live_children;
    std::vector<float> child_main;
    std::vector<float> child_cross;
    std::vector<float> child_x;
    std::vector<float> child_y;
    float content_main{0.0f};
    float content_cross{0.0f};
    bool is_baseline_layout{false};
  };

  static float sanitize_non_negative(float v) noexcept {
    return (std::isfinite(v) && v > 0.0f) ? v : 0.0f;
  }

  static float resolve_definite_dim(const Dimension& d, float parent_size) noexcept {
    if (!std::isfinite(d.value)) return kUnconstrained;
    switch (d.tag) {
      case 1:
        return std::max(0.0f, d.value);
      case 2:
        return (std::isfinite(parent_size) && parent_size >= 0.0f)
                   ? std::max(0.0f, parent_size * (d.value * 0.01f))
                   : kUnconstrained;
      default:
        return kUnconstrained;
    }
  }

  static float clamp_axis_size(
      float val,
      const Dimension& min_d,
      const Dimension& max_d,
      float parent_size) noexcept {
    val = sanitize_non_negative(val);
    const float max_v = resolve_definite_dim(max_d, parent_size);
    if (max_v >= 0.0f) {
      val = std::min(val, max_v);
    }
    const float min_v = resolve_definite_dim(min_d, parent_size);
    if (min_v >= 0.0f) {
      val = std::max(val, min_v);
    }
    return sanitize_non_negative(val);
  }

  NveIntrinsicSize measure_text_fallback(
      const ShadowNodeEntry& entry,
      float max_w,
      MeasureMode w_mode) const {
    const float font_size =
        (std::isfinite(entry.style.font_size) && entry.style.font_size > 0.0f)
            ? entry.style.font_size
            : 16.0f;
    const bool has_bound =
        (w_mode != MeasureMode::Unconstrained) && std::isfinite(max_w) && max_w >= 0.0f;
    const float bound_w = has_bound ? max_w : 0.0f;

    const size_t len = entry.text.size();
    if (len == 0) {
      return NveIntrinsicSize{(w_mode == MeasureMode::Exact && has_bound) ? bound_w : 0.0f, 0.0f};
    }

    if (text_measurer_) {
      const NveIntrinsicSize measured = text_measurer_(entry.text, font_size, bound_w);
      float out_w = sanitize_non_negative(measured.width);
      const float out_h = sanitize_non_negative(measured.height);
      if (w_mode == MeasureMode::Exact && has_bound) {
        out_w = bound_w;
      } else if (w_mode == MeasureMode::AtMost && has_bound) {
        out_w = std::min(out_w, bound_w);
      }
      return NveIntrinsicSize{out_w, out_h};
    }

    const float char_width = font_size * 0.55f;
    const float line_height = font_size * 1.3f;
    const float intrinsic_w = static_cast<float>(len) * char_width + 4.0f;
    float out_w = intrinsic_w;
    if (w_mode == MeasureMode::Exact && has_bound) {
      out_w = bound_w;
    } else if (w_mode == MeasureMode::AtMost && has_bound) {
      out_w = std::min(intrinsic_w, bound_w);
    }
    float out_h = line_height;
    if (out_w > 0.0f && intrinsic_w > out_w) {
      out_h = std::ceil(intrinsic_w / out_w) * line_height;
    }
    return NveIntrinsicSize{out_w, out_h};
  }

  float compute_node_baseline(
      uint32_t node_id,
      float w,
      float h,
      uint64_t measure_gen,
      uint32_t depth) {
    const float safe_w = sanitize_non_negative(w);
    const float safe_h = sanitize_non_negative(h);
    if (depth > 512) return safe_h;
    auto it = nodes_.find(node_id);
    if (it == nodes_.end()) return safe_h;
    ShadowNodeEntry& node = it->second;

    if (node.kind == NVE_COMP_TEXT && node.children.empty()) {
      return compute_text_leaf_baseline(node, safe_h);
    }
    if (node.children.empty()) {
      return safe_h;
    }

    if (node.baseline_gen == measure_gen &&
        std::fabs(node.cached_baseline_w - safe_w) <= 1e-3f &&
        std::fabs(node.cached_baseline_h - safe_h) <= 1e-3f) {
      return node.cached_baseline;
    }

    const LayoutProps& nl = node.layout;
    const float pad_w = sanitize_non_negative(nl.padding.left) + sanitize_non_negative(nl.padding.right);
    const float pad_h = sanitize_non_negative(nl.padding.top) + sanitize_non_negative(nl.padding.bottom);
    const float inner_w = std::max(0.0f, safe_w - pad_w);
    const float inner_h = std::max(0.0f, safe_h - pad_h);

    const FlexLineResult line = resolve_flex_line(
        node,
        inner_w,
        MeasureMode::Exact,
        inner_h,
        MeasureMode::Exact,
        /*compute_positions=*/true,
        measure_gen,
        depth + 1);

    float result = safe_h;
    if (!line.live_children.empty()) {
      const bool is_row = (nl.flex_direction == 1 || nl.flex_direction == 3);
      size_t baseline_idx = 0;
      for (size_t i = 0; i < line.live_children.size(); ++i) {
        const auto& child = nodes_.at(line.live_children[i]);
        if (resolve_child_align(nl.align_items, child.align_self, is_row) == 4u /* Baseline */) {
          baseline_idx = i;
          break;
        }
      }
      const float child_w = is_row ? line.child_main[baseline_idx] : line.child_cross[baseline_idx];
      const float child_h = is_row ? line.child_cross[baseline_idx] : line.child_main[baseline_idx];
      const float child_b = compute_node_baseline(
          line.live_children[baseline_idx], child_w, child_h, measure_gen, depth + 1);
      result = line.child_y[baseline_idx] + child_b;
    }

    node.baseline_gen = measure_gen;
    node.cached_baseline_w = safe_w;
    node.cached_baseline_h = safe_h;
    node.cached_baseline = result;
    return result;
  }

  FlexLineResult resolve_flex_line(
      const ShadowNodeEntry& node,
      float inner_w,
      MeasureMode inner_w_mode,
      float inner_h,
      MeasureMode inner_h_mode,
      bool compute_positions,
      uint64_t measure_gen,
      uint32_t depth) {
    FlexLineResult res{};
    if (depth > 512) return res;

    res.live_children.reserve(node.children.size());
    for (uint32_t cid : node.children) {
      if (nodes_.find(cid) != nodes_.end()) {
        res.live_children.push_back(cid);
      }
    }
    const size_t count = res.live_children.size();
    if (count == 0) return res;

    const LayoutProps& nl = node.layout;
    const bool is_row = (nl.flex_direction == 1 || nl.flex_direction == 3);
    const bool is_reverse = (nl.flex_direction == 2 || nl.flex_direction == 3);
    const float pad_l = sanitize_non_negative(nl.padding.left);
    const float pad_t = sanitize_non_negative(nl.padding.top);
    const float gap = sanitize_non_negative(nl.gap);
    const float total_gap = (count > 1) ? gap * static_cast<float>(count - 1) : 0.0f;

    const bool w_bounded = (inner_w_mode != MeasureMode::Unconstrained) && std::isfinite(inner_w);
    const bool h_bounded = (inner_h_mode != MeasureMode::Unconstrained) && std::isfinite(inner_h);
    const MeasureMode eff_w_mode = w_bounded ? inner_w_mode : MeasureMode::Unconstrained;
    const MeasureMode eff_h_mode = h_bounded ? inner_h_mode : MeasureMode::Unconstrained;
    const float safe_inner_w = w_bounded ? std::max(0.0f, inner_w) : 0.0f;
    const float safe_inner_h = h_bounded ? std::max(0.0f, inner_h) : 0.0f;

    const MeasureMode main_mode = is_row ? eff_w_mode : eff_h_mode;
    const MeasureMode cross_mode = is_row ? eff_h_mode : eff_w_mode;
    const float main_avail = is_row ? safe_inner_w : safe_inner_h;
    const float cross_avail = is_row ? safe_inner_h : safe_inner_w;
    const float main_pct_base = (main_mode == MeasureMode::Exact) ? main_avail : kUnconstrained;
    const float cross_pct_base = (cross_mode == MeasureMode::Exact) ? cross_avail : kUnconstrained;

    std::vector<uint8_t> child_align(count, 0u);
    for (size_t i = 0; i < count; ++i) {
      const auto& child = nodes_.at(res.live_children[i]);
      child_align[i] = resolve_child_align(nl.align_items, child.align_self, is_row);
      if (is_row && child_align[i] == 4u /* Baseline */) {
        res.is_baseline_layout = true;
      }
    }

    res.child_main.assign(count, 0.0f);
    res.child_cross.assign(count, 0.0f);
    std::vector<bool> child_cross_auto(count, false);
    std::vector<bool> child_cross_deferred_stretch(count, false);
    std::vector<float> measured_w(count, -1.0f);
    std::vector<bool> measured_exact_w(count, false);

    float fixed_main = 0.0f;
    for (size_t i = 0; i < count; ++i) {
      const auto& child = nodes_.at(res.live_children[i]);
      const LayoutProps& cl = child.layout;
      const float c_mw = cl.margin.left + cl.margin.right;
      const float c_mh = cl.margin.top + cl.margin.bottom;
      const float m_margin = is_row ? c_mw : c_mh;
      const float c_margin = is_row ? c_mh : c_mw;
      const Dimension& m_dim = is_row ? cl.width : cl.height;
      const Dimension& c_dim = is_row ? cl.height : cl.width;
      const Dimension& m_min = is_row ? cl.min_width : cl.min_height;
      const Dimension& m_max = is_row ? cl.max_width : cl.max_height;
      const Dimension& c_min = is_row ? cl.min_height : cl.min_width;
      const Dimension& c_max = is_row ? cl.max_height : cl.max_width;

      const float def_basis = (main_mode != MeasureMode::Unconstrained)
                                  ? resolve_definite_dim(child.flex_basis, main_pct_base)
                                  : kUnconstrained;
      const float def_main =
          (def_basis >= 0.0f) ? def_basis : resolve_definite_dim(m_dim, main_pct_base);
      const float def_cross = resolve_definite_dim(c_dim, cross_pct_base);

      const bool is_stretch = (def_cross < 0.0f && child_align[i] == 0u /* Stretch */);
      const bool cross_stretched =
          is_stretch && (cross_mode == MeasureMode::Exact) && !res.is_baseline_layout;
      child_cross_deferred_stretch[i] = is_stretch && res.is_baseline_layout;
      child_cross_auto[i] = (def_cross < 0.0f && !cross_stretched);

      float resolved_main =
          (def_main >= 0.0f) ? clamp_axis_size(def_main, m_min, m_max, main_pct_base) : kUnconstrained;
      float resolved_cross =
          (def_cross >= 0.0f)
              ? clamp_axis_size(def_cross, c_min, c_max, cross_pct_base)
              : (cross_stretched
                     ? clamp_axis_size(std::max(0.0f, cross_avail - c_margin), c_min, c_max, cross_pct_base)
                     : kUnconstrained);

      if (resolved_main < 0.0f || resolved_cross < 0.0f) {
        float c_avail_w = 0.0f;
        MeasureMode c_w_mode = MeasureMode::Unconstrained;
        float c_avail_h = 0.0f;
        MeasureMode c_h_mode = MeasureMode::Unconstrained;

        if (is_row) {
          if (resolved_main >= 0.0f) {
            c_avail_w = resolved_main;
            c_w_mode = MeasureMode::Exact;
          } else if (eff_w_mode != MeasureMode::Unconstrained) {
            c_avail_w = std::max(0.0f, safe_inner_w - c_mw);
            c_w_mode = MeasureMode::AtMost;
          }
          if (resolved_cross >= 0.0f) {
            c_avail_h = resolved_cross;
            c_h_mode = MeasureMode::Exact;
          } else if (eff_h_mode != MeasureMode::Unconstrained) {
            c_avail_h = std::max(0.0f, safe_inner_h - c_mh);
            c_h_mode = MeasureMode::AtMost;
          }
        } else {
          if (resolved_cross >= 0.0f) {
            c_avail_w = resolved_cross;
            c_w_mode = MeasureMode::Exact;
          } else if (eff_w_mode != MeasureMode::Unconstrained) {
            c_avail_w = std::max(0.0f, safe_inner_w - c_mw);
            c_w_mode = MeasureMode::AtMost;
          }
          if (resolved_main >= 0.0f) {
            c_avail_h = resolved_main;
            c_h_mode = MeasureMode::Exact;
          }
        }

        const NveIntrinsicSize measured = measure_intrinsic_size(
            res.live_children[i], c_avail_w, c_w_mode, c_avail_h, c_h_mode, measure_gen, depth + 1);
        measured_w[i] = measured.width;
        measured_exact_w[i] = (c_w_mode == MeasureMode::Exact);

        if (resolved_cross < 0.0f) {
          resolved_cross =
              clamp_axis_size(is_row ? measured.height : measured.width, c_min, c_max, cross_pct_base);
        }
        if (resolved_main < 0.0f) {
          if (!is_row && std::fabs(resolved_cross - measured.width) > 1e-3f) {
            // Column child's width was clamped by min_width/max_width: re-measure wrapped height.
            const NveIntrinsicSize remeasured = measure_intrinsic_size(
                res.live_children[i],
                resolved_cross,
                MeasureMode::Exact,
                0.0f,
                MeasureMode::Unconstrained,
                measure_gen,
                depth + 1);
            measured_w[i] = resolved_cross;
            measured_exact_w[i] = true;
            resolved_main = clamp_axis_size(remeasured.height, m_min, m_max, main_pct_base);
          } else {
            resolved_main =
                clamp_axis_size(is_row ? measured.width : measured.height, m_min, m_max, main_pct_base);
          }
        }
      } else {
        measured_w[i] = is_row ? resolved_main : resolved_cross;
        measured_exact_w[i] = true;
      }

      res.child_main[i] = resolved_main;
      res.child_cross[i] = resolved_cross;
      fixed_main += res.child_main[i] + m_margin;
    }

    // Iterative CSS Flexbox grow/shrink resolution with min/max clamping freeze rounds.
    const bool can_grow = (main_mode == MeasureMode::Exact);
    const bool can_shrink = (main_mode != MeasureMode::Unconstrained);
    const float initial_free = main_avail - fixed_main - total_gap;
    const bool growing = can_grow && (initial_free > 1e-4f);
    const bool shrinking = can_shrink && (initial_free < -1e-4f);

    if (growing || shrinking) {
      std::vector<bool> frozen(count, false);
      const std::vector<float> base_main = res.child_main;
      for (size_t i = 0; i < count; ++i) {
        const LayoutProps& cl = nodes_.at(res.live_children[i]).layout;
        const float factor =
            growing ? sanitize_non_negative(cl.flex_grow) : sanitize_non_negative(cl.flex_shrink);
        if (factor <= 0.0f || (shrinking && base_main[i] <= 0.0f)) {
          frozen[i] = true;
        }
      }

      for (size_t iter = 0; iter < count; ++iter) {
        float used_by_frozen = total_gap;
        float base_unfrozen = 0.0f;
        float total_weight = 0.0f;
        for (size_t i = 0; i < count; ++i) {
          const LayoutProps& cl = nodes_.at(res.live_children[i]).layout;
          const float m_margin = is_row ? (cl.margin.left + cl.margin.right)
                                        : (cl.margin.top + cl.margin.bottom);
          if (frozen[i]) {
            used_by_frozen += res.child_main[i] + m_margin;
          } else {
            base_unfrozen += base_main[i] + m_margin;
            total_weight += growing ? sanitize_non_negative(cl.flex_grow)
                                    : (sanitize_non_negative(cl.flex_shrink) * base_main[i]);
          }
        }
        if (total_weight <= 0.0f) break;

        const float remaining = main_avail - used_by_frozen - base_unfrozen;
        if ((growing && remaining <= 1e-4f) || (shrinking && remaining >= -1e-4f)) {
          for (size_t i = 0; i < count; ++i) {
            if (!frozen[i]) {
              res.child_main[i] = base_main[i];
            }
          }
          break;
        }

        bool any_clamped = false;
        for (size_t i = 0; i < count; ++i) {
          if (frozen[i]) continue;
          const LayoutProps& cl = nodes_.at(res.live_children[i]).layout;
          const Dimension& m_min = is_row ? cl.min_width : cl.min_height;
          const Dimension& m_max = is_row ? cl.max_width : cl.max_height;
          const float weight = growing ? sanitize_non_negative(cl.flex_grow)
                                       : (sanitize_non_negative(cl.flex_shrink) * base_main[i]);
          const float unclamped = base_main[i] + remaining * (weight / total_weight);
          const float clamped = clamp_axis_size(unclamped, m_min, m_max, main_pct_base);
          res.child_main[i] = clamped;
          if (std::fabs(clamped - unclamped) > 1e-4f) {
            frozen[i] = true;
            any_clamped = true;
          }
        }
        if (!any_clamped) break;
      }
    }

    // Post-flex wrapped text & nested container re-measurement at resolved final width.
    if (is_row) {
      for (size_t i = 0; i < count; ++i) {
        if (!child_cross_auto[i]) continue;
        const auto& child = nodes_.at(res.live_children[i]);
        const bool width_changed = std::fabs(res.child_main[i] - measured_w[i]) > 1e-3f;
        const bool container_needs_exact = (!child.children.empty() && !measured_exact_w[i]);
        if (width_changed || container_needs_exact) {
          const LayoutProps& cl = child.layout;
          const NveIntrinsicSize remeasured = measure_intrinsic_size(
              res.live_children[i],
              res.child_main[i],
              MeasureMode::Exact,
              0.0f,
              MeasureMode::Unconstrained,
              measure_gen,
              depth + 1);
          res.child_cross[i] =
              clamp_axis_size(remeasured.height, cl.min_height, cl.max_height, cross_pct_base);
        }
      }
    }

    // Baseline computation & line cross-size calculation.
    float max_baseline_ascent = 0.0f;
    float max_baseline_descent = 0.0f;
    float max_non_baseline_cross = 0.0f;
    std::vector<float> child_baseline(count, 0.0f);

    for (size_t i = 0; i < count; ++i) {
      const LayoutProps& cl = nodes_.at(res.live_children[i]).layout;
      const float c_margin = is_row ? (cl.margin.top + cl.margin.bottom)
                                    : (cl.margin.left + cl.margin.right);
      if (is_row && child_align[i] == 4u /* Baseline */) {
        const float b = compute_node_baseline(
            res.live_children[i], res.child_main[i], res.child_cross[i], measure_gen, depth + 1);
        child_baseline[i] = b;
        const float ascent = b + cl.margin.top;
        const float descent = (res.child_cross[i] - b) + cl.margin.bottom;
        max_baseline_ascent = std::max(max_baseline_ascent, ascent);
        max_baseline_descent = std::max(max_baseline_descent, descent);
      } else {
        max_non_baseline_cross = std::max(max_non_baseline_cross, res.child_cross[i] + c_margin);
      }
    }

    const float baseline_line_cross =
        res.is_baseline_layout ? std::max(0.0f, max_baseline_ascent + max_baseline_descent) : 0.0f;
    const float natural_line_cross = std::max(max_non_baseline_cross, baseline_line_cross);

    if (res.is_baseline_layout) {
      for (size_t i = 0; i < count; ++i) {
        if (child_cross_deferred_stretch[i]) {
          const LayoutProps& cl = nodes_.at(res.live_children[i]).layout;
          const float c_margin = cl.margin.top + cl.margin.bottom;
          res.child_cross[i] = clamp_axis_size(
              std::max(0.0f, natural_line_cross - c_margin),
              cl.min_height,
              cl.max_height,
              cross_pct_base);
        }
      }
    }

    float used_main = total_gap;
    for (size_t i = 0; i < count; ++i) {
      const LayoutProps& cl = nodes_.at(res.live_children[i]).layout;
      const float m_margin = is_row ? (cl.margin.left + cl.margin.right)
                                    : (cl.margin.top + cl.margin.bottom);
      used_main += res.child_main[i] + m_margin;
    }
    res.content_main = std::max(0.0f, used_main);
    res.content_cross = std::max(0.0f, natural_line_cross);

    if (!compute_positions) {
      return res;
    }

    res.child_x.assign(count, 0.0f);
    res.child_y.assign(count, 0.0f);

    const float rem_main = main_avail - used_main;
    float leading_main = 0.0f;
    float between_extra = 0.0f;
    switch (nl.justify_content) {
      case 1: /* Center */
        leading_main = rem_main * 0.5f;
        break;
      case 2: /* FlexEnd */
        leading_main = rem_main;
        break;
      case 3: /* SpaceBetween */
        if (count > 1 && rem_main > 0.0f) {
          between_extra = rem_main / static_cast<float>(count - 1);
        }
        break;
      case 4: /* SpaceAround */
        if (rem_main > 0.0f) {
          between_extra = rem_main / static_cast<float>(count);
          leading_main = between_extra * 0.5f;
        } else {
          leading_main = rem_main * 0.5f;
        }
        break;
      case 5: /* SpaceEvenly */
        if (rem_main > 0.0f) {
          between_extra = rem_main / static_cast<float>(count + 1);
          leading_main = between_extra;
        } else {
          leading_main = rem_main * 0.5f;
        }
        break;
      case 0: /* FlexStart */
      default:
        break;
    }

    const float cross_box = res.is_baseline_layout ? natural_line_cross : cross_avail;
    float cursor = is_reverse
        ? ((is_row ? (pad_l + safe_inner_w) : (pad_t + safe_inner_h)) - leading_main)
        : ((is_row ? pad_l : pad_t) + leading_main);

    for (size_t i = 0; i < count; ++i) {
      const LayoutProps& cl = nodes_.at(res.live_children[i]).layout;
      const float m_margin_start = is_row ? cl.margin.left : cl.margin.top;
      const float m_margin_end = is_row ? cl.margin.right : cl.margin.bottom;
      const float c_margin_start = is_row ? cl.margin.top : cl.margin.left;
      const float c_margin_end = is_row ? cl.margin.bottom : cl.margin.right;

      const float rem_cross = cross_box - (res.child_cross[i] + c_margin_start + c_margin_end);
      float cross_offset = c_margin_start;
      switch (child_align[i]) {
        case 2: /* Center */
          cross_offset = c_margin_start + rem_cross * 0.5f;
          break;
        case 3: /* FlexEnd */
          cross_offset = c_margin_start + rem_cross;
          break;
        case 4: /* Baseline (Row only) */
          cross_offset = max_baseline_ascent - child_baseline[i];
          break;
        case 0: /* Stretch */
        case 1: /* FlexStart */
        default:
          cross_offset = c_margin_start;
          break;
      }

      float main_pos = 0.0f;
      if (is_reverse) {
        cursor -= m_margin_end + res.child_main[i];
        main_pos = cursor;
        cursor -= m_margin_start + gap + between_extra;
      } else {
        main_pos = cursor + m_margin_start;
        cursor += m_margin_start + res.child_main[i] + m_margin_end + gap + between_extra;
      }

      res.child_x[i] = is_row ? main_pos : (pad_l + cross_offset);
      res.child_y[i] = is_row ? (pad_t + cross_offset) : main_pos;
    }

    return res;
  }

  NveIntrinsicSize measure_intrinsic_size(
      uint32_t node_id,
      float avail_w,
      MeasureMode w_mode,
      float avail_h,
      MeasureMode h_mode,
      uint64_t measure_gen,
      uint32_t depth) {
    if (depth > 512) return NveIntrinsicSize{0.0f, 0.0f};
    auto it = nodes_.find(node_id);
    if (it == nodes_.end()) return NveIntrinsicSize{0.0f, 0.0f};
    ShadowNodeEntry& node = it->second;

    const bool w_ok = (w_mode != MeasureMode::Unconstrained) && std::isfinite(avail_w);
    const bool h_ok = (h_mode != MeasureMode::Unconstrained) && std::isfinite(avail_h);
    const MeasureMode eff_w_mode = w_ok ? w_mode : MeasureMode::Unconstrained;
    const MeasureMode eff_h_mode = h_ok ? h_mode : MeasureMode::Unconstrained;
    const float norm_avail_w = w_ok ? std::max(0.0f, avail_w) : 0.0f;
    const float norm_avail_h = h_ok ? std::max(0.0f, avail_h) : 0.0f;

    if (node.measure_gen == measure_gen &&
        node.cached_w_mode == static_cast<uint8_t>(eff_w_mode) &&
        node.cached_h_mode == static_cast<uint8_t>(eff_h_mode) &&
        (eff_w_mode == MeasureMode::Unconstrained ||
         std::fabs(node.cached_avail_w - norm_avail_w) <= 1e-3f) &&
        (eff_h_mode == MeasureMode::Unconstrained ||
         std::fabs(node.cached_avail_h - norm_avail_h) <= 1e-3f)) {
      return node.cached_measure;
    }

    const LayoutProps& l = node.layout;
    const float pct_base_w = (eff_w_mode == MeasureMode::Exact) ? norm_avail_w : kUnconstrained;
    const float pct_base_h = (eff_h_mode == MeasureMode::Exact) ? norm_avail_h : kUnconstrained;

    float known_w = (eff_w_mode == MeasureMode::Exact)
                        ? norm_avail_w
                        : resolve_definite_dim(l.width, pct_base_w);
    float known_h = (eff_h_mode == MeasureMode::Exact)
                        ? norm_avail_h
                        : resolve_definite_dim(l.height, pct_base_h);
    if (eff_w_mode != MeasureMode::Exact && known_w >= 0.0f) {
      known_w = clamp_axis_size(known_w, l.min_width, l.max_width, pct_base_w);
    }
    if (eff_h_mode != MeasureMode::Exact && known_h >= 0.0f) {
      known_h = clamp_axis_size(known_h, l.min_height, l.max_height, pct_base_h);
    }

    const float pad_w = sanitize_non_negative(l.padding.left) + sanitize_non_negative(l.padding.right);
    const float pad_h = sanitize_non_negative(l.padding.top) + sanitize_non_negative(l.padding.bottom);

    NveIntrinsicSize result{0.0f, 0.0f};
    if (known_w >= 0.0f && known_h >= 0.0f) {
      result = NveIntrinsicSize{known_w, known_h};
    } else if (node.kind == NVE_COMP_TEXT && node.children.empty()) {
      MeasureMode text_w_mode = MeasureMode::Unconstrained;
      float outer_bound_w = 0.0f;
      if (known_w >= 0.0f) {
        text_w_mode = MeasureMode::Exact;
        outer_bound_w = known_w;
      } else {
        if (eff_w_mode == MeasureMode::AtMost) {
          text_w_mode = MeasureMode::AtMost;
          outer_bound_w = norm_avail_w;
        }
        const float max_w_dim = resolve_definite_dim(l.max_width, pct_base_w);
        if (max_w_dim >= 0.0f) {
          outer_bound_w =
              (text_w_mode == MeasureMode::AtMost) ? std::min(outer_bound_w, max_w_dim) : max_w_dim;
          text_w_mode = MeasureMode::AtMost;
        }
      }

      const float inner_text_w =
          (text_w_mode != MeasureMode::Unconstrained) ? std::max(0.0f, outer_bound_w - pad_w) : 0.0f;
      NveIntrinsicSize measured = measure_text_fallback(node, inner_text_w, text_w_mode);

      result.width = (known_w >= 0.0f)
                         ? known_w
                         : clamp_axis_size(measured.width + pad_w, l.min_width, l.max_width, pct_base_w);
      if (known_h < 0.0f && text_w_mode != MeasureMode::Unconstrained &&
          result.width > outer_bound_w + 1e-3f) {
        // min_width expanded width beyond the upper bound: re-measure height at the expanded width.
        measured = measure_text_fallback(node, std::max(0.0f, result.width - pad_w), MeasureMode::Exact);
      }
      result.height = (known_h >= 0.0f)
                          ? known_h
                          : clamp_axis_size(measured.height + pad_h, l.min_height, l.max_height, pct_base_h);
    } else if (node.children.empty()) {
      result.width = (known_w >= 0.0f)
                         ? known_w
                         : clamp_axis_size(pad_w, l.min_width, l.max_width, pct_base_w);
      result.height = (known_h >= 0.0f)
                          ? known_h
                          : clamp_axis_size(pad_h, l.min_height, l.max_height, pct_base_h);
    } else {
      MeasureMode inner_w_mode = MeasureMode::Unconstrained;
      float outer_bound_w = 0.0f;
      if (known_w >= 0.0f) {
        inner_w_mode = MeasureMode::Exact;
        outer_bound_w = known_w;
      } else {
        if (eff_w_mode == MeasureMode::AtMost) {
          inner_w_mode = MeasureMode::AtMost;
          outer_bound_w = norm_avail_w;
        }
        const float max_w_dim = resolve_definite_dim(l.max_width, pct_base_w);
        if (max_w_dim >= 0.0f) {
          outer_bound_w =
              (inner_w_mode == MeasureMode::AtMost) ? std::min(outer_bound_w, max_w_dim) : max_w_dim;
          inner_w_mode = MeasureMode::AtMost;
        }
      }

      MeasureMode inner_h_mode = MeasureMode::Unconstrained;
      float outer_bound_h = 0.0f;
      if (known_h >= 0.0f) {
        inner_h_mode = MeasureMode::Exact;
        outer_bound_h = known_h;
      } else {
        const float max_h_dim = resolve_definite_dim(l.max_height, pct_base_h);
        if (max_h_dim >= 0.0f) {
          inner_h_mode = MeasureMode::AtMost;
          outer_bound_h = max_h_dim;
        }
      }

      const float inner_avail_w =
          (inner_w_mode != MeasureMode::Unconstrained) ? std::max(0.0f, outer_bound_w - pad_w) : 0.0f;
      const float inner_avail_h =
          (inner_h_mode != MeasureMode::Unconstrained) ? std::max(0.0f, outer_bound_h - pad_h) : 0.0f;

      const FlexLineResult line = resolve_flex_line(
          node,
          inner_avail_w,
          inner_w_mode,
          inner_avail_h,
          inner_h_mode,
          /*compute_positions=*/false,
          measure_gen,
          depth + 1);

      const bool is_row = (l.flex_direction == 1 || l.flex_direction == 3);
      const float content_w = pad_w + (is_row ? line.content_main : line.content_cross);
      const float content_h = pad_h + (is_row ? line.content_cross : line.content_main);

      result.width = (known_w >= 0.0f)
                         ? known_w
                         : clamp_axis_size(content_w, l.min_width, l.max_width, pct_base_w);
      result.height = (known_h >= 0.0f)
                          ? known_h
                          : clamp_axis_size(content_h, l.min_height, l.max_height, pct_base_h);
    }

    node.measure_gen = measure_gen;
    node.cached_avail_w = norm_avail_w;
    node.cached_avail_h = norm_avail_h;
    node.cached_w_mode = static_cast<uint8_t>(eff_w_mode);
    node.cached_h_mode = static_cast<uint8_t>(eff_h_mode);
    node.cached_measure = result;
    return result;
  }

  void compute_fallback_layout(
      uint32_t node_id,
      float x,
      float y,
      float w,
      float h,
      uint64_t ts,
      uint64_t measure_gen,
      uint32_t depth = 0) {
    if (depth > 512) return;
    auto it = nodes_.find(node_id);
    if (it == nodes_.end()) return;
    ShadowNodeEntry& node = it->second;

    const float safe_x = std::isfinite(x) ? x : 0.0f;
    const float safe_y = std::isfinite(y) ? y : 0.0f;
    const float safe_w = sanitize_non_negative(w);
    const float safe_h = sanitize_non_negative(h);

    const LayoutRect next_rect{safe_x, safe_y, safe_w, safe_h};
    const bool changed = !node.has_computed_layout || rect_changed(node.computed_layout, next_rect);
    node.computed_layout = next_rect;
    node.has_computed_layout = true;
    if ((node.event_mask & kEventMaskLayout) != 0 && changed) {
      std::lock_guard<std::mutex> lock(event_mutex_);
      event_ring_.push_layout(
          config_.surface_id,
          node.node_id,
          ts,
          next_rect.x,
          next_rect.y,
          next_rect.width,
          next_rect.height);
    }

    if (node.children.empty()) return;

    const LayoutProps& nl = node.layout;
    const bool is_row = (nl.flex_direction == 1 || nl.flex_direction == 3);
    const float pad_w = sanitize_non_negative(nl.padding.left) + sanitize_non_negative(nl.padding.right);
    const float pad_h = sanitize_non_negative(nl.padding.top) + sanitize_non_negative(nl.padding.bottom);
    const float inner_w = std::max(0.0f, safe_w - pad_w);
    const float inner_h = std::max(0.0f, safe_h - pad_h);

    const FlexLineResult line = resolve_flex_line(
        node,
        inner_w,
        MeasureMode::Exact,
        inner_h,
        MeasureMode::Exact,
        /*compute_positions=*/true,
        measure_gen,
        depth);

    for (size_t i = 0; i < line.live_children.size(); ++i) {
      const float cw = is_row ? line.child_main[i] : line.child_cross[i];
      const float ch = is_row ? line.child_cross[i] : line.child_main[i];
      compute_fallback_layout(
          line.live_children[i], line.child_x[i], line.child_y[i], cw, ch, ts, measure_gen, depth + 1);
    }
  }
#endif

  void flush_custom_component_props() {
    for (auto& [_, entry] : nodes_) {
      if (!entry.raw_props_dirty || entry.custom_name.empty()) continue;
      entry.raw_props_dirty = false;
      auto it = custom_components_.find(entry.custom_name);
      if (it != custom_components_.end() && it->second.on_update_props) {
        it->second.on_update_props(entry.node_id, entry.raw_props);
      }
    }
  }

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
  bool intercept_rn_raw_event(const facebook::react::RawEvent& raw_event) {
    auto family = raw_event.shadowNodeFamily.lock();
    if (!family) return false;
    const auto surface_id = static_cast<uint32_t>(family->getSurfaceId());
    if (surface_id != config_.surface_id) return false;

    const auto node_id = static_cast<uint32_t>(family->getTag());
    const uint64_t ts = now_us();

    std::lock_guard<std::mutex> lock(event_mutex_);
    if (!raw_event.eventPayload) {
      event_ring_.push_custom(surface_id, node_id, ts, raw_event.type, nullptr, 0);
      return true;
    }

    switch (raw_event.eventPayload->getType()) {
      case facebook::react::EventPayloadType::PointerEvent: {
        const float x = static_cast<float>(
            raw_event.eventPayload->extractValue({"locationX"}).value_or(
                raw_event.eventPayload->extractValue({"clientX"}).value_or(0.0)));
        const float y = static_cast<float>(
            raw_event.eventPayload->extractValue({"locationY"}).value_or(
                raw_event.eventPayload->extractValue({"clientY"}).value_or(0.0)));
        event_ring_.push_press(surface_id, node_id, ts, x, y);
        return true;
      }
      case facebook::react::EventPayloadType::ScrollEvent: {
        const float ox = static_cast<float>(
            raw_event.eventPayload->extractValue({"contentOffset", "x"}).value_or(0.0));
        const float oy = static_cast<float>(
            raw_event.eventPayload->extractValue({"contentOffset", "y"}).value_or(0.0));
        const float cw = static_cast<float>(
            raw_event.eventPayload->extractValue({"contentSize", "width"}).value_or(0.0));
        const float ch = static_cast<float>(
            raw_event.eventPayload->extractValue({"contentSize", "height"}).value_or(0.0));
        event_ring_.push_scroll(surface_id, node_id, ts, ox, oy, cw, ch);
        return true;
      }
      case facebook::react::EventPayloadType::ValueFactory:
      default: {
        facebook::jsi::Value val = raw_event.eventPayload->asJSIValue(jsi_recorder_);
        jsi_recorder_.record_custom_event(event_ring_, surface_id, node_id, ts, raw_event.type, val);
        return true;
      }
    }
  }

  void commit_rn_shadow_tree(bool sync_mount) {
    if (!rn_ui_manager_) return;
    const auto surface_id = static_cast<facebook::react::SurfaceId>(config_.surface_id);
    rn_ui_manager_->getShadowTreeRegistry().visit(
        surface_id,
        [sync_mount](const facebook::react::ShadowTree& shadow_tree) {
          facebook::react::ShadowTree::CommitOptions options{};
          options.enableStateReconciliation = false;
          options.mountSynchronously = sync_mount;
          shadow_tree.commit(
              [](const facebook::react::RootShadowNode& old_root)
                  -> facebook::react::RootShadowNode::Unshared {
                return std::static_pointer_cast<facebook::react::RootShadowNode>(
                    old_root.clone({}));
              },
              options);
        });
  }
#endif

  NveSurfaceConfig config_;
  std::unordered_map<uint32_t, ShadowNodeEntry> nodes_;
  std::unordered_map<std::string, CustomComponentDescriptor> custom_components_;
  TextMeasurerCallback text_measurer_;
  uint64_t gen_counter_{0};
  mutable std::mutex event_mutex_;
  EventRingWriter event_ring_;
  std::vector<uint8_t> drained_snapshot_;

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  YGConfigRef yg_config_{nullptr};
#endif

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
  std::shared_ptr<facebook::react::UIManager> rn_ui_manager_;
  std::shared_ptr<const facebook::react::EventListener> rn_event_listener_;
  MockJsiRecorderRuntime jsi_recorder_;
#endif
};

bool fabric_host_get_node_layout(NveFabricHostHandle host, uint32_t node_id, LayoutRect* out_rect) {
  if (!host || !out_rect) return false;
  return static_cast<FabricHost*>(host)->get_node_layout(node_id, out_rect);
}

bool fabric_host_has_yoga_layout() noexcept {
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  return true;
#else
  return false;
#endif
}

bool fabric_host_set_surface_size(
    NveFabricHostHandle host,
    float width,
    float height,
    float scale_factor) {
  if (!host) return false;
  static_cast<FabricHost*>(host)->set_surface_size(width, height, scale_factor);
  return true;
}

void fabric_host_enqueue_press(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    float x,
    float y) {
  if (!host) return;
  static_cast<FabricHost*>(host)->enqueue_press(node_id, timestamp_us, x, y);
}

void fabric_host_enqueue_text_change(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    std::string_view text) {
  if (!host) return;
  static_cast<FabricHost*>(host)->enqueue_text_change(node_id, timestamp_us, text);
}

void fabric_host_enqueue_custom_event(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    std::string_view name,
    const uint8_t* data,
    size_t len) {
  if (!host) return;
  static_cast<FabricHost*>(host)->enqueue_custom_event(node_id, timestamp_us, name, data, len);
}

void fabric_host_set_text_measurer(
    NveFabricHostHandle host,
    TextMeasurerCallback measurer) {
  if (!host) return;
  static_cast<FabricHost*>(host)->set_text_measurer(std::move(measurer));
}

void fabric_host_register_custom_component(
    NveFabricHostHandle host,
    CustomComponentDescriptor descriptor) {
  if (!host) return;
  static_cast<FabricHost*>(host)->register_custom_component(std::move(descriptor));
}

std::vector<RenderedNodeSnapshot> fabric_host_snapshot_nodes(NveFabricHostHandle host) {
  if (!host) return {};
  return static_cast<FabricHost*>(host)->snapshot_nodes();
}

std::vector<uint8_t> fabric_host_serialize_frame0(NveFabricHostHandle host, uint64_t frame_id) {
  if (!host) return {};
  return static_cast<FabricHost*>(host)->serialize_frame0(frame_id);
}

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
void fabric_host_attach_rn_uimanager(
    NveFabricHostHandle host,
    std::shared_ptr<facebook::react::UIManager> ui_manager) {
  if (!host) return;
  static_cast<FabricHost*>(host)->attach_rn_uimanager(std::move(ui_manager));
}
#endif

} // namespace nve

extern "C" {

NveFabricHostHandle nve_fabric_host_create(NveSurfaceConfig config) {
  return new nve::FabricHost(config);
}

void nve_fabric_host_destroy(NveFabricHostHandle host) {
  delete static_cast<nve::FabricHost*>(host);
}

int32_t nve_fabric_host_submit_frame(
    NveFabricHostHandle host,
    NveBufferSlice command_buffer,
    NveFrameStats* out_stats) {
  if (!host) return -100;
  return static_cast<nve::FabricHost*>(host)->submit_frame(command_buffer, out_stats);
}

NveBufferSlice nve_fabric_host_drain_events(NveFabricHostHandle host) {
  if (!host) return NveBufferSlice{nullptr, 0};
  return static_cast<nve::FabricHost*>(host)->drain_events();
}

uint32_t nve_fabric_host_node_count(NveFabricHostHandle host) {
  if (!host) return 0;
  return static_cast<nve::FabricHost*>(host)->node_count();
}

} // extern "C"
