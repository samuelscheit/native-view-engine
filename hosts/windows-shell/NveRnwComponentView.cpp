#include "NveRnwComponentView.hpp"

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cmath>
#include <cstring>
#include <utility>

namespace nve::win {

namespace {

std::string ascii_lower(std::string_view sv) {
  std::string out;
  out.reserve(sv.size());
  for (unsigned char ch : sv) {
    out.push_back(static_cast<char>(std::tolower(ch)));
  }
  return out;
}

ImageResizeMode extract_resize_mode(const nve::RawPropMap* props) {
  if (props == nullptr) {
    return ImageResizeMode::Cover;
  }
  for (const char* key : {"resizeMode", "resize_mode"}) {
    const auto it = props->find(key);
    if (it != props->end()) {
      if (const auto* s = std::get_if<std::string>(&it->second)) {
        return parse_image_resize_mode(*s);
      }
    }
  }
  return ImageResizeMode::Cover;
}

nve::LayoutRect intersect_rects(const nve::LayoutRect& a, const nve::LayoutRect& b) noexcept {
  const float x0 = std::max(a.x, b.x);
  const float y0 = std::max(a.y, b.y);
  const float x1 = std::min(a.x + a.width, b.x + b.width);
  const float y1 = std::min(a.y + a.height, b.y + b.height);
  return nve::LayoutRect{x0, y0, std::max(0.0f, x1 - x0), std::max(0.0f, y1 - y0)};
}

bool rect_contains(const nve::LayoutRect& r, float px, float py) noexcept {
  return r.width > 0.0f && r.height > 0.0f &&
         px >= r.x && px <= (r.x + r.width) &&
         py >= r.y && py <= (r.y + r.height);
}

} // namespace

ImageResizeMode parse_image_resize_mode(std::string_view raw) noexcept {
  if (raw.empty()) {
    return ImageResizeMode::Cover;
  }
  const std::string lower = ascii_lower(raw);
  if (lower == "contain") {
    return ImageResizeMode::Contain;
  }
  if (lower == "stretch" || lower == "fill") {
    return ImageResizeMode::Stretch;
  }
  if (lower == "center") {
    return ImageResizeMode::Center;
  }
  return ImageResizeMode::Cover;
}

// -----------------------------------------------------------------------------
// NveWinVisualTree
// -----------------------------------------------------------------------------

void NveWinVisualTree::Adapter::update_root(
    const NveSceneView& /*view*/,
    const NveRenderNode& node) {
  root_background_rgba = node.bg_rgba;
  root_layout = nve::LayoutRect{node.x, node.y, node.width, node.height};
}

NveWinNodeVisual NveWinVisualTree::Adapter::create_view(
    const NveSceneView& /*view*/,
    const NveRenderNode& node) {
  ++created_count;
  NveWinNodeVisual visual{};
  visual.node_id = node.node_id;
  visual.kind = node.kind;
  visual.view_tag = nve::view_tag_for_render_node(node);
  return visual;
}

void NveWinVisualTree::Adapter::update_view(
    const NveSceneView& view,
    NveWinNodeVisual& visual,
    const NveRenderNode& node) {
  ++updated_count;
  visual.node_id = node.node_id;
  visual.kind = node.kind;
  visual.view_tag = nve::view_tag_for_render_node(node);
  visual.is_text_leaf = (node.flags & NVE_NODE_LEAF_TEXT) != 0u;
  visual.interactive =
      (node.kind == NVE_NODE_KIND_PRESSABLE) || ((node.flags & NVE_NODE_INTERACTIVE) != 0u);
  visual.clip_bounds =
      (node.kind == NVE_NODE_KIND_SCROLL_VIEW) || ((node.flags & NVE_NODE_CLIP_BOUNDS) != 0u);

  const std::string_view payload = nve::scene_node_text(view, node);
  if (node.kind == NVE_NODE_KIND_IMAGE) {
    visual.image_src.assign(payload.data(), payload.size());
    visual.text.clear();
    visual.draw_list.clear();
    if (raw_props != nullptr) {
      const auto it = raw_props->find(node.node_id);
      visual.resize_mode =
          (it != raw_props->end()) ? extract_resize_mode(&it->second) : ImageResizeMode::Cover;
    }
  } else if (node.kind == NVE_NODE_KIND_SKIA_CANVAS) {
    const auto* raw_ptr = reinterpret_cast<const uint8_t*>(payload.data());
    const bool same =
        visual.draw_list.size() == payload.size() &&
        (payload.empty() ||
         std::memcmp(visual.draw_list.data(), raw_ptr, payload.size()) == 0);
    if (!same) {
      visual.draw_list.assign(raw_ptr, raw_ptr + payload.size());
    }
    visual.text.clear();
    visual.image_src.clear();
  } else {
    visual.text.assign(payload.data(), payload.size());
    visual.image_src.clear();
    visual.draw_list.clear();
  }

  visual.layout = nve::LayoutRect{node.x, node.y, node.width, node.height};
  visual.background_rgba = node.bg_rgba;
  visual.text_color_rgba = node.text_rgba;
  visual.border_color_rgba = node.border_rgba;
  visual.border_width = node.border_width;
  visual.border_radius = node.border_radius;
  visual.opacity = static_cast<float>(node.opacity_u8) / 255.0f;
  visual.font_size = node.font_size;
  visual.z_index = node.z_index;
}

void NveWinVisualTree::Adapter::destroy_view(uint32_t /*node_id*/, NveWinNodeVisual& /*visual*/) {
  ++destroyed_count;
}

bool NveWinVisualTree::sync(const NveSceneView& view, const RawPropByNode* raw_props) {
  adapter_.raw_props = raw_props;
  const bool ok = reconciler_.sync(view, adapter_);
  // Propagate raw-prop-only resizeMode changes on retained Image nodes even if layout/src did not change.
  if (ok && raw_props != nullptr) {
    for (const auto& [nid, rp] : *raw_props) {
      if (NveWinNodeVisual* visual = reconciler_.find(nid);
          visual != nullptr && visual->kind == NVE_NODE_KIND_IMAGE) {
        visual->resize_mode = extract_resize_mode(&rp);
      }
    }
  }
  adapter_.raw_props = nullptr;
  return ok;
}

void NveWinVisualTree::clear() {
  reconciler_.clear(adapter_);
  adapter_.root_children.clear();
  adapter_.root_background_rgba = 0u;
  adapter_.root_layout = nve::LayoutRect{};
}

bool NveWinVisualTree::set_scroll_offset(uint32_t node_id, float offset_x, float offset_y) {
  NveWinNodeVisual* node = reconciler_.find(node_id);
  if (node == nullptr || node->kind != NVE_NODE_KIND_SCROLL_VIEW) {
    return false;
  }
  const float max_x = std::max(0.0f, node->content_width - node->layout.width);
  const float max_y = std::max(0.0f, node->content_height - node->layout.height);
  const float nx = std::clamp(std::isfinite(offset_x) ? offset_x : 0.0f, 0.0f, max_x);
  const float ny = std::clamp(std::isfinite(offset_y) ? offset_y : 0.0f, 0.0f, max_y);
  node->scroll_offset_x = nx;
  node->scroll_offset_y = ny;
  return true;
}

std::optional<std::pair<float, float>> NveWinVisualTree::scroll_offset(
    uint32_t node_id) const noexcept {
  const NveWinNodeVisual* node = reconciler_.find(node_id);
  if (node == nullptr || node->kind != NVE_NODE_KIND_SCROLL_VIEW) {
    return std::nullopt;
  }
  return std::make_pair(node->scroll_offset_x, node->scroll_offset_y);
}

std::optional<nve::LayoutRect> NveWinVisualTree::absolute_rect(uint32_t node_id) const {
  if (node_id == NVE_ROOT_NODE_ID) {
    return adapter_.root_layout;
  }
  const NveWinNodeVisual* node = reconciler_.find(node_id);
  if (node == nullptr) {
    return std::nullopt;
  }

  float abs_x = node->layout.x;
  float abs_y = node->layout.y;
  uint32_t cur_parent = node->parent_id;
  uint32_t hops = 0;
  while (cur_parent != 0u && cur_parent != NVE_ROOT_NODE_ID && hops < 512u) {
    const NveWinNodeVisual* parent = reconciler_.find(cur_parent);
    if (parent == nullptr) {
      break;
    }
    if (parent->kind == NVE_NODE_KIND_SCROLL_VIEW) {
      abs_x -= parent->scroll_offset_x;
      abs_y -= parent->scroll_offset_y;
    }
    abs_x += parent->layout.x;
    abs_y += parent->layout.y;
    cur_parent = parent->parent_id;
    ++hops;
  }
  return nve::LayoutRect{abs_x, abs_y, node->layout.width, node->layout.height};
}

void NveWinVisualTree::append_draw_order(
    uint32_t node_id,
    float origin_x,
    float origin_y,
    float inherited_opacity,
    bool has_clip,
    nve::LayoutRect clip_rect,
    std::vector<FlattenedVisualNode>& out,
    uint32_t depth) const {
  if (depth > 512u) {
    return;
  }
  const NveWinNodeVisual* node = reconciler_.find(node_id);
  if (node == nullptr) {
    return;
  }

  const float clamped_opacity = std::clamp(node->opacity, 0.0f, 1.0f);
  const float effective_opacity = inherited_opacity * clamped_opacity;
  const float abs_x = origin_x + node->layout.x;
  const float abs_y = origin_y + node->layout.y;

  out.push_back(FlattenedVisualNode{
      .node_id = node_id,
      .abs_x = abs_x,
      .abs_y = abs_y,
      .effective_opacity = effective_opacity,
      .has_clip = has_clip,
      .clip_rect = clip_rect,
  });

  if (node->children.empty()) {
    return;
  }

  const nve::LayoutRect self_rect{abs_x, abs_y, node->layout.width, node->layout.height};
  const bool next_has_clip = has_clip || node->clip_bounds;
  const nve::LayoutRect next_clip =
      node->clip_bounds ? (has_clip ? intersect_rects(clip_rect, self_rect) : self_rect) : clip_rect;

  const float child_origin_x =
      abs_x - (node->kind == NVE_NODE_KIND_SCROLL_VIEW ? node->scroll_offset_x : 0.0f);
  const float child_origin_y =
      abs_y - (node->kind == NVE_NODE_KIND_SCROLL_VIEW ? node->scroll_offset_y : 0.0f);

  std::vector<uint32_t> ordered = node->children;
  std::stable_sort(ordered.begin(), ordered.end(), [this](uint32_t a, uint32_t b) {
    const NveWinNodeVisual* va = reconciler_.find(a);
    const NveWinNodeVisual* vb = reconciler_.find(b);
    return (va ? va->z_index : 0) < (vb ? vb->z_index : 0);
  });

  for (uint32_t child_id : ordered) {
    append_draw_order(
        child_id,
        child_origin_x,
        child_origin_y,
        effective_opacity,
        next_has_clip,
        next_clip,
        out,
        depth + 1u);
  }
}

std::vector<FlattenedVisualNode> NveWinVisualTree::build_draw_order() const {
  std::vector<FlattenedVisualNode> out;
  out.reserve(reconciler_.size());

  std::vector<uint32_t> ordered_roots = adapter_.root_children;
  std::stable_sort(ordered_roots.begin(), ordered_roots.end(), [this](uint32_t a, uint32_t b) {
    const NveWinNodeVisual* va = reconciler_.find(a);
    const NveWinNodeVisual* vb = reconciler_.find(b);
    return (va ? va->z_index : 0) < (vb ? vb->z_index : 0);
  });

  for (uint32_t child_id : ordered_roots) {
    append_draw_order(child_id, 0.0f, 0.0f, 1.0f, false, nve::LayoutRect{}, out, 0u);
  }
  return out;
}

std::optional<HitTestResult> NveWinVisualTree::hit_test_interactive(
    float surface_x,
    float surface_y) const {
  const std::vector<FlattenedVisualNode> order = build_draw_order();
  for (auto it = order.rbegin(); it != order.rend(); ++it) {
    const NveWinNodeVisual* node = reconciler_.find(it->node_id);
    if (node == nullptr || !node->interactive || it->effective_opacity <= 0.0f) {
      continue;
    }
    if (it->has_clip && !rect_contains(it->clip_rect, surface_x, surface_y)) {
      continue;
    }
    const nve::LayoutRect bounds{it->abs_x, it->abs_y, node->layout.width, node->layout.height};
    if (rect_contains(bounds, surface_x, surface_y)) {
      return HitTestResult{
          .node_id = node->node_id,
          .local_x = surface_x - it->abs_x,
          .local_y = surface_y - it->abs_y,
      };
    }
  }
  return std::nullopt;
}

std::optional<uint32_t> NveWinVisualTree::hit_test_scrollable(
    float surface_x,
    float surface_y) const {
  const std::vector<FlattenedVisualNode> order = build_draw_order();
  for (auto it = order.rbegin(); it != order.rend(); ++it) {
    const NveWinNodeVisual* node = reconciler_.find(it->node_id);
    if (node == nullptr || node->kind != NVE_NODE_KIND_SCROLL_VIEW ||
        it->effective_opacity <= 0.0f) {
      continue;
    }
    if (it->has_clip && !rect_contains(it->clip_rect, surface_x, surface_y)) {
      continue;
    }
    const nve::LayoutRect bounds{it->abs_x, it->abs_y, node->layout.width, node->layout.height};
    if (rect_contains(bounds, surface_x, surface_y)) {
      return node->node_id;
    }
  }
  return std::nullopt;
}

// -----------------------------------------------------------------------------
// NveRnwComponentHost
// -----------------------------------------------------------------------------

NveRnwComponentHost::NveRnwComponentHost(std::shared_ptr<DirectWriteTextMeasurer> text_measurer)
    : text_measurer_(
          text_measurer ? std::move(text_measurer) : std::make_shared<DirectWriteTextMeasurer>()) {
  session_.set_text_measurer(make_directwrite_text_measurer(text_measurer_));
}

bool NveRnwComponentHost::attach_interface(
    NveGetInterfaceFn get_interface,
    NveSurfaceConfig config) {
  if (!session_.attach_interface(get_interface, config)) {
    return false;
  }
  visual_tree_.clear();
  raw_props_initialized_ = false;
  sync_visuals_and_notify(/*check_raw_props=*/true);
  (void)session_.drain_events();
  return true;
}

bool NveRnwComponentHost::load_dylib(
    std::string_view path,
    NveSurfaceConfig config,
    int* out_err_code,
    std::string* out_err_msg) {
  if (!session_.load_dylib(path, config, out_err_code, out_err_msg)) {
    return false;
  }
  visual_tree_.clear();
  raw_props_initialized_ = false;
  sync_visuals_and_notify(/*check_raw_props=*/true);
  (void)session_.drain_events();
  return true;
}

bool NveRnwComponentHost::reload_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg) {
  if (!session_.reload_dylib(path, out_err_code, out_err_msg)) {
    return false;
  }
  raw_props_initialized_ = false;
  sync_visuals_and_notify(/*check_raw_props=*/true);
  (void)session_.drain_events();
  return true;
}

