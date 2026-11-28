#include "NveWinD2DInternal.hpp"

#include <nve/host/urls.hpp>

#include <shellapi.h>
#include <windowsx.h>

#include <algorithm>
#include <cmath>

namespace nve::win {

namespace {

constexpr wchar_t kWindowClass[] = L"NveWinHostWindow";
constexpr DWORD kWindowStyle = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
constexpr float kWheelLineDips = 16.0f;  // one wheel "line" (x SPI_GETWHEELSCROLLLINES / CHARS)

UINT wheel_units(bool horizontal) noexcept {
  UINT units = 3u;
  (void)::SystemParametersInfoW(horizontal ? SPI_GETWHEELSCROLLCHARS : SPI_GETWHEELSCROLLLINES, 0, &units, 0);
  return units == WHEEL_PAGESCROLL ? 10u : units;
}

LONG px(float dips, float scale) noexcept { return static_cast<LONG>(std::lround(dips * scale)); }

}  // namespace

LPCWSTR system_cursor(uint8_t kind) noexcept {
  switch (kind) {
    case NVE_CURSOR_KIND_POINTER: return IDC_HAND;
    case NVE_CURSOR_KIND_TEXT: return IDC_IBEAM;
    case NVE_CURSOR_KIND_GRAB:
    case NVE_CURSOR_KIND_GRABBING:
    case NVE_CURSOR_KIND_MOVE: return IDC_SIZEALL;
    case NVE_CURSOR_KIND_NOT_ALLOWED: return IDC_NO;
    case NVE_CURSOR_KIND_COL_RESIZE: return IDC_SIZEWE;
    case NVE_CURSOR_KIND_ROW_RESIZE: return IDC_SIZENS;
    case NVE_CURSOR_KIND_CROSSHAIR: return IDC_CROSS;
    case NVE_CURSOR_KIND_PROGRESS: return IDC_APPSTARTING;
    case NVE_CURSOR_KIND_WAIT: return IDC_WAIT;
    default: return IDC_ARROW;  // auto / default
  }
}

NveWinHostWindow::NveWinHostWindow(WinHostWindowOptions options) : options_(std::move(options)) {}

NveWinHostWindow::~NveWinHostWindow() {
  on_destroy_ = {};
  wake_hwnd_.store(nullptr, std::memory_order_release);
  if (host_ != nullptr) {
    host_->set_wake_poster({});  // drains in-flight wakes
    host_->set_on_change({});
  }
  inputs_.detach();
  discard_render_target();
  if (hwnd_ != nullptr && ::IsWindow(hwnd_)) ::DestroyWindow(hwnd_);
  if (d2d_factory_ != nullptr) d2d_factory_->Release();
}

RECT NveWinHostWindow::outer_rect(int width_dips, int height_dips) const noexcept {
  const UINT dpi = hwnd_ != nullptr ? ::GetDpiForWindow(hwnd_) : ::GetDpiForSystem();
  RECT r{0, 0, ::MulDiv(std::max(1, width_dips), static_cast<int>(dpi), 96), ::MulDiv(std::max(1, height_dips), static_cast<int>(dpi), 96)};
  ::AdjustWindowRectExForDpi(&r, kWindowStyle, FALSE, 0, dpi);
  return r;
}

bool NveWinHostWindow::create(HINSTANCE instance, std::string* error) {
  const auto fail = [error](const char* why) {
    if (error != nullptr && error->empty()) *error = why;
    return false;
  };
  if (host_ == nullptr) {
    host_ = NveWinHost::create(WinHostConfig{options_.surface_id, static_cast<float>(std::max(1, options_.width)),
                                             static_cast<float>(std::max(1, options_.height)), 1.0f},
                               error);
    if (host_ == nullptr) return fail("creating the runtime surface failed");
  }
  if (d2d_factory_ == nullptr && FAILED(::D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &d2d_factory_))) {
    d2d_factory_ = nullptr;
    return fail("D2D1CreateFactory failed");
  }
  WNDCLASSEXW wc{};
  wc.cbSize = sizeof(wc);
  wc.style = CS_HREDRAW | CS_VREDRAW;
  wc.lpfnWndProc = &NveWinHostWindow::static_wnd_proc;
  wc.hInstance = instance;
  wc.hCursor = ::LoadCursorW(nullptr, IDC_ARROW);
  wc.lpszClassName = kWindowClass;
  (void)::RegisterClassExW(&wc);  // a second window reuses the class

