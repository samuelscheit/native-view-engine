#pragma once

#include "nve_draw_list.hpp"
#include "nve_host_runtime.hpp"

#include <cairo.h>
#include <gtk/gtk.h>

#include <atomic>
#include <cstdint>
#include <list>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nve {

/// Canonical cross-platform accessibility roles inferred from `NveRenderNode` and raw props.
enum class GtkA11yRole : uint8_t {
  None = 0,
  Group,
  Button,
  Text,
  Header,
  TextBox,
  ScrollArea,
  Image,
  List,
  ListItem,
  CheckBox,
  Link,
};

/// Normalized accessibility & TextInput metadata extracted from `raw_props` + `NveRenderNode`.
struct GtkNodeSemantics {
  GtkA11yRole role{GtkA11yRole::Group};
  GtkAccessibleRole gtk_role{GTK_ACCESSIBLE_ROLE_GROUP};
  bool explicit_role{false};
  std::string label;
  bool has_explicit_label{false};
  std::string hint;
  std::string value;
  bool has_explicit_value{false};
  bool disabled{false};
  std::string test_id;
  std::string placeholder;
  bool secure_text_entry{false};
  bool multiline{false};

  bool operator==(const GtkNodeSemantics& other) const noexcept = default;
  bool operator!=(const GtkNodeSemantics& other) const noexcept = default;
};

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

/// Bounded-LRU `GdkTexture` decoder & cache supporting `data:image/...;base64,...` and `file://...`.
class GtkTextureLruCache {
 public:
  explicit GtkTextureLruCache(size_t max_entries = 64u);
  ~GtkTextureLruCache();

  GtkTextureLruCache(const GtkTextureLruCache&) = delete;
  GtkTextureLruCache& operator=(const GtkTextureLruCache&) = delete;

  /// Return a borrowed `GdkTexture*` (owned by the cache) for `uri`, decoding on cache miss.
  GdkTexture* get(std::string_view uri);

  void clear();
  size_t size() const noexcept { return entries_.size(); }
  uint64_t hits() const noexcept { return hits_; }
  uint64_t misses() const noexcept { return misses_; }

 private:
  struct Entry {
    GdkTexture* texture{nullptr};
    std::list<std::string>::iterator lru_it;
  };

  size_t max_entries_{64u};
  std::list<std::string> lru_;
  std::unordered_map<std::string, Entry> entries_;
  uint64_t hits_{0};
  uint64_t misses_{0};
};

/// Stream-decode and paint all 8 `NVE_DRAW1` opcodes onto a Cairo context (`cairo_t*`).
/// Validates the payload first and guarantees balanced `cairo_save()` / `cairo_restore()`
/// even on corrupt payloads or unbalanced clip stacks.
DrawListDecodeResult render_draw_list_to_cairo(
    cairo_t* cr,
    double width,
    double height,
    std::span<const uint8_t> bytes) noexcept;

