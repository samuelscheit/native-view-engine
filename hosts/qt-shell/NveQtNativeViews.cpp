#include "NveQtNativeViews.hpp"

#include <QPalette>
#include <QVBoxLayout>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <utility>

namespace nve::qt {

namespace {

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

QColor fallback_material_tint(nve::BlurMaterial mat, float intensity) noexcept {
  int base_alpha = 160;
  switch (mat) {
    case nve::BlurMaterial::UltraThin:
      base_alpha = 80;
      break;
    case nve::BlurMaterial::Thin:
      base_alpha = 120;
      break;
    case nve::BlurMaterial::Regular:
    case nve::BlurMaterial::SystemMaterial:
      base_alpha = 160;
      break;
    case nve::BlurMaterial::Thick:
      base_alpha = 205;
      break;
    case nve::BlurMaterial::Chrome:
      base_alpha = 185;
      break;
  }
  const int alpha = std::clamp(static_cast<int>(std::lround(base_alpha * intensity)), 0, 255);
  return QColor(245, 247, 250, alpha);
}

bool is_blur_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindBlurSurface || k == nve::kNativeKindBlurAlias;
}

bool is_webview_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindWebView || k == nve::kNativeKindWebViewAlias;
}

bool is_video_kind(std::string_view k) noexcept {
  return k == nve::kNativeKindVideoPlayer || k == nve::kNativeKindVideoAlias;
}

void mount_backend_child(QWidget* container, QWidget* child) {
  if (container == nullptr || child == nullptr) return;
  auto* layout = new QVBoxLayout(container);
  layout->setContentsMargins(0, 0, 0, 0);
  layout->setSpacing(0);
  child->setParent(container);
  layout->addWidget(child);
}

nve::NativeViewAdapter make_blur_adapter() {
  nve::NativeViewAdapter adapter{};
  adapter.create = [](uint32_t, std::string_view, const nve::NvnpDecoder*) -> void* {
    return new NveQtBlurSurfaceWidget();
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::BlurSurfaceProps props{};
    if (nve::decode_blur_surface_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveQtBlurSurfaceWidget*>(inst)->applyBlurProps(props);
    }
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* widget = static_cast<NveQtBlurSurfaceWidget*>(inst);
    widget->setParent(nullptr);
    delete widget;
  };
  return adapter;
}

nve::NativeViewAdapter make_webview_adapter(
    std::function<std::unique_ptr<NveQtWebViewBackend>(uint32_t)> factory) {
  nve::NativeViewAdapter adapter{};
  adapter.create = [factory = std::move(factory)](
                       uint32_t node_id, std::string_view, const nve::NvnpDecoder*) -> void* {
    std::unique_ptr<NveQtWebViewBackend> backend;
    if (factory) {
      backend = factory(node_id);
    }
    return new NveQtWebViewWidget(std::move(backend));
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::WebViewProps props{};
    if (nve::decode_web_view_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveQtWebViewWidget*>(inst)->applyWebViewProps(props);
    }
  };
  adapter.dispatch_command =
      [](void* inst, uint32_t, std::string_view, const nve::NvncDecoder& dec) -> bool {
    if (inst == nullptr) return false;
    nve::WebViewCommand cmd{};
    if (nve::decode_web_view_command(dec, cmd) != nve::NativeViewCodecError::Ok) {
      return false;
    }
    return static_cast<NveQtWebViewWidget*>(inst)->dispatchWebViewCommand(cmd);
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* widget = static_cast<NveQtWebViewWidget*>(inst);
    widget->stopAndUnmount();
    widget->setParent(nullptr);
    delete widget;
  };
  return adapter;
}

nve::NativeViewAdapter make_video_adapter(
    std::function<std::unique_ptr<NveQtVideoBackend>(uint32_t)> factory) {
  nve::NativeViewAdapter adapter{};
  adapter.create = [factory = std::move(factory)](
                       uint32_t node_id, std::string_view, const nve::NvnpDecoder*) -> void* {
    std::unique_ptr<NveQtVideoBackend> backend;
    if (factory) {
      backend = factory(node_id);
    }
    return new NveQtVideoPlayerWidget(std::move(backend));
  };
  adapter.update_props = [](void* inst, uint32_t, const nve::NvnpDecoder& dec) {
    if (inst == nullptr) return;
    nve::VideoPlayerProps props{};
    if (nve::decode_video_player_props(dec, props) == nve::NativeViewCodecError::Ok) {
      static_cast<NveQtVideoPlayerWidget*>(inst)->applyVideoProps(props);
    }
  };
  adapter.dispatch_command =
      [](void* inst, uint32_t, std::string_view, const nve::NvncDecoder& dec) -> bool {
    if (inst == nullptr) return false;
    nve::VideoPlayerCommand cmd{};
    if (nve::decode_video_player_command(dec, cmd) != nve::NativeViewCodecError::Ok) {
      return false;
    }
    return static_cast<NveQtVideoPlayerWidget*>(inst)->dispatchVideoCommand(cmd);
  };
  adapter.unmount = [](void* inst, uint32_t) {
    if (inst == nullptr) return;
    auto* widget = static_cast<NveQtVideoPlayerWidget*>(inst);
    widget->stopAndUnmount();
    widget->setParent(nullptr);
    delete widget;
  };
  return adapter;
}

} // namespace

