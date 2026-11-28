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
//
// `dispatch_controlled_edit` is that whole edit path, shared by every host: detect the edit,
// dispatch it (which may destroy the field), re-find the field, apply the synchronous answer.
#pragma once

#include <algorithm>
#include <string>
#include <string_view>
#include <utility>
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
    std::string answer(value);  // `value` may view `app_value_` itself
    synced_ = true;
    in_flight_.clear();
    const bool write = answer != dispatched_;
    if (write) dispatched_ = answer;
    app_value_ = std::move(answer);
    return write;
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

/// A controlled field re-found after a `TextChange` dispatch, with the app's current value.
template <class Field>
struct ControlledAnswer {
  Field* field = nullptr;            // null: the field is gone, or the app answers asynchronously
  ControlledText* text = nullptr;    // `field`'s controller
  std::string_view value;            // the app's value now (republished or kept)
};

/// Outcome of `dispatch_controlled_edit`.
struct ControlledEdit {
  bool dispatched = false;  // the widget's text was a user edit, sent as `TextChange`
  bool changed = false;     // the dispatch changed the scene
  bool alive = false;       // the field survived the dispatch (and the app answered synchronously)
  bool written = false;     // the app transformed or rejected the edit: its value was written back
};

/// The one edit path of a controlled native field showing `current` after a change
/// notification, shared by every host:
///  1. `text.user_edited(current)`: host writes and repeats are not edits;
///  2. `dispatch(sent)` sends `TextChange` and pumps. It may destroy the field (the app removes
///     the input), so neither `text` nor `current` is touched afterwards;
///  3. `refind()` returns the field by node id as a `ControlledAnswer` (null when gone, or when
///     the app answers asynchronously: a paired client);
///  4. a transformed or rejected edit (`ControlledText::app_answered`) is shown again through
///     `write(field, value)`, inside a host write.
template <class Field, class Dispatch, class Refind, class Write>
ControlledEdit dispatch_controlled_edit(ControlledText& text, std::string_view current, Dispatch&& dispatch,
                                        Refind&& refind, Write&& write) {
  ControlledEdit out;
  if (!text.user_edited(current)) return out;
  const std::string sent(current);  // `current` may view the field's own storage
  out.dispatched = true;
  out.changed = dispatch(std::string_view(sent));
  const ControlledAnswer<Field> answer = refind();
  if (answer.field == nullptr || answer.text == nullptr) return out;
  out.alive = true;
  if (answer.text->app_answered(answer.value)) {
    const auto scope = answer.text->host_write();
    write(*answer.field, std::string_view(answer.text->app_value()));
    out.written = true;
  }
  return out;
}

}  // namespace nve::host
