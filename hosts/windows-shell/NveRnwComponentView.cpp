#include "NveRnwComponentView.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <utility>

namespace nve::win {

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

bool NveRnwComponentHost::update_props(const nve::PropMap& props) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.update_props(props, /*auto_render=*/false);
  if (dirty) {
    const int32_t rc = session_.render(nullptr);
    if (rc == 0) {
      sync_visuals_and_notify(/*check_raw_props=*/true);
    }
  }
  (void)session_.drain_events();
  return dirty;
}

bool NveRnwComponentHost::update_props(const NveMapEntry* entries, uint32_t count) {
  if (!session_.is_valid()) {
    return false;
  }
  const bool dirty = session_.update_props(entries, count, /*auto_render=*/false);
  if (dirty) {
    const int32_t rc = session_.render(nullptr);
    if (rc == 0) {
      sync_visuals_and_notify(/*check_raw_props=*/true);
    }
  }
  (void)session_.drain_events();
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
  const NveEvent ev = nve::make_scroll_event(
      surface_id(),
      node_id,
      nve::current_timestamp_us(),
      offset_x,
      offset_y,
      content_width,
      content_height);

  const uint32_t dirty =
      session_.dispatch_events(nve::make_single_event_batch(ev), /*auto_render=*/false);
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
    case nve::EventKind::CompositionStart:
    case nve::EventKind::CompositionUpdate:
    case nve::EventKind::CompositionEnd:
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
    default:
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
              } else if (cmd == L"reloadDylib") {
                auto reader = args.CommandArgs();
                if (reader && reader.ValueType() == rnw::JSValueType::Array &&
                    reader.GetNextArrayItem() &&
                    reader.ValueType() == rnw::JSValueType::String) {
                  const std::string arg = winrt::to_string(reader.GetString());
                  (void)user_data->host->reload_dylib(arg);
                  args.Handled(true);
                }
              }
            });
      });

  return true;
}

#endif // defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

} // namespace nve::win