  const RECT frame = outer_rect(options_.width, options_.height);
  const std::wstring title = wide(options_.title);
  hwnd_ = ::CreateWindowExW(0, kWindowClass, title.c_str(), kWindowStyle, CW_USEDEFAULT, CW_USEDEFAULT,
                            frame.right - frame.left, frame.bottom - frame.top, nullptr, nullptr, instance, this);
  if (hwnd_ == nullptr) {
    wake_hwnd_.store(nullptr, std::memory_order_release);  // WM_NCCREATE may have published it
    return fail("CreateWindowExW failed");
  }
  // Wakes fire on any thread, also inline inside attach / restore_state: always queue them.
  host_->set_wake_poster([this] {
    if (HWND target = wake_hwnd_.load(std::memory_order_acquire); target != nullptr) {
      (void)::PostMessageW(target, WM_APP_WAKE, 0, 0);
    }
  });
  host_->set_on_change([this] { on_change(); });
  inputs_.attach(hwnd_, host_.get());
  sync_surface_size();
  return true;
}

void NveWinHostWindow::show(int cmd_show) {
  if (hwnd_ == nullptr) return;
  ::ShowWindow(hwnd_, cmd_show);
  ::UpdateWindow(hwnd_);
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
  const UINT dpi = hwnd_ != nullptr ? ::GetDpiForWindow(hwnd_) : 0u;
  return dpi > 0u ? static_cast<float>(dpi) / 96.0f : 1.0f;
}

bool NveWinHostWindow::save_screenshot_bmp(std::string_view out_bmp_path, std::string* out_error) const {
  const WinHostConfig& c = host_->config();
  const auto size = [&c](float dips) { return static_cast<uint32_t>(std::max(1.0f, std::round(dips * c.scale_factor))); };
  return render_visual_tree_to_bmp(host_->visual_tree(), host_->text_measurer(), size(c.width), size(c.height),
                                   c.scale_factor, out_bmp_path, out_error);
}

void NveWinHostWindow::set_title(std::string_view title) {
  options_.title.assign(title);
  if (hwnd_ != nullptr) ::SetWindowTextW(hwnd_, wide(options_.title).c_str());
}

void NveWinHostWindow::set_bounds(int x, int y, int width, int height) {
  if (hwnd_ == nullptr || width <= 0 || height <= 0) return;
  const RECT outer = outer_rect(width, height);  // client DIPs -> outer device pixels, as in `create`
  ::SetWindowPos(hwnd_, nullptr, x, y, outer.right - outer.left, outer.bottom - outer.top, SWP_NOZORDER | SWP_NOACTIVATE);
}

void NveWinHostWindow::focus() {
  if (hwnd_ == nullptr) return;
  ::SetForegroundWindow(hwnd_);
  ::SetFocus(hwnd_);
}

void NveWinHostWindow::close() {
  if (hwnd_ != nullptr && ::IsWindow(hwnd_)) ::DestroyWindow(hwnd_);
}

void NveWinHostWindow::on_change() {
  if (hwnd_ == nullptr) return;
  const Rect damage = host_->take_damage();  // only what changed (hover without app changes: nothing)
  if (!damage.empty()) {
    const float s = dpi_scale();
    const RECT r{px(damage.x, s) - 1, px(damage.y, s) - 1, px(damage.right(), s) + 1, px(damage.bottom(), s) + 1};
    ::InvalidateRect(hwnd_, &r, FALSE);
  }
  schedule_input_sync();
}

void NveWinHostWindow::schedule_input_sync() {
  if (hwnd_ == nullptr || input_sync_posted_) return;
  input_sync_posted_ = ::PostMessageW(hwnd_, WM_APP_SYNC_INPUTS, 0, 0) != FALSE;
}

void NveWinHostWindow::open_link(const std::string& href) {
  if (!host::openable_link(href)) return;  // http(s) / mailto only
  const std::wstring target = wide(href);
  SHELLEXECUTEINFOW info{};
  info.cbSize = sizeof(info);
  info.fMask = SEE_MASK_ASYNCOK | SEE_MASK_FLAG_NO_UI;  // never block the UI thread
  info.hwnd = hwnd_;
  info.lpVerb = L"open";
  info.lpFile = target.c_str();
  info.nShow = SW_SHOWNORMAL;
  (void)::ShellExecuteExW(&info);
}

