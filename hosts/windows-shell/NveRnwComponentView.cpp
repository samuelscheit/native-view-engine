#include "NveRnwComponentView.hpp"

#include <algorithm>
#include <cmath>
#include <utility>

namespace nve::win {

// -----------------------------------------------------------------------------
// NveWinVisualTree
// -----------------------------------------------------------------------------

void NveWinVisualTree::Adapter::update_root(const nve::RenderedNodeSnapshot& snap) {
  root_background_rgba = snap.background_rgba;
  root_layout = snap.layout;
}

NveWinNodeVisual NveWinVisualTree::Adapter::create_view(const nve::RenderedNodeSnapshot& snap) {
  ++created_count;
  NveWinNodeVisual visual{};
  visual.node_id = snap.node_id;
  visual.kind = snap.kind;
  visual.view_tag = nve::view_tag_for_snapshot(snap);
  return visual;
}

void NveWinVisualTree::Adapter::update_view(
    NveWinNodeVisual& view,
    const nve::RenderedNodeSnapshot& snap) {
  ++updated_count;
  view.node_id = snap.node_id;
  view.kind = snap.kind;
  view.view_tag = nve::view_tag_for_snapshot(snap);
  view.is_text_leaf = (snap.kind == NVE_COMP_TEXT && snap.children.empty());
  view.interactive =
      (snap.kind == NVE_COMP_PRESSABLE || (snap.event_mask & nve::kEventMaskPress) != 0);
  view.custom_name = snap.custom_name;
  view.text = snap.text;
  view.layout = snap.layout;
  view.background_rgba = snap.background_rgba;
  view.text_color_rgba = snap.text_color_rgba;
  view.border_color_rgba = snap.border_color_rgba;
  view.border_width = snap.border_width;
  view.border_radius = snap.border_radius;
  view.opacity = snap.opacity;
  view.font_size = snap.font_size;
  view.z_index = snap.z_index;
  view.event_mask = snap.event_mask;
}

void NveWinVisualTree::Adapter::destroy_view(uint32_t /*node_id*/, NveWinNodeVisual& /*view*/) {
  ++destroyed_count;
}

void NveWinVisualTree::sync(const std::vector<nve::RenderedNodeSnapshot>& snapshots) {
  adapter_.root_children.clear();
  reconciler_.sync(snapshots, adapter_);
}

void NveWinVisualTree::clear() {
  reconciler_.clear(adapter_);
  adapter_.root_children.clear();
  adapter_.root_background_rgba = 0;
  adapter_.root_layout = nve::LayoutRect{};
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
  while (cur_parent != 0 && cur_parent != NVE_ROOT_NODE_ID && hops < 512) {
    const NveWinNodeVisual* parent = reconciler_.find(cur_parent);
    if (parent == nullptr) {
      break;
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
    std::vector<FlattenedVisualNode>& out,
    uint32_t depth) const {
  if (depth > 512) {
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
  });

  if (node->children.empty()) {
    return;
  }

  std::vector<uint32_t> ordered = node->children;
  std::stable_sort(ordered.begin(), ordered.end(), [this](uint32_t a, uint32_t b) {
    const NveWinNodeVisual* va = reconciler_.find(a);
    const NveWinNodeVisual* vb = reconciler_.find(b);
    const int32_t za = (va != nullptr) ? va->z_index : 0;
    const int32_t zb = (vb != nullptr) ? vb->z_index : 0;
    return za < zb;
  });

  for (uint32_t child_id : ordered) {
    append_draw_order(child_id, abs_x, abs_y, effective_opacity, out, depth + 1);
  }
}

std::vector<FlattenedVisualNode> NveWinVisualTree::build_draw_order() const {
  std::vector<FlattenedVisualNode> out;
  out.reserve(reconciler_.size());

  std::vector<uint32_t> ordered_roots = adapter_.root_children;
  std::stable_sort(ordered_roots.begin(), ordered_roots.end(), [this](uint32_t a, uint32_t b) {
    const NveWinNodeVisual* va = reconciler_.find(a);
    const NveWinNodeVisual* vb = reconciler_.find(b);
    const int32_t za = (va != nullptr) ? va->z_index : 0;
    const int32_t zb = (vb != nullptr) ? vb->z_index : 0;
    return za < zb;
  });

  for (uint32_t child_id : ordered_roots) {
    append_draw_order(child_id, 0.0f, 0.0f, 1.0f, out, 0);
  }
  return out;
}

std::optional<HitTestResult> NveWinVisualTree::hit_test_interactive(
    float surface_x,
    float surface_y) const {
  const std::vector<FlattenedVisualNode> order = build_draw_order();
  for (auto it = order.rbegin(); it != order.rend(); ++it) {
    const NveWinNodeVisual* node = reconciler_.find(it->node_id);
    if (node == nullptr || !node->interactive) {
      continue;
    }
    if (it->effective_opacity <= 0.0f) {
      continue;
    }
    if (node->layout.width <= 0.0f || node->layout.height <= 0.0f) {
      continue;
    }
    const float x0 = it->abs_x;
    const float y0 = it->abs_y;
    const float x1 = x0 + node->layout.width;
    const float y1 = y0 + node->layout.height;
    if (surface_x >= x0 && surface_x <= x1 && surface_y >= y0 && surface_y <= y1) {
      return HitTestResult{
          .node_id = node->node_id,
          .local_x = surface_x - x0,
          .local_y = surface_y - y0,
      };
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
  sync_visuals_and_notify();
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
  sync_visuals_and_notify();
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
  sync_visuals_and_notify();
  (void)session_.drain_events();
  return true;
}

void NveRnwComponentHost::reset() noexcept {
  visual_tree_.clear();
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
  if (session_.is_valid()) {
    sync_visuals_and_notify();
  }
}

bool NveRnwComponentHost::update_props_json(std::string_view props_json) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.update_props_json(props_json, /*auto_render=*/false);
  if (dirty) {
    (void)render_frame(nullptr);
  } else {
    (void)session_.drain_events();
  }
  return dirty;
}

bool NveRnwComponentHost::apply_hot_patch_json(std::string_view patch_json) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.bridge()->apply_hot_patch_json(patch_json, /*auto_render=*/false);
  if (dirty) {
    (void)render_frame(nullptr);
  }
  return dirty;
}

int32_t NveRnwComponentHost::render_frame(NveFrameStats* out_stats) {
  if (!session_.is_valid()) {
    return -100;
  }
  const int32_t rc = session_.render(out_stats);
  if (rc == 0) {
    sync_visuals_and_notify();
  }
  (void)session_.drain_events();
  return rc;
}

uint32_t NveRnwComponentHost::dispatch_press(uint32_t node_id, float local_x, float local_y) {
  if (!session_.is_valid()) {
    return 0;
  }
  const uint32_t dirty =
      session_.dispatch_press(node_id, local_x, local_y, /*auto_render=*/false);
  if (dirty != 0) {
    (void)render_frame(nullptr);
  } else {
    (void)session_.drain_events();
  }
  return dirty;
}

uint32_t NveRnwComponentHost::dispatch_click_at(float surface_x, float surface_y) {
  if (!session_.is_valid()) {
    return 0;
  }
  const auto hit = visual_tree_.hit_test_interactive(surface_x, surface_y);
  if (!hit.has_value()) {
    return 0;
  }
  return dispatch_press(hit->node_id, hit->local_x, hit->local_y);
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

void NveRnwComponentHost::sync_visuals_and_notify() {
  visual_tree_.sync(session_.snapshot_nodes());
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