bool NveRnwComponentHost::connect_paired(
    const nve::PairClientConfig& pair_config,
    NveSurfaceConfig surface_config,
    nve::PairClient::WakeCallback on_frame_ready,
    std::string* out_error) {
  if (!session_.connect_paired(
          pair_config, surface_config, std::move(on_frame_ready), out_error)) {
    return false;
  }
  visual_tree_.clear();
  raw_props_initialized_ = false;
  if (session_.paired_frame_count() > 0u) {
    sync_visuals_and_notify(/*check_raw_props=*/true);
  }
  return true;
}

bool NveRnwComponentHost::connect_paired(
    std::string_view endpoint,
    std::string_view token,
    std::string_view transport_override,
    NveSurfaceConfig surface_config,
    nve::PairClient::WakeCallback on_frame_ready,
    std::string* out_error) {
  nve::PairClientConfig cfg{};
  if (!nve::parse_pair_endpoint(endpoint, transport_override, cfg, out_error)) {
    return false;
  }
  cfg.pairing_token.assign(token.data(), token.size());
  return connect_paired(cfg, surface_config, std::move(on_frame_ready), out_error);
}

size_t NveRnwComponentHost::poll_paired_frames(NveFrameStats* out_last_stats) {
  const size_t applied = session_.poll_paired_frames(out_last_stats);
  if (applied > 0u) {
    sync_visuals_and_notify(/*check_raw_props=*/true);
  }
  return applied;
}