/// Native GTK4 host shell backed by `nve::NativeHostSession` and
/// `nve::IncrementalSceneReconciler<GtkWidget*, GtkViewAdapter>`.
///
/// Embeds any NVE Rust component (`nve_get_interface` or dynamically loaded `cdylib`)
/// inside a native GTK4 `GtkFixed` root widget with O(changed) `NveSceneView` reconciliation,
/// real Pango (`PangoLayout`) text measurement, `GtkPicture` image rendering with bounded
/// `GdkTexture` LRU caching, scoped CSS styling, `GtkGestureClick` press dispatch,
/// `GtkScrolledWindow` scroll-event dispatch, and dynamic surface resize tracking.
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
  bool attach_interface(NveGetInterfaceFn get_interface, uint32_t surface_id = 0);

  /// Load an NVE `cdylib` (`.so` / `.dylib`) and mount its initial frame.
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

  /// Connect in paired thin-client mode (zero local cdylib required). Incoming `NVD1` frames wake
  /// the GLib main loop via `g_main_context_invoke`; UI input events stream upstream as
  /// `WireKind::EventBatch`.
  bool connect_paired_endpoint(
      std::string_view endpoint,
      std::string_view token,
      std::string_view transport = {},
      std::string* out_error = nullptr,
      uint32_t surface_id = 0);
  bool connect_paired(
      const PairClientConfig& config,
      std::string* out_error = nullptr,
      uint32_t surface_id = 0);
  void disconnect_paired() noexcept;
  size_t poll_paired_frames();
  bool is_paired() const noexcept { return session_.is_paired(); }
  bool is_paired_connected() const noexcept { return session_.is_paired_connected(); }

  /// Zero-copy instant-boot Frame-0 replay from an `NVERELOC` file before any dylib loads.
  bool replay_snapshot(
      std::string_view path,
      uint32_t expected_schema_version,
      NveFrameStats* out_stats = nullptr,
      SnapshotStatus* out_status = nullptr);

  /// Freeze-dries the active shadow tree plus `NveDevApi::snapshot_state()` to `path`.
  bool save_snapshot(
      std::string_view path,
      uint32_t schema_version,
      std::string_view meta = {},
      SnapshotStatus* out_status = nullptr) const;

  bool has_replayed_frame0() const noexcept { return session_.has_replayed_frame0(); }

  /// Push typed props to `NveComponentApi::set_props`, sync GTK widgets if dirty,
  /// and drain outbound events.
  bool apply_props(const PropMap& props);
  bool apply_props(const NveMapEntry* entries, uint32_t count);

  /// Apply one binary `NVP1` Tier-1 hot-patch frame (`NveDevApi::apply_hot_patch`) and sync GTK widgets.
  bool apply_hot_patch(std::span<const uint8_t> patch_nvp1);

  /// Render a new frame from the active app, reconcile GTK widgets, and drain events.
  int32_t render_frame(NveFrameStats* out_stats = nullptr);

  /// Dispatch a native press event at `(x, y)` on `node_id`, sync GTK widgets, and drain events.
  uint32_t simulate_press(uint32_t node_id, float x = 0.0f, float y = 0.0f);

  /// Dispatch UTF-8 `TextChange`, Return-key `onSubmit`, `Focus`/`Blur`, or `Scroll` events.
  uint32_t dispatch_text_change(uint32_t node_id, std::string_view text);
  uint32_t dispatch_submit(uint32_t node_id, std::string_view text);
  uint32_t dispatch_focus(uint32_t node_id, bool focused);
  uint32_t dispatch_scroll(
      uint32_t node_id,
      float offset_x,
      float offset_y,
      float content_width,
      float content_height);

  /// Query extracted per-node accessibility & TextInput semantics for `node_id`.
  std::optional<GtkNodeSemantics> accessibility_info_for_node_id(uint32_t node_id) const;

  /// Update surface viewport dimensions and re-reconcile widget frames immediately.
  void set_surface_size(float width, float height, float scale_factor = 0.0f);

  /// Poll `root_widget_` allocation size & scale factor and propagate changes.
  void sync_surface_size_from_widget();

  /// Connect `notify::default-width` / `notify::default-height` on a hosting `GtkWindow`.
  void attach_window_resize(GtkWindow* window);

  /// Measure UTF-8 text using GTK4's `PangoLayout`.
  NveIntrinsicSize measure_text(
      std::string_view text,
      float font_size,
      float max_width = 0.0f) const;

  /// Look up the retained `GtkWidget*` for a node ID (`NVE_ROOT_NODE_ID` returns `root_widget()`).
  GtkWidget* widget_for_node_id(uint32_t node_id) const noexcept;

  GtkWidget* root_widget() const noexcept { return root_widget_; }
  GtkWidget* host_widget() const noexcept { return viewport_widget_; }

  uint32_t node_count() const noexcept;
  uint32_t surface_id() const noexcept { return config_.surface_id; }
  uint32_t host_instance_id() const noexcept { return host_instance_id_; }
  bool is_loaded() const noexcept { return session_.is_valid() || session_.is_paired(); }

  /// Total incremental widget update passes applied via `IncrementalSceneReconciler`.
  uint64_t reconcile_update_count() const noexcept { return reconcile_update_count_; }

  /// Access the bounded `GdkTexture` LRU image cache (for inspection/tests).
  const GtkTextureLruCache& texture_cache() const noexcept { return texture_cache_; }

  /// Inspect cached `NVE_DRAW1` bytes and redraw count for a `NVE_NODE_KIND_SKIA_CANVAS` node.
  std::span<const uint8_t> canvas_draw_list_for_node_id(uint32_t node_id) const noexcept;
  uint64_t canvas_redraw_count_for_node_id(uint32_t node_id) const noexcept;

  void set_event_callback(EventCallback callback);

  const NativeHostSession& session() const noexcept { return session_; }
  NativeHostSession& session() noexcept { return session_; }

  const std::string& active_css() const noexcept { return active_css_; }

  /// Retained-view adapter passed to `nve::IncrementalSceneReconciler<GtkWidget*, GtkViewAdapter>`.
  struct GtkViewAdapter {
    NveGtkHost* host{nullptr};

    void update_root(const NveSceneView& view, const NveRenderNode& node);
    GtkWidget* create_view(const NveSceneView& view, const NveRenderNode& node);
    void update_view(const NveSceneView& view, GtkWidget*& view_handle, const NveRenderNode& node);
    void destroy_view(uint32_t node_id, GtkWidget*& view_handle);

    template <typename LookupFn>
    void sync_children(
        uint32_t parent_id,
        GtkWidget** parent_view,
        std::span<const uint32_t> child_ids,
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

  struct ScrollBinding {
    NveGtkHost* host{nullptr};
    uint32_t node_id{0};
    GtkWidget* scrolled{nullptr};
    GtkAdjustment* hadj{nullptr};
    GtkAdjustment* vadj{nullptr};
    gulong h_handler{0};
    gulong v_handler{0};
    float content_width{0.0f};
    float content_height{0.0f};
  };

  struct EntryBinding {
    NveGtkHost* host{nullptr};
    uint32_t node_id{0};
    GtkWidget* widget{nullptr};
    gulong changed_handler{0};
    gulong activate_handler{0};
    GtkEventController* focus_controller{nullptr};
    bool updating_programmatically{false};
    bool user_editing{false};
    bool has_synced_snapshot{false};
    std::string last_snapshot_text;
    std::string last_dispatched_text;
  };

  struct CanvasBinding {
    NveGtkHost* host{nullptr};
    uint32_t node_id{0};
    GtkWidget* drawing_area{nullptr};
    std::vector<uint8_t> draw_list_bytes;
    uint64_t redraw_count{0};
  };

  struct WindowBinding {
    GtkWindow* window{nullptr};
    gulong width_handler{0};
    gulong height_handler{0};
    gulong destroy_handler{0};
  };

  void install_pango_text_measurer();
  void sync_views(bool check_raw_props = false);
  void refresh_raw_props_from_view(const NveSceneView& view);
  const GtkNodeSemantics* find_semantics(uint32_t node_id) const noexcept;
  void apply_accessibility_to_widget(GtkWidget* widget, const GtkNodeSemantics& sem);
  void apply_picture_fit(GtkWidget* picture, const std::string& resize_mode);
  EntryBinding* ensure_entry_binding(uint32_t node_id, GtkWidget* widget);
  void remove_entry_binding(uint32_t node_id, GtkWidget* widget);
  CanvasBinding* ensure_canvas_binding(uint32_t node_id, GtkWidget* widget);
  void remove_canvas_binding(uint32_t node_id, GtkWidget* widget);
  ScrollBinding* ensure_scroll_binding(uint32_t node_id, GtkWidget* widget);
  void remove_scroll_binding(uint32_t node_id);
  uint32_t dispatch_event_batch(NveEventBatch batch);
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
  static void on_scroll_value_changed(GtkAdjustment* adjustment, gpointer user_data);
  static void on_entry_changed(GtkEditable* editable, gpointer user_data);
  static void on_entry_activate(GtkEntry* entry, gpointer user_data);
  static void on_entry_focus_enter(GtkEventControllerFocus* controller, gpointer user_data);
  static void on_entry_focus_leave(GtkEventControllerFocus* controller, gpointer user_data);
  static void on_canvas_draw(
      GtkDrawingArea* area,
      cairo_t* cr,
      int width,
      int height,
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
  IncrementalSceneReconciler<GtkWidget*, GtkViewAdapter> reconciler_;
  GtkTextureLruCache texture_cache_{64u};

  GtkWidget* viewport_widget_{nullptr};
  GtkWidget* root_widget_{nullptr};
  GdkDisplay* display_{nullptr};
  GtkCssProvider* css_provider_{nullptr};
  guint tick_callback_id_{0};

  int last_observed_width_{0};
  int last_observed_height_{0};
  int last_observed_scale_{0};

  std::unordered_map<uint32_t, LayoutRect> node_layouts_;
  std::unordered_map<uint32_t, GtkNodeSemantics> node_semantics_;
  std::unordered_map<uint32_t, std::string> node_image_srcs_;
  std::unordered_map<uint32_t, std::string> node_image_resize_modes_;
  std::unordered_map<GtkWidget*, GtkWidget*> scroll_contents_;
  std::unordered_map<uint32_t, std::unique_ptr<ClickBinding>> click_bindings_;
  std::unordered_map<uint32_t, std::unique_ptr<ScrollBinding>> scroll_bindings_;
  std::unordered_map<uint32_t, std::unique_ptr<EntryBinding>> entry_bindings_;
  std::unordered_map<uint32_t, std::unique_ptr<CanvasBinding>> canvas_bindings_;
  std::unordered_map<uint32_t, std::string> node_css_;
  std::string root_css_;
  std::string active_css_;
  bool css_dirty_{false};
  bool raw_props_initialized_{false};
  bool syncing_views_{false};
  uint64_t reconcile_update_count_{0};

  WindowBinding window_binding_{};
  std::shared_ptr<std::atomic<bool>> alive_{std::make_shared<std::atomic<bool>>(true)};
};

} // namespace nve
