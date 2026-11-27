// Win32 + Direct2D / DirectWrite host window and the painting API (Windows only).
#pragma once

#if !defined(_WIN32)
#error "NveWinHostWindow.hpp is Windows-only"
#endif

#include "NveWinHost.hpp"
#include "NveWinTextInputs.hpp"

#include <nve/canvas/draw_list.hpp>

#include <d2d1.h>
#include <windows.h>

#include <atomic>
#include <cstdint>
#include <functional>
#include <span>
#include <string>
#include <string_view>

namespace nve::win {

/// Runtime wake (any thread) -> `NveWinHost::pump()` on the UI thread.
inline constexpr UINT WM_APP_WAKE = WM_APP + 1u;
/// Re-place the native `EDIT` overlays after the visual tree changed (coalesced, never inline).
inline constexpr UINT WM_APP_SYNC_INPUTS = WM_APP + 2u;

struct WinHostWindowOptions {
  std::string title{"NVE Windows Host Shell"};
  int width = 640;  // client DIPs
  int height = 420;
  uint32_t surface_id = 1u;
  bool quit_on_destroy = true;
};

/// Paints all 8 `NVE_DRAW1` opcodes into `bounds` (validated first; clip stack balanced even for
/// corrupt payloads or unbalanced clips). `factory` builds polylines / stroke styles (nullable).
canvas::DrawListDecodeResult render_draw_list_to_d2d(ID2D1RenderTarget* rt, ID2D1SolidColorBrush* brush,
                                                     ID2D1Factory* factory, const D2D1_RECT_F& bounds,
                                                     std::span<const uint8_t> bytes) noexcept;

/// Paints `tree` (one `BeginDraw` / `EndDraw`) onto any render target: the window's
/// `ID2D1HwndRenderTarget` or an offscreen `ID2D1DCRenderTarget`. `factory` created `rt`.
HRESULT paint_visual_tree_d2d(ID2D1RenderTarget* rt, ID2D1Factory* factory, const NveWinVisualTree& tree,
                              const DirectWriteTextMeasurer& text);

/// Headless: paints `tree` into a 32-bit DIB and writes `out_bmp_path` (no window needed).
bool render_visual_tree_to_bmp(const NveWinVisualTree& tree, const DirectWriteTextMeasurer& text, uint32_t width_px,
                               uint32_t height_px, float scale_factor, std::string_view out_bmp_path,
                               std::string* out_error = nullptr);

/// System cursor (`IDC_*`) of an `NVE_CURSOR_KIND_*`.
[[nodiscard]] LPCWSTR system_cursor(uint8_t kind) noexcept;

/// Per-monitor-DPI-aware top-level window showing one `NveWinHost` surface: Direct2D painting,
/// press / link / wheel / hover / cursor input, native `EDIT` text inputs, queued runtime wakes.
class NveWinHostWindow {
 public:
  explicit NveWinHostWindow(WinHostWindowOptions options = {});
  ~NveWinHostWindow();
  NveWinHostWindow(const NveWinHostWindow&) = delete;
  NveWinHostWindow& operator=(const NveWinHostWindow&) = delete;

  /// Creates the window (hidden until `show`) and sizes the surface to its client area.
  bool create(HINSTANCE instance);
  void show(int cmd_show = SW_SHOWDEFAULT);
  /// Runs the thread's message loop until `WM_QUIT`.
  static int run_message_loop();

  /// Renders the current tree to a `.bmp` (client size x DPI scale).
  bool save_screenshot_bmp(std::string_view out_bmp_path, std::string* out_error = nullptr) const;

  void set_title(std::string_view title);
  void set_bounds(int x, int y, int width, int height);
  void focus();
  void close();
  /// `WM_CLOSE`: return `false` to veto.
  void set_on_close_request(std::function<bool(uint32_t)> callback) { on_close_request_ = std::move(callback); }
  void set_on_destroy(std::function<void(uint32_t)> callback) { on_destroy_ = std::move(callback); }

  [[nodiscard]] uint32_t surface_id() const noexcept { return options_.surface_id; }
  [[nodiscard]] HWND hwnd() const noexcept { return hwnd_; }
  [[nodiscard]] float dpi_scale() const noexcept;
  [[nodiscard]] NveWinHost& host() noexcept { return host_; }
  [[nodiscard]] const NveWinHost& host() const noexcept { return host_; }
  [[nodiscard]] const WinTextInputs& text_inputs() const noexcept { return inputs_; }

 private:
  static LRESULT CALLBACK static_wnd_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);
  LRESULT handle_message(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

  bool ensure_render_target();
  void discard_render_target() noexcept;
  void on_paint();
  void sync_surface_size();
  void on_wheel(WPARAM wparam, LPARAM lparam, bool horizontal);
  void schedule_input_sync();
  [[nodiscard]] float dip(int device_px) const noexcept { return static_cast<float>(device_px) / dpi_scale(); }

  WinHostWindowOptions options_;
  NveWinHost host_;
  WinTextInputs inputs_;
  HWND hwnd_ = nullptr;
  std::atomic<HWND> wake_hwnd_{nullptr};  // read by wake posters on any thread; null once destroyed
  ID2D1Factory* d2d_factory_ = nullptr;
  ID2D1HwndRenderTarget* render_target_ = nullptr;
  float pointer_x_ = -1.0f;  // last pointer position (DIPs) for WM_SETCURSOR
  float pointer_y_ = -1.0f;
  bool tracking_leave_ = false;
  bool input_sync_posted_ = false;
  std::function<bool(uint32_t)> on_close_request_;
  std::function<void(uint32_t)> on_destroy_;
};

}  // namespace nve::win
