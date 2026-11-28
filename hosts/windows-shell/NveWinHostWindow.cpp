#include "NveWinD2DInternal.hpp"
#include "NveWinKeyMap.hpp"

#include <nve_a11y_win.h>
#include <nve/host/urls.hpp>

#include <shellapi.h>
#include <windowsx.h>

#include <algorithm>
#include <charconv>
#include <cmath>
#include <utility>

extern "C" {

static int nve_win_scene_resource_lookup(void* ctx, uint32_t resource_id, uint64_t expected_kind,
                                         const uint8_t** out_ptr, size_t* out_len) {
  if (ctx == nullptr || out_ptr == nullptr || out_len == nullptr || resource_id == 0u) return 0;
  const auto* guard = static_cast<const nve::host::RuntimeSceneGuard*>(ctx);
  const auto res = guard->resource(resource_id);
  if (!res || res->kind.value != expected_kind) return 0;
  *out_ptr = res->bytes.data();
  *out_len = res->bytes.size();
  return 1;
}

static void nve_win_a11y_action_cb(void* ctx, const NveA11yWinAction* action) {
  if (ctx == nullptr || action == nullptr) return;
  static_cast<nve::win::NveWinHostWindow*>(ctx)->enqueue_a11y_action(*action);
}

}  // extern "C"

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

std::string format_numeric_value(double v) {
  if (!std::isfinite(v)) return {};
  char buf[64]{};
  const auto [ptr, ec] = std::to_chars(buf, buf + sizeof(buf), v);
  return ec == std::errc{} ? std::string(buf, static_cast<size_t>(ptr - buf)) : std::string{};
}

std::string sanitize_single_line(std::string_view raw) {
  std::string out;
  out.reserve(raw.size());
  for (const char c : raw) {
    if (c != '\r' && c != '\n') out.push_back(c);
  }
  return out;
}

size_t utf16_offset_for_char_index(std::string_view utf8, size_t target_chars) noexcept {
  size_t i = 0u, chars = 0u, u16 = 0u;
  while (i < utf8.size() && chars < target_chars) {
    const auto lead = static_cast<unsigned char>(utf8[i]);
    const size_t step = (lead & 0x80u) == 0u ? 1u : (lead & 0xE0u) == 0xC0u ? 2u : (lead & 0xF0u) == 0xE0u ? 3u : 4u;
    u16 += step == 4u ? 2u : 1u;
    i += std::min(step, utf8.size() - i);
    ++chars;
  }
  return u16;
}

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
  alive_->store(false, std::memory_order_release);
  on_destroy_ = {};
  wake_hwnd_.store(nullptr, std::memory_order_release);
  if (host_ != nullptr) {
    host_->set_wake_poster({});  // drains in-flight wakes
    host_->set_on_change({});
    host_->set_on_scene_sync({});
  }
  destroy_a11y();
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
  // Initialize UIA adapter before `show()` (never inside `WM_GETOBJECT`).
  a11y_ = nve_a11y_win_create(hwnd_, ::GetFocus() == hwnd_ ? 1 : 0, &nve_win_a11y_action_cb, this);
  // Wakes fire on any thread, also inline inside attach / restore_state: always queue them.
  host_->set_wake_poster([this] {
    if (HWND target = wake_hwnd_.load(std::memory_order_acquire); target != nullptr) {
      (void)::PostMessageW(target, WM_APP_WAKE, 0, 0);
    }
  });
  host_->set_on_change([this] { on_change(); });
  host_->set_on_scene_sync([this](const host::RuntimeSceneGuard& g) { sync_a11y_with_guard(g); });
  inputs_.attach(hwnd_, host_.get());
  inputs_.set_on_tab([this](bool reverse) {
    const auto alive = alive_;
    if (host_ == nullptr) return;
    const uint32_t next = host_->step_tab_focus(reverse);
    if (!alive->load(std::memory_order_acquire)) return;
    sync_native_focus(next);
    sync_a11y_now();
  });
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

