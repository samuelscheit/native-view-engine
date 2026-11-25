#include "NveWinNativeViews.hpp"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <utility>

namespace nve::win {

namespace {

#if NVE_NV_ENABLE_WEBVIEW || NVE_NV_ENABLE_VIDEO
std::string normalize_url_for_scheme_check(std::string_view raw) {
  std::string norm;
  norm.reserve(raw.size());
  for (char ch : raw) {
    const unsigned char u = static_cast<unsigned char>(ch);
    if (u <= 0x20u || u == 0x7Fu) continue;
    norm.push_back(static_cast<char>(std::tolower(u)));
  }
  return norm;
}
#endif

#if NVE_NV_ENABLE_BLUR
float base_blur_radius(nve::BlurMaterial mat) noexcept {
  switch (mat) {
    case nve::BlurMaterial::UltraThin:
      return 12.0f;
    case nve::BlurMaterial::Thin:
      return 18.0f;
    case nve::BlurMaterial::Regular:
      return 24.0f;
    case nve::BlurMaterial::Thick:
      return 32.0f;
    case nve::BlurMaterial::Chrome:
      return 28.0f;
    case nve::BlurMaterial::SystemMaterial:
      return 24.0f;
  }
  return 24.0f;
}

float base_material_alpha(nve::BlurMaterial mat) noexcept {
  switch (mat) {
    case nve::BlurMaterial::UltraThin:
      return 0.45f;
    case nve::BlurMaterial::Thin:
      return 0.60f;
    case nve::BlurMaterial::Regular:
      return 0.72f;
    case nve::BlurMaterial::Thick:
      return 0.85f;
    case nve::BlurMaterial::Chrome:
      return 0.80f;
    case nve::BlurMaterial::SystemMaterial:
      return 0.75f;
  }
  return 0.72f;
}

bool is_blur_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindBlurSurface || k == nve::kNativeKindBlurAlias;
}

nve::NativeViewAdapter make_blur_adapter() {
  nve::NativeViewAdapter adapter{};
  adapter.create = [](uint32_t, std::string_view, const nve::NvnpDecoder*) -> void* {
    return new NveWinBlurSurfaceView();
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::BlurSurfaceProps props{};
    if (nve::decode_blur_surface_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveWinBlurSurfaceView*>(inst)->apply_props(props);
    }
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* view = static_cast<NveWinBlurSurfaceView*>(inst);
    view->mark_unmounted();
    delete view;
  };
  return adapter;
}
#endif // NVE_NV_ENABLE_BLUR

#if NVE_NV_ENABLE_WEBVIEW
bool is_webview_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindWebView || k == nve::kNativeKindWebViewAlias;
}

nve::NativeViewAdapter make_webview_adapter(
    std::function<std::unique_ptr<NveWinWebViewBackend>(uint32_t)> factory) {
  nve::NativeViewAdapter adapter{};
  adapter.create = [factory = std::move(factory)](
                       uint32_t node_id, std::string_view, const nve::NvnpDecoder*) -> void* {
    std::unique_ptr<NveWinWebViewBackend> backend;
    if (factory) {
      backend = factory(node_id);
    }
    return new NveWinWebView(std::move(backend));
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::WebViewProps props{};
    if (nve::decode_web_view_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveWinWebView*>(inst)->apply_props(props);
    }
  };
  adapter.dispatch_command =
      [](void* inst, uint32_t, std::string_view, const nve::NvncDecoder& dec) -> bool {
    if (inst == nullptr) return false;
    nve::WebViewCommand cmd{};
    if (nve::decode_web_view_command(dec, cmd) != nve::NativeViewCodecError::Ok) {
      return false;
    }
    return static_cast<NveWinWebView*>(inst)->dispatch_command(cmd);
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* view = static_cast<NveWinWebView*>(inst);
    view->unmount();
    delete view;
  };
  return adapter;
}
#endif // NVE_NV_ENABLE_WEBVIEW

