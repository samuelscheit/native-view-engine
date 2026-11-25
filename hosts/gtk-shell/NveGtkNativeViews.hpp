#pragma once

#include "nve_native_view.hpp"

#include <gtk/gtk.h>

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <string_view>

namespace nve::gtk {

/// Returns `true` if `url` uses an allowed WebView scheme (`https://`, `http://`, `about:blank`).
/// Rejects `javascript:`, `vbscript:`, `data:`, and control-character obfuscated URLs.
bool is_safe_gtk_webview_url(std::string_view url) noexcept;

/// Returns `true` if `uri` is a safe media URI (`https://`, `http://`, `file://`).
bool is_safe_gtk_media_uri(std::string_view uri) noexcept;

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

/// GTK4 native blur surface backed by a CSS-styled tinted `GtkBox` and `GtkCssProvider` for
/// `"nve.blur_surface"` / `"nve.blur"`.
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
  const std::string& backdrop_filter_spec() const noexcept { return backdrop_filter_spec_; }
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
  std::string backdrop_filter_spec_;
  bool unmounted_{false};
};

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
class NveGtkWebView {
 public:
  explicit NveGtkWebView(std::unique_ptr<NveGtkWebViewBackend> backend = nullptr);
  ~NveGtkWebView();

  NveGtkWebView(const NveGtkWebView&) = delete;
  NveGtkWebView& operator=(const NveGtkWebView&) = delete;

  void apply_props(const nve::WebViewProps& props);
  bool dispatch_command(const nve::WebViewCommand& cmd);
  void unmount();

  GtkWidget* widget() const noexcept { return box_; }
  const std::string& last_requested_url() const noexcept { return last_requested_url_; }
  bool allows_back_forward() const noexcept { return allows_back_forward_; }
  size_t navigation_count() const noexcept { return navigation_count_; }
  size_t reload_count() const noexcept { return reload_count_; }
  size_t go_back_count() const noexcept { return go_back_count_; }
  size_t go_forward_count() const noexcept { return go_forward_count_; }
  const std::string& last_evaluated_script() const noexcept { return last_evaluated_script_; }
  size_t evaluate_script_count() const noexcept { return evaluate_script_count_; }
  bool unmounted() const noexcept { return unmounted_; }
  NveGtkWebViewBackend* backend() const noexcept { return backend_.get(); }

 private:
  bool load_safe_url(std::string_view url);

  GtkWidget* box_{nullptr};
  std::unique_ptr<NveGtkWebViewBackend> backend_;
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
class NveGtkVideoPlayer {
 public:
  explicit NveGtkVideoPlayer(std::unique_ptr<NveGtkVideoBackend> backend = nullptr);
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
  size_t play_count() const noexcept { return play_count_; }
  size_t pause_count() const noexcept { return pause_count_; }
  size_t seek_count() const noexcept { return seek_count_; }
  size_t volume_change_count() const noexcept { return volume_change_count_; }
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
  size_t play_count_{0u};
  size_t pause_count_{0u};
  size_t seek_count_{0u};
  size_t volume_change_count_{0u};
  bool unmounted_{false};
};

/// Optional backend factory hooks injected when registering GTK4 native views.
struct NveGtkNativeViewHooks {
  std::function<std::unique_ptr<NveGtkWebViewBackend>(uint32_t node_id)> create_webview_backend;
  std::function<std::unique_ptr<NveGtkVideoBackend>(uint32_t node_id)> create_video_backend;
};

/// Registers `BlurSurface`, `WebView`, and `VideoPlayer` (canonical kinds + aliases) in `registry`.
void register_gtk_native_views(nve::NativeViewRegistry& registry, NveGtkNativeViewHooks hooks = {});

/// Binds GTK4 native view kinds registered in `registry` to `host`.
void bind_gtk_native_views_to_fabric_host(
    NveFabricHostHandle host,
    nve::NativeViewRegistry& registry);

/// Returns the mounted `GtkWidget*` for `node_id`, or `nullptr`.
GtkWidget* native_widget_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;

NveGtkBlurSurface* blur_surface_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
NveGtkWebView* web_view_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
NveGtkVideoPlayer* video_player_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;

} // namespace nve::gtk