void NveRnwComponentHost::disconnect_paired() noexcept {
  session_.disconnect_paired();
  visual_tree_.clear();
  raw_props_initialized_ = false;
}

bool NveRnwComponentHost::replay_frame0_snapshot(
    std::string_view snapshot_path,
    uint32_t expected_schema_version,
    NveSurfaceConfig config,
    uint64_t* out_restore_us) {
  session_.set_surface_config(config);
  using Clock = std::chrono::steady_clock;
  const auto t0 = Clock::now();
  if (!session_.replay_frame0_snapshot(snapshot_path, expected_schema_version)) {
    return false;
  }
  visual_tree_.clear();
  raw_props_initialized_ = false;
  sync_visuals_and_notify(/*check_raw_props=*/true);
  const auto t1 = Clock::now();
  if (out_restore_us != nullptr) {
    *out_restore_us = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t1 - t0).count());
  }
  return true;
}

bool NveRnwComponentHost::save_snapshot(
    std::string_view snapshot_path,
    uint32_t schema_version) const {
  return session_.save_snapshot(snapshot_path, schema_version);
}

void NveRnwComponentHost::reset() noexcept {
  visual_tree_.clear();
  raw_props_initialized_ = false;
  session_.reset();
}

void NveRnwComponentHost::set_event_callback(nve::EventCallback cb) {
  session_.set_event_callback(std::move(cb));
}

