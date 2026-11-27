// NVE host runtime: controlled native text fields.
//
// A native text widget (`GtkEntry`, a Win32 `EDIT`, ...) shows the app's `nve.text_input.value`
// and is edited by the user at the same time. `ControlledText` decides, per published value and
// per native edit, whether the widget must show the app's value and whether an edit is
// dispatched as `TextChange`: an app that echoes, transforms, or rejects input never fights the
// user's caret or IME composition, and the host's own writes are never reported as edits.
#pragma once

#include <string>
#include <string_view>

namespace nve::host {

class ControlledText {
 public:
  /// The widget now shows `text` after a change notification; `true` when it is a user edit to
  /// dispatch as `TextChange` (not a host write, not a repeat of the last dispatched text).
  [[nodiscard]] bool user_edited(std::string_view text) {
    if (writing_ || text == dispatched_) return false;
    dispatched_.assign(text);
    user_editing_ = true;
    return true;
  }

  /// The app published `value` while the widget shows `shown`; `true` when the host must write
  /// `value` into the widget (wrap the write in `host_write()`): on the first publish, and when
  /// the app changed the value to something that is neither its last value nor the shown text.
  /// The echo of the user's own edit is absorbed.
  [[nodiscard]] bool app_published(std::string_view value, std::string_view shown) {
    bool write = false;
    if (user_editing_ && value == dispatched_) {
      user_editing_ = false;
    } else if (!synced_ || (value != app_value_ && value != shown)) {
      dispatched_.assign(value);
      user_editing_ = false;
      write = true;
    }
    app_value_.assign(value);
    synced_ = true;
    return write;
  }

  /// Scope of a host write into the widget: change notifications raised inside are not edits.
  class HostWrite {
   public:
    explicit HostWrite(ControlledText& t) noexcept : t_(t), prev_(t.writing_) { t_.writing_ = true; }
    ~HostWrite() { t_.writing_ = prev_; }
    HostWrite(const HostWrite&) = delete;
    HostWrite& operator=(const HostWrite&) = delete;

   private:
    ControlledText& t_;
    bool prev_;
  };
  [[nodiscard]] HostWrite host_write() noexcept { return HostWrite(*this); }

  [[nodiscard]] bool writing() const noexcept { return writing_; }

 private:
  std::string app_value_;   // last value the app published
  std::string dispatched_;  // last text sent as `TextChange` (or written by the host)
  bool user_editing_ = false;
  bool synced_ = false;
  bool writing_ = false;
};

}  // namespace nve::host