#if NVE_NV_ENABLE_VIDEO
bool is_video_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindVideoPlayer || k == nve::kNativeKindVideoAlias;
}

nve::NativeViewAdapter make_video_adapter(
    std::function<std::unique_ptr<NveWinVideoBackend>(uint32_t)> factory) {
  nve::NativeViewAdapter adapter{};
  adapter.create = [factory = std::move(factory)](
                       uint32_t node_id, std::string_view, const nve::NvnpDecoder*) -> void* {
    std::unique_ptr<NveWinVideoBackend> backend;
    if (factory) {
      backend = factory(node_id);
    }
    return new NveWinVideoPlayer(std::move(backend));
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::VideoPlayerProps props{};
    if (nve::decode_video_player_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveWinVideoPlayer*>(inst)->apply_props(props);
    }
  };
  adapter.dispatch_command =
      [](void* inst, uint32_t, std::string_view, const nve::NvncDecoder& dec) -> bool {
    if (inst == nullptr) return false;
    nve::VideoPlayerCommand cmd{};
    if (nve::decode_video_player_command(dec, cmd) != nve::NativeViewCodecError::Ok) {
      return false;
    }
    return static_cast<NveWinVideoPlayer*>(inst)->dispatch_command(cmd);
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* player = static_cast<NveWinVideoPlayer*>(inst);
    player->unmount();
    delete player;
  };
  return adapter;
}
#endif // NVE_NV_ENABLE_VIDEO

} // namespace

#if NVE_NV_ENABLE_BLUR
WinTintColor unpack_win_tint_rrggbbaa(uint32_t rrggbbaa) noexcept {
  WinTintColor out{};
  out.r8 = static_cast<uint8_t>((rrggbbaa >> 24u) & 0xFFu);
  out.g8 = static_cast<uint8_t>((rrggbbaa >> 16u) & 0xFFu);
  out.b8 = static_cast<uint8_t>((rrggbbaa >> 8u) & 0xFFu);
  out.a8 = static_cast<uint8_t>(rrggbbaa & 0xFFu);
  out.r = static_cast<float>(out.r8) / 255.0f;
  out.g = static_cast<float>(out.g8) / 255.0f;
  out.b = static_cast<float>(out.b8) / 255.0f;
  out.a = static_cast<float>(out.a8) / 255.0f;
  return out;
}

WinSystemBackdropType map_win_backdrop_type(nve::BlurMaterial material) noexcept {
  switch (material) {
    case nve::BlurMaterial::UltraThin:
    case nve::BlurMaterial::Thin:
      return WinSystemBackdropType::TransientWindow;
    case nve::BlurMaterial::Regular:
    case nve::BlurMaterial::SystemMaterial:
      return WinSystemBackdropType::MainWindow;
    case nve::BlurMaterial::Thick:
    case nve::BlurMaterial::Chrome:
      return WinSystemBackdropType::TabbedWindow;
  }
  return WinSystemBackdropType::MainWindow;
}

void NveWinBlurSurfaceView::apply_props(const nve::BlurSurfaceProps& props) noexcept {
  const float clamped =
      std::isfinite(props.intensity) ? std::clamp(props.intensity, 0.0f, 1.0f) : 1.0f;
  material_ = props.material;
  backdrop_type_ = map_win_backdrop_type(props.material);
  intensity_ = clamped;
  blur_radius_ = base_blur_radius(props.material) * clamped;
  effective_alpha_ = base_material_alpha(props.material) * clamped;
  has_tint_ = props.tint.has_value();
  tint_rgba_ = props.tint.value_or(0u);
  tint_color_ = has_tint_ ? unpack_win_tint_rrggbbaa(tint_rgba_) : WinTintColor{};
  ++update_count_;
}

