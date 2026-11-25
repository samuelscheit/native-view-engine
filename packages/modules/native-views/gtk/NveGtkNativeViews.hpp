#pragma once

#include "nve_native_view.hpp"

#ifndef NVE_NV_ENABLE_BLUR
#define NVE_NV_ENABLE_BLUR 1
#endif
#ifndef NVE_NV_ENABLE_WEBVIEW
#define NVE_NV_ENABLE_WEBVIEW 1
#endif
#ifndef NVE_NV_ENABLE_VIDEO
#define NVE_NV_ENABLE_VIDEO 1
#endif

#include <gtk/gtk.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <string_view>

namespace nve::gtk {

#if NVE_NV_ENABLE_BLUR
/// Unpacked RGBA components decoded from `0xRRGGBBAA` native tint values.
struct GtkTintRgba {
  uint8_t r{0u};
  uint8_t g{0u};
  uint8_t b{0u};
  uint8_t a{0u};
  double alpha_unit{0.0};
};

GtkTintRgba unpack_gtk_tint_rrggbbaa(uint32_t rrggbbaa) noexcept;
double base_gtk_blur_radius(nve::BlurMaterial material) noexcept;

/// GTK4 translucent frosted tint surface backed by a CSS-styled `GtkBox` for
/// `"nve.blur_surface"` / `"nve.blur"`. GTK4 does not provide compositor `backdrop-filter`,
/// so this surface renders an honest translucent tint without blurring its own children.
class NveGtkBlurSurface {
 public:
  explicit NveGtkBlurSurface(uint32_t node_id);
  ~NveGtkBlurSurface();

  NveGtkBlurSurface(const NveGtkBlurSurface&) = delete;
  NveGtkBlurSurface& operator=(const NveGtkBlurSurface&) = delete;

  void apply_props(const nve::BlurSurfaceProps& props);
  void unmount();

  GtkWidget* widget() const noexcept { return box_; }
  nve::BlurMaterial material() const noexcept { return material_; }
  float intensity() const noexcept { return intensity_; }
  double blur_radius() const noexcept { return blur_radius_; }
  bool has_tint() const noexcept { return has_tint_; }
  uint32_t tint_rgba() const noexcept { return tint_rgba_; }
  const std::string& css_class() const noexcept { return css_class_; }
  const std::string& active_css() const noexcept { return active_css_; }
  bool unmounted() const noexcept { return unmounted_; }

 private:
  uint32_t node_id_{0u};
  GtkWidget* box_{nullptr};
  GtkCssProvider* css_provider_{nullptr};
  bool provider_attached_{false};
  nve::BlurMaterial material_{nve::BlurMaterial::Regular};
  float intensity_{1.0f};
  double blur_radius_{20.0};
  bool has_tint_{false};
  uint32_t tint_rgba_{0u};
  std::string css_class_;
  std::string active_css_;
  bool unmounted_{false};
};
#endif // NVE_NV_ENABLE_BLUR

#if NVE_NV_ENABLE_WEBVIEW
/// Returns `true` if `url` uses an allowed WebView scheme (`https://`, `http://`, `about:blank`).
/// Rejects `javascript:`, `vbscript:`, `data:`, and control-character obfuscated URLs.
bool is_safe_gtk_webview_url(std::string_view url) noexcept;

/// Pluggable GTK4 WebView backend interface (e.g., WebKitGTK `WebKitWebView` or mock controller).
class NveGtkWebViewBackend {
 public:
  virtual ~NveGtkWebViewBackend() = default;
  virtual GtkWidget* widget() { return nullptr; }
  virtual void set_allows_back_forward(bool allowed) = 0;
  virtual bool navigate(std::string_view url) = 0;
  virtual void reload() = 0;
  virtual bool go_back() = 0;
  virtual bool go_forward() = 0;
  virtual void evaluate_script(std::string_view script) = 0;
  virtual void stop_and_close() = 0;
};

/// GTK4 `GtkBox` container adapter for `"nve.web_view"` / `"nve.webview"`.
/// Requires a non-null injected `NveGtkWebViewBackend`; mounting without one returns `nullptr`.
class NveGtkWebView {
 public:
  explicit NveGtkWebView(std::unique_ptr<NveGtkWebViewBackend> backend);
  ~NveGtkWebView();

  NveGtkWebView(const NveGtkWebView&) = delete;
  NveGtkWebView& operator=(const NveGtkWebView&) = delete;

  void apply_props(const nve::WebViewProps& props);
  bool dispatch_command(const nve::WebViewCommand& cmd);
  void unmount();