void NveRnwComponentHost::set_on_visual_tree_changed(std::function<void()> cb) {
  on_visual_tree_changed_ = std::move(cb);
}

void NveRnwComponentHost::set_surface_size(float width, float height, float scale_factor) {
  if (width <= 0.0f || height <= 0.0f || !std::isfinite(width) || !std::isfinite(height)) {
    return;
  }
  session_.set_surface_size(width, height, scale_factor);
  if (session_.active_fabric_host() != nullptr) {
    sync_visuals_and_notify(/*check_raw_props=*/false);
  }
}

bool NveRnwComponentHost::update_props_json(std::string_view props_json) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.update_props_json(props_json, /*auto_render=*/false);
  if (dirty) {
    const int32_t rc = session_.render(nullptr);
    if (rc == 0) {
      sync_visuals_and_notify(/*check_raw_props=*/true);
    }
    (void)session_.drain_events();
  } else {
    (void)session_.drain_events();
  }
  return dirty;
}

bool NveRnwComponentHost::apply_hot_patch(std::span<const uint8_t> patch_nvp1) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.bridge()->apply_hot_patch(patch_nvp1, /*auto_render=*/false);
  if (dirty) {
    const int32_t rc = session_.render(nullptr);
    if (rc == 0) {
      sync_visuals_and_notify(/*check_raw_props=*/true);
    }
    (void)session_.drain_events();
  }
  return dirty;
}

