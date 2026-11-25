#include "NveWinHostWindow.hpp"
#include "NveWinD2DInternal.hpp"

#if defined(_WIN32)

#include <algorithm>
#include <cmath>
#include <string>
#include <windowsx.h>

namespace nve::win {

namespace {

constexpr wchar_t kWindowClassName[] = L"NveWinHostWindowClass";

} // namespace

// -----------------------------------------------------------------------------
// NveWinHostWindow
// -----------------------------------------------------------------------------

NveWinHostWindow::NveWinHostWindow() {
  host_.set_on_visual_tree_changed([this]() {
    if (hwnd_ != nullptr) {
      ::InvalidateRect(hwnd_, nullptr, FALSE);
    }
  });
}

NveWinHostWindow::~NveWinHostWindow() {
  wake_hwnd_.store(nullptr, std::memory_order_release);
  host_.disconnect_paired();
  host_.set_on_visual_tree_changed({});
  discard_device_resources();
  if (d2d_factory_ != nullptr) {
    d2d_factory_->Release();
    d2d_factory_ = nullptr;
  }
  if (hwnd_ != nullptr && ::IsWindow(hwnd_)) {
    ::DestroyWindow(hwnd_);
    hwnd_ = nullptr;
  }
}

bool NveWinHostWindow::create(HINSTANCE instance, const WinHostWindowOptions& options) {
  options_ = options;
  if (!ensure_d2d_factory()) {
    return false;
  }

  WNDCLASSEXW wc{};
  wc.cbSize = sizeof(wc);
  wc.style = CS_HREDRAW | CS_VREDRAW;
  wc.lpfnWndProc = &NveWinHostWindow::static_wnd_proc;
  wc.hInstance = instance;
  wc.hCursor = ::LoadCursorW(nullptr, reinterpret_cast<LPCWSTR>(IDC_ARROW));
  wc.lpszClassName = kWindowClassName;
  ::RegisterClassExW(&wc);

  const std::wstring title_w = DirectWriteTextMeasurer::utf8_to_wide(
      options_.title.empty() ? "NVE Windows Host Shell" : options_.title);

  RECT desired{0, 0, options_.width, options_.height};
  const DWORD style = WS_OVERLAPPEDWINDOW;
  ::AdjustWindowRect(&desired, style, FALSE);

  hwnd_ = ::CreateWindowExW(
      0,
      kWindowClassName,
      title_w.c_str(),
      style,
      CW_USEDEFAULT,
      CW_USEDEFAULT,
      desired.right - desired.left,
      desired.bottom - desired.top,
      nullptr,
      nullptr,
      instance,
      this);

  return hwnd_ != nullptr;
}

void NveWinHostWindow::show(int cmd_show) {
  if (hwnd_ != nullptr) {
    ::ShowWindow(hwnd_, cmd_show);
    ::UpdateWindow(hwnd_);
  }
}

int NveWinHostWindow::run_message_loop() {
  MSG msg{};
  while (::GetMessageW(&msg, nullptr, 0, 0) > 0) {
    ::TranslateMessage(&msg);
    ::DispatchMessageW(&msg);
  }
  return static_cast<int>(msg.wParam);
}

float NveWinHostWindow::dpi_scale() const noexcept {
  if (hwnd_ == nullptr) {
    return 1.0f;
  }
  const UINT dpi = ::GetDpiForWindow(hwnd_);
  return (dpi > 0) ? (static_cast<float>(dpi) / 96.0f) : 1.0f;
}

NveSurfaceConfig NveWinHostWindow::current_surface_config() const noexcept {
  const float scale = dpi_scale();
  float dip_w = static_cast<float>(options_.width > 0 ? options_.width : 640);
  float dip_h = static_cast<float>(options_.height > 0 ? options_.height : 420);
  if (hwnd_ != nullptr) {
    RECT rc{};
    if (::GetClientRect(hwnd_, &rc)) {
      const float px_w = static_cast<float>(std::max<LONG>(0, rc.right - rc.left));
      const float px_h = static_cast<float>(std::max<LONG>(0, rc.bottom - rc.top));
      if (px_w > 0.0f && px_h > 0.0f) {
        dip_w = px_w / scale;
        dip_h = px_h / scale;
      }
    }
  }
  return NveSurfaceConfig{
      .surface_id = options_.surface_id,
      .width = dip_w,
      .height = dip_h,
      .scale_factor = scale,
  };
}

bool NveWinHostWindow::attach_interface(NveGetInterfaceFn get_interface) {
  return host_.attach_interface(get_interface, current_surface_config());
}

bool NveWinHostWindow::load_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg) {
  return host_.load_dylib(path, current_surface_config(), out_err_code, out_err_msg);
}