void NveWinHostWindow::sync_native_focus(uint32_t focused_node) {
  if (hwnd_ == nullptr) return;
  if (HWND edit = inputs_.control_for_node(focused_node); edit != nullptr && ::IsWindow(edit)) {
    ::SetFocus(edit);
  } else {
    ::SetFocus(hwnd_);
  }
}

void NveWinHostWindow::on_key(WPARAM wparam, LPARAM lparam, bool pressed) {
  const auto alive = alive_;
  const MappedWinKey mk = map_win32_key(wparam, lparam, pressed);
  const uint32_t prev_focus = host_->focused_node();
  const KeyResult r = host_->handle_key(pressed, mk.key, mk.code, mk.flags, mk.key_code, false);
  if (!alive->load(std::memory_order_acquire)) return;
  if (!r.href.empty()) open_link(r.href);
  if (r.focused_node != prev_focus) {
    sync_native_focus(r.focused_node);
    sync_a11y_now();
  }
}

void NveWinHostWindow::sync_a11y_with_guard(const host::RuntimeSceneGuard& guard) {
  if (a11y_ == nullptr || !nve_a11y_win_is_active(a11y_)) return;
  (void)nve_a11y_win_sync_scene(a11y_, &guard.raw(), &nve_win_scene_resource_lookup,
                                const_cast<host::RuntimeSceneGuard*>(&guard), host_->focused_node(), dpi_scale(),
                                options_.title.c_str());
}

void NveWinHostWindow::sync_a11y_now() {
  if (a11y_ == nullptr || host_ == nullptr || !nve_a11y_win_is_active(a11y_)) return;
  auto guard = host_->session().runtime().acquire_scene();
  if (guard) sync_a11y_with_guard(*guard);
}

void NveWinHostWindow::destroy_a11y() noexcept {
  NveA11yWinHandle* h = std::exchange(a11y_, nullptr);
  if (h != nullptr) nve_a11y_win_destroy(h);
  std::lock_guard<std::mutex> lock(a11y_mu_);
  a11y_actions_.clear();
}

void NveWinHostWindow::enqueue_a11y_action(const NveA11yWinAction& action) {
  QueuedA11yAction q;
  q.node_id = action.node_id;
  q.kind = action.kind;
  q.x = action.x;
  q.y = action.y;
  q.numeric_value = action.numeric_value;
  q.anchor_char = action.anchor_char;
  q.focus_char = action.focus_char;
  if (action.text_ptr != nullptr && action.text_len > 0u) {
    q.text.assign(reinterpret_cast<const char*>(action.text_ptr), action.text_len);
  }
  // Always enqueue + post to the UI thread: `ActionBridge::dispatch` holds its callback-teardown
  // lock, so executing synchronously here could self-deadlock on window destruction or re-enter UIA.
  {
    std::lock_guard<std::mutex> lock(a11y_mu_);
    a11y_actions_.push_back(std::move(q));
  }
  if (HWND target = wake_hwnd_.load(std::memory_order_acquire); target != nullptr) {
    (void)::PostMessageW(target, WM_APP_A11Y_ACTION, 0, 0);
  }
}

