#include "NveGtkNativeViews.hpp"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdio>
#include <utility>

namespace nve::gtk {

namespace {

#if NVE_NV_ENABLE_WEBVIEW || NVE_NV_ENABLE_VIDEO
std::string normalize_url_scheme(std::string_view raw) {
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
void load_css_provider(GtkCssProvider* provider, const std::string& css) {
  if (provider == nullptr) return;
#if GTK_CHECK_VERSION(4, 12, 0)
  gtk_css_provider_load_from_string(provider, css.c_str());
#else
  gtk_css_provider_load_from_data(provider, css.data(), static_cast<gssize>(css.size()));
#endif
}

GtkTintRgba fallback_material_tint(nve::BlurMaterial mat, float intensity) noexcept {
  double base_alpha = 0.62;
  switch (mat) {
    case nve::BlurMaterial::UltraThin:
      base_alpha = 0.32;
      break;
    case nve::BlurMaterial::Thin:
      base_alpha = 0.48;
      break;
    case nve::BlurMaterial::Regular:
    case nve::BlurMaterial::SystemMaterial:
      base_alpha = 0.62;
      break;
    case nve::BlurMaterial::Thick:
      base_alpha = 0.80;
      break;
    case nve::BlurMaterial::Chrome:
      base_alpha = 0.72;
      break;
  }
  const double a = std::clamp(base_alpha * static_cast<double>(intensity), 0.0, 1.0);
  return GtkTintRgba{245u, 247u, 250u, static_cast<uint8_t>(std::lround(a * 255.0)), a};
}

bool is_blur_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindBlurSurface || k == nve::kNativeKindBlurAlias;
}

nve::NativeViewAdapter make_blur_adapter() {
  nve::NativeViewAdapter adapter{};
  adapter.create = [](uint32_t node_id, std::string_view, const nve::NvnpDecoder*) -> void* {
    return new NveGtkBlurSurface(node_id);
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::BlurSurfaceProps props{};
    if (nve::decode_blur_surface_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveGtkBlurSurface*>(inst)->apply_props(props);
    }
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* surface = static_cast<NveGtkBlurSurface*>(inst);
    surface->unmount();
    delete surface;
  };
  return adapter;
}
#endif // NVE_NV_ENABLE_BLUR

#if NVE_NV_ENABLE_WEBVIEW
bool is_webview_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindWebView || k == nve::kNativeKindWebViewAlias;
}

nve::NativeViewAdapter make_webview_adapter(
    std::function<std::unique_ptr<NveGtkWebViewBackend>(uint32_t)> factory) {
  nve::NativeViewAdapter adapter{};
  adapter.create = [factory = std::move(factory)](
                       uint32_t node_id, std::string_view, const nve::NvnpDecoder*) -> void* {
    if (!factory) return nullptr;
    auto backend = factory(node_id);
    if (!backend) return nullptr;
    return new NveGtkWebView(std::move(backend));
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::WebViewProps props{};
    if (nve::decode_web_view_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveGtkWebView*>(inst)->apply_props(props);
    }
  };
  adapter.dispatch_command =
      [](void* inst, uint32_t, std::string_view, const nve::NvncDecoder& dec) -> bool {
    if (inst == nullptr) return false;
    nve::WebViewCommand cmd{};
    if (nve::decode_web_view_command(dec, cmd) != nve::NativeViewCodecError::Ok) return false;
    return static_cast<NveGtkWebView*>(inst)->dispatch_command(cmd);
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* view = static_cast<NveGtkWebView*>(inst);
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
    std::function<std::unique_ptr<NveGtkVideoBackend>(uint32_t)> factory) {
  nve::NativeViewAdapter adapter{};
  adapter.create = [factory = std::move(factory)](
                       uint32_t node_id, std::string_view, const nve::NvnpDecoder*) -> void* {
    if (!factory) return nullptr;
    auto backend = factory(node_id);
    if (!backend) return nullptr;
    return new NveGtkVideoPlayer(std::move(backend));
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::VideoPlayerProps props{};
    if (nve::decode_video_player_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveGtkVideoPlayer*>(inst)->apply_props(props);
    }
  };
  adapter.dispatch_command =
      [](void* inst, uint32_t, std::string_view, const nve::NvncDecoder& dec) -> bool {
    if (inst == nullptr) return false;
    nve::VideoPlayerCommand cmd{};
    if (nve::decode_video_player_command(dec, cmd) != nve::NativeViewCodecError::Ok) return false;
    return static_cast<NveGtkVideoPlayer*>(inst)->dispatch_command(cmd);
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* player = static_cast<NveGtkVideoPlayer*>(inst);
    player->unmount();
    delete player;
  };
  return adapter;
}
#endif // NVE_NV_ENABLE_VIDEO

} // namespace

#if NVE_NV_ENABLE_BLUR
GtkTintRgba unpack_gtk_tint_rrggbbaa(uint32_t rrggbbaa) noexcept {
  GtkTintRgba out{};
  out.r = static_cast<uint8_t>((rrggbbaa >> 24u) & 0xFFu);
  out.g = static_cast<uint8_t>((rrggbbaa >> 16u) & 0xFFu);
  out.b = static_cast<uint8_t>((rrggbbaa >> 8u) & 0xFFu);
  out.a = static_cast<uint8_t>(rrggbbaa & 0xFFu);
  out.alpha_unit = static_cast<double>(out.a) / 255.0;
  return out;
}

double base_gtk_blur_radius(nve::BlurMaterial material) noexcept {
  switch (material) {
    case nve::BlurMaterial::UltraThin:
      return 8.0;
    case nve::BlurMaterial::Thin:
      return 14.0;
    case nve::BlurMaterial::Regular:
      return 20.0;
    case nve::BlurMaterial::Thick:
      return 30.0;
    case nve::BlurMaterial::Chrome:
      return 24.0;
    case nve::BlurMaterial::SystemMaterial:
      return 20.0;
  }
  return 20.0;
}

NveGtkBlurSurface::NveGtkBlurSurface(uint32_t node_id) : node_id_(node_id) {
  box_ = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
  g_object_ref_sink(box_);
  gtk_widget_set_overflow(box_, GTK_OVERFLOW_HIDDEN);
  css_class_ = "nve-gtk-blur-" + std::to_string(node_id_);
  gtk_widget_add_css_class(box_, css_class_.c_str());

  css_provider_ = gtk_css_provider_new();
  if (GdkDisplay* display = gdk_display_get_default(); display != nullptr) {
    gtk_style_context_add_provider_for_display(
        display, GTK_STYLE_PROVIDER(css_provider_), GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
    provider_attached_ = true;
  }
  apply_props(nve::BlurSurfaceProps{});
}

NveGtkBlurSurface::~NveGtkBlurSurface() {
  unmount();
}

void NveGtkBlurSurface::apply_props(const nve::BlurSurfaceProps& props) {
  const float clamped =
      std::isfinite(props.intensity) ? std::clamp(props.intensity, 0.0f, 1.0f) : 1.0f;
  material_ = props.material;
  intensity_ = clamped;
  blur_radius_ = base_gtk_blur_radius(props.material) * static_cast<double>(clamped);
  has_tint_ = props.tint.has_value();
  tint_rgba_ = props.tint.value_or(0u);

  const GtkTintRgba rgba =
      has_tint_ ? unpack_gtk_tint_rrggbbaa(tint_rgba_) : fallback_material_tint(props.material, clamped);

  char css_buf[192];
  std::snprintf(
      css_buf,
      sizeof(css_buf),
      ".%s { background-color: rgba(%u, %u, %u, %.4f); }",
      css_class_.c_str(),
      static_cast<unsigned>(rgba.r),
      static_cast<unsigned>(rgba.g),
      static_cast<unsigned>(rgba.b),
      rgba.alpha_unit);
  active_css_ = css_buf;

  if (css_provider_ != nullptr) {
    load_css_provider(css_provider_, active_css_);
  }
  if (box_ != nullptr) {
    gtk_widget_set_opacity(box_, static_cast<double>(clamped));
  }
}

void NveGtkBlurSurface::unmount() {
  if (unmounted_) return;
  unmounted_ = true;
  if (css_provider_ != nullptr) {
    if (provider_attached_) {
      if (GdkDisplay* display = gdk_display_get_default(); display != nullptr) {
        gtk_style_context_remove_provider_for_display(display, GTK_STYLE_PROVIDER(css_provider_));
      }
      provider_attached_ = false;
    }
    g_object_unref(css_provider_);
    css_provider_ = nullptr;
  }
  if (box_ != nullptr) {
    if (gtk_widget_get_parent(box_) != nullptr) {
      gtk_widget_unparent(box_);
    }
    g_object_unref(box_);
    box_ = nullptr;
  }
}

NveGtkBlurSurface* blur_surface_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_blur_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveGtkBlurSurface*>(registry.instance_for(node_id));
}
#endif // NVE_NV_ENABLE_BLUR

#if NVE_NV_ENABLE_WEBVIEW
bool is_safe_gtk_webview_url(std::string_view url) noexcept {
  const std::string norm = normalize_url_scheme(url);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank";
}

NveGtkWebView::NveGtkWebView(std::unique_ptr<NveGtkWebViewBackend> backend)
    : backend_(std::move(backend)) {
  box_ = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
  g_object_ref_sink(box_);
  if (backend_) {
    backend_->set_allows_back_forward(allows_back_forward_);
    if (GtkWidget* child = backend_->widget(); child != nullptr) {
      gtk_box_append(GTK_BOX(box_), child);
    }
  }
}

NveGtkWebView::~NveGtkWebView() {
  unmount();
}

bool NveGtkWebView::load_safe_url(std::string_view url) {
  if (!backend_ || !is_safe_gtk_webview_url(url)) return false;
  if (!backend_->navigate(url)) return false;
  last_requested_url_.assign(url);
  return true;
}

void NveGtkWebView::apply_props(const nve::WebViewProps& props) {
  if (!backend_) return;
  allows_back_forward_ = props.allows_back_forward;
  backend_->set_allows_back_forward(allows_back_forward_);
  if (last_requested_url_ != props.url) {
    (void)load_safe_url(props.url);
  }
}

bool NveGtkWebView::dispatch_command(const nve::WebViewCommand& cmd) {
  if (!backend_) return false;
  switch (cmd.kind) {
    case nve::WebViewCommandKind::Navigate:
      return load_safe_url(cmd.text);
    case nve::WebViewCommandKind::Reload:
      backend_->reload();
      return true;
    case nve::WebViewCommandKind::GoBack:
      if (!allows_back_forward_) return false;
      return backend_->go_back();
    case nve::WebViewCommandKind::GoForward:
      if (!allows_back_forward_) return false;
      return backend_->go_forward();
    case nve::WebViewCommandKind::EvaluateScript:
      backend_->evaluate_script(cmd.text);
      return true;
  }
  return false;
}

void NveGtkWebView::unmount() {
  if (unmounted_) return;
  unmounted_ = true;
  if (backend_) backend_->stop_and_close();
  if (box_ != nullptr) {
    if (gtk_widget_get_parent(box_) != nullptr) {
      gtk_widget_unparent(box_);
    }
    g_object_unref(box_);
    box_ = nullptr;
  }
}

NveGtkWebView* web_view_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_webview_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveGtkWebView*>(registry.instance_for(node_id));
}
#endif // NVE_NV_ENABLE_WEBVIEW