int32_t NveRnwComponentHost::render_frame(NveFrameStats* out_stats) {
  if (!session_.is_valid() && !session_.is_paired()) {
    return -100;
  }
  if (session_.is_paired()) {
    (void)poll_paired_frames(out_stats);
    return 0;
  }
  const int32_t rc = session_.render(out_stats);
  if (rc == 0) {
    sync_visuals_and_notify(/*check_raw_props=*/false);
  }
  (void)session_.drain_events();
  return rc;
}

uint32_t NveRnwComponentHost::dispatch_press(uint32_t node_id, float local_x, float local_y) {
  if (!session_.is_valid() && !session_.is_paired()) {
    return 0;
  }
  const uint32_t dirty =
      session_.dispatch_press(node_id, local_x, local_y, /*auto_render=*/false);
  if (dirty != 0 && session_.is_valid()) {
    (void)render_frame(nullptr);
  } else {
    (void)session_.drain_events();
  }
  return dirty;
}

uint32_t NveRnwComponentHost::dispatch_click_at(float surface_x, float surface_y) {
  if (!session_.is_valid() && !session_.is_paired()) {
    return 0;
  }
  const auto hit = visual_tree_.hit_test_interactive(surface_x, surface_y);
  if (!hit.has_value()) {
    return 0;
  }
  return dispatch_press(hit->node_id, hit->local_x, hit->local_y);
}

uint32_t NveRnwComponentHost::dispatch_scroll(
    uint32_t node_id,
    float offset_x,
    float offset_y,
    float content_width,
    float content_height) {
  if (!session_.is_valid() && !session_.is_paired()) {
    return 0u;
  }
  NveDirectEvent ev{};
  ev.surface_id = surface_id();
  ev.node_id = node_id;
  ev.timestamp_us = nve::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(nve::EventKind::Scroll);
  ev.payload_tag = 2u;
  ev.x = offset_x;
  ev.y = offset_y;
  ev.width = content_width;
  ev.height = content_height;
  ev.value = nve::make_null_value();

  const uint32_t dirty =
      session_.dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/false);
  if (dirty != 0u && session_.is_valid()) {
    (void)render_frame(nullptr);
  } else {
    (void)session_.drain_events();
  }
  return dirty;
}