bool NveWinHostWindow::reload_dylib(
    std::string_view path,
    int* out_err_code,
    std::string* out_err_msg) {
  return host_.reload_dylib(path, out_err_code, out_err_msg);
}

bool NveWinHostWindow::connect_paired(
    std::string_view endpoint,
    std::string_view token,
    std::string_view transport_override,
    std::string* out_error) {
  nve::PairClientConfig cfg{};
  if (!nve::parse_pair_endpoint(endpoint, transport_override, cfg, out_error)) {
    return false;
  }
  cfg.pairing_token.assign(token.data(), token.size());
  return connect_paired(cfg, out_error);
}

bool NveWinHostWindow::connect_paired(
    const nve::PairClientConfig& pair_config,
    std::string* out_error) {
  auto wake_cb = [this]() {
    if (HWND target = wake_hwnd_.load(std::memory_order_acquire); target != nullptr) {
      ::PostMessageW(target, WM_APP_PAIR_FRAME, 0, 0);
    }
  };
  return host_.connect_paired(pair_config, current_surface_config(), std::move(wake_cb), out_error);
}

void NveWinHostWindow::disconnect_paired() noexcept {
  host_.disconnect_paired();
}

void NveWinHostWindow::set_title(std::string_view title) {
  options_.title.assign(title.data(), title.size());
  if (hwnd_ != nullptr) {
    const std::wstring wtitle = DirectWriteTextMeasurer::utf8_to_wide(options_.title);
    ::SetWindowTextW(hwnd_, wtitle.c_str());
  }
}

void NveWinHostWindow::set_bounds(int x, int y, int width, int height) {
  if (width > 0) options_.width = width;
  if (height > 0) options_.height = height;
  if (hwnd_ != nullptr && width > 0 && height > 0) {
    ::SetWindowPos(hwnd_, nullptr, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
  }
}

void NveWinHostWindow::focus() {
  if (hwnd_ != nullptr) {
    ::SetForegroundWindow(hwnd_);
    ::SetFocus(hwnd_);
  }
}

void NveWinHostWindow::close() {
  if (hwnd_ != nullptr && ::IsWindow(hwnd_)) {
    ::DestroyWindow(hwnd_);
  }
}

void NveWinHostWindow::set_on_close_request(std::function<bool(uint32_t)> callback) {
  on_close_request_ = std::move(callback);
}

void NveWinHostWindow::set_on_destroy(std::function<void(uint32_t)> callback) {
  on_destroy_ = std::move(callback);
}

bool NveWinHostWindow::save_screenshot_bmp(
    std::string_view out_bmp_path,
    std::string* out_error) const {
  const NveSurfaceConfig cfg =
      (host_.is_valid() || host_.is_paired()) ? host_.surface_config() : current_surface_config();
  const float scale = (cfg.scale_factor > 0.0f) ? cfg.scale_factor : 1.0f;
  const uint32_t px_w = static_cast<uint32_t>(std::max(1.0f, std::round(cfg.width * scale)));
  const uint32_t px_h = static_cast<uint32_t>(std::max(1.0f, std::round(cfg.height * scale)));
  return render_visual_tree_to_bmp(
      host_.visual_tree(), *host_.text_measurer(), px_w, px_h, scale, out_bmp_path, out_error);
}

bool NveWinHostWindow::ensure_d2d_factory() {
  if (d2d_factory_ != nullptr) {
    return true;
  }
  const HRESULT hr = ::D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &d2d_factory_);
  return SUCCEEDED(hr) && d2d_factory_ != nullptr;
}