#if defined(_WIN32)
bool NveWinBlurSurfaceView::apply_to_hwnd(HWND hwnd) const noexcept {
  if (hwnd == nullptr) return false;
  HMODULE dwm = GetModuleHandleW(L"dwmapi.dll");
  if (dwm == nullptr) {
    dwm = LoadLibraryW(L"dwmapi.dll");
  }
  if (dwm == nullptr) return false;
  using DwmSetWindowAttributeFn = HRESULT(WINAPI*)(HWND, DWORD, LPCVOID, DWORD);
  auto fn = reinterpret_cast<DwmSetWindowAttributeFn>(
      GetProcAddress(dwm, "DwmSetWindowAttribute"));
  if (fn == nullptr) return false;
  constexpr DWORD kDwmwaSystemBackdropType = 38u;
  const uint32_t backdrop = static_cast<uint32_t>(backdrop_type_);
  return SUCCEEDED(fn(hwnd, kDwmwaSystemBackdropType, &backdrop, sizeof(backdrop)));
}
#endif

NveWinBlurSurfaceView* blur_surface_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_blur_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveWinBlurSurfaceView*>(registry.instance_for(node_id));
}
#endif // NVE_NV_ENABLE_BLUR

#if NVE_NV_ENABLE_WEBVIEW
bool is_safe_win_webview_url(std::string_view url) noexcept {
  const std::string norm = normalize_url_for_scheme_check(url);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank";
}

NveWinWebView::NveWinWebView(std::unique_ptr<NveWinWebViewBackend> backend)
    : backend_(std::move(backend)) {
  if (backend_) {
    backend_->set_allows_back_forward(allows_back_forward_);
  }
}

NveWinWebView::~NveWinWebView() {
  unmount();
}

bool NveWinWebView::load_safe_url(std::string_view url) {
  if (!is_safe_win_webview_url(url)) {
    return false;
  }
  if (backend_ && !backend_->navigate(url)) {
    return false;
  }
  last_requested_url_.assign(url);
  ++navigation_count_;
  return true;
}

void NveWinWebView::apply_props(const nve::WebViewProps& props) {
  allows_back_forward_ = props.allows_back_forward;
  if (backend_) {
    backend_->set_allows_back_forward(allows_back_forward_);
  }
  if (last_requested_url_ != props.url) {
    (void)load_safe_url(props.url);
  }
}

bool NveWinWebView::dispatch_command(const nve::WebViewCommand& cmd) {
  switch (cmd.kind) {
    case nve::WebViewCommandKind::Navigate:
      return load_safe_url(cmd.text);
    case nve::WebViewCommandKind::Reload:
      ++reload_count_;
      if (backend_) backend_->reload();
      return true;
    case nve::WebViewCommandKind::GoBack:
      if (!allows_back_forward_) return false;
      ++go_back_count_;
      if (backend_) (void)backend_->go_back();
      return true;
    case nve::WebViewCommandKind::GoForward:
      if (!allows_back_forward_) return false;
      ++go_forward_count_;
      if (backend_) (void)backend_->go_forward();
      return true;
    case nve::WebViewCommandKind::EvaluateScript:
      last_evaluated_script_.assign(cmd.text);
      ++evaluate_script_count_;
      if (backend_) backend_->evaluate_script(cmd.text);
      return true;
  }
  return false;
}

void NveWinWebView::unmount() {
  if (unmounted_) return;
  unmounted_ = true;
  if (backend_) {
    backend_->close();
  }
}

NveWinWebView* web_view_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_webview_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveWinWebView*>(registry.instance_for(node_id));
}
#endif // NVE_NV_ENABLE_WEBVIEW

#if NVE_NV_ENABLE_VIDEO
bool is_safe_win_media_uri(std::string_view uri) noexcept {
  const std::string norm = normalize_url_for_scheme_check(uri);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm.starts_with("file://");
}

NveWinVideoPlayer::NveWinVideoPlayer(std::unique_ptr<NveWinVideoBackend> backend)
    : backend_(std::move(backend)) {}

NveWinVideoPlayer::~NveWinVideoPlayer() {
  unmount();
}

