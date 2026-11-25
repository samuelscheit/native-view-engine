#pragma once

#include "nve_native_view.hpp"

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <string_view>

#ifndef NVE_NV_ENABLE_BLUR
#define NVE_NV_ENABLE_BLUR 1
#endif
#ifndef NVE_NV_ENABLE_WEBVIEW
#define NVE_NV_ENABLE_WEBVIEW 1
#endif
#ifndef NVE_NV_ENABLE_VIDEO
#define NVE_NV_ENABLE_VIDEO 1
#endif

#if defined(_WIN32) && NVE_NV_ENABLE_BLUR
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif

namespace nve::win {

#if NVE_NV_ENABLE_BLUR
/// Win32 DWM system backdrop type values matching `DWM_SYSTEMBACKDROP_TYPE` (Win11 22H2+).
enum class WinSystemBackdropType : uint32_t {
  Auto = 0u,
  None = 1u,
  MainWindow = 2u,       // Mica
  TransientWindow = 3u,  // Acrylic
  TabbedWindow = 4u,     // Mica Alt
};

/// Unpacked RGBA color decoded from `0xRRGGBBAA` packed native tint values.
struct WinTintColor {
  uint8_t r8{0u};
  uint8_t g8{0u};
  uint8_t b8{0u};
  uint8_t a8{0u};
  float r{0.0f};
  float g{0.0f};
  float b{0.0f};
  float a{0.0f};
};

/// Unpacks a `0xRRGGBBAA` color value into 8-bit and normalized `[0, 1]` channels.
WinTintColor unpack_win_tint_rrggbbaa(uint32_t rrggbbaa) noexcept;

/// Maps `nve::BlurMaterial` to the Win32 DWM backdrop type (Mica / Acrylic / Mica Alt).
WinSystemBackdropType map_win_backdrop_type(nve::BlurMaterial material) noexcept;

/// Win32 Mica / Acrylic backdrop descriptor and Direct2D translucent frosted tint state for
/// `"nve.blur_surface"` / `"nve.blur"`.
class NveWinBlurSurfaceView {
 public:
  NveWinBlurSurfaceView() = default;

  void apply_props(const nve::BlurSurfaceProps& props) noexcept;
  void mark_unmounted() noexcept { unmounted_ = true; }

#if defined(_WIN32)
  /// Applies `DWMWA_SYSTEMBACKDROP_TYPE` (attribute 38) to `hwnd` via `dwmapi.dll` if available.
  bool apply_to_hwnd(HWND hwnd) const noexcept;
#endif

  nve::BlurMaterial material() const noexcept { return material_; }
  WinSystemBackdropType backdrop_type() const noexcept { return backdrop_type_; }
  float intensity() const noexcept { return intensity_; }
  float blur_radius() const noexcept { return blur_radius_; }
  float effective_alpha() const noexcept { return effective_alpha_; }
  bool has_tint() const noexcept { return has_tint_; }
  uint32_t tint_rgba() const noexcept { return tint_rgba_; }
  WinTintColor tint_color() const noexcept { return tint_color_; }
  uint32_t update_count() const noexcept { return update_count_; }
  bool unmounted() const noexcept { return unmounted_; }

 private:
  nve::BlurMaterial material_{nve::BlurMaterial::Regular};
  WinSystemBackdropType backdrop_type_{WinSystemBackdropType::MainWindow};
  float intensity_{1.0f};
  float blur_radius_{24.0f};
  float effective_alpha_{0.72f};
  bool has_tint_{false};
  uint32_t tint_rgba_{0u};
  WinTintColor tint_color_{};
  uint32_t update_count_{0u};
  bool unmounted_{false};
};
#endif // NVE_NV_ENABLE_BLUR

#if NVE_NV_ENABLE_WEBVIEW
/// Returns `true` if `url` uses an allowed WebView scheme (`https://`, `http://`, `about:blank`).
/// Rejects `javascript:`, `vbscript:`, `data:`, and control-character obfuscated URLs.
bool is_safe_win_webview_url(std::string_view url) noexcept;

/// Pluggable WebView2 backend interface so a real `ICoreWebView2Controller` binds when present.
class NveWinWebViewBackend {
 public:
  virtual ~NveWinWebViewBackend() = default;
  virtual void set_allows_back_forward(bool allowed) = 0;
  virtual bool navigate(std::string_view url) = 0;
  virtual void reload() = 0;
  virtual bool go_back() = 0;
  virtual bool go_forward() = 0;
  virtual void evaluate_script(std::string_view script) = 0;
  virtual void close() = 0;
};

/// Windows WebView2 controller adapter for `"nve.web_view"` / `"nve.webview"`.
/// Requires a non-null injected `NveWinWebViewBackend`; mounting without one returns `nullptr`.
class NveWinWebView {
 public:
  explicit NveWinWebView(std::unique_ptr<NveWinWebViewBackend> backend);
  ~NveWinWebView();