bool NveWinHostWindow::ensure_render_target() {
  if (render_target_ != nullptr) return true;
  if (hwnd_ == nullptr || d2d_factory_ == nullptr) return false;
  RECT rc{};
  ::GetClientRect(hwnd_, &rc);
  const D2D1_SIZE_U size = D2D1::SizeU(static_cast<UINT32>(std::max<LONG>(1, rc.right - rc.left)),
                                       static_cast<UINT32>(std::max<LONG>(1, rc.bottom - rc.top)));
  if (FAILED(d2d_factory_->CreateHwndRenderTarget(D2D1::RenderTargetProperties(),
                                                  D2D1::HwndRenderTargetProperties(hwnd_, size), &render_target_))) {
    render_target_ = nullptr;
    return false;
  }
  const float dpi = 96.0f * dpi_scale();
  render_target_->SetDpi(dpi, dpi);
  return true;
}

void NveWinHostWindow::discard_render_target() noexcept {
  cache_.release();  // its bitmaps belong to the target
  if (render_target_ == nullptr) return;
  render_target_->Release();
  render_target_ = nullptr;
}

void NveWinHostWindow::on_paint() {
  PAINTSTRUCT ps{};
  ::BeginPaint(hwnd_, &ps);
  const float s = dpi_scale();
  const Rect dirty{static_cast<float>(ps.rcPaint.left) / s, static_cast<float>(ps.rcPaint.top) / s,
                   static_cast<float>(ps.rcPaint.right - ps.rcPaint.left) / s,
                   static_cast<float>(ps.rcPaint.bottom - ps.rcPaint.top) / s};
  if (!dirty.empty() && ensure_render_target() &&
      paint_visual_tree_d2d(render_target_, d2d_factory_, host_->visual_tree(), host_->text_measurer(), cache_, &dirty) ==
          D2DERR_RECREATE_TARGET) {
    discard_render_target();  // device lost: rebuild on the next paint
    ::InvalidateRect(hwnd_, nullptr, FALSE);
  }
  ::EndPaint(hwnd_, &ps);
}

void NveWinHostWindow::sync_surface_size() {
  RECT rc{};
  if (hwnd_ == nullptr || !::GetClientRect(hwnd_, &rc)) return;
  const LONG w = rc.right - rc.left, h = rc.bottom - rc.top;
  if (w <= 0 || h <= 0) return;  // minimized
  if (render_target_ != nullptr) {
    (void)render_target_->Resize(D2D1::SizeU(static_cast<UINT32>(w), static_cast<UINT32>(h)));
    const float dpi = 96.0f * dpi_scale();
    render_target_->SetDpi(dpi, dpi);
  }
  host_->set_surface_size(dip(w), dip(h), dpi_scale());
  schedule_input_sync();  // fonts and placement follow the DPI scale
  ::InvalidateRect(hwnd_, nullptr, FALSE);
}

void NveWinHostWindow::on_wheel(WPARAM wparam, LPARAM lparam, bool horizontal) {
  POINT pt{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};  // screen coordinates
  ::ScreenToClient(hwnd_, &pt);
  const float notches = static_cast<float>(GET_WHEEL_DELTA_WPARAM(wparam)) / static_cast<float>(WHEEL_DELTA);
  const float step = notches * static_cast<float>(wheel_units(horizontal)) * kWheelLineDips;
  // Wheel up (positive) scrolls content down (offset decreases); tilt right (positive) scrolls right.
  (void)host_->wheel_at(dip(pt.x), dip(pt.y), horizontal ? step : 0.0f, horizontal ? 0.0f : -step);
}

LRESULT CALLBACK NveWinHostWindow::static_wnd_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam) {
  NveWinHostWindow* self = nullptr;
  if (msg == WM_NCCREATE) {
    self = static_cast<NveWinHostWindow*>(reinterpret_cast<const CREATESTRUCTW*>(lparam)->lpCreateParams);
    ::SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    self->hwnd_ = hwnd;
    self->wake_hwnd_.store(hwnd, std::memory_order_release);
  } else {
    self = reinterpret_cast<NveWinHostWindow*>(::GetWindowLongPtrW(hwnd, GWLP_USERDATA));
  }
  return self != nullptr ? self->handle_message(hwnd, msg, wparam, lparam) : ::DefWindowProcW(hwnd, msg, wparam, lparam);
}

