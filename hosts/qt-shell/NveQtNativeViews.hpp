#pragma once

#include "nve_native_view.hpp"

#include <QColor>
#include <QGraphicsBlurEffect>
#include <QString>
#include <QWidget>

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <string_view>

namespace nve::qt {

/// Returns `true` if `url` uses an allowed WebView scheme (`https://`, `http://`, `about:blank`).
/// Rejects `javascript:`, `vbscript:`, `data:`, and control-character obfuscated URLs.
bool is_safe_qt_webview_url(std::string_view url) noexcept;

/// Returns `true` if `uri` is a safe media URI (`https://`, `http://`, `file://`).
bool is_safe_qt_media_uri(std::string_view uri) noexcept;

/// Converts a packed `0xRRGGBBAA` native color to `QColor`.
QColor color_from_rrggbbaa(uint32_t rrggbbaa) noexcept;

/// Maps `nve::BlurMaterial` to a base Gaussian blur radius in logical pixels.
qreal base_qt_blur_radius(nve::BlurMaterial material) noexcept;

/// Qt 6 native blur surface widget backed by `QGraphicsBlurEffect` and a translucent tinted
/// `QWidget` for `"nve.blur_surface"` / `"nve.blur"`.
class NveQtBlurSurfaceWidget : public QWidget {
 public:
  explicit NveQtBlurSurfaceWidget(QWidget* parent = nullptr);

  void applyBlurProps(const nve::BlurSurfaceProps& props);

  nve::BlurMaterial blurMaterial() const noexcept { return material_; }
  float blurIntensity() const noexcept { return intensity_; }
  qreal blurRadius() const noexcept { return blur_radius_; }
  bool hasTint() const noexcept { return has_tint_; }
  uint32_t tintRgba() const noexcept { return tint_rgba_; }
  QColor effectiveTintColor() const noexcept { return effective_tint_; }
  QGraphicsBlurEffect* blurEffect() const noexcept { return blur_effect_; }

 private:
  QGraphicsBlurEffect* blur_effect_{nullptr};
  nve::BlurMaterial material_{nve::BlurMaterial::Regular};
  float intensity_{1.0f};
  qreal blur_radius_{20.0};
  bool has_tint_{false};
  uint32_t tint_rgba_{0u};
  QColor effective_tint_;
};

/// Pluggable Qt WebView backend interface (e.g., `QWebEngineView` or a custom/mock controller).
class NveQtWebViewBackend {
 public:
  virtual ~NveQtWebViewBackend() = default;
  virtual QWidget* widget() { return nullptr; }
  virtual void setAllowsBackForward(bool allowed) = 0;
  virtual bool navigate(std::string_view url) = 0;
  virtual void reload() = 0;
  virtual bool goBack() = 0;
  virtual bool goForward() = 0;
  virtual void evaluateScript(std::string_view script) = 0;
  virtual void stopAndClose() = 0;
};

/// Qt 6 `QWidget` container adapter for `"nve.web_view"` / `"nve.webview"`.
class NveQtWebViewWidget : public QWidget {
 public:
  explicit NveQtWebViewWidget(
      std::unique_ptr<NveQtWebViewBackend> backend = nullptr,
      QWidget* parent = nullptr);
  ~NveQtWebViewWidget() override;

  void applyWebViewProps(const nve::WebViewProps& props);
  bool dispatchWebViewCommand(const nve::WebViewCommand& cmd);
  void stopAndUnmount();

  const std::string& lastRequestedUrl() const noexcept { return last_requested_url_; }
  bool allowsBackForward() const noexcept { return allows_back_forward_; }
  size_t navigationCount() const noexcept { return navigation_count_; }
  size_t reloadCount() const noexcept { return reload_count_; }
  size_t goBackCount() const noexcept { return go_back_count_; }
  size_t goForwardCount() const noexcept { return go_forward_count_; }
  const std::string& lastEvaluatedScript() const noexcept { return last_evaluated_script_; }
  size_t evaluateScriptCount() const noexcept { return evaluate_script_count_; }
  NveQtWebViewBackend* backend() const noexcept { return backend_.get(); }

