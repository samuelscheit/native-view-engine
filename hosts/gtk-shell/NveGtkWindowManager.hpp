#pragma once

#include "NveGtkHost.hpp"

#include <cstdint>
#include <functional>
#include <map>
#include <memory>
#include <string>
#include <string_view>
#include <vector>

namespace nve {

/// Descriptor for opening a top-level GTK4 window managed by `NveGtkWindowManager`.
struct GtkWindowSpec {
  uint32_t surface_id{1};
  std::string title{"NVE GTK4 Host Shell"};
  int width{640};
  int height{420};
  uint32_t parent_surface_id{0};
  bool modal{false};
  bool cascade_close{false};
};

/// Multi-window GTK4 lifecycle manager mapping `surface_id -> (GtkWindow*, NveGtkHost)`.
class NveGtkWindowManager {
 public:
  using CloseVetoCallback = std::function<bool(uint32_t)>;
  using WindowClosedCallback = std::function<void(uint32_t)>;
  using AllWindowsClosedCallback = std::function<void()>;

  NveGtkWindowManager() = default;
  ~NveGtkWindowManager();

  NveGtkWindowManager(const NveGtkWindowManager&) = delete;
  NveGtkWindowManager& operator=(const NveGtkWindowManager&) = delete;

  /// Opens a top-level `GtkWindow` hosting an independent `NveGtkHost` for `spec.surface_id`.
  NveGtkHost* open_window(
      const GtkWindowSpec& spec,
      GtkApplication* app = nullptr,
      bool present = false);

  GtkWindow* window_for_surface(uint32_t surface_id) const noexcept;
  NveGtkHost* host_for_surface(uint32_t surface_id) noexcept;
  const NveGtkHost* host_for_surface(uint32_t surface_id) const noexcept;

  uint32_t active_modal_blocker(uint32_t surface_id) const noexcept;
  bool focus_window(uint32_t surface_id);
  bool set_title(uint32_t surface_id, std::string_view title);
  bool set_bounds(uint32_t surface_id, int width, int height);

  /// Runs close-veto callback; if allowed, cascades close to owned children bottom-up.
  bool request_close(uint32_t surface_id);

  /// Immediately closes `surface_id` and owned cascading children bottom-up.
  bool close_window(uint32_t surface_id);
  void close_all();

  size_t window_count() const noexcept { return entries_.size(); }
  uint32_t focused_surface_id() const noexcept { return focused_surface_id_; }
  std::vector<uint32_t> active_surface_ids() const;

  void set_should_close_callback(CloseVetoCallback cb) { should_close_ = std::move(cb); }
  void set_on_window_closed_callback(WindowClosedCallback cb) { on_window_closed_ = std::move(cb); }
  void set_on_all_windows_closed_callback(AllWindowsClosedCallback cb) {
    on_all_closed_ = std::move(cb);
  }

 private:
  struct CloseSignalCtx {
    NveGtkWindowManager* manager{nullptr};
    uint32_t surface_id{0};
  };

  struct Entry {
    GtkWindowSpec spec{};
    GtkWindow* window{nullptr};
    std::unique_ptr<NveGtkHost> host;
    std::unique_ptr<CloseSignalCtx> signal_ctx;
    gulong close_handler{0};
    std::vector<uint32_t> children;
  };

  static gboolean on_gtk_close_request(GtkWindow* window, gpointer user_data);
  void collect_cascade_post_order(uint32_t surface_id, std::vector<uint32_t>& out) const;

  std::map<uint32_t, Entry> entries_;
  uint32_t focused_surface_id_{0};
  CloseVetoCallback should_close_;
  WindowClosedCallback on_window_closed_;
  AllWindowsClosedCallback on_all_closed_;
};

} // namespace nve