void NveWinHostWindow::execute_a11y_action(const QueuedA11yAction& action) {
  const auto alive = alive_;
  if (host_ == nullptr || action.node_id == 0u) return;
  const uint32_t id = action.node_id;
  const WinNodeVisual* v = host_->visual_tree().find(id);
  if (v == nullptr) return;
  const bool disabled = host_->is_node_disabled(id);
  const auto dispatch_target_key = [&](std::string_view key, uint32_t vk) {
    if (disabled) return;
    if (host_->focused_node() != id) {
      (void)host_->set_focused_node(id);
      if (!alive->load(std::memory_order_acquire)) return;
    }
    (void)host_->dispatch_key(id, NVE_INTERACTION_EVENT_KEY_DOWN, key, key, 0u, vk);
  };
  switch (action.kind) {
    case NVE_A11Y_WIN_ACTION_CLICK:
    case NVE_A11Y_WIN_ACTION_EXPAND:
    case NVE_A11Y_WIN_ACTION_COLLAPSE:
      if (disabled) break;
      if (action.kind == NVE_A11Y_WIN_ACTION_CLICK && !v->wants(NVE_INTERACTION_EVENT_PRESS) &&
          host::openable_link(v->href)) {
        open_link(v->href);
      } else {
        (void)host_->simulate_press(id, v->local.width * 0.5f, v->local.height * 0.5f);
      }
      break;
    case NVE_A11Y_WIN_ACTION_FOCUS:
      if (disabled) break;
      (void)host_->set_focused_node(id);
      if (!alive->load(std::memory_order_acquire)) return;
      sync_native_focus(host_->focused_node());
      sync_a11y_now();
      break;
    case NVE_A11Y_WIN_ACTION_BLUR:
      if (host_->focused_node() == id) {
        (void)host_->set_focused_node(0u);
        if (!alive->load(std::memory_order_acquire)) return;
        sync_a11y_now();
      }
      break;
    case NVE_A11Y_WIN_ACTION_SET_VALUE:
      if (disabled || (v->kind == host::NodeKind::TextInput && v->input.read_only)) break;
      (void)host_->dispatch_text_change(id, (v->kind == host::NodeKind::TextInput && !v->input.multiline)
                                                ? sanitize_single_line(action.text)
                                                : action.text);
      break;
    case NVE_A11Y_WIN_ACTION_SET_NUMERIC_VALUE:
      if (!disabled) (void)host_->dispatch_text_change(id, format_numeric_value(action.numeric_value));
      break;
    case NVE_A11Y_WIN_ACTION_REPLACE_SELECTED_TEXT: {
      if (disabled || v->kind != host::NodeKind::TextInput || v->input.read_only) break;
      const bool multiline = v->input.multiline;
      const std::string replacement = multiline ? action.text : sanitize_single_line(action.text);
      const std::wstring wcur = wide(v->input.value);
      DWORD s0 = static_cast<DWORD>(wcur.size()), s1 = static_cast<DWORD>(wcur.size());
      if (HWND edit = inputs_.control_for_node(id); edit != nullptr && ::IsWindow(edit)) {
        ::SendMessageW(edit, EM_GETSEL, reinterpret_cast<WPARAM>(&s0), reinterpret_cast<LPARAM>(&s1));
      }
      const size_t lo = std::min<size_t>(std::min(s0, s1), wcur.size());
      const size_t hi = std::min<size_t>(std::max(s0, s1), wcur.size());
      std::wstring wnext;
      wnext.append(wcur.data(), lo);
      wnext.append(wide(replacement));
      wnext.append(wcur.data() + hi, wcur.size() - hi);
      if (host_->focused_node() != id) {
        (void)host_->set_focused_node(id);
        if (!alive->load(std::memory_order_acquire)) return;
        sync_native_focus(host_->focused_node());
      }
      (void)host_->dispatch_text_change(id, narrow(wnext));
      break;
    }
    case NVE_A11Y_WIN_ACTION_SET_TEXT_SELECTION: {
      if (disabled || v->kind != host::NodeKind::TextInput) break;
      const size_t a16 = utf16_offset_for_char_index(v->input.value, action.anchor_char);
      const size_t f16 = utf16_offset_for_char_index(v->input.value, action.focus_char);
      if (host_->focused_node() != id) {
        (void)host_->set_focused_node(id);
        if (!alive->load(std::memory_order_acquire)) return;
        sync_native_focus(host_->focused_node());
      }
      if (HWND edit = inputs_.control_for_node(id); edit != nullptr && ::IsWindow(edit)) {
        ::SendMessageW(edit, EM_SETSEL, static_cast<WPARAM>(std::min(a16, f16)),
                       static_cast<LPARAM>(std::max(a16, f16)));
      }
      sync_a11y_now();
      break;
    }
    case NVE_A11Y_WIN_ACTION_SCROLL_INTO_VIEW:
      if (host_->scroll_into_view(id) && alive->load(std::memory_order_acquire)) sync_a11y_now();
      break;
    case NVE_A11Y_WIN_ACTION_SET_SCROLL_OFFSET:
      (void)host_->dispatch_scroll(id, action.x, action.y);
      break;
    case NVE_A11Y_WIN_ACTION_SCROLL_BY:
      (void)host_->dispatch_scroll(id, v->scroll_x + action.x, v->scroll_y + action.y);
      break;
    case NVE_A11Y_WIN_ACTION_INCREMENT: dispatch_target_key("ArrowUp", VK_UP); break;
    case NVE_A11Y_WIN_ACTION_DECREMENT: dispatch_target_key("ArrowDown", VK_DOWN); break;
    case NVE_A11Y_WIN_ACTION_SHOW_CONTEXT_MENU: dispatch_target_key("ContextMenu", VK_APPS); break;
    default: break;
  }
}