 private:
  bool loadSafeUrl(std::string_view url);

  std::unique_ptr<NveQtWebViewBackend> backend_;
  std::string last_requested_url_;
  bool allows_back_forward_{true};
  size_t navigation_count_{0u};
  size_t reload_count_{0u};
  size_t go_back_count_{0u};
  size_t go_forward_count_{0u};
  std::string last_evaluated_script_;
  size_t evaluate_script_count_{0u};
  bool unmounted_{false};
};

/// Pluggable Qt Multimedia video backend interface (e.g., `QMediaPlayer` + `QVideoWidget`).
class NveQtVideoBackend {
 public:
  virtual ~NveQtVideoBackend() = default;
  virtual QWidget* widget() { return nullptr; }
  virtual void setSourceUri(std::string_view uri) = 0;
  virtual void setMuted(bool muted) = 0;
  virtual void setLooping(bool looping) = 0;
  virtual void setControls(bool controls) = 0;
  virtual void play() = 0;
  virtual void pause() = 0;
  virtual void seekMs(uint64_t ms) = 0;
  virtual void setVolume(float volume) = 0;
  virtual void stopAndClose() = 0;
};

/// Qt 6 `QWidget` container adapter for `"nve.video_player"` / `"nve.video"`.
class NveQtVideoPlayerWidget : public QWidget {
 public:
  explicit NveQtVideoPlayerWidget(
      std::unique_ptr<NveQtVideoBackend> backend = nullptr,
      QWidget* parent = nullptr);
  ~NveQtVideoPlayerWidget() override;

  void applyVideoProps(const nve::VideoPlayerProps& props);
  bool dispatchVideoCommand(const nve::VideoPlayerCommand& cmd);
  void stopAndUnmount();

  const std::string& sourceUri() const noexcept { return source_uri_; }
  bool autoplay() const noexcept { return autoplay_; }
  bool muted() const noexcept { return muted_; }
  bool looping() const noexcept { return looping_; }
  bool controls() const noexcept { return controls_; }
  bool isPlaying() const noexcept { return playing_; }
  uint64_t positionMs() const noexcept { return position_ms_; }
  float volume() const noexcept { return volume_; }
  size_t playCount() const noexcept { return play_count_; }
  size_t pauseCount() const noexcept { return pause_count_; }
  size_t seekCount() const noexcept { return seek_count_; }
  size_t volumeChangeCount() const noexcept { return volume_change_count_; }
  NveQtVideoBackend* backend() const noexcept { return backend_.get(); }

 private:
  std::unique_ptr<NveQtVideoBackend> backend_;
  std::string source_uri_;
  bool autoplay_{false};
  bool muted_{false};
  bool looping_{false};
  bool controls_{true};
  bool playing_{false};
  uint64_t position_ms_{0u};
  float volume_{1.0f};
  size_t play_count_{0u};
  size_t pause_count_{0u};
  size_t seek_count_{0u};
  size_t volume_change_count_{0u};
  bool unmounted_{false};
};

/// Optional backend factory hooks injected when registering Qt 6 native views.
struct NveQtNativeViewHooks {
  std::function<std::unique_ptr<NveQtWebViewBackend>(uint32_t node_id)> create_webview_backend;
  std::function<std::unique_ptr<NveQtVideoBackend>(uint32_t node_id)> create_video_backend;
};

/// Registers `BlurSurface`, `WebView`, and `VideoPlayer` (canonical kinds + aliases) in `registry`.
void register_qt_native_views(nve::NativeViewRegistry& registry, NveQtNativeViewHooks hooks = {});

/// Binds Qt native view kinds registered in `registry` to `host`.
void bind_qt_native_views_to_fabric_host(
    NveFabricHostHandle host,
    nve::NativeViewRegistry& registry);

/// Returns the mounted `QWidget*` tracked by `registry` for `node_id`, or `nullptr`.
QWidget* native_widget_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;

NveQtBlurSurfaceWidget* blur_surface_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
NveQtWebViewWidget* web_view_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
NveQtVideoPlayerWidget* video_player_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;

} // namespace nve::qt