uint32_t NveRnwComponentHost::dispatch_wheel_at(
    float surface_x,
    float surface_y,
    float delta_x,
    float delta_y) {
  if (!session_.is_valid() && !session_.is_paired()) {
    return 0u;
  }
  const auto scroll_id = visual_tree_.hit_test_scrollable(surface_x, surface_y);
  if (!scroll_id.has_value()) {
    return 0u;
  }
  NveWinNodeVisual* scroll_node = visual_tree_.find(*scroll_id);
  if (scroll_node == nullptr) {
    return 0u;
  }
  const float next_x = scroll_node->scroll_offset_x + delta_x;
  const float next_y = scroll_node->scroll_offset_y + delta_y;
  (void)visual_tree_.set_scroll_offset(*scroll_id, next_x, next_y);
  const float eff_x = scroll_node->scroll_offset_x;
  const float eff_y = scroll_node->scroll_offset_y;
  const float cw = std::max(scroll_node->content_width, scroll_node->layout.width);
  const float ch = std::max(scroll_node->content_height, scroll_node->layout.height);
  if (on_visual_tree_changed_) {
    on_visual_tree_changed_();
  }
  return dispatch_scroll(*scroll_id, eff_x, eff_y, cw, ch);
}

std::optional<NveIntrinsicSize> NveRnwComponentHost::measure(
    NveSizeConstraints constraints) const {
  if (!session_.is_valid()) {
    return std::nullopt;
  }
  return session_.bridge()->measure(constraints);
}

std::vector<nve::DecodedEvent> NveRnwComponentHost::drain_events() {
  return session_.drain_events();
}

void NveRnwComponentHost::sync_visuals_and_notify(bool check_raw_props) {
  const NveSceneView view = session_.scene_view();
  bool need_raw_props = check_raw_props || !raw_props_initialized_;
  if (!need_raw_props && view.nodes != nullptr) {
    for (uint32_t i = 0u; i < view.dirty_count; ++i) {
      const uint32_t idx = view.dirty_indices[i];
      if (idx < view.node_count && view.nodes[idx].kind == NVE_NODE_KIND_IMAGE) {
        need_raw_props = true;
        break;
      }
    }
  }

  if (need_raw_props) {
    NveWinVisualTree::RawPropByNode raw_props;
    for (auto& snap : session_.snapshot_nodes()) {
      if (!snap.raw_props.empty()) {
        raw_props.emplace(snap.node_id, std::move(snap.raw_props));
      }
    }
    (void)visual_tree_.sync(view, &raw_props);
    raw_props_initialized_ = true;
  } else {
    (void)visual_tree_.sync(view, nullptr);
  }

  if (on_visual_tree_changed_) {
    on_visual_tree_changed_();
  }
}

// -----------------------------------------------------------------------------
// Modern react-native-windows 0.83+/0.88+ Composition Fabric integration
// -----------------------------------------------------------------------------

#if defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

