#include "NveRnwComponentView.hpp"

#if defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

#include <nve/host/utf8.h>

#include <winrt/Microsoft.UI.Dispatching.h>

#include <string_view>
#include <utility>
#include <vector>

namespace nve::win {

namespace {

namespace rnw = winrt::Microsoft::ReactNative;
namespace rnw_comp = winrt::Microsoft::ReactNative::Composition;
namespace rnw_input = winrt::Microsoft::ReactNative::Composition::Input;

/// Owned copy of one app-emitted event (the runtime's batch is borrowed for the sink call only).
struct EmittedEvent {
  NveDirectEvent ev{};
  std::string name;
  std::string text;
  std::string value_str;  // NVE_VAL_STR
  std::string payload;    // NVE_VAL_BYTES (custom event payload, forwarded as `payloadJson` when UTF-8)
};

struct NveRnwComponentUserData : winrt::implements<NveRnwComponentUserData, winrt::Windows::Foundation::IInspectable> {
  std::shared_ptr<NveWinHost> host;
  rnw::EventEmitter event_emitter{nullptr};
  rnw::ComponentView::PointerReleased_revoker pointer_released_revoker{};
  rnw::ComponentView::Destroying_revoker destroying_revoker{};
};

std::string_view slice(NveStrSlice s) noexcept {
  return s.ptr != nullptr ? std::string_view(reinterpret_cast<const char*>(s.ptr), s.len) : std::string_view();
}

void write_event(rnw::IJSValueWriter const& writer, const EmittedEvent& e) {
  writer.WriteObjectBegin();
  writer.WritePropertyName(L"surfaceId");
  writer.WriteInt64(static_cast<int64_t>(e.ev.surface_id));
  writer.WritePropertyName(L"nodeId");
  writer.WriteInt64(static_cast<int64_t>(e.ev.node_id));
  writer.WritePropertyName(L"timestampUs");
  writer.WriteInt64(static_cast<int64_t>(e.ev.timestamp_us));
  writer.WritePropertyName(L"kind");
  writer.WriteInt64(static_cast<int64_t>(e.ev.kind));
  writer.WritePropertyName(L"name");
  writer.WriteString(winrt::to_hstring(e.name));
  for (const auto& [key, v] : {std::pair{L"x", e.ev.x}, std::pair{L"y", e.ev.y}, std::pair{L"width", e.ev.width},
                               std::pair{L"height", e.ev.height}}) {
    writer.WritePropertyName(key);
    writer.WriteDouble(static_cast<double>(v));
  }
  if (!e.text.empty()) {
    writer.WritePropertyName(L"text");
    writer.WriteString(winrt::to_hstring(e.text));
  }
  if (!e.payload.empty() &&
      nve_utf8_valid(reinterpret_cast<const uint8_t*>(e.payload.data()), e.payload.size()) != 0) {
    writer.WritePropertyName(L"payloadJson");
    writer.WriteString(winrt::to_hstring(e.payload));
  }
  writer.WritePropertyName(L"value");
  switch (e.ev.value.tag) {
    case NVE_VAL_BOOL: writer.WriteBoolean(e.ev.value.data.bool_val != 0u); break;
    case NVE_VAL_I64: writer.WriteInt64(e.ev.value.data.i64_val); break;
    case NVE_VAL_F64: writer.WriteDouble(e.ev.value.data.f64_val); break;
    case NVE_VAL_STR: writer.WriteString(winrt::to_hstring(e.value_str)); break;
    default: writer.WriteNull(); break;  // bytes travel as `payloadJson`; arrays / maps are not forwarded
  }
  writer.WriteObjectEnd();
}

winrt::com_ptr<NveRnwComponentUserData> get_user_data(rnw::ComponentView const& view) {
  if (!view) return nullptr;
  auto raw = view.UserData();
  if (!raw) return nullptr;
  winrt::com_ptr<NveRnwComponentUserData> out;
  out.copy_from(winrt::get_self<NveRnwComponentUserData>(raw));
  return out;
}

winrt::com_ptr<NveRnwComponentUserData> initialize_view_host(rnw_comp::ViewComponentView const& view,
                                                             const NveRnwComponentRegistrationConfig& config) {
  auto user_data = winrt::make_self<NveRnwComponentUserData>();
  view.UserData(user_data.as<winrt::Windows::Foundation::IInspectable>());
  const rnw::LayoutMetrics metrics = view.LayoutMetrics();
  user_data->host = NveWinHost::create(WinHostConfig{
      .surface_id = view.Tag() > 0 ? static_cast<uint32_t>(view.Tag()) : 1u,
      .width = metrics.Frame.Width > 0.0f ? metrics.Frame.Width : 480.0f,
      .height = metrics.Frame.Height > 0.0f ? metrics.Frame.Height : 320.0f,
      .scale_factor = metrics.PointScaleFactor > 0.0f ? metrics.PointScaleFactor : 1.0f,
  });
  if (user_data->host == nullptr) return user_data;  // no runtime / DirectWrite: an empty view

  // Runtime wakes (timers, paired frames) arrive on any thread: pump on this view's UI thread.
  const auto queue = winrt::Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread();
  if (queue) {
    user_data->host->set_wake_poster([queue, host = std::weak_ptr<NveWinHost>(user_data->host)] {
      (void)queue.TryEnqueue([host] {
        if (const auto h = host.lock()) (void)h->pump();
      });
    });
  }

  auto weak = user_data->get_weak();
  user_data->host->set_event_callback([weak](std::span<const NveDirectEvent> events) {
    auto strong = weak.get();
    if (!strong || !strong->event_emitter) return;
    for (const NveDirectEvent& ev : events) {
      EmittedEvent e{ev, std::string(slice(ev.name)), std::string(slice(ev.text)),
                     ev.value.tag == NVE_VAL_STR ? std::string(slice(ev.value.data.str_val)) : std::string(),
                     ev.value.tag == NVE_VAL_BYTES && ev.value.data.bytes_val.ptr != nullptr
                         ? std::string(reinterpret_cast<const char*>(ev.value.data.bytes_val.ptr), ev.value.data.bytes_val.len)
                         : std::string()};
      e.ev.name = e.ev.text = NveStrSlice{nullptr, 0u};  // borrowed: never touched after this call
      const std::wstring name = e.name.empty() ? std::wstring(L"nveEvent") : std::wstring(winrt::to_hstring(e.name));
      strong->event_emitter.DispatchEvent(name, [e = std::move(e)](rnw::IJSValueWriter const& w) { write_event(w, e); });
    }
  });

  if (!config.default_dylib_path.empty()) {
    (void)user_data->host->load_dylib(config.default_dylib_path);
  } else if (config.default_interface != nullptr) {
    (void)user_data->host->attach_interface(config.default_interface);
  }

  user_data->pointer_released_revoker = view.PointerReleased(
      winrt::auto_revoke, [weak](winrt::Windows::Foundation::IInspectable const& sender,
                                 rnw_input::PointerRoutedEventArgs const& args) {
        auto strong = weak.get();
        auto comp_view = sender.try_as<rnw::ComponentView>();
        // A strong host copy: the press may make JS destroy the view (`Destroying` resets `host`).
        const std::shared_ptr<NveWinHost> host = strong ? strong->host : nullptr;
        if (!host || !comp_view) return;
        const auto pos = args.GetCurrentPoint(comp_view.Tag()).Position();
        if (host->press_at(pos.X, pos.Y).dispatched) args.Handled(true);
      });
  user_data->destroying_revoker = view.Destroying(
      winrt::auto_revoke, [weak](winrt::Windows::Foundation::IInspectable const&, rnw::ComponentView const&) {
        if (auto strong = weak.get()) {
          strong->event_emitter = nullptr;
          if (strong->host != nullptr) strong->host->set_wake_poster({});
          strong->host.reset();
        }
      });
  return user_data;
}

}  // namespace

std::shared_ptr<NveWinHost> get_nve_host_from_component_view(rnw::ComponentView const& view) {
  auto user_data = get_user_data(view);
  return user_data ? user_data->host : nullptr;
}

bool register_nve_rnw_component(rnw::IReactPackageBuilder const& package_builder, NveRnwComponentRegistrationConfig config) {
  if (!package_builder) return false;
  auto fabric_builder = package_builder.try_as<rnw::IReactPackageBuilderFabric>();
  if (!fabric_builder) return false;

  fabric_builder.AddViewComponent(config.component_name, [config = std::move(config)](
                                                             rnw::IReactViewComponentBuilder const& builder) {
    auto comp_builder = builder.as<rnw_comp::IReactCompositionViewComponentBuilder>();
    if (config.use_content_island) {
      comp_builder.SetContentIslandComponentViewInitializer([config](rnw_comp::ContentIslandComponentView const& view) {
        auto user_data = initialize_view_host(view, config);
        if (config.create_content_island) {
          if (auto island = config.create_content_island(view, user_data->host)) view.Connect(island);
        }
        if (config.on_view_initialized) config.on_view_initialized(view, user_data->host);
      });
    } else {
      comp_builder.SetViewComponentViewInitializer([config](rnw_comp::ViewComponentView const& view) {
        auto user_data = initialize_view_host(view, config);
        if (config.on_view_initialized) config.on_view_initialized(view, user_data->host);
      });
    }
    comp_builder.SetUpdateLayoutMetricsHandler(
        [](rnw::ComponentView const& source, rnw::LayoutMetrics const& metrics, rnw::LayoutMetrics const&) {
          const auto user_data = get_user_data(source);
          if (const std::shared_ptr<NveWinHost> host = user_data ? user_data->host : nullptr) {
            host->set_surface_size(metrics.Frame.Width, metrics.Frame.Height, metrics.PointScaleFactor);
          }
        });
    builder.SetUpdateEventEmitterHandler([](rnw::ComponentView const& source, rnw::EventEmitter const& emitter) {
      if (auto user_data = get_user_data(source)) user_data->event_emitter = emitter;
    });
    builder.SetCustomCommandHandler([](rnw::ComponentView const& source, rnw::HandleCommandArgs const& args) {
      auto user_data = get_user_data(source);
      // A strong copy: the pump / reload may run app code that destroys the view.
      const std::shared_ptr<NveWinHost> host = user_data ? user_data->host : nullptr;
      if (!host) return;
      const auto cmd = args.CommandName();
      if (cmd == L"render") {
        (void)host->pump();
        args.Handled(true);
      } else if (cmd == L"reloadDylib") {
        auto reader = args.CommandArgs();
        if (reader && reader.ValueType() == rnw::JSValueType::Array && reader.GetNextArrayItem() &&
            reader.ValueType() == rnw::JSValueType::String) {
          (void)host->reload_dylib(winrt::to_string(reader.GetString()));
          args.Handled(true);
        }
      }
    });
  });
  return true;
}

}  // namespace nve::win

#endif  // defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC
