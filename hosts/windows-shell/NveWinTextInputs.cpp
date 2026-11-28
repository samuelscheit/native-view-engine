#include "NveWinTextInputs.hpp"
#include "NveWinTextMeasurer.hpp"

#include <commctrl.h>

#include <algorithm>
#include <cmath>
#include <utility>
#include <vector>

namespace nve::win {

namespace {

COLORREF colorref(uint32_t rgba) noexcept {
  return RGB(rgba & 0xFFu, (rgba >> 8u) & 0xFFu, (rgba >> 16u) & 0xFFu);
}

LONG px(float dip, float scale) noexcept {
  const float v = std::round(dip * scale);
  return std::isfinite(v) ? static_cast<LONG>(std::clamp(v, -1.0e6f, 1.0e6f)) : 0;
}

/// `\n` -> `\r\n` (what a multi-line `EDIT` displays as a line break).
std::wstring to_control_text(std::string_view utf8) {
  const std::wstring w = wide(utf8);
  std::wstring out;
  out.reserve(w.size());
  for (size_t i = 0; i < w.size(); ++i) {
    if (w[i] == L'\n' && (i == 0u || w[i - 1u] != L'\r')) out += L'\r';
    out += w[i];
  }
  return out;
}

}  // namespace

std::string WinTextInputs::control_text(HWND control) {
  const int len = ::GetWindowTextLengthW(control);
  std::wstring w(static_cast<size_t>(std::max(0, len)) + 1u, L'\0');
  w.resize(static_cast<size_t>(std::max(0, ::GetWindowTextW(control, w.data(), len + 1))));
  std::erase_if(w, [](wchar_t c) { return c == L'\r'; });
  return narrow(w);
}

HWND WinTextInputs::control_for_node(uint32_t node_id) const noexcept {
  const auto it = fields_.find(node_id);
  return it != fields_.end() ? it->second.hwnd : nullptr;
}

uint32_t WinTextInputs::node_of(HWND control) const noexcept {
  for (const auto& [id, f] : fields_) {
    if (f.hwnd == control) return id;
  }
  return 0u;
}

void WinTextInputs::destroy(Field& f) {
  // Forget the control first: `DestroyWindow` raises EN_KILLFOCUS, which must not find it.
  const HWND hwnd = std::exchange(f.hwnd, nullptr);
  if (hwnd != nullptr) ::DestroyWindow(hwnd);  // WM_NCDESTROY removes the subclass
  if (f.font != nullptr) ::DeleteObject(f.font);
  if (f.background != nullptr) ::DeleteObject(f.background);
  f = Field{};
}

void WinTextInputs::clear() {
  const Busy busy(busy_);
  for (auto& [id, f] : fields_) destroy(f);
  fields_.clear();
}

void WinTextInputs::detach() {
  clear();
  host_ = nullptr;
  parent_ = nullptr;
}

void WinTextInputs::sync(float scale) {
  if (host_ == nullptr || parent_ == nullptr) return;
  // Destroying / recreating / disabling / moving controls raises EN_* notifications: those are
  // the host's own changes, not user input (and must not re-enter the tree mid-sync).
  const Busy busy(busy_);
  const NveWinVisualTree& tree = host_->visual_tree();
  for (auto it = fields_.begin(); it != fields_.end();) {
    const WinNodeVisual* v = tree.find(it->first);
    if (v == nullptr || v->kind != host::NodeKind::TextInput) {
      destroy(it->second);
      it = fields_.erase(it);
    } else {
      ++it;
    }
  }
  const std::vector<uint32_t> ids(tree.text_inputs().begin(), tree.text_inputs().end());
  for (const uint32_t id : ids) {
    const WinNodeVisual* v = tree.find(id);
    if (v == nullptr) continue;
    Field& f = fields_[id];
    if (f.hwnd != nullptr && (f.multiline != v->input.multiline || f.secure != v->input.secure)) {
      destroy(f);  // `ES_MULTILINE` / `ES_PASSWORD` are creation styles
      v = tree.find(id);  // (never stale: notifications are ignored while busy; re-read anyway)
      if (v == nullptr) continue;
    }
    const auto frame = tree.frame_of(id);
    if (!frame) continue;
    if (f.hwnd == nullptr) create(id, f, *v);
    if (f.hwnd != nullptr) update(f, *v, *frame, scale);
  }
}

void WinTextInputs::create(uint32_t node_id, Field& f, const WinNodeVisual& v) {
  const DWORD style = WS_CHILD | WS_TABSTOP | WS_CLIPSIBLINGS |
                      (v.input.multiline ? ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN | WS_VSCROLL : ES_AUTOHSCROLL) |
                      (v.input.secure && !v.input.multiline ? ES_PASSWORD : 0);
  f.hwnd = ::CreateWindowExW(0, L"EDIT", L"", style, 0, 0, 0, 0, parent_,
                             reinterpret_cast<HMENU>(static_cast<UINT_PTR>(next_control_id_++)),
                             ::GetModuleHandleW(nullptr), nullptr);
  if (f.hwnd == nullptr) return;
  f.multiline = v.input.multiline;
  f.secure = v.input.secure;
  ::SetWindowSubclass(f.hwnd, &WinTextInputs::control_proc, node_id, reinterpret_cast<DWORD_PTR>(this));
}

void WinTextInputs::update(Field& f, const WinNodeVisual& v, const PaintFrame& frame, float scale) {
  // Controlled value: adopt the app's value unless it merely echoes what the user is typing.
  if (f.text.app_published(v.input.value, control_text(f.hwnd))) {
    const auto write = f.text.host_write();
    ::SetWindowTextW(f.hwnd, to_control_text(v.input.value).c_str());
  }
  if (f.placeholder != v.input.placeholder) {
    f.placeholder = v.input.placeholder;
    const std::wstring cue = wide(f.placeholder);
    ::SendMessageW(f.hwnd, EM_SETCUEBANNER, TRUE, reinterpret_cast<LPARAM>(cue.c_str()));
  }
  // 0 (and anything above the control's range) is the control's maximum.
  constexpr uint32_t kEditMax = 0x7FFFFFFEu;
  const uint32_t limit = v.input.max_length == 0u ? kEditMax : std::min(v.input.max_length, kEditMax);
  ::SendMessageW(f.hwnd, EM_SETLIMITTEXT, static_cast<WPARAM>(limit), 0);
  ::SendMessageW(f.hwnd, EM_SETREADONLY, v.input.read_only ? TRUE : FALSE, 0);
  ::EnableWindow(f.hwnd, v.interaction.disabled ? FALSE : TRUE);

  // Typography (GDI font in device pixels) and colours.
  const TextPaint& t = v.text_style;
  const LONG height = -std::max<LONG>(1, px(t.font_size, scale));
  std::wstring key = wide(t.font_family) + L'|' + std::to_wstring(height) + L'|' + std::to_wstring(t.font_weight) +
                     L'|' + std::to_wstring(t.font_style) + L'|' + std::to_wstring(t.text_decoration);
  bool restyled = false;
  if (key != f.font_key) {
    // GDI takes one face name: the CSS list resolves exactly as the painter's (first installed
    // family, generics mapped), never the raw list GDI would silently replace.
    const std::wstring family = host_->text_measurer().resolve_family(t.font_family);
    HFONT font = ::CreateFontW(height, 0, 0, 0, std::clamp<int>(t.font_weight, 1, 1000),
                               t.font_style != NVE_FONT_STYLE_NORMAL ? TRUE : FALSE,
                               (t.text_decoration & NVE_TEXT_DECORATION_UNDERLINE) != 0u ? TRUE : FALSE,
                               (t.text_decoration & NVE_TEXT_DECORATION_LINE_THROUGH) != 0u ? TRUE : FALSE,
                               DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
                               DEFAULT_PITCH | FF_DONTCARE, family.c_str());
    if (font != nullptr) {
      ::SendMessageW(f.hwnd, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
      if (f.font != nullptr) ::DeleteObject(f.font);
      f.font = font;
      f.font_key = std::move(key);
      restyled = true;
    }
  }
  const COLORREF fg = colorref(t.color);
  const COLORREF bg = (v.visual.background >> 24u) != 0u ? colorref(v.visual.background) : ::GetSysColor(COLOR_WINDOW);
  if (f.background == nullptr || bg != f.background_color || fg != f.text_color) {
    if (f.background != nullptr) ::DeleteObject(f.background);
    f.background = ::CreateSolidBrush(bg);
    f.background_color = bg;
    f.text_color = fg;
    restyled = true;
  }
  if (restyled) ::InvalidateRect(f.hwnd, nullptr, TRUE);
  place(f, frame, scale);
}

void WinTextInputs::place(Field& f, const PaintFrame& frame, float scale) {
  const Rect& r = frame.rect;
  const Rect shown = frame.clipped ? r.intersect(frame.clip) : r;
  // Hidden (`visibility` / opacity) hides the control; merely scrolled or clipped out of view
  // keeps it shown with an empty region, so a focused field keeps focus and its IME state.
  const bool visible = frame.visible && frame.opacity > 0.0f;
  const RECT placed{px(r.x, scale), px(r.y, scale), px(r.right(), scale), px(r.bottom(), scale)};
  if (visible != f.visible || !::EqualRect(&placed, &f.placed)) {
    ::SetWindowPos(f.hwnd, nullptr, placed.left, placed.top, placed.right - placed.left, placed.bottom - placed.top,
                   SWP_NOZORDER | SWP_NOACTIVATE | (visible ? SWP_SHOWWINDOW : SWP_HIDEWINDOW));
    f.placed = placed;
    f.visible = visible;
  }
  // Clipped by a scroll / overflow ancestor: show only the visible part (control coordinates).
  const bool clipped = visible && !(shown == r);
  const RECT region = shown.empty() ? RECT{0, 0, 0, 0}
                                    : RECT{px(shown.x, scale) - placed.left, px(shown.y, scale) - placed.top,
                                           px(shown.right(), scale) - placed.left, px(shown.bottom(), scale) - placed.top};
  if (clipped && (!f.clipped || !::EqualRect(&region, &f.region))) {
    HRGN rgn = ::CreateRectRgnIndirect(&region);
    if (rgn != nullptr && ::SetWindowRgn(f.hwnd, rgn, TRUE) == 0) ::DeleteObject(rgn);  // owned by the window on success
    f.region = region;
  } else if (!clipped && f.clipped) {
    ::SetWindowRgn(f.hwnd, nullptr, TRUE);
  }
  f.clipped = clipped;
}

bool WinTextInputs::on_command(WPARAM wparam, LPARAM lparam) {
  const auto control = reinterpret_cast<HWND>(lparam);
  const uint32_t id = control != nullptr ? node_of(control) : 0u;
  if (id == 0u || host_ == nullptr) return false;
  if (busy_) return true;  // our own sync / teardown, not the user
  Field& f = fields_.at(id);
  switch (HIWORD(wparam)) {
    case EN_CHANGE: {
      if (f.text.writing()) return true;  // a host write
      // The shared edit path: the dispatch may remove the field (re-found by id) or destroy the
      // window owning these inputs (`alive`, checked before `this` is touched again); a
      // transformed or rejected answer of an in-process app is shown now (a rejecting app
      // republishes nothing).
      NveWinHost* const host = host_;
      (void)host::dispatch_controlled_edit<Field>(
          f.text, control_text(control),
          [host, id](std::string_view sent) { return host->dispatch_text_change(id, sent); },  // overlay re-sync is posted
          [this, id, alive = alive_]() -> host::ControlledAnswer<Field> {
            if (!alive->load(std::memory_order_acquire) || host_ == nullptr) return {};
            const auto it = fields_.find(id);
            const WinNodeVisual* v = host_->is_paired() ? nullptr : host_->visual_tree().find(id);
            if (it == fields_.end() || it->second.hwnd == nullptr || v == nullptr) return {};
            return {&it->second, &it->second.text, v->input.value};
          },
          [](Field& field, std::string_view value) {
            ::SetWindowTextW(field.hwnd, to_control_text(value).c_str());
            const int end = ::GetWindowTextLengthW(field.hwnd);  // caret after the answer
            ::SendMessageW(field.hwnd, EM_SETSEL, static_cast<WPARAM>(end), static_cast<LPARAM>(end));
          });
      return true;
    }
    case EN_SETFOCUS: (void)host_->dispatch_focus(id, true); return true;
    case EN_KILLFOCUS: (void)host_->dispatch_focus(id, false); return true;
    default: return false;
  }
}

HBRUSH WinTextInputs::on_ctl_color(HDC dc, HWND control) {
  const uint32_t id = node_of(control);
  if (id == 0u) return nullptr;
  const Field& f = fields_.at(id);
  ::SetTextColor(dc, f.text_color);
  ::SetBkColor(dc, f.background_color);
  return f.background;
}

LRESULT CALLBACK WinTextInputs::control_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam, UINT_PTR node_id,
                                             DWORD_PTR self_ptr) {
  auto* self = reinterpret_cast<WinTextInputs*>(self_ptr);
  const auto id = static_cast<uint32_t>(node_id);
  const auto it = self->fields_.find(id);
  const bool single_line = it != self->fields_.end() && !it->second.multiline;
  switch (msg) {
    case WM_KEYDOWN:
      if (wparam == VK_RETURN && single_line && self->host_ != nullptr) {
        (void)self->host_->dispatch_submit(id, control_text(hwnd));
        return 0;
      }
      break;
    case WM_CHAR:
      if (wparam == L'\r' && single_line) return 0;  // no "invalid key" beep after a submit
      break;
    case WM_NCDESTROY: ::RemoveWindowSubclass(hwnd, &WinTextInputs::control_proc, node_id); break;
    default: break;
  }
  return ::DefSubclassProc(hwnd, msg, wparam, lparam);
}

}  // namespace nve::win