namespace {

namespace rnw = winrt::Microsoft::ReactNative;
namespace rnw_comp = winrt::Microsoft::ReactNative::Composition;
namespace rnw_input = winrt::Microsoft::ReactNative::Composition::Input;

struct NveRnwComponentUserData
    : winrt::implements<NveRnwComponentUserData, winrt::Windows::Foundation::IInspectable> {
  std::shared_ptr<NveRnwComponentHost> host{std::make_shared<NveRnwComponentHost>()};
  rnw::EventEmitter event_emitter{nullptr};
  rnw::ComponentView::PointerReleased_revoker pointer_released_revoker{};
  rnw::ComponentView::Destroying_revoker destroying_revoker{};
};

void write_decoded_event_to_js(rnw::IJSValueWriter const& writer, const nve::DecodedEvent& ev) {
  writer.WriteObjectBegin();
  writer.WritePropertyName(L"surfaceId");
  writer.WriteInt64(static_cast<int64_t>(ev.surface_id));
  writer.WritePropertyName(L"nodeId");
  writer.WriteInt64(static_cast<int64_t>(ev.node_id));
  writer.WritePropertyName(L"timestampUs");
  writer.WriteInt64(static_cast<int64_t>(ev.timestamp_us));
  writer.WritePropertyName(L"name");
  writer.WriteString(winrt::to_hstring(ev.name));

  switch (ev.kind) {
    case nve::EventKind::Press:
    case nve::EventKind::LongPress:
      writer.WritePropertyName(L"x");
      writer.WriteDouble(static_cast<double>(ev.x));
      writer.WritePropertyName(L"y");
      writer.WriteDouble(static_cast<double>(ev.y));
      break;
    case nve::EventKind::Scroll:
      writer.WritePropertyName(L"x");
      writer.WriteDouble(static_cast<double>(ev.x));
      writer.WritePropertyName(L"y");
      writer.WriteDouble(static_cast<double>(ev.y));
      writer.WritePropertyName(L"contentWidth");
      writer.WriteDouble(static_cast<double>(ev.width));
      writer.WritePropertyName(L"contentHeight");
      writer.WriteDouble(static_cast<double>(ev.height));
      break;
    case nve::EventKind::TextChange:
      writer.WritePropertyName(L"text");
      writer.WriteString(winrt::to_hstring(ev.text));
      break;
    case nve::EventKind::Layout:
      writer.WritePropertyName(L"x");
      writer.WriteDouble(static_cast<double>(ev.x));
      writer.WritePropertyName(L"y");
      writer.WriteDouble(static_cast<double>(ev.y));
      writer.WritePropertyName(L"width");
      writer.WriteDouble(static_cast<double>(ev.width));
      writer.WritePropertyName(L"height");
      writer.WriteDouble(static_cast<double>(ev.height));
      break;
    case nve::EventKind::Custom:
      if (!ev.payload_bytes.empty()) {
        writer.WritePropertyName(L"payloadJson");
        writer.WriteString(winrt::to_hstring(ev.payload_as_string()));
      }
      break;
    case nve::EventKind::Focus:
    case nve::EventKind::Blur:
      break;
  }
  writer.WriteObjectEnd();
}

winrt::com_ptr<NveRnwComponentUserData> get_user_data(rnw::ComponentView const& view) {
  if (!view) {
    return nullptr;
  }
  auto raw = view.UserData();
  if (!raw) {
    return nullptr;
  }
  winrt::com_ptr<NveRnwComponentUserData> out;
  out.copy_from(winrt::get_self<NveRnwComponentUserData>(raw));
  return out;
}

winrt::com_ptr<NveRnwComponentUserData> initialize_view_host(
    rnw_comp::ViewComponentView const& view,
    const NveRnwComponentRegistrationConfig& config) {
  auto user_data = winrt::make_self<NveRnwComponentUserData>();
  view.UserData(user_data.as<winrt::Windows::Foundation::IInspectable>());

  auto weak_data = user_data->get_weak();
  user_data->host->set_event_callback([weak_data](const nve::DecodedEvent& ev) {
    auto strong = weak_data.get();
    if (!strong || !strong->event_emitter) {
      return;
    }
    const nve::DecodedEvent copy = ev;
    strong->event_emitter.DispatchEvent(
        winrt::to_hstring(copy.name),
        [copy](rnw::IJSValueWriter const& writer) { write_decoded_event_to_js(writer, copy); });
  });

  const rnw::LayoutMetrics metrics = view.LayoutMetrics();
  NveSurfaceConfig surface_cfg{};
  surface_cfg.surface_id = (view.Tag() > 0) ? static_cast<uint32_t>(view.Tag()) : 1u;
  surface_cfg.width = (metrics.Frame.Width > 0.0f) ? metrics.Frame.Width : 480.0f;
  surface_cfg.height = (metrics.Frame.Height > 0.0f) ? metrics.Frame.Height : 320.0f;
  surface_cfg.scale_factor = (metrics.PointScaleFactor > 0.0f) ? metrics.PointScaleFactor : 1.0f;

  if (!config.default_dylib_path.empty()) {
    (void)user_data->host->load_dylib(config.default_dylib_path, surface_cfg);
  } else if (config.default_interface != nullptr) {
    (void)user_data->host->attach_interface(config.default_interface, surface_cfg);
  }

  user_data->pointer_released_revoker = view.PointerReleased(
      winrt::auto_revoke,
      [weak_data](
          winrt::Windows::Foundation::IInspectable const& sender,
          rnw_input::PointerRoutedEventArgs const& args) {
        auto strong = weak_data.get();
        if (!strong || !strong->host) {
          return;
        }
        auto comp_view = sender.try_as<rnw::ComponentView>();
        if (!comp_view) {
          return;
        }
        auto pt = args.GetCurrentPoint(comp_view.Tag());
        const auto pos = pt.Position();
        if (strong->host->dispatch_click_at(pos.X, pos.Y) > 0) {
          args.Handled(true);
        }
      });

  user_data->destroying_revoker = view.Destroying(
      winrt::auto_revoke,
      [weak_data](
          winrt::Windows::Foundation::IInspectable const&,
          rnw::ComponentView const&) {
        if (auto strong = weak_data.get()) {
          strong->event_emitter = nullptr;
          if (strong->host) {
            strong->host->reset();
          }
        }
      });

  return user_data;
}

} // namespace