  NveWinWebView(const NveWinWebView&) = delete;
  NveWinWebView& operator=(const NveWinWebView&) = delete;

  void apply_props(const nve::WebViewProps& props);
  bool dispatch_command(const nve::WebViewCommand& cmd);
  void unmount();

  const std::string& last_requested_url() const noexcept { return last_requested_url_; }
  bool allows_back_forward() const noexcept { return allows_back_forward_; }
  bool unmounted() const noexcept { return unmounted_; }
  NveWinWebViewBackend* backend() const noexcept { return backend_.get(); }

 private:
  bool load_safe_url(std::string_view url);

  std::unique_ptr<NveWinWebViewBackend> backend_;
  std::string last_requested_url_;
  bool allows_back_forward_{true};
  bool unmounted_{false};
};
#endif // NVE_NV_ENABLE_WEBVIEW

#if NVE_NV_ENABLE_VIDEO
/// Returns `true` if `uri` is a safe media source (`https://`, `http://`, `file://`).
bool is_safe_win_media_uri(std::string_view uri) noexcept;

/// Pluggable Media Foundation backend interface (`IMFMediaEngine` / `IMFMediaSession`).
class NveWinVideoBackend {
 public:
  virtual ~NveWinVideoBackend() = default;
  virtual void set_source_uri(std::string_view uri) = 0;
  virtual void set_muted(bool muted) = 0;
  virtual void set_looping(bool looping) = 0;
  virtual void set_controls(bool controls) = 0;
  virtual void play() = 0;
  virtual void pause() = 0;
  virtual void seek_ms(uint64_t ms) = 0;
  virtual void set_volume(float volume) = 0;
  virtual void close() = 0;
};

/// Windows Media Foundation video playback adapter for `"nve.video_player"` / `"nve.video"`.
/// Requires a non-null injected `NveWinVideoBackend`; mounting without one returns `nullptr`.
class NveWinVideoPlayer {
 public:
  explicit NveWinVideoPlayer(std::unique_ptr<NveWinVideoBackend> backend);
  ~NveWinVideoPlayer();

  NveWinVideoPlayer(const NveWinVideoPlayer&) = delete;
  NveWinVideoPlayer& operator=(const NveWinVideoPlayer&) = delete;

  void apply_props(const nve::VideoPlayerProps& props);
  bool dispatch_command(const nve::VideoPlayerCommand& cmd);
  void unmount();

  const std::string& source_uri() const noexcept { return source_uri_; }
  bool autoplay() const noexcept { return autoplay_; }
  bool muted() const noexcept { return muted_; }
  bool looping() const noexcept { return looping_; }
  bool controls() const noexcept { return controls_; }
  bool is_playing() const noexcept { return playing_; }
  uint64_t position_ms() const noexcept { return position_ms_; }
  float volume() const noexcept { return volume_; }
  bool unmounted() const noexcept { return unmounted_; }
  NveWinVideoBackend* backend() const noexcept { return backend_.get(); }

 private:
  std::unique_ptr<NveWinVideoBackend> backend_;
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

/// Optional backend factory hooks injected when registering enabled Windows native views.
struct NveWinNativeViewHooks {
#if NVE_NV_ENABLE_WEBVIEW
  std::function<std::unique_ptr<NveWinWebViewBackend>(uint32_t node_id)> create_webview_backend;
#endif
#if NVE_NV_ENABLE_VIDEO
  std::function<std::unique_ptr<NveWinVideoBackend>(uint32_t node_id)> create_video_backend;
#endif
};

/// Registers all compile-time enabled Windows native views (canonical kinds + aliases) in `registry`.
void register_win_native_views(nve::NativeViewRegistry& registry, NveWinNativeViewHooks hooks = {});

/// Binds compile-time enabled Windows native view kinds registered in `registry` to `host`.
void bind_win_native_views_to_fabric_host(
    NveFabricHostHandle host,
    nve::NativeViewRegistry& registry);

#if NVE_NV_ENABLE_BLUR
NveWinBlurSurfaceView* blur_surface_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
#endif
#if NVE_NV_ENABLE_WEBVIEW
NveWinWebView* web_view_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
#endif
#if NVE_NV_ENABLE_VIDEO
NveWinVideoPlayer* video_player_for_node(
    const nve::NativeViewRegistry& registry,
    uint32_t node_id) noexcept;
#endif

} // namespace nve::win