bool is_safe_qt_webview_url(std::string_view url) noexcept {
  const std::string norm = normalize_url_scheme(url);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank";
}

bool is_safe_qt_media_uri(std::string_view uri) noexcept {
  const std::string norm = normalize_url_scheme(uri);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm.starts_with("file://");
}

QColor color_from_rrggbbaa(uint32_t rrggbbaa) noexcept {
  const int r = static_cast<int>((rrggbbaa >> 24u) & 0xFFu);
  const int g = static_cast<int>((rrggbbaa >> 16u) & 0xFFu);
  const int b = static_cast<int>((rrggbbaa >> 8u) & 0xFFu);
  const int a = static_cast<int>(rrggbbaa & 0xFFu);
  return QColor(r, g, b, a);
}

qreal base_qt_blur_radius(nve::BlurMaterial material) noexcept {
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

NveQtBlurSurfaceWidget::NveQtBlurSurfaceWidget(QWidget* parent)
    : QWidget(parent), blur_effect_(new QGraphicsBlurEffect(this)) {
  setAttribute(Qt::WA_StyledBackground, true);
  setAutoFillBackground(true);
  blur_effect_->setBlurHints(QGraphicsBlurEffect::PerformanceHint);
  blur_effect_->setBlurRadius(blur_radius_);
  setGraphicsEffect(blur_effect_);
  applyBlurProps(nve::BlurSurfaceProps{});
}

void NveQtBlurSurfaceWidget::applyBlurProps(const nve::BlurSurfaceProps& props) {
  const float clamped =
      std::isfinite(props.intensity) ? std::clamp(props.intensity, 0.0f, 1.0f) : 1.0f;
  material_ = props.material;
  intensity_ = clamped;
  blur_radius_ = base_qt_blur_radius(props.material) * static_cast<qreal>(clamped);
  has_tint_ = props.tint.has_value();
  tint_rgba_ = props.tint.value_or(0u);
  effective_tint_ = has_tint_ ? color_from_rrggbbaa(tint_rgba_)
                              : fallback_material_tint(props.material, clamped);

  if (blur_effect_ != nullptr) {
    blur_effect_->setBlurRadius(blur_radius_);
  }
  QPalette pal = palette();
  pal.setColor(QPalette::Window, effective_tint_);
  setPalette(pal);
}

NveQtWebViewWidget::NveQtWebViewWidget(
    std::unique_ptr<NveQtWebViewBackend> backend,
    QWidget* parent)
    : QWidget(parent), backend_(std::move(backend)) {
  if (backend_) {
    backend_->setAllowsBackForward(allows_back_forward_);
    mount_backend_child(this, backend_->widget());
  }
}

NveQtWebViewWidget::~NveQtWebViewWidget() {
  stopAndUnmount();
}

bool NveQtWebViewWidget::loadSafeUrl(std::string_view url) {
  if (!is_safe_qt_webview_url(url)) return false;
  if (backend_ && !backend_->navigate(url)) return false;
  last_requested_url_.assign(url);
  ++navigation_count_;
  return true;
}

void NveQtWebViewWidget::applyWebViewProps(const nve::WebViewProps& props) {
  allows_back_forward_ = props.allows_back_forward;
  if (backend_) {
    backend_->setAllowsBackForward(allows_back_forward_);
  }
  if (last_requested_url_ != props.url) {
    (void)loadSafeUrl(props.url);
  }
}

bool NveQtWebViewWidget::dispatchWebViewCommand(const nve::WebViewCommand& cmd) {
  switch (cmd.kind) {
    case nve::WebViewCommandKind::Navigate:
      return loadSafeUrl(cmd.text);
    case nve::WebViewCommandKind::Reload:
      ++reload_count_;
      if (backend_) backend_->reload();
      return true;
    case nve::WebViewCommandKind::GoBack:
      if (!allows_back_forward_) return false;
      ++go_back_count_;
      if (backend_) (void)backend_->goBack();
      return true;
    case nve::WebViewCommandKind::GoForward:
      if (!allows_back_forward_) return false;
      ++go_forward_count_;
      if (backend_) (void)backend_->goForward();
      return true;
    case nve::WebViewCommandKind::EvaluateScript:
      last_evaluated_script_.assign(cmd.text);
      ++evaluate_script_count_;
      if (backend_) backend_->evaluateScript(cmd.text);
      return true;
  }
  return false;
}

void NveQtWebViewWidget::stopAndUnmount() {
  if (unmounted_) return;
  unmounted_ = true;
  if (backend_) {
    backend_->stopAndClose();
  }
}

NveQtVideoPlayerWidget::NveQtVideoPlayerWidget(
    std::unique_ptr<NveQtVideoBackend> backend,
    QWidget* parent)
    : QWidget(parent), backend_(std::move(backend)) {
  if (backend_) {
    mount_backend_child(this, backend_->widget());
  }
}

NveQtVideoPlayerWidget::~NveQtVideoPlayerWidget() {
  stopAndUnmount();
}

void NveQtVideoPlayerWidget::applyVideoProps(const nve::VideoPlayerProps& props) {
  if (source_uri_ != props.source_uri && is_safe_qt_media_uri(props.source_uri)) {
    source_uri_.assign(props.source_uri);
    if (backend_) backend_->setSourceUri(source_uri_);
  }
  autoplay_ = props.autoplay;
  muted_ = props.muted;
  looping_ = props.looping;
  controls_ = props.controls;
  if (backend_) {
    backend_->setMuted(muted_);
    backend_->setLooping(looping_);
    backend_->setControls(controls_);
  }
  if (autoplay_ && !source_uri_.empty() && !playing_) {
    playing_ = true;
    ++play_count_;
    if (backend_) backend_->play();
  }
}

bool NveQtVideoPlayerWidget::dispatchVideoCommand(const nve::VideoPlayerCommand& cmd) {
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
      if (backend_) backend_->seekMs(position_ms_);
      return true;
    case nve::VideoPlayerCommandKind::SetVolume:
      if (!std::isfinite(cmd.volume)) return false;
      volume_ = std::clamp(cmd.volume, 0.0f, 1.0f);
      ++volume_change_count_;
      if (backend_) backend_->setVolume(volume_);
      return true;
  }
  return false;
}

