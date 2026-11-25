#pragma once

#include "nve_host_runtime.hpp"

#include <gtk/gtk.h>

#include <cstdint>
#include <memory>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace nve {

namespace gtk_detail {

/// Cast a typed C callback function pointer to GLib's `GCallback` without triggering
/// GCC/Clang `-Wcast-function-type` under `-Wall -Wextra -Werror`.
template <typename Fn>
inline GCallback to_gcallback(Fn fn) noexcept {
#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wcast-function-type"
#endif
  const GCallback cb = G_CALLBACK(fn);
#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic pop
#endif
  return cb;
}

} // namespace gtk_detail

/// Native GTK4 host shell backed by `nve::NativeHostSession` and
/// `nve::RetainedViewReconciler<GtkWidget*, GtkViewAdapter>`.
///
/// Embeds any NVE Rust component (`nve_get_interface` or dynamically loaded `cdylib`)
/// inside a native GTK4 `GtkFixed` root widget with real Pango (`PangoLayout`) text
/// measurement, scoped CSS styling, `GtkGestureClick` press dispatch, and dynamic
/// surface resize tracking.
class NveGtkHost {
 public:
  struct GtkViewAdapter;

  explicit NveGtkHost(
      NveSurfaceConfig config = NveSurfaceConfig{1u, 640.0f, 420.0f, 1.0f});
  ~NveGtkHost();

  NveGtkHost(const NveGtkHost&) = delete;
  NveGtkHost& operator=(const NveGtkHost&) = delete;
  NveGtkHost(NveGtkHost&&) = delete;
  NveGtkHost& operator=(NveGtkHost&&) = delete;

  /// Attach an in-process `nve_get_interface` entry point directly.
  /// If `surface_id != 0`, updates the configured surface ID before mounting.
  bool attach_interface(NveGetInterfaceFn get_interface, uint32_t surface_id = 0);

  /// Load an NVE `cdylib` (`.so` / `.dylib`) and mount its initial frame.
  /// On failure, populates `out_err_code` (`1`=open, `2`=no entry, `3`=no surface)
  /// and `out_err_msg`, leaving any previously mounted app intact.
  bool load_dylib(
      std::string_view path,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr,
      uint32_t surface_id = 0);

  /// Atomically hot-reload an NVE `cdylib`, preserving component state via
  /// `NveDevApi::snapshot_state` / `restore_state` and re-reconciling GTK widgets.
  bool reload_dylib(
      std::string_view path,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr);

  /// Zero-copy instant-boot Frame-0 replay from an `NVE_SNAP` file before any dylib loads.
  /// Immediately synchronizes GTK4 widgets so Frame 0 paints on first draw, and caches the
  /// snapshot's state section for automatic restoration when `load_dylib()` / `attach_interface()`
  /// runs next.
  bool replay_snapshot(
      std::string_view path,
      uint32_t expected_schema_version,
      NveFrameStats* out_stats = nullptr,
      SnapshotStatus* out_status = nullptr);

  /// Freeze-dries the active shadow tree (live app or replayed Frame-0 preview) plus
  /// `NveDevApi::snapshot_state()` and persists it atomically to `path`.
  bool save_snapshot(
      std::string_view path,
      uint32_t schema_version,
      std::string_view meta = {},
      SnapshotStatus* out_status = nullptr) const;

  /// True when a Frame-0 snapshot is mounted prior to loading a live dylib/interface.
  bool has_replayed_frame0() const noexcept { return session_.has_replayed_frame0(); }

  /// Push UTF-8 JSON props to `NveComponentApi::set_props`, sync GTK widgets if
  /// dirty, and drain outbound events.
  bool apply_props_json(std::string_view props_json);

  /// Apply a Tier-1 JSON hot patch (`NveDevApi::apply_hot_patch`) and sync GTK widgets.
  bool apply_hot_patch_json(std::string_view patch_json);

  /// Render a new frame from the active app, reconcile GTK widgets, and drain events.
  int32_t render_frame(NveFrameStats* out_stats = nullptr);

  /// Dispatch a native press event at `(x, y)` on `node_id`, sync GTK widgets,
  /// and drain outbound events.
  uint32_t simulate_press(uint32_t node_id, float x = 0.0f, float y = 0.0f);

  /// Update surface viewport dimensions (logical pixels + optional scale factor)
  /// and re-reconcile widget frames immediately.
  void set_surface_size(float width, float height, float scale_factor = 0.0f);

  /// Poll `root_widget_` allocation size & scale factor and propagate changes
  /// to `set_surface_size`.
  void sync_surface_size_from_widget();

  /// Connect `notify::default-width` / `notify::default-height` on a hosting
  /// `GtkWindow` so window resizes trigger immediate layout sync.
  void attach_window_resize(GtkWindow* window);

  /// Measure UTF-8 text using GTK4's `PangoLayout` at exact pixel font size and
  /// optional wrapping `max_width` (`<= 0` or non-finite means unconstrained).
  NveIntrinsicSize measure_text(
      std::string_view text,
      float font_size,
      float max_width = 0.0f) const;

