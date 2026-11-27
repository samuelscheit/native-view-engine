#pragma once

#include "NveWinVisualTree.hpp"

#include <nve/host/host_session.hpp>

#include <cstdint>
#include <functional>
#include <memory>
#include <span>
#include <string>
#include <string_view>

#if defined(_WIN32)
#include "NveWinTextMeasurer.hpp"
#endif

namespace nve::win {

struct WinHostConfig {
  uint32_t surface_id = 1u;
  float width = 640.0f;  // DIPs
  float height = 420.0f;
  float scale_factor = 1.0f;
};

/// Outcome of a primary-button release at a surface point.
struct PressResult {
  uint32_t node_id = 0u;  // the pressed node (0: nothing pressable there)
  bool dispatched = false;
  bool changed = false;   // the press re-rendered the scene
  std::string href;       // a link nobody intercepts: the window opens it natively
};

/// One `libnve_runtime` surface (`HostSession`) mirrored into the portable `NveWinVisualTree`:
/// app sources (in-process, `cdylib` with state-preserving hot reload, paired `nve dev`), app-state
/// files, props / hot patches, and surface-level input (press / link, wheel scroll, hover, cursor,
/// text, focus). The Win32 window (`NveWinHostWindow`), headless screenshots, the window manager,
/// and the react-native-windows component all drive this class; on Windows it installs the
/// DirectWrite measurer into the runtime's layout. Single-threaded: call it on the UI thread and
/// marshal wakes there (`set_wake_poster`).
class NveWinHost {
 public:
  using EventCallback = host::HostSession::EventSink;
  using ChangeCallback = std::function<void()>;

  explicit NveWinHost(WinHostConfig config = {});
  ~NveWinHost();
  NveWinHost(const NveWinHost&) = delete;
  NveWinHost& operator=(const NveWinHost&) = delete;

  // --- App sources --------------------------------------------------------------------------
  bool attach_interface(NveGetInterfaceFn get_interface);
  bool load_dylib(std::string_view path, host::ModuleError* out_error = nullptr, std::string* out_message = nullptr);
  /// Hot-swaps the app `cdylib`, carrying `NveDevApi` state across the reload.
  bool reload_dylib(std::string_view path, host::ModuleError* out_error = nullptr, std::string* out_message = nullptr);
  /// Thin-client mode against an `nve dev` server (`nve://`, `quic://`, `wt://`, `ws(s)://`).
  bool connect_paired(std::string_view uri, std::string_view token, std::string* out_error = nullptr);
  void disconnect_paired();
  [[nodiscard]] bool is_paired() const noexcept;
  /// Applies one `NVE2` frame directly (C++ apps without an `nve_get_interface` module).
  bool submit_frame(std::span<const uint8_t> nve2);

  // --- App state & dev loop -----------------------------------------------------------------
  /// App-state files are the runtime's crash-atomic, checksummed `StateFile`.
  bool save_state(std::string_view path, uint32_t schema_version);
  bool restore_state(std::string_view path, uint32_t schema_version);
  /// Deletes the state file (cold start, `--reset-state`).
  bool clear_state(std::string_view path);
  bool apply_props(std::span<const NveMapEntry> props);
  bool apply_hot_patch(std::span<const uint8_t> patch_nvp1);
  void set_event_callback(EventCallback callback);
  /// Runtime wakes (paired frames, timers, scroll) arrive on ANY thread: `poster` must hop to the
  /// UI thread and call `pump()` there. `{}` removes it.
  void set_wake_poster(host::HostSession::WakePoster poster);
  /// Called on the UI thread whenever the visual tree changed (repaint, resync overlays).
  void set_on_change(ChangeCallback callback) { on_change_ = std::move(callback); }

  /// Ticks the runtime and mirrors the scene; returns the views created or updated.
  size_t pump();

  // --- Input (surface DIPs) -----------------------------------------------------------------
  bool simulate_press(uint32_t node_id, float x = 0.0f, float y = 0.0f);
  /// Presses the topmost node under `(x, y)` that takes presses, else reports its link.
  PressResult press_at(float x, float y);
  /// Scrolls the innermost scroll container under `(x, y)` that can still move by `(dx, dy)`
  /// (chaining outwards at its edges); `true` when one moved.
  bool wheel_at(float x, float y, float dx, float dy);
  /// Pointer moved to `(x, y)`: `PointerLeave` / `PointerEnter` on hover-target changes and
  /// `PointerMove` to subscribers.
  void pointer_move(float x, float y);
  void pointer_leave();
  /// `NVE_CURSOR_KIND_*` at `(x, y)`: the nearest explicit cursor, `POINTER` over links, else `AUTO`.
  [[nodiscard]] uint8_t cursor_at(float x, float y) const;
  bool dispatch_text_change(uint32_t node_id, std::string_view text);
  bool dispatch_submit(uint32_t node_id, std::string_view text);
  bool dispatch_focus(uint32_t node_id, bool focused);
  /// Scrolls `node_id` to the absolute offset (runtime-clamped) and reports it to the app.
  bool dispatch_scroll(uint32_t node_id, float offset_x, float offset_y);

  // --- Surface ------------------------------------------------------------------------------
  void set_surface_size(float width, float height, float scale_factor = 0.0f);
  [[nodiscard]] const WinHostConfig& config() const noexcept { return config_; }

  // --- Introspection ------------------------------------------------------------------------
  [[nodiscard]] const NveWinVisualTree& visual_tree() const noexcept { return tree_; }
  [[nodiscard]] size_t view_count() const noexcept { return tree_.size(); }
  [[nodiscard]] uint32_t surface_id() const noexcept { return config_.surface_id; }
  [[nodiscard]] bool is_loaded() const noexcept { return session_->has_app(); }
  /// The runtime's message for the last failed call on this thread.
  [[nodiscard]] std::string last_error() const { return session_->last_error(); }
  [[nodiscard]] uint32_t hovered_node() const noexcept { return hovered_; }
  [[nodiscard]] host::HostSession& session() noexcept { return *session_; }
#if defined(_WIN32)
  [[nodiscard]] const DirectWriteTextMeasurer& text_measurer() const noexcept { return measurer_; }
#endif

 private:
  bool dispatch(const NveDirectEvent& ev);
  /// The app source changed (`HostSession::source_generation()` moved): the next sync is a full
  /// pass keeping views whose node still matches.
  void source_changed();
  void sync_views();
  void notify_if_changed(uint64_t before);
  /// Shows `node` scrolled to `(x, y)` and reports it (`Scroll` to subscribers).
  bool scrolled(const WinNodeVisual& node, float x, float y);
  /// `kind` at surface `(x, y)` to `node`, in its local coordinates.
  bool dispatch_pointer(uint32_t node, NveEventKind kind, float x, float y);

  WinHostConfig config_;
#if defined(_WIN32)
  DirectWriteTextMeasurer measurer_;
#endif
  std::unique_ptr<host::HostSession> session_;
  NveWinVisualTree tree_;
  ChangeCallback on_change_;
  uint32_t hovered_ = 0u;
  bool syncing_ = false;
};

}  // namespace nve::win