bool NveWinHostWindow::ensure_render_target() {
  if (render_target_ != nullptr) {
    return true;
  }
  if (hwnd_ == nullptr || !ensure_d2d_factory()) {
    return false;
  }

  RECT rc{};
  ::GetClientRect(hwnd_, &rc);
  const D2D1_SIZE_U size = D2D1::SizeU(
      static_cast<UINT32>(std::max<LONG>(1, rc.right - rc.left)),
      static_cast<UINT32>(std::max<LONG>(1, rc.bottom - rc.top)));

  const HRESULT hr = d2d_factory_->CreateHwndRenderTarget(
      D2D1::RenderTargetProperties(),
      D2D1::HwndRenderTargetProperties(hwnd_, size),
      &render_target_);
  if (FAILED(hr) || render_target_ == nullptr) {
    return false;
  }

  const float dpi = dpi_scale() * 96.0f;
  render_target_->SetDpi(dpi, dpi);
  return true;
}

void NveWinHostWindow::discard_device_resources() noexcept {
  if (render_target_ != nullptr) {
    d2d_internal::clear_d2d_bitmap_cache_for_target(render_target_);
    render_target_->Release();
    render_target_ = nullptr;
  }
}

void NveWinHostWindow::on_paint() {
  PAINTSTRUCT ps{};
  ::BeginPaint(hwnd_, &ps);
  if (ensure_render_target()) {
    const HRESULT hr =
        paint_visual_tree_d2d(render_target_, host_.visual_tree(), *host_.text_measurer());
    if (hr == D2DERR_RECREATE_TARGET) {
      discard_device_resources();
      ::InvalidateRect(hwnd_, nullptr, FALSE);
    }
  }
  ::EndPaint(hwnd_, &ps);
}

void NveWinHostWindow::on_resize(UINT width_px, UINT height_px) {
  if (width_px == 0 || height_px == 0) {
    return;
  }
  if (render_target_ != nullptr) {
    (void)render_target_->Resize(D2D1::SizeU(width_px, height_px));
  }
  const float scale = dpi_scale();
  host_.set_surface_size(
      static_cast<float>(width_px) / scale, static_cast<float>(height_px) / scale, scale);
  ::InvalidateRect(hwnd_, nullptr, FALSE);
}

void NveWinHostWindow::on_left_button_up(int x_px, int y_px) {
  const float scale = dpi_scale();
  const float dip_x = static_cast<float>(x_px) / scale;
  const float dip_y = static_cast<float>(y_px) / scale;
  if (host_.dispatch_click_at(dip_x, dip_y) > 0) {
    ::InvalidateRect(hwnd_, nullptr, FALSE);
  }
}

void NveWinHostWindow::on_mouse_wheel(
    int screen_x,
    int screen_y,
    short wheel_delta,
    bool horizontal) {
  if (hwnd_ == nullptr) {
    return;
  }
  POINT pt{screen_x, screen_y};
  ::ScreenToClient(hwnd_, &pt);
  const float scale = dpi_scale();
  const float dip_x = static_cast<float>(pt.x) / scale;
  const float dip_y = static_cast<float>(pt.y) / scale;
  const float step = (static_cast<float>(wheel_delta) / static_cast<float>(WHEEL_DELTA)) * 45.0f;
  const float dx = horizontal ? step : 0.0f;
  const float dy = horizontal ? 0.0f : -step;
  (void)host_.dispatch_wheel_at(dip_x, dip_y, dx, dy);
  ::InvalidateRect(hwnd_, nullptr, FALSE);
}

