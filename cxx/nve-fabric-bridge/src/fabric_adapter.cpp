#include "nve_abi.h"
#include "nve_rn_component.hpp"
#include "command_decoder.hpp"
#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
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
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  YGNodeRef yg_node{nullptr};
#else
  uint64_t measure_gen{0};
  float cached_avail_w{-2.0f};
  float cached_avail_h{-2.0f};
  bool cached_w_def{false};
  bool cached_h_def{false};
  NveIntrinsicSize cached_measure{};
#endif
};

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
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
    if (entry.kind == NVE_COMP_SCROLL_VIEW) {
      YGNodeStyleSetOverflow(entry.yg_node, YGOverflowScroll);
    }
    if (entry.kind == NVE_COMP_TEXT && entry.children.empty()) {
      YGNodeSetNodeType(entry.yg_node, YGNodeTypeText);
      YGNodeSetMeasureFunc(entry.yg_node, &measure_text_node);
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
    if (!parent.children.empty() && YGNodeHasMeasureFunc(parent.yg_node)) {
      YGNodeSetMeasureFunc(parent.yg_node, nullptr);
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
    if (root_it->second.layout.width.tag == 0) {
      YGNodeStyleSetWidth(root_yg, config_.width);
    }
    if (root_it->second.layout.height.tag == 0) {
      YGNodeStyleSetHeight(root_yg, config_.height);
    }
    YGNodeCalculateLayout(root_yg, config_.width, config_.height, YGDirectionLTR);

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
    const float root_w = resolve_definite_dim(root_it->second.layout.width, config_.width);
    const float root_h = resolve_definite_dim(root_it->second.layout.height, config_.height);
    compute_fallback_layout(
        NVE_ROOT_NODE_ID,
        0.0f,
        0.0f,
        (root_w >= 0.0f) ? root_w : config_.width,
        (root_h >= 0.0f) ? root_h : config_.height,
        ts,
        measure_gen);
#endif
  }

#if !(defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA)
  static constexpr float kUnconstrained = -1.0f;

  static float resolve_definite_dim(const Dimension& d, float parent_size) noexcept {
    switch (d.tag) {
      case 1:
        return std::max(0.0f, d.value);
      case 2:
        return (parent_size >= 0.0f) ? std::max(0.0f, parent_size * (d.value * 0.01f))
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
    val = std::max(0.0f, val);
    const float max_v = resolve_definite_dim(max_d, parent_size);
    if (max_v >= 0.0f) {
      val = std::min(val, max_v);
    }
    const float min_v = resolve_definite_dim(min_d, parent_size);
    if (min_v >= 0.0f) {
      val = std::max(val, min_v);
    }
    return std::max(0.0f, val);
  }

  NveIntrinsicSize measure_text_fallback(const ShadowNodeEntry& entry, float max_w) const {
    const float font_size =
        (std::isfinite(entry.style.font_size) && entry.style.font_size > 0.0f)
            ? entry.style.font_size
            : 16.0f;
    const size_t len = entry.text.size();
    if (len == 0) {
      return NveIntrinsicSize{0.0f, 0.0f};
    }
    if (text_measurer_) {
      const NveIntrinsicSize measured = text_measurer_(entry.text, font_size, max_w);
      float out_w = (std::isfinite(measured.width) && measured.width >= 0.0f) ? measured.width : 0.0f;
      float out_h = (std::isfinite(measured.height) && measured.height >= 0.0f) ? measured.height : 0.0f;
      if (max_w > 0.0f) {
        out_w = std::min(out_w, max_w);
      }
      return NveIntrinsicSize{out_w, out_h};
    }
    const float char_width = font_size * 0.55f;
    const float line_height = font_size * 1.3f;
    const float intrinsic_w = static_cast<float>(len) * char_width + 4.0f;
    const float out_w = (max_w > 0.0f) ? std::min(intrinsic_w, max_w) : intrinsic_w;
    float out_h = line_height;
    if (out_w > 0.0f && intrinsic_w > out_w) {
      out_h = std::ceil(intrinsic_w / out_w) * line_height;
    }
    return NveIntrinsicSize{out_w, out_h};
  }

  NveIntrinsicSize measure_intrinsic_size(
      uint32_t node_id,
      float avail_w,
      float avail_h,
      bool parent_w_def,
      bool parent_h_def,
      uint64_t measure_gen,
      uint32_t depth) {
    if (depth > 512) return NveIntrinsicSize{0.0f, 0.0f};
    auto it = nodes_.find(node_id);
    if (it == nodes_.end()) return NveIntrinsicSize{0.0f, 0.0f};
    ShadowNodeEntry& node = it->second;

    if (node.measure_gen == measure_gen &&
        node.cached_w_def == parent_w_def &&
        node.cached_h_def == parent_h_def &&
        std::fabs(node.cached_avail_w - avail_w) <= 1e-3f &&
        std::fabs(node.cached_avail_h - avail_h) <= 1e-3f) {
      return node.cached_measure;
    }

    const LayoutProps& l = node.layout;
    const float pct_base_w = parent_w_def ? avail_w : kUnconstrained;
    const float pct_base_h = parent_h_def ? avail_h : kUnconstrained;

    float known_w = resolve_definite_dim(l.width, pct_base_w);
    float known_h = resolve_definite_dim(l.height, pct_base_h);
    if (known_w >= 0.0f) {
      known_w = clamp_axis_size(known_w, l.min_width, l.max_width, pct_base_w);
    }
    if (known_h >= 0.0f) {
      known_h = clamp_axis_size(known_h, l.min_height, l.max_height, pct_base_h);
    }

    NveIntrinsicSize result{0.0f, 0.0f};
    if (known_w >= 0.0f && known_h >= 0.0f) {
      result = NveIntrinsicSize{known_w, known_h};
    } else if (node.kind == NVE_COMP_TEXT && node.children.empty()) {
      float text_max_w = 0.0f;
      if (known_w >= 0.0f) {
        text_max_w = known_w;
      } else {
        float bound_w = avail_w;
        const float max_w_dim = resolve_definite_dim(l.max_width, pct_base_w);
        if (max_w_dim >= 0.0f) {
          bound_w = (bound_w >= 0.0f) ? std::min(bound_w, max_w_dim) : max_w_dim;
        }
        if (bound_w > 0.0f) {
          text_max_w = bound_w;
        }
      }
      const NveIntrinsicSize measured = measure_text_fallback(node, text_max_w);
      result.width = (known_w >= 0.0f)
                         ? known_w
                         : clamp_axis_size(measured.width, l.min_width, l.max_width, pct_base_w);
      result.height = (known_h >= 0.0f)
                          ? known_h
                          : clamp_axis_size(measured.height, l.min_height, l.max_height, pct_base_h);
    } else {
      const float pad_w = std::max(0.0f, l.padding.left) + std::max(0.0f, l.padding.right);
      const float pad_h = std::max(0.0f, l.padding.top) + std::max(0.0f, l.padding.bottom);
      if (node.children.empty()) {
        result.width = (known_w >= 0.0f)
                           ? known_w
                           : clamp_axis_size(pad_w, l.min_width, l.max_width, pct_base_w);
        result.height = (known_h >= 0.0f)
                            ? known_h
                            : clamp_axis_size(pad_h, l.min_height, l.max_height, pct_base_h);
      } else {
        const bool is_row = (l.flex_direction == 1 || l.flex_direction == 3);
        const float gap = std::max(0.0f, l.gap);

        float bound_w = known_w;
        if (bound_w < 0.0f) {
          bound_w = avail_w;
          const float max_w_dim = resolve_definite_dim(l.max_width, pct_base_w);
          if (max_w_dim >= 0.0f) {
            bound_w = (bound_w >= 0.0f) ? std::min(bound_w, max_w_dim) : max_w_dim;
          }
        }
        const float inner_avail_w =
            (bound_w >= 0.0f) ? std::max(0.0f, bound_w - pad_w) : kUnconstrained;

        const float inner_avail_h =
            (known_h >= 0.0f) ? std::max(0.0f, known_h - pad_h) : kUnconstrained;

        float sum_main = 0.0f;
        float max_cross = 0.0f;
        size_t valid_children = 0;
        for (uint32_t cid : node.children) {
          auto cit = nodes_.find(cid);
          if (cit == nodes_.end()) continue;
          const LayoutProps& cl = cit->second.layout;
          const float c_mw = cl.margin.left + cl.margin.right;
          const float c_mh = cl.margin.top + cl.margin.bottom;
          const float c_avail_w =
              (inner_avail_w >= 0.0f) ? std::max(0.0f, inner_avail_w - c_mw) : kUnconstrained;
          const float c_avail_h =
              (inner_avail_h >= 0.0f) ? std::max(0.0f, inner_avail_h - c_mh) : kUnconstrained;
          const NveIntrinsicSize cs = measure_intrinsic_size(
              cid, c_avail_w, c_avail_h, known_w >= 0.0f, known_h >= 0.0f, measure_gen, depth + 1);
          const float outer_w = std::max(0.0f, cs.width + c_mw);
          const float outer_h = std::max(0.0f, cs.height + c_mh);
          if (is_row) {
            sum_main += outer_w;
            max_cross = std::max(max_cross, outer_h);
          } else {
            sum_main += outer_h;
            max_cross = std::max(max_cross, outer_w);
          }
          ++valid_children;
        }
        if (valid_children > 1) {
          sum_main += gap * static_cast<float>(valid_children - 1);
        }
        const float content_w = pad_w + (is_row ? sum_main : max_cross);
        const float content_h = pad_h + (is_row ? max_cross : sum_main);
        result.width = (known_w >= 0.0f)
                           ? known_w
                           : clamp_axis_size(content_w, l.min_width, l.max_width, pct_base_w);
        result.height = (known_h >= 0.0f)
                            ? known_h
                            : clamp_axis_size(content_h, l.min_height, l.max_height, pct_base_h);
      }
    }

    node.measure_gen = measure_gen;
    node.cached_avail_w = avail_w;
    node.cached_avail_h = avail_h;
    node.cached_w_def = parent_w_def;
    node.cached_h_def = parent_h_def;
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

    const LayoutRect next_rect{x, y, w, h};
    const bool changed = !node.has_computed_layout || rect_changed(node.computed_layout, next_rect);
    node.computed_layout = next_rect;
    node.has_computed_layout = true;
    if ((node.event_mask & kEventMaskLayout) != 0 && changed) {
      std::lock_guard<std::mutex> lock(event_mutex_);
      event_ring_.push_layout(
          config_.surface_id, node.node_id, ts, next_rect.x, next_rect.y, next_rect.width, next_rect.height);
    }

    if (node.children.empty()) return;

    std::vector<uint32_t> live_children;
    live_children.reserve(node.children.size());
    for (uint32_t cid : node.children) {
      if (nodes_.find(cid) != nodes_.end()) {
        live_children.push_back(cid);
      }
    }
    if (live_children.empty()) return;

    const LayoutProps& nl = node.layout;
    const bool is_row = (nl.flex_direction == 1 || nl.flex_direction == 3);
    const bool is_reverse = (nl.flex_direction == 2 || nl.flex_direction == 3);
    const float pad_l = std::max(0.0f, nl.padding.left);
    const float pad_r = std::max(0.0f, nl.padding.right);
    const float pad_t = std::max(0.0f, nl.padding.top);
    const float pad_b = std::max(0.0f, nl.padding.bottom);
    const float inner_x = pad_l;
    const float inner_y = pad_t;
    const float inner_w = std::max(0.0f, w - pad_l - pad_r);
    const float inner_h = std::max(0.0f, h - pad_t - pad_b);
    const float main_avail = is_row ? inner_w : inner_h;
    const float cross_avail = is_row ? inner_h : inner_w;
    const float gap = std::max(0.0f, nl.gap);
    const size_t count = live_children.size();
    const float total_gap = (count > 1) ? gap * static_cast<float>(count - 1) : 0.0f;

    float fixed_main = 0.0f;
    float total_grow = 0.0f;
    std::vector<float> child_main(count, 0.0f);
    std::vector<float> child_cross(count, 0.0f);
    std::vector<bool> child_cross_auto(count, false);

    for (size_t i = 0; i < count; ++i) {
      const auto& child = nodes_.at(live_children[i]);
      const LayoutProps& cl = child.layout;
      const float m_margin = is_row ? (cl.margin.left + cl.margin.right) : (cl.margin.top + cl.margin.bottom);
      const float c_margin = is_row ? (cl.margin.top + cl.margin.bottom) : (cl.margin.left + cl.margin.right);
      const Dimension& m_dim = is_row ? cl.width : cl.height;
      const Dimension& c_dim = is_row ? cl.height : cl.width;
      const Dimension& m_min = is_row ? cl.min_width : cl.min_height;
      const Dimension& m_max = is_row ? cl.max_width : cl.max_height;
      const Dimension& c_min = is_row ? cl.min_height : cl.min_width;
      const Dimension& c_max = is_row ? cl.max_height : cl.max_width;

      const float def_main = resolve_definite_dim(m_dim, main_avail);
      const float def_cross = resolve_definite_dim(c_dim, cross_avail);
      const bool cross_stretched = (def_cross < 0.0f && nl.align_items == 0);
      child_cross_auto[i] = (def_cross < 0.0f && !cross_stretched);

      float resolved_main =
          (def_main >= 0.0f) ? clamp_axis_size(def_main, m_min, m_max, main_avail) : kUnconstrained;
      float resolved_cross =
          (def_cross >= 0.0f)
              ? clamp_axis_size(def_cross, c_min, c_max, cross_avail)
              : (cross_stretched
                     ? clamp_axis_size(std::max(0.0f, cross_avail - c_margin), c_min, c_max, cross_avail)
                     : kUnconstrained);

      if (resolved_main < 0.0f || resolved_cross < 0.0f) {
        const float c_mw = cl.margin.left + cl.margin.right;
        const float c_mh = cl.margin.top + cl.margin.bottom;
        const float c_avail_w = is_row
            ? ((resolved_main >= 0.0f) ? resolved_main : std::max(0.0f, inner_w - c_mw))
            : ((resolved_cross >= 0.0f) ? resolved_cross : std::max(0.0f, inner_w - c_mw));
        const float c_avail_h = is_row
            ? ((resolved_cross >= 0.0f) ? resolved_cross : std::max(0.0f, inner_h - c_mh))
            : ((resolved_main >= 0.0f) ? resolved_main : kUnconstrained);
        const bool c_w_def = is_row ? (resolved_main >= 0.0f) : (resolved_cross >= 0.0f);
        const bool c_h_def = is_row ? (resolved_cross >= 0.0f) : (resolved_main >= 0.0f);
        const NveIntrinsicSize measured = measure_intrinsic_size(
            live_children[i], c_avail_w, c_avail_h, c_w_def, c_h_def, measure_gen, depth + 1);
        if (resolved_main < 0.0f) {
          resolved_main = clamp_axis_size(is_row ? measured.width : measured.height, m_min, m_max, main_avail);
        }
        if (resolved_cross < 0.0f) {
          resolved_cross = clamp_axis_size(is_row ? measured.height : measured.width, c_min, c_max, cross_avail);
        }
      }

      child_main[i] = resolved_main;
      child_cross[i] = resolved_cross;
      fixed_main += child_main[i] + m_margin;
      total_grow += std::max(0.0f, cl.flex_grow);
    }

    const std::vector<float> pre_flex_main = child_main;
    const float free_main = main_avail - fixed_main - total_gap;
    if (free_main > 0.0f && total_grow > 0.0f) {
      for (size_t i = 0; i < count; ++i) {
        const LayoutProps& cl = nodes_.at(live_children[i]).layout;
        if (cl.flex_grow > 0.0f) {
          const Dimension& m_min = is_row ? cl.min_width : cl.min_height;
          const Dimension& m_max = is_row ? cl.max_width : cl.max_height;
          child_main[i] =
              clamp_axis_size(child_main[i] + free_main * (cl.flex_grow / total_grow), m_min, m_max, main_avail);
        }
      }
    } else if (free_main < 0.0f) {
      float total_scaled_shrink = 0.0f;
      for (size_t i = 0; i < count; ++i) {
        const LayoutProps& cl = nodes_.at(live_children[i]).layout;
        total_scaled_shrink += std::max(0.0f, cl.flex_shrink) * child_main[i];
      }
      if (total_scaled_shrink > 0.0f) {
        const float overflow = -free_main;
        for (size_t i = 0; i < count; ++i) {
          const LayoutProps& cl = nodes_.at(live_children[i]).layout;
          const float weight = std::max(0.0f, cl.flex_shrink) * child_main[i];
          if (weight > 0.0f) {
            const Dimension& m_min = is_row ? cl.min_width : cl.min_height;
            const Dimension& m_max = is_row ? cl.max_width : cl.max_height;
            const float shrunk = std::max(0.0f, child_main[i] - overflow * (weight / total_scaled_shrink));
            child_main[i] = clamp_axis_size(shrunk, m_min, m_max, main_avail);
          }
        }
      }
    }

    // Re-measure Auto cross-height on Row children whose width changed after flex_grow/flex_shrink
    // (so wrapped text inside a flex item gets its accurate post-flex height).
    if (is_row) {
      for (size_t i = 0; i < count; ++i) {
        if (child_cross_auto[i] && std::fabs(child_main[i] - pre_flex_main[i]) > 1e-3f) {
          const LayoutProps& cl = nodes_.at(live_children[i]).layout;
          const NveIntrinsicSize remeasured = measure_intrinsic_size(
              live_children[i], child_main[i], kUnconstrained, true, false, measure_gen, depth + 1);
          child_cross[i] = clamp_axis_size(remeasured.height, cl.min_height, cl.max_height, cross_avail);
        }
      }
    }

    float used_main = total_gap;
    for (size_t i = 0; i < count; ++i) {
      const LayoutProps& cl = nodes_.at(live_children[i]).layout;
      const float m_margin = is_row ? (cl.margin.left + cl.margin.right) : (cl.margin.top + cl.margin.bottom);
      used_main += child_main[i] + m_margin;
    }

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

    float cursor = is_reverse
        ? ((is_row ? (inner_x + inner_w) : (inner_y + inner_h)) - leading_main)
        : ((is_row ? inner_x : inner_y) + leading_main);

    for (size_t i = 0; i < count; ++i) {
      const LayoutProps& cl = nodes_.at(live_children[i]).layout;
      const float m_margin_start = is_row ? cl.margin.left : cl.margin.top;
      const float m_margin_end = is_row ? cl.margin.right : cl.margin.bottom;
      const float c_margin_start = is_row ? cl.margin.top : cl.margin.left;
      const float c_margin_end = is_row ? cl.margin.bottom : cl.margin.right;

      const float rem_cross = cross_avail - (child_cross[i] + c_margin_start + c_margin_end);
      float cross_offset = c_margin_start;
      switch (nl.align_items) {
        case 2: /* Center */
          cross_offset = c_margin_start + rem_cross * 0.5f;
          break;
        case 3: /* FlexEnd */
          cross_offset = c_margin_start + rem_cross;
          break;
        case 0: /* Stretch */
        case 1: /* FlexStart */
        case 4: /* Baseline */
        default:
          cross_offset = c_margin_start;
          break;
      }

      float main_pos = 0.0f;
      if (is_reverse) {
        cursor -= m_margin_end + child_main[i];
        main_pos = cursor;
        cursor -= m_margin_start + gap + between_extra;
      } else {
        main_pos = cursor + m_margin_start;
        cursor += m_margin_start + child_main[i] + m_margin_end + gap + between_extra;
      }

      const float cx = is_row ? main_pos : (inner_x + cross_offset);
      const float cy = is_row ? (inner_y + cross_offset) : main_pos;
      const float cw = is_row ? child_main[i] : child_cross[i];
      const float ch = is_row ? child_cross[i] : child_main[i];
      compute_fallback_layout(live_children[i], cx, cy, cw, ch, ts, measure_gen, depth + 1);
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