#if NVE_NV_ENABLE_VIDEO
bool is_safe_gtk_media_uri(std::string_view uri) noexcept {
  const std::string norm = normalize_url_scheme(uri);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm.starts_with("file://");
}

NveGtkVideoPlayer::NveGtkVideoPlayer(std::unique_ptr<NveGtkVideoBackend> backend)
    : backend_(std::move(backend)) {
  box_ = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
  g_object_ref_sink(box_);
  if (backend_) {
    if (GtkWidget* child = backend_->widget(); child != nullptr) {
      gtk_box_append(GTK_BOX(box_), child);
    }
  }
}

NveGtkVideoPlayer::~NveGtkVideoPlayer() {
  unmount();
}

void NveGtkVideoPlayer::apply_props(const nve::VideoPlayerProps& props) {
  if (!backend_) return;
  if (source_uri_ != props.source_uri && is_safe_gtk_media_uri(props.source_uri)) {
    source_uri_.assign(props.source_uri);
    backend_->set_source_uri(source_uri_);
  }
  autoplay_ = props.autoplay;
  muted_ = props.muted;
  looping_ = props.looping;
  controls_ = props.controls;
  backend_->set_muted(muted_);
  backend_->set_looping(looping_);
  backend_->set_controls(controls_);
  if (autoplay_ && !source_uri_.empty() && !playing_) {
    playing_ = true;
    backend_->play();
  }
}

