#pragma once

#include "NveWinHostWindow.hpp"

#include <cstdint>
#include <functional>
#include <map>
#include <memory>
#include <string>
#include <string_view>
#include <vector>

namespace nve::win {

/// Descriptor for opening a window managed by `NveWinWindowManager`.
struct WinManagedWindowSpec {
  uint32_t surface_id{1};
  std::string title{"NVE Windows Host Shell"};
  int x{0};
  int y{0};
  int width{640};
  int height{420};
  uint32_t parent_surface_id{0};
  bool modal{false};
  bool always_on_top{false};
  bool cascade_close{false};
};

/// Multi-window lifecycle manager mapping `surface_id -> NveWinHostWindow` (and portable
/// `NveRnwComponentHost` on non-Windows platforms for cross-platform CTest coverage).
///
/// Forces `quit_on_destroy = false` on individual Win32 windows and posts `WM_QUIT` (and sets
/// `quit_posted()`) only when the final managed window closes.
class NveWinWindowManager {
 public:
  using CloseVetoCallback = std::function<bool(uint32_t)>;
  using WindowClosedCallback = std::function<void(uint32_t)>;
  using AllWindowsClosedCallback = std::function<void()>;

  NveWinWindowManager() = default;
  ~NveWinWindowManager();

  NveWinWindowManager(const NveWinWindowManager&) = delete;
  NveWinWindowManager& operator=(const NveWinWindowManager&) = delete;

#if defined(_WIN32)
  /// Opens a real Win32 `NveWinHostWindow` with `quit_on_destroy = false`.
  NveWinHostWindow* open_window(
      HINSTANCE instance,
      const WinManagedWindowSpec& spec,
      bool show_window = true);
  NveWinHostWindow* window_for_surface(uint32_t surface_id) noexcept;
  const NveWinHostWindow* window_for_surface(uint32_t surface_id) const noexcept;
  int run_message_loop();
#endif

  /// Portable window registration (also usable on Windows in headless tests without creating an HWND).
  NveRnwComponentHost* open_headless_window(const WinManagedWindowSpec& spec);

  NveRnwComponentHost* host_for_surface(uint32_t surface_id) noexcept;
  const NveRnwComponentHost* host_for_surface(uint32_t surface_id) const noexcept;
  const WinManagedWindowSpec* spec_for_surface(uint32_t surface_id) const noexcept;

  uint32_t active_modal_blocker(uint32_t surface_id) const noexcept;
  bool focus_window(uint32_t surface_id);
  bool set_title(uint32_t surface_id, std::string_view title);
  bool set_bounds(uint32_t surface_id, int x, int y, int width, int height);

  /// Invokes close-veto callback; if allowed, cascades close to owned children bottom-up.
  bool request_close(uint32_t surface_id);

  /// Immediately closes `surface_id` and owned cascading children bottom-up.
  bool close_window(uint32_t surface_id);
  void close_all();

  size_t window_count() const noexcept { return entries_.size(); }
  uint32_t focused_surface_id() const noexcept { return focused_surface_id_; }
  bool quit_posted() const noexcept { return quit_posted_; }
  std::vector<uint32_t> active_surface_ids() const;

  void set_should_close_callback(CloseVetoCallback cb) { should_close_ = std::move(cb); }
  void set_on_window_closed_callback(WindowClosedCallback cb) { on_window_closed_ = std::move(cb); }
  void set_on_all_windows_closed_callback(AllWindowsClosedCallback cb) {
    on_all_closed_ = std::move(cb);
  }

 private:
  struct Entry {
    WinManagedWindowSpec spec{};
    std::vector<uint32_t> children;
#if defined(_WIN32)
    std::unique_ptr<NveWinHostWindow> window;
#endif
    std::unique_ptr<NveRnwComponentHost> portable_host;
  };

  void collect_cascade_post_order(uint32_t surface_id, std::vector<uint32_t>& out) const;
  bool register_entry(const WinManagedWindowSpec& spec, Entry&& entry);

  std::map<uint32_t, Entry> entries_;
  uint32_t focused_surface_id_{0};
  bool closing_internal_{false};
  bool quit_posted_{false};
  CloseVetoCallback should_close_;
  WindowClosedCallback on_window_closed_;
  AllWindowsClosedCallback on_all_closed_;
};

} // namespace nve::win