void NveQtVideoPlayerWidget::stopAndUnmount() {
  if (unmounted_) return;
  unmounted_ = true;
  playing_ = false;
  if (backend_) {
    backend_->stopAndClose();
  }
}

void register_qt_native_views(nve::NativeViewRegistry& registry, NveQtNativeViewHooks hooks) {
  registry.register_adapter(nve::kNativeKindBlurSurface, make_blur_adapter());
  registry.register_adapter(nve::kNativeKindBlurAlias, make_blur_adapter());
  registry.register_adapter(nve::kNativeKindWebView, make_webview_adapter(hooks.create_webview_backend));
  registry.register_adapter(
      nve::kNativeKindWebViewAlias,
      make_webview_adapter(std::move(hooks.create_webview_backend)));
  registry.register_adapter(nve::kNativeKindVideoPlayer, make_video_adapter(hooks.create_video_backend));
  registry.register_adapter(
      nve::kNativeKindVideoAlias,
      make_video_adapter(std::move(hooks.create_video_backend)));
}

void bind_qt_native_views_to_fabric_host(
    NveFabricHostHandle host,
    nve::NativeViewRegistry& registry) {
  if (host == nullptr) return;
  registry.bind_to_fabric_host(host, nve::kNativeKindBlurSurface);
  registry.bind_to_fabric_host(host, nve::kNativeKindBlurAlias);
  registry.bind_to_fabric_host(host, nve::kNativeKindWebView);
  registry.bind_to_fabric_host(host, nve::kNativeKindWebViewAlias);
  registry.bind_to_fabric_host(host, nve::kNativeKindVideoPlayer);
  registry.bind_to_fabric_host(host, nve::kNativeKindVideoAlias);
}

QWidget* native_widget_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  const std::string_view k = registry.kind_for(node_id);
  if (!is_blur_kind(k) && !is_webview_kind(k) && !is_video_kind(k)) return nullptr;
  return static_cast<QWidget*>(registry.instance_for(node_id));
}

NveQtBlurSurfaceWidget* blur_surface_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_blur_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveQtBlurSurfaceWidget*>(registry.instance_for(node_id));
}

NveQtWebViewWidget* web_view_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_webview_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveQtWebViewWidget*>(registry.instance_for(node_id));
}

NveQtVideoPlayerWidget* video_player_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept {
  if (!is_video_kind(registry.kind_for(node_id))) return nullptr;
  return static_cast<NveQtVideoPlayerWidget*>(registry.instance_for(node_id));
}

} // namespace nve::qt