std::shared_ptr<NveRnwComponentHost> get_nve_host_from_component_view(
    rnw::ComponentView const& view) {
  auto user_data = get_user_data(view);
  return user_data ? user_data->host : nullptr;
}

bool register_nve_rnw_component(
    rnw::IReactPackageBuilder const& package_builder,
    NveRnwComponentRegistrationConfig config) {
  if (!package_builder) {
    return false;
  }
  auto fabric_builder = package_builder.try_as<rnw::IReactPackageBuilderFabric>();
  if (!fabric_builder) {
    return false;
  }

  fabric_builder.AddViewComponent(
      config.component_name,
      [config = std::move(config)](rnw::IReactViewComponentBuilder const& builder) {
        auto comp_builder = builder.as<rnw_comp::IReactCompositionViewComponentBuilder>();

        if (config.use_content_island) {
          comp_builder.SetContentIslandComponentViewInitializer(
              [config](rnw_comp::ContentIslandComponentView const& view) {
                auto user_data = initialize_view_host(view, config);
                if (config.create_content_island) {
                  auto island = config.create_content_island(view, user_data->host);
                  if (island) {
                    view.Connect(island);
                  }
                }
                if (config.on_view_initialized) {
                  config.on_view_initialized(view, user_data->host);
                }
              });
        } else {
          comp_builder.SetViewComponentViewInitializer(
              [config](rnw_comp::ViewComponentView const& view) {
                auto user_data = initialize_view_host(view, config);
                if (config.on_view_initialized) {
                  config.on_view_initialized(view, user_data->host);
                }
              });
        }

        comp_builder.SetUpdateLayoutMetricsHandler(
            [](rnw::ComponentView const& source,
               rnw::LayoutMetrics const& new_metrics,
               rnw::LayoutMetrics const& /*old_metrics*/) {
              auto user_data = get_user_data(source);
              if (user_data && user_data->host) {
                user_data->host->set_surface_size(
                    new_metrics.Frame.Width,
                    new_metrics.Frame.Height,
                    new_metrics.PointScaleFactor);
              }
            });

        builder.SetUpdateEventEmitterHandler(
            [](rnw::ComponentView const& source, rnw::EventEmitter const& emitter) {
              auto user_data = get_user_data(source);
              if (user_data) {
                user_data->event_emitter = emitter;
              }
            });

        builder.SetCustomCommandHandler(
            [](rnw::ComponentView const& source, rnw::HandleCommandArgs const& args) {
              auto user_data = get_user_data(source);
              if (!user_data || !user_data->host) {
                return;
              }
              const auto cmd = args.CommandName();
              if (cmd == L"render") {
                (void)user_data->host->render_frame(nullptr);
                args.Handled(true);
              } else if (cmd == L"reloadDylib" || cmd == L"updatePropsJson") {
                auto reader = args.CommandArgs();
                if (reader && reader.ValueType() == rnw::JSValueType::Array &&
                    reader.GetNextArrayItem() &&
                    reader.ValueType() == rnw::JSValueType::String) {
                  const std::string arg = winrt::to_string(reader.GetString());
                  if (cmd == L"reloadDylib") {
                    (void)user_data->host->reload_dylib(arg);
                  } else {
                    (void)user_data->host->update_props_json(arg);
                  }
                  args.Handled(true);
                }
              }
            });
      });

  return true;
}

#endif // defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

} // namespace nve::win