LRESULT NveWinHostWindow::handle_message(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam) {
  if (host_ == nullptr) return ::DefWindowProcW(hwnd, msg, wparam, lparam);  // WM_NCCREATE .. create()'s end
  switch (msg) {
    case WM_ERASEBKGND: return 1;
    case WM_PAINT: on_paint(); return 0;
    case WM_SIZE: sync_surface_size(); return 0;
    case WM_DPICHANGED: {
      const auto* suggested = reinterpret_cast<const RECT*>(lparam);
      ::SetWindowPos(hwnd_, nullptr, suggested->left, suggested->top, suggested->right - suggested->left,
                     suggested->bottom - suggested->top, SWP_NOZORDER | SWP_NOACTIVATE);
      sync_surface_size();  // (WM_SIZE may not fire when the pixel size is unchanged)
      return 0;
    }
    case WM_APP_WAKE: (void)host_->pump(); return 0;
    case WM_APP_SYNC_INPUTS:
      input_sync_posted_ = false;
      inputs_.sync(dpi_scale());
      return 0;
    case WM_LBUTTONDOWN:
      ::SetFocus(hwnd_);    // clicking the surface blurs a focused text input
      ::SetCapture(hwnd_);  // the release arrives here even outside the window
      (void)host_->pointer_down(dip(GET_X_LPARAM(lparam)), dip(GET_Y_LPARAM(lparam)));
      return 0;
    case WM_LBUTTONUP: {
      ::ReleaseCapture();
      const PressResult r = host_->pointer_up(dip(GET_X_LPARAM(lparam)), dip(GET_Y_LPARAM(lparam)));
      if (!r.href.empty()) open_link(r.href);
      return 0;
    }
    case WM_MOUSEWHEEL: on_wheel(wparam, lparam, false); return 0;
    case WM_MOUSEHWHEEL: on_wheel(wparam, lparam, true); return 0;
    case WM_MOUSEMOVE:
      if (!tracking_leave_) {
        TRACKMOUSEEVENT tme{sizeof(TRACKMOUSEEVENT), TME_LEAVE, hwnd_, 0};
        tracking_leave_ = ::TrackMouseEvent(&tme) != FALSE;
      }
      host_->pointer_move(dip(GET_X_LPARAM(lparam)), dip(GET_Y_LPARAM(lparam)));
      return 0;
    case WM_MOUSELEAVE:
      tracking_leave_ = false;
      host_->pointer_leave();
      return 0;
    case WM_SETCURSOR:
      if (reinterpret_cast<HWND>(wparam) == hwnd_ && LOWORD(lparam) == HTCLIENT) {
        POINT pt{};  // where the cursor is now (not the last WM_MOUSEMOVE)
        if (::GetCursorPos(&pt) && ::ScreenToClient(hwnd_, &pt)) {
          ::SetCursor(::LoadCursorW(nullptr, system_cursor(host_->cursor_at(dip(pt.x), dip(pt.y)))));
          return TRUE;
        }
      }
      break;  // child EDITs and the frame set their own
    case WM_COMMAND:
      if (inputs_.on_command(wparam, lparam)) return 0;
      break;
    case WM_CTLCOLOREDIT:
    case WM_CTLCOLORSTATIC:  // read-only / disabled EDITs
      if (HBRUSH brush = inputs_.on_ctl_color(reinterpret_cast<HDC>(wparam), reinterpret_cast<HWND>(lparam))) {
        return reinterpret_cast<LRESULT>(brush);
      }
      break;
    case WM_CLOSE:
      if (!on_close_request_ || on_close_request_(options_.surface_id)) ::DestroyWindow(hwnd_);
      return 0;
    case WM_DESTROY: {
      wake_hwnd_.store(nullptr, std::memory_order_release);  // later wakes are dropped
      host_->set_on_change({});  // no repaint / overlay resync from here on
      inputs_.detach();          // destroying EDITs raises EN_KILLFOCUS: ignored once detached
      discard_render_target();
      hwnd_ = nullptr;
      // `on_destroy` may delete `this` (window manager): detach it from the HWND first so the
      // trailing WM_NCDESTROY never reaches a dead object, and touch no member afterwards.
      ::SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
      const auto on_destroy = on_destroy_;
      const bool quit = options_.quit_on_destroy;
      const uint32_t sid = options_.surface_id;
      if (on_destroy) on_destroy(sid);
      if (quit) ::PostQuitMessage(0);
      return 0;
    }
    default: break;
  }
  return ::DefWindowProcW(hwnd, msg, wparam, lparam);
}

}  // namespace nve::win