void NveWinVideoPlayer::apply_props(const nve::VideoPlayerProps& props) {
  const bool uri_changed = source_uri_ != props.source_uri;
  if (uri_changed && is_safe_win_media_uri(props.source_uri)) {
    source_uri_.assign(props.source_uri);
    if (backend_) backend_->set_source_uri(source_uri_);
  }
  autoplay_ = props.autoplay;
  muted_ = props.muted;
  looping_ = props.looping;
  controls_ = props.controls;
  if (backend_) {
    backend_->set_muted(muted_);
    backend_->set_looping(looping_);
    backend_->set_controls(controls_);
  }
  if (autoplay_ && !source_uri_.empty() && !playing_) {
    playing_ = true;
    ++play_count_;
    if (backend_) backend_->play();
  }
}

bool NveWinVideoPlayer::dispatch_command(const nve::VideoPlayerCommand& cmd) {
  switch (cmd.kind) {
    case nve::VideoPlayerCommandKind::Play:
      playing_ = true;
      ++play_count_;
      if (backend_) backend_->play();
      return true;
    case nve::VideoPlayerCommandKind::Pause:
      playing_ = false;
      ++pause_count_;
      if (backend_) backend_->pause();
      return true;
    case nve::VideoPlayerCommandKind::SeekMs:
      position_ms_ = cmd.seek_ms;
      ++seek_count_;
      if (backend_) backend_->seek_ms(position_ms_);
      return true;
    case nve::VideoPlayerCommandKind::SetVolume:
      if (!std::isfinite(cmd.volume)) return false;
      volume_ = std::clamp(cmd.volume, 0.0f, 1.0f);
      ++volume_change_count_;
      if (backend_) backend_->set_volume(volume_);
      return true;
  }
  return false;
}

void NveWinVideoPlayer::unmount() {
  if (unmounted_) return;
  unmounted_ = true;
  playing_ = false;
  if (backend_) {
    backend_->close();
  }
}

NveWinVideoPlayer* video_player_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_video_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveWinVideoPlayer*>(registry.instance_for(node_id));
}
#endif // NVE_NV_ENABLE_VIDEO

void register_win_native_views(nve::NativeViewRegistry& registry, NveWinNativeViewHooks hooks) {
  (void)registry;
  (void)hooks;
#if NVE_NV_ENABLE_BLUR
  registry.register_adapter(nve::kNativeKindBlurSurface, make_blur_adapter());
  registry.register_adapter(nve::kNativeKindBlurAlias, make_blur_adapter());
#endif
#if NVE_NV_ENABLE_WEBVIEW
  registry.register_adapter(nve::kNativeKindWebView, make_webview_adapter(hooks.create_webview_backend));
  registry.register_adapter(
      nve::kNativeKindWebViewAlias,
      make_webview_adapter(std::move(hooks.create_webview_backend)));
#endif
#if NVE_NV_ENABLE_VIDEO
  registry.register_adapter(nve::kNativeKindVideoPlayer, make_video_adapter(hooks.create_video_backend));
  registry.register_adapter(
      nve::kNativeKindVideoAlias,
      make_video_adapter(std::move(hooks.create_video_backend)));
#endif
}

void bind_win_native_views_to_fabric_host(
    NveFabricHostHandle host,
    nve::NativeViewRegistry& registry) {
  (void)registry;
  if (host == nullptr) return;
#if NVE_NV_ENABLE_BLUR
  registry.bind_to_fabric_host(host, nve::kNativeKindBlurSurface);
  registry.bind_to_fabric_host(host, nve::kNativeKindBlurAlias);
#endif
#if NVE_NV_ENABLE_WEBVIEW
  registry.bind_to_fabric_host(host, nve::kNativeKindWebView);
  registry.bind_to_fabric_host(host, nve::kNativeKindWebViewAlias);
#endif
#if NVE_NV_ENABLE_VIDEO
  registry.bind_to_fabric_host(host, nve::kNativeKindVideoPlayer);
  registry.bind_to_fabric_host(host, nve::kNativeKindVideoAlias);
#endif
}

} // namespace nve::win
