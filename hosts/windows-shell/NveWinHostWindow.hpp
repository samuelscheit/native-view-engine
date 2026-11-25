#pragma once

#include "NveRnwComponentView.hpp"

#include <cstdint>
#include <string>
#include <string_view>

#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <d2d1.h>
#include <windows.h>
#endif

namespace nve::win {

/// Options for creating a standalone Win32 + Direct2D/DirectWrite NVE host window.
struct WinHostWindowOptions {
  std::string title{"NVE Windows Host Shell"};
  int width{640};
  int height{420};
  uint32_t surface_id{1};
  bool quit_on_destroy{true};
};

#if defined(_WIN32)

/// Paint an entire reconciled `NveWinVisualTree` onto any Direct2D render target
/// (`ID2D1HwndRenderTarget` for live windows or `ID2D1DCRenderTarget` for offscreen screenshots).
HRESULT paint_visual_tree_d2d(
    ID2D1RenderTarget* rt,
    const NveWinVisualTree& tree,
    const DirectWriteTextMeasurer& measurer);

/// Headlessly render `tree` into a 32-bit BGRA DIB via Direct2D/DirectWrite and write a standard
/// `.bmp` image to `out_bmp_path`. Works without an interactive desktop window for CI/testing.
bool render_visual_tree_to_bmp(
    const NveWinVisualTree& tree,
    const DirectWriteTextMeasurer& measurer,
    uint32_t width_px,
    uint32_t height_px,
    float scale_factor,
    std::string_view out_bmp_path,
    std::string* out_error = nullptr);

/// Standalone Win32 + Direct2D/DirectWrite host window wrapping `NveRnwComponentHost`.
class NveWinHostWindow {
 public:
  NveWinHostWindow();
  ~NveWinHostWindow();

  NveWinHostWindow(const NveWinHostWindow&) = delete;
  NveWinHostWindow& operator=(const NveWinHostWindow&) = delete;

  /// Create the Win32 window and initialize the Direct2D factory.
  bool create(HINSTANCE instance, const WinHostWindowOptions& options = {});

  /// Show and update the window.
  void show(int cmd_show = SW_SHOWDEFAULT);

  /// Run the Win32 message loop until `WM_QUIT`.
  int run_message_loop();

  /// Attach a statically compiled NVE interface (`nve_get_interface`).
  bool attach_interface(NveGetInterfaceFn get_interface);

  /// Load an NVE Rust `.dll` into the host window.
  bool load_dylib(
      std::string_view path,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr);

  /// Hot-reload a replacement NVE Rust `.dll`, preserving component state.
  bool reload_dylib(
      std::string_view path,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr);

  /// Render the current visual tree to a `.bmp` file via the shared Direct2D/DirectWrite painter.
  bool save_screenshot_bmp(std::string_view out_bmp_path, std::string* out_error = nullptr) const;

  HWND hwnd() const noexcept { return hwnd_; }
  float dpi_scale() const noexcept;
  NveRnwComponentHost& host() noexcept { return host_; }
  const NveRnwComponentHost& host() const noexcept { return host_; }

 private:
  static LRESULT CALLBACK static_wnd_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);
  LRESULT handle_message(UINT msg, WPARAM wparam, LPARAM lparam);

  bool ensure_d2d_factory();
  bool ensure_render_target();
  void discard_device_resources() noexcept;
  NveSurfaceConfig current_surface_config() const noexcept;
  void on_paint();
  void on_resize(UINT width_px, UINT height_px);
  void on_left_button_up(int x_px, int y_px);
  void on_mouse_wheel(int screen_x, int screen_y, short wheel_delta, bool horizontal);

  WinHostWindowOptions options_{};
  HWND hwnd_{nullptr};
  ID2D1Factory* d2d_factory_{nullptr};
  ID2D1HwndRenderTarget* render_target_{nullptr};
  NveRnwComponentHost host_{};
};

#endif // defined(_WIN32)

} // namespace nve::win