LRESULT CALLBACK NveWinHostWindow::static_wnd_proc(
    HWND hwnd,
    UINT msg,
    WPARAM wparam,
    LPARAM lparam) {
  NveWinHostWindow* self = nullptr;
  if (msg == WM_NCCREATE) {
    const auto* cs = reinterpret_cast<const CREATESTRUCTW*>(lparam);
    self = static_cast<NveWinHostWindow*>(cs->lpCreateParams);
    ::SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    if (self != nullptr) {
      self->hwnd_ = hwnd;
      self->wake_hwnd_.store(hwnd, std::memory_order_release);
    }
  } else {
    self = reinterpret_cast<NveWinHostWindow*>(::GetWindowLongPtrW(hwnd, GWLP_USERDATA));
  }

  if (self != nullptr) {
    return self->handle_message(msg, wparam, lparam);
  }
  return ::DefWindowProcW(hwnd, msg, wparam, lparam);
}

LRESULT NveWinHostWindow::handle_message(UINT msg, WPARAM wparam, LPARAM lparam) {
  switch (msg) {
    case WM_ERASEBKGND:
      return 1;
    case WM_PAINT:
      on_paint();
      return 0;
    case WM_SIZE:
      on_resize(LOWORD(lparam), HIWORD(lparam));
      return 0;
    case WM_DPICHANGED: {
      const float new_dpi = static_cast<float>(HIWORD(wparam));
      const float new_scale = (new_dpi > 0.0f) ? (new_dpi / 96.0f) : 1.0f;
      if (render_target_ != nullptr) {
        render_target_->SetDpi(new_dpi, new_dpi);
      }
      if (const auto* suggested = reinterpret_cast<const RECT*>(lparam)) {
        ::SetWindowPos(
            hwnd_,
            nullptr,
            suggested->left,
            suggested->top,
            suggested->right - suggested->left,
            suggested->bottom - suggested->top,
            SWP_NOZORDER | SWP_NOACTIVATE);
      }
      RECT client{};
      if (::GetClientRect(hwnd_, &client)) {
        const float dip_w = static_cast<float>(std::max<LONG>(1, client.right - client.left)) / new_scale;
        const float dip_h = static_cast<float>(std::max<LONG>(1, client.bottom - client.top)) / new_scale;
        host_.set_surface_size(dip_w, dip_h, new_scale);
      }
      ::InvalidateRect(hwnd_, nullptr, FALSE);
      return 0;
    }
    case WM_LBUTTONUP:
      on_left_button_up(GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam));
      return 0;
    case WM_MOUSEWHEEL:
      on_mouse_wheel(
          GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam), GET_WHEEL_DELTA_WPARAM(wparam), false);
      return 0;
    case WM_MOUSEHWHEEL:
      on_mouse_wheel(
          GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam), GET_WHEEL_DELTA_WPARAM(wparam), true);
      return 0;
    case WM_APP_PAIR_FRAME:
      if (host_.poll_paired_frames() > 0u && hwnd_ != nullptr) {
        ::InvalidateRect(hwnd_, nullptr, FALSE);
      }
      return 0;
    case WM_CLOSE:
      if (on_close_request_ && !on_close_request_(options_.surface_id)) {
        return 0;
      }
      ::DestroyWindow(hwnd_);
      return 0;
    case WM_DESTROY: {
      wake_hwnd_.store(nullptr, std::memory_order_release);
      host_.disconnect_paired();
      discard_device_resources();
      hwnd_ = nullptr;
      auto destroy_cb = on_destroy_;
      const uint32_t sid = options_.surface_id;
      if (destroy_cb) {
        destroy_cb(sid);
      }
      if (options_.quit_on_destroy) {
        ::PostQuitMessage(0);
      }
      return 0;
    }
    default:
      return ::DefWindowProcW(hwnd_, msg, wparam, lparam);
  }
}

} // namespace nve::win

#endif // defined(_WIN32)