  GtkWidget* widget() const noexcept { return box_; }
  const std::string& last_requested_url() const noexcept { return last_requested_url_; }
  bool allows_back_forward() const noexcept { return allows_back_forward_; }
  bool unmounted() const noexcept { return unmounted_; }
  NveGtkWebViewBackend* backend() const noexcept { return backend_.get(); }

 private:
  bool load_safe_url(std::string_view url);

  GtkWidget* box_{nullptr};
  std::unique_ptr<NveGtkWebViewBackend> backend_;
  std::string last_requested_url_;
  bool allows_back_forward_{true};
  bool unmounted_{false};
};
#endif // NVE_NV_ENABLE_WEBVIEW

#if NVE_NV_ENABLE_VIDEO
/// Returns `true` if `uri` is a safe media URI (`https://`, `http://`, `file://`).
bool is_safe_gtk_media_uri(std::string_view uri) noexcept;

/// Pluggable GTK4 video backend interface (e.g., `GtkVideo` / `GtkMediaFile` / GStreamer).
class NveGtkVideoBackend {
 public:
  virtual ~NveGtkVideoBackend() = default;
  virtual GtkWidget* widget() { return nullptr; }
  virtual void set_source_uri(std::string_view uri) = 0;
  virtual void set_muted(bool muted) = 0;
  virtual void set_looping(bool looping) = 0;
  virtual void set_controls(bool controls) = 0;
  virtual void play() = 0;
  virtual void pause() = 0;
  virtual void seek_ms(uint64_t ms) = 0;
  virtual void set_volume(float volume) = 0;
  virtual void stop_and_close() = 0;
};

/// GTK4 `GtkBox` container adapter for `"nve.video_player"` / `"nve.video"`.
/// Requires a non-null injected `NveGtkVideoBackend`; mounting without one returns `nullptr`.
class NveGtkVideoPlayer {
 public:
  explicit NveGtkVideoPlayer(std::unique_ptr<NveGtkVideoBackend> backend);
  ~NveGtkVideoPlayer();

  NveGtkVideoPlayer(const NveGtkVideoPlayer&) = delete;
  NveGtkVideoPlayer& operator=(const NveGtkVideoPlayer&) = delete;

  void apply_props(const nve::VideoPlayerProps& props);
  bool dispatch_command(const nve::VideoPlayerCommand& cmd);
  void unmount();

  GtkWidget* widget() const noexcept { return box_; }
  const std::string& source_uri() const noexcept { return source_uri_; }
  bool autoplay() const noexcept { return autoplay_; }
  bool muted() const noexcept { return muted_; }
  bool looping() const noexcept { return looping_; }
  bool controls() const noexcept { return controls_; }
  bool is_playing() const noexcept { return playing_; }
  uint64_t position_ms() const noexcept { return position_ms_; }
  float volume() const noexcept { return volume_; }
  bool unmounted() const noexcept { return unmounted_; }
  NveGtkVideoBackend* backend() const noexcept { return backend_.get(); }

 private:
  GtkWidget* box_{nullptr};
  std::unique_ptr<NveGtkVideoBackend> backend_;
  std::string source_uri_;
  bool autoplay_{false};
  bool muted_{false};
  bool looping_{false};
  bool controls_{true};
  bool playing_{false};
  uint64_t position_ms_{0u};
  float volume_{1.0f};
  bool unmounted_{false};
};
#endif // NVE_NV_ENABLE_VIDEO

/// Optional backend factory hooks injected when registering enabled GTK4 native views.
struct NveGtkNativeViewHooks {
#if NVE_NV_ENABLE_WEBVIEW
  std::function<std::unique_ptr<NveGtkWebViewBackend>(uint32_t node_id)> create_webview_backend;
#endif
#if NVE_NV_ENABLE_VIDEO
  std::function<std::unique_ptr<NveGtkVideoBackend>(uint32_t node_id)> create_video_backend;
#endif
};

/// Registers compile-time enabled GTK4 native views (canonical kinds + aliases) in `registry`.
void register_gtk_native_views(nve::NativeViewRegistry& registry, NveGtkNativeViewHooks hooks = {});

/// Binds compile-time enabled GTK4 native view kinds registered in `registry` to `host`.
void bind_gtk_native_views_to_fabric_host(
    NveFabricHostHandle host,
    nve::NativeViewRegistry& registry);

/// Returns the mounted `GtkWidget*` for `node_id`, or `nullptr`.
GtkWidget* native_widget_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;

#if NVE_NV_ENABLE_BLUR
NveGtkBlurSurface* blur_surface_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
#endif
#if NVE_NV_ENABLE_WEBVIEW
NveGtkWebView* web_view_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
#endif
#if NVE_NV_ENABLE_VIDEO
NveGtkVideoPlayer* video_player_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
#endif

} // namespace nve::gtk