void NveWinHostWindow::drain_a11y_actions() {
  const auto alive = alive_;
  std::vector<QueuedA11yAction> batch;
  {
    std::lock_guard<std::mutex> lock(a11y_mu_);
    batch.swap(a11y_actions_);
  }
  for (const QueuedA11yAction& action : batch) {
    if (!alive->load(std::memory_order_acquire) || hwnd_ == nullptr) break;
    execute_a11y_action(action);
  }
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
    case WM_APP_A11Y_ACTION: drain_a11y_actions(); return 0;
    case WM_GETOBJECT:
      if (a11y_ != nullptr && nve_a11y_win_wants_wm_getobject(static_cast<intptr_t>(lparam))) {
        if (!nve_a11y_win_is_active(a11y_)) {
          nve_a11y_win_activate(a11y_);
          sync_a11y_now();
        }
        intptr_t res = 0;
        if (nve_a11y_win_handle_wm_getobject(a11y_, static_cast<uintptr_t>(wparam), static_cast<intptr_t>(lparam),
                                             &res)) {
          return static_cast<LRESULT>(res);
        }
      }
      break;
    case WM_SETFOCUS:
    case WM_EXITMENULOOP:
    case WM_EXITSIZEMOVE:
      if (a11y_ != nullptr) nve_a11y_win_set_window_focus(a11y_, 1);
      break;
    case WM_KILLFOCUS:
      // Moving focus to a child native `EDIT` control keeps focus inside this top-level window.
      if (wparam != 0 && ::IsChild(hwnd_, reinterpret_cast<HWND>(wparam))) break;
      if (a11y_ != nullptr) nve_a11y_win_set_window_focus(a11y_, 0);
      break;
    case WM_ENTERMENULOOP:
    case WM_ENTERSIZEMOVE:
      if (a11y_ != nullptr) nve_a11y_win_set_window_focus(a11y_, 0);
      break;
    case WM_KEYDOWN:
    case WM_SYSKEYDOWN: on_key(wparam, lparam, true); return 0;
    case WM_KEYUP:
    case WM_SYSKEYUP: on_key(wparam, lparam, false); return 0;
    case WM_LBUTTONDOWN: {
      const auto alive = alive_;
      const float x = dip(GET_X_LPARAM(lparam)), y = dip(GET_Y_LPARAM(lparam));
      ::SetFocus(hwnd_);    // clicking the surface blurs a focused text input
      ::SetCapture(hwnd_);  // the release arrives here even outside the window
      const uint32_t focus_id = host_->focus_target(x, y);
      const bool focus_changed = host_->set_focused_node(focus_id);
      if (!alive->load(std::memory_order_acquire)) return 0;
      if (focus_changed) sync_a11y_now();
      (void)host_->pointer_down(x, y);
      return 0;
    }
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
      alive_->store(false, std::memory_order_release);
      wake_hwnd_.store(nullptr, std::memory_order_release);  // later wakes are dropped
      host_->set_on_change({});      // no repaint / overlay resync from here on
      host_->set_on_scene_sync({});  // no UIA sync during teardown
      destroy_a11y();                // detach UIA action callbacks before destroying HWND
      inputs_.detach();              // destroying EDITs raises EN_KILLFOCUS: ignored once detached
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