  /// Look up the retained `GtkWidget*` for a node ID (`NVE_ROOT_NODE_ID` returns `root_widget()`).
  GtkWidget* widget_for_node_id(uint32_t node_id) const noexcept;

  /// Root `GtkFixed` content container holding NVE's top-level child widgets.
  GtkWidget* root_widget() const noexcept { return root_widget_; }

  /// Freely resizable viewport widget (`GTK_POLICY_EXTERNAL`, min size 1x1) wrapping
  /// `root_widget()` for `gtk_window_set_child(...)` so the window can shrink and grow.
  GtkWidget* host_widget() const noexcept { return viewport_widget_; }

  /// Total active shadow nodes (including root) when an app is loaded, else `0`.
  uint32_t node_count() const noexcept;

  uint32_t surface_id() const noexcept { return config_.surface_id; }
  uint32_t host_instance_id() const noexcept { return host_instance_id_; }
  bool is_loaded() const noexcept { return session_.is_valid(); }

  /// Register or replace the outbound component event callback.
  void set_event_callback(EventCallback callback);

  /// Access the underlying shared `NativeHostSession`.
  const NativeHostSession& session() const noexcept { return session_; }
  NativeHostSession& session() noexcept { return session_; }

  /// Return the last stylesheet loaded into the root `GtkCssProvider` (for inspection/tests).
  const std::string& active_css() const noexcept { return active_css_; }

  /// Retained-view adapter passed to `nve::RetainedViewReconciler<GtkWidget*, GtkViewAdapter>`.
  struct GtkViewAdapter {
    NveGtkHost* host{nullptr};

    void update_root(const RenderedNodeSnapshot& snap);
    GtkWidget* create_view(const RenderedNodeSnapshot& snap);
    void update_view(GtkWidget*& view, const RenderedNodeSnapshot& snap);
    void destroy_view(uint32_t node_id, GtkWidget*& view);

    template <typename LookupFn>
    void sync_children(
        uint32_t parent_id,
        GtkWidget** parent_view,
        const std::vector<uint32_t>& child_ids,
        LookupFn&& lookup_fn) {
      std::vector<std::pair<uint32_t, GtkWidget*>> resolved;
      resolved.reserve(child_ids.size());
      for (uint32_t cid : child_ids) {
        GtkWidget** child_ptr = lookup_fn(cid);
        if (child_ptr != nullptr && *child_ptr != nullptr) {
          resolved.emplace_back(cid, *child_ptr);
        }
      }
      host->sync_children_impl(parent_id, parent_view, resolved);
    }
  };

 private:
  struct ClickBinding {
    NveGtkHost* host{nullptr};
    uint32_t node_id{0};
    GtkGesture* gesture{nullptr};
  };

  struct WindowBinding {
    GtkWindow* window{nullptr};
    gulong width_handler{0};
    gulong height_handler{0};
    gulong destroy_handler{0};
  };

  void install_pango_text_measurer();
  void sync_views();
  void sync_children_impl(
      uint32_t parent_id,
      GtkWidget** parent_view,
      const std::vector<std::pair<uint32_t, GtkWidget*>>& resolved_children);
  void ensure_click_gesture(uint32_t node_id, GtkWidget* widget, bool interactive);
  void remove_click_gesture(uint32_t node_id, GtkWidget* widget);
  void set_node_css(uint32_t node_id, std::string rule);
  void erase_node_css(uint32_t node_id);
  void flush_css_if_dirty();
  void detach_window_resize();
  LayoutRect layout_for_node(uint32_t node_id) const noexcept;
  std::string scoped_node_class(uint32_t node_id) const;
  std::string scoped_root_class() const;

  static void on_gesture_released(
      GtkGestureClick* gesture,
      gint n_press,
      gdouble x,
      gdouble y,
      gpointer user_data);
  static gboolean on_tick_callback(
      GtkWidget* widget,
      GdkFrameClock* frame_clock,
      gpointer user_data);
  static void on_window_size_notify(
      GObject* object,
      GParamSpec* pspec,
      gpointer user_data);
  static void on_window_destroy(GtkWidget* widget, gpointer user_data);

  uint32_t host_instance_id_{0};
  NveSurfaceConfig config_{};
  NativeHostSession session_;
  GtkViewAdapter adapter_{this};
  RetainedViewReconciler<GtkWidget*, GtkViewAdapter> reconciler_;

  GtkWidget* viewport_widget_{nullptr};
  GtkWidget* root_widget_{nullptr};
  GdkDisplay* display_{nullptr};
  GtkCssProvider* css_provider_{nullptr};
  guint tick_callback_id_{0};

  int last_observed_width_{0};
  int last_observed_height_{0};
  int last_observed_scale_{0};

  std::unordered_map<uint32_t, LayoutRect> node_layouts_;
  std::unordered_map<GtkWidget*, GtkWidget*> scroll_contents_;
  std::unordered_map<uint32_t, std::unique_ptr<ClickBinding>> click_bindings_;
  std::unordered_map<uint32_t, std::string> node_css_;
  std::string root_css_;
  std::string active_css_;
  bool css_dirty_{false};

  WindowBinding window_binding_{};
};

} // namespace nve