bool NveGtkVideoPlayer::dispatch_command(const nve::VideoPlayerCommand& cmd) {
  if (!backend_) return false;
  switch (cmd.kind) {
    case nve::VideoPlayerCommandKind::Play:
      playing_ = true;
      backend_->play();
      return true;
    case nve::VideoPlayerCommandKind::Pause:
      playing_ = false;
      backend_->pause();
      return true;
    case nve::VideoPlayerCommandKind::SeekMs:
      position_ms_ = cmd.seek_ms;
      backend_->seek_ms(position_ms_);
      return true;
    case nve::VideoPlayerCommandKind::SetVolume:
      if (!std::isfinite(cmd.volume)) return false;
      volume_ = std::clamp(cmd.volume, 0.0f, 1.0f);
      backend_->set_volume(volume_);
      return true;
  }
  return false;
}

void NveGtkVideoPlayer::unmount() {
  if (unmounted_) return;
  unmounted_ = true;
  playing_ = false;
  if (backend_) backend_->stop_and_close();
  if (box_ != nullptr) {
    if (gtk_widget_get_parent(box_) != nullptr) {
      gtk_widget_unparent(box_);
    }
    g_object_unref(box_);
    box_ = nullptr;
  }
}

NveGtkVideoPlayer* video_player_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_video_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveGtkVideoPlayer*>(registry.instance_for(node_id));
}
#endif // NVE_NV_ENABLE_VIDEO

void register_gtk_native_views(nve::NativeViewRegistry& registry, NveGtkNativeViewHooks hooks) {
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

void bind_gtk_native_views_to_fabric_host(
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

GtkWidget* native_widget_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  const std::string_view k = registry.kind_for(node_id);
  (void)k;
#if NVE_NV_ENABLE_BLUR
  if (is_blur_kind(k)) {
    auto* b = static_cast<NveGtkBlurSurface*>(registry.instance_for(node_id));
    return b != nullptr ? b->widget() : nullptr;
  }
#endif
#if NVE_NV_ENABLE_WEBVIEW
  if (is_webview_kind(k)) {
    auto* w = static_cast<NveGtkWebView*>(registry.instance_for(node_id));
    return w != nullptr ? w->widget() : nullptr;
  }
#endif
#if NVE_NV_ENABLE_VIDEO
  if (is_video_kind(k)) {
    auto* v = static_cast<NveGtkVideoPlayer*>(registry.instance_for(node_id));
    return v != nullptr ? v->widget() : nullptr;
  }
#endif
  return nullptr;
}

} // namespace nve::gtk
