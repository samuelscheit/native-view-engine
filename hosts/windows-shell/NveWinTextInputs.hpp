// Native Win32 `EDIT` controls over the host window's `TextInput` nodes (Windows only): IME, caret,
// selection, clipboard, and accessibility come from the platform. Each control follows its node's
// painted rect (scroll / clip aware), typography, colours, and `nve.text_input` row; edits reach the
// app through `host::ControlledText` (`TextChange`), Enter submits single-line fields (`Submit`),
// and focus changes dispatch `Focus` / `Blur`.
#pragma once

#if !defined(_WIN32)
#error "NveWinTextInputs.hpp is Windows-only"
#endif

#include "NveWinHost.hpp"

#include <nve/host/controlled_text.hpp>

#include <windows.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <string>
#include <unordered_map>

namespace nve::win {

class WinTextInputs {
 public:
  WinTextInputs() = default;
  ~WinTextInputs() {
    alive_->store(false, std::memory_order_release);
    detach();
  }
  WinTextInputs(const WinTextInputs&) = delete;
  WinTextInputs& operator=(const WinTextInputs&) = delete;

  /// Binds the parent window (`WS_CLIPCHILDREN`) and the host whose inputs it mirrors.
  void attach(HWND parent, NveWinHost* host) noexcept {
    parent_ = parent;
    host_ = host;
  }
  /// Creates, places, restyles, and destroys controls to match the visual tree (`scale`: DPI / 96).
  void sync(float scale);
  /// Destroys every control.
  void clear();
  /// Destroys every control and forgets the window / host (their teardown raises no events).
  void detach();

  /// `WM_COMMAND` from a control (`EN_CHANGE` / `EN_SETFOCUS` / `EN_KILLFOCUS`); `true` when handled.
  bool on_command(WPARAM wparam, LPARAM lparam);
  /// `WM_CTLCOLOREDIT` / `WM_CTLCOLORSTATIC`: the control's colours and background brush (or null).
  HBRUSH on_ctl_color(HDC dc, HWND control);

  [[nodiscard]] HWND control_for_node(uint32_t node_id) const noexcept;
  [[nodiscard]] size_t size() const noexcept { return fields_.size(); }

  /// UTF-8 text of an `EDIT` with `\r\n` folded to `\n`.
  [[nodiscard]] static std::string control_text(HWND control);

 private:
  struct Field {
    HWND hwnd = nullptr;
    HFONT font = nullptr;
    HBRUSH background = nullptr;
    COLORREF text_color = 0;
    COLORREF background_color = 0;
    std::wstring font_key;
    std::string placeholder;
    RECT placed{};
    RECT region{};  // window region while clipped (control coordinates)
    bool visible = false;
    bool clipped = false;
    bool multiline = false;
    bool secure = false;
    host::ControlledText text;
  };

  /// Scope of the host's own control changes: EN_* notifications raised inside are ignored.
  class Busy {
   public:
    explicit Busy(bool& flag) noexcept : flag_(flag), prev_(flag) { flag_ = true; }
    ~Busy() { flag_ = prev_; }
    Busy(const Busy&) = delete;
    Busy& operator=(const Busy&) = delete;

   private:
    bool& flag_;
    bool prev_;
  };

  static LRESULT CALLBACK control_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam, UINT_PTR node_id,
                                       DWORD_PTR self);
  [[nodiscard]] uint32_t node_of(HWND control) const noexcept;
  void create(uint32_t node_id, Field& f, const WinNodeVisual& v);
  void update(Field& f, const WinNodeVisual& v, const PaintFrame& frame, float scale);
  void place(Field& f, const PaintFrame& frame, float scale);
  static void destroy(Field& f);

  HWND parent_ = nullptr;
  NveWinHost* host_ = nullptr;
  std::unordered_map<uint32_t, Field> fields_;  // packed `NodeId` -> control
  UINT next_control_id_ = 0x4E00u;
  bool busy_ = false;
  /// Cleared on destruction: an `EN_CHANGE` dispatch may destroy the window owning `this`.
  std::shared_ptr<std::atomic<bool>> alive_ = std::make_shared<std::atomic<bool>>(true);
};

}  // namespace nve::win
