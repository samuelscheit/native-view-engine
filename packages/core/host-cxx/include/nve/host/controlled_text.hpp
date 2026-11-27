// NVE host runtime: controlled native text fields.
//
// A native text widget (`GtkEntry`, a Win32 `EDIT`, ...) shows the app's `nve.text_input.value`
// and is edited by the user at the same time. `ControlledText` decides, per published value and
// per native edit, whether the widget must show the app's value and whether an edit is
// dispatched as `TextChange`: an app that echoes, transforms, or rejects input never fights the
// user's caret or IME composition, and the host's own writes are never reported as edits.
//
// Two answer modes:
// - Synchronous (in-process / `cdylib` apps): the `TextChange` dispatch returns with the app's
//   answer applied. The host then calls `app_answered(current value)`: an accepted edit stays, a
//   transformed or REJECTED one (the app kept its old value, so nothing was republished) is
//   written back. A controlled field without a `TextChange` handler is therefore read-only, as in
//   React.
// - Asynchronous (paired thin client): answers arrive later, while the user may keep typing. The
//   host only calls `app_published`; values that lag behind the user's typing are not written.
#pragma once

#include <algorithm>
#include <string>
#include <string_view>
#include <vector>

namespace nve::host {

class ControlledText {
 public:
  /// The widget now shows `text` after a change notification; `true` when it is a user edit to
  /// dispatch as `TextChange` (not a host write, not a repeat of the last dispatched text).
  [[nodiscard]] bool user_edited(std::string_view text) {
    if (writing_ || text == dispatched_) return false;
    dispatched_.assign(text);
    if (in_flight_.size() == kMaxInFlight) in_flight_.erase(in_flight_.begin());
    in_flight_.emplace_back(text);
    return true;
  }

  /// The app published `value` while the widget shows `shown`; `true` when the host must write
  /// `value` into the widget (wrap the write in `host_write()`): on the first publish, and when
  /// the app changed the value to something that is neither its last value nor the shown text.
  /// Echoes of in-flight edits (the latest or, paired, an older one) are absorbed.
  [[nodiscard]] bool app_published(std::string_view value, std::string_view shown) {
    bool write = false;
    if (const auto echo = std::find(in_flight_.begin(), in_flight_.end(), value); echo != in_flight_.end()) {
      in_flight_.erase(in_flight_.begin(), echo + 1);
    } else if (!synced_ || (value != app_value_ && value != shown)) {
      dispatched_.assign(value);
      in_flight_.clear();
      write = true;
    }
    app_value_.assign(value);
    synced_ = true;
    return write;
  }

  /// Synchronous answer: the `TextChange` dispatch for the last edit returned and the app's
  /// current value is `value` (republished or not). `true` when the host must write `value`
  /// into the widget (the app transformed or rejected the edit).
  [[nodiscard]] bool app_answered(std::string_view value) {
    app_value_.assign(value);
    synced_ = true;
    in_flight_.clear();
    if (value == dispatched_) return false;
    dispatched_.assign(value);
    return true;
  }

  [[nodiscard]] const std::string& app_value() const noexcept { return app_value_; }

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
  static constexpr size_t kMaxInFlight = 32u;

  std::string app_value_;               // last value the app published
  std::string dispatched_;              // last text sent as `TextChange` (or written by the host)
  std::vector<std::string> in_flight_;  // dispatched edits not yet echoed, oldest first
  bool synced_ = false;
  bool writing_ = false;
};

}  // namespace nve::host
