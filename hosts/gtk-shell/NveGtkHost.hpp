#pragma once

#include <cairo.h>
#include <gtk/gtk.h>

#include <nve/canvas/canvas.hpp>
#include <nve/host/core_resources.hpp>
#include <nve/host/host_session.hpp>
#include <nve/host/retained_tree.hpp>
#include <nve/host/urls.hpp>

#include <atomic>
#include <cstdint>
#include <functional>
#include <list>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace nve::gtk {

struct GtkNodeView;

/// Accessibility & text-input semantics applied to one node's widget (from `nve.accessibility`,
/// `nve.text_input`, `nve.interaction`, and the node kind).
struct GtkNodeSemantics {
  GtkAccessibleRole role = GTK_ACCESSIBLE_ROLE_GENERIC;
  std::string label;
  std::string hint;
  std::string value_text;
  std::string placeholder;
  uint8_t level = 0u;
  uint8_t checked = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t selected = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t expanded = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t pressed = NVE_ARIA_TRI_STATE_UNSET;
  float value_now = NAN;
  float value_min = NAN;
  float value_max = NAN;
  bool disabled = false;
  bool hidden = false;
  bool secure = false;
  bool multiline = false;
  bool read_only = false;

  friend bool operator==(const GtkNodeSemantics& a, const GtkNodeSemantics& b) noexcept;
};

namespace gtk_detail {

/// Cast a typed C callback to GLib's `GCallback` without `-Wcast-function-type`.
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

}  // namespace gtk_detail

/// Bounded-LRU `GdkTexture` cache for `nve.res.image_src` resources: `data:image/...;base64,`,
/// `file://` / absolute-path URLs, and inline encoded image bytes (keyed by content hash).
class GtkTextureLruCache {
 public:
  explicit GtkTextureLruCache(size_t max_entries = 64u);
  ~GtkTextureLruCache();
  GtkTextureLruCache(const GtkTextureLruCache&) = delete;
  GtkTextureLruCache& operator=(const GtkTextureLruCache&) = delete;

  /// Borrowed texture (owned by the cache), decoded on a miss; `nullptr` when undecodable.
  GdkTexture* get(const host::ImageSource& src);

  void clear();
  [[nodiscard]] size_t size() const noexcept { return entries_.size(); }
  [[nodiscard]] uint64_t hits() const noexcept { return hits_; }
  [[nodiscard]] uint64_t misses() const noexcept { return misses_; }

 private:
  struct Entry {
    GdkTexture* texture = nullptr;
    std::list<std::string>::iterator lru_it;
  };
  size_t max_entries_;
  std::list<std::string> lru_;
  std::unordered_map<std::string, Entry> entries_;
  uint64_t hits_ = 0u;
  uint64_t misses_ = 0u;
};

/// Paints all 8 `NVE_DRAW1` opcodes onto `cr` (validated first; balanced save/restore even on
/// corrupt payloads or unbalanced clip stacks).
canvas::DrawListDecodeResult render_draw_list_to_cairo(cairo_t* cr, double width, double height,
                                                       std::span<const uint8_t> bytes) noexcept;

struct GtkHostConfig {
  uint32_t surface_id = 1u;
  float width = 640.0f;
  float height = 420.0f;
  float scale_factor = 1.0f;
};

/// Native GTK4 host: one `libnve_runtime` surface (`HostSession`) mirrored into retained GTK
/// widgets by `RetainedTree` (O(changed) per frame). View → `GtkFixed`, Text → `GtkLabel`,
/// Image → `GtkPicture` (LRU `GdkTexture`s), TextInput → `GtkEntry` / `GtkTextView`,
/// Scroll → `GtkScrolledWindow`, Link → `GtkFixed` (link role, opens `href`), `nve.canvas` →
/// `GtkDrawingArea` (cairo `NVE_DRAW1`). Scoped CSS for visuals and inherited typography, GTK
/// accessible roles/properties/states, Pango text measurement fed to layout, press / text /
/// submit / focus / scroll events, live resize, and hot-reload / paired / app-state lifecycles.
class NveGtkHost {
 public:
  using EventCallback = host::HostSession::EventSink;

  explicit NveGtkHost(GtkHostConfig config = {});
  ~NveGtkHost();
  NveGtkHost(const NveGtkHost&) = delete;
  NveGtkHost& operator=(const NveGtkHost&) = delete;

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

  /// Ticks the runtime and mirrors the scene into widgets; returns the views created/updated.
  size_t pump();

  // --- Input --------------------------------------------------------------------------------
  bool simulate_press(uint32_t node_id, float x = 0.0f, float y = 0.0f);
  bool dispatch_text_change(uint32_t node_id, std::string_view text);
  bool dispatch_submit(uint32_t node_id, std::string_view text);
  bool dispatch_focus(uint32_t node_id, bool focused);
  bool dispatch_scroll(uint32_t node_id, float offset_x, float offset_y, float content_width, float content_height);

  // --- Surface ------------------------------------------------------------------------------
  void set_surface_size(float width, float height, float scale_factor = 0.0f);
  void sync_surface_size_from_widget();
  void attach_window_resize(GtkWindow* window);
  /// Pango measurement (also installed as the runtime's text measurer).
  [[nodiscard]] NveIntrinsicSize measure_text(std::string_view text, float font_size, uint16_t font_weight = 400u,
                                              float max_width = 0.0f) const;

  // --- Introspection ------------------------------------------------------------------------
  [[nodiscard]] std::optional<GtkNodeSemantics> accessibility_info_for_node_id(uint32_t node_id) const;
  [[nodiscard]] GtkWidget* widget_for_node_id(uint32_t node_id) const noexcept;
  [[nodiscard]] GtkWidget* root_widget() const noexcept { return root_widget_; }
  [[nodiscard]] GtkWidget* host_widget() const noexcept { return viewport_widget_; }
  [[nodiscard]] size_t view_count() const noexcept { return tree_.size(); }
  [[nodiscard]] uint32_t surface_id() const noexcept { return config_.surface_id; }
  [[nodiscard]] uint32_t host_instance_id() const noexcept { return host_instance_id_; }
  [[nodiscard]] bool is_loaded() const noexcept { return session_->has_app(); }
  /// The runtime's message for the last failed call on this thread.
  [[nodiscard]] std::string last_error() const { return session_->last_error(); }
  [[nodiscard]] uint64_t reconcile_update_count() const noexcept { return reconcile_update_count_; }
  [[nodiscard]] const GtkTextureLruCache& texture_cache() const noexcept { return textures_; }
  [[nodiscard]] std::span<const uint8_t> canvas_draw_list_for_node_id(uint32_t node_id) const noexcept;
  [[nodiscard]] uint64_t canvas_redraw_count_for_node_id(uint32_t node_id) const noexcept;
  [[nodiscard]] const std::string& active_css() const noexcept { return active_css_; }
  [[nodiscard]] host::HostSession& session() noexcept { return *session_; }

  /// `RetainedTree` adapter (NveGtkReconciler.cpp).
  struct Adapter {
    using Handle = GtkNodeView*;
    NveGtkHost* host = nullptr;
    uint32_t tag(const host::NodeData& node) const;
    Handle create(const host::NodeData& node);
    void update(Handle& view, const host::NodeData& node);
    void update_root(const host::NodeData& node);
    void destroy(uint32_t id, Handle& view);
    void set_children(const host::NodeData& parent, Handle* parent_view, std::span<Handle* const> kids);
  };

 private:
  friend struct Adapter;
  friend struct GtkNodeView;

  struct WindowBinding {
    GtkWindow* window = nullptr;
    gulong width_handler = 0u;
    gulong height_handler = 0u;
    gulong destroy_handler = 0u;
  };

  bool dispatch(const NveDirectEvent& ev);
  void install_wake_poster();
  void sync_views();
  void apply_semantics(GtkNodeView& view, const host::NodeData& node);
  void apply_css(GtkNodeView& view, const host::NodeData& node);
  void update_image(GtkNodeView& view, const host::NodeData& node);
  void update_canvas(GtkNodeView& view, uint32_t index);
  void update_scroll(GtkNodeView& view, const host::NodeData& node);
  void update_text_input(GtkNodeView& view, const host::NodeData& node);
  void update_press(GtkNodeView& view, const host::NodeData& node);
  void update_scroll_extents();
  void set_css_rule(uint32_t node_id, std::string rule);
  void flush_css();
  void detach_window_resize();
  [[nodiscard]] std::string scoped_class(uint32_t node_id) const;

  static gboolean on_tick(GtkWidget* widget, GdkFrameClock* clock, gpointer user_data);
  static void on_window_size_notify(GObject* object, GParamSpec* pspec, gpointer user_data);
  static void on_window_destroy(GtkWidget* widget, gpointer user_data);
  static void measure_trampoline(void* ctx, NveStrSlice text, float font_size, uint32_t font_weight, float max_width,
                                 float* out_width, float* out_height);
  /// Payload of published resource `id` of `kind` (empty when absent); valid only while
  /// `sync_views` holds the scene.
  [[nodiscard]] std::span<const uint8_t> scene_resource(uint32_t id, host::Key64 kind) const noexcept;

  uint32_t host_instance_id_;
  GtkHostConfig config_;
  std::unique_ptr<host::HostSession> session_;
  host::SceneTables tables_;
  canvas::CanvasTable canvas_;
  const host::RuntimeSceneGuard* scene_ = nullptr;  // set during `sync_views`
  host::RetainedTree<Adapter> tree_;
  Adapter adapter_{this};
  GtkTextureLruCache textures_{64u};
  std::unordered_set<GtkNodeView*> scroll_views_;
  std::unordered_map<uint32_t, std::string> css_rules_;
  std::string active_css_;
  bool css_dirty_ = false;
  bool syncing_ = false;
  uint64_t reconcile_update_count_ = 0u;

  GtkWidget* viewport_widget_ = nullptr;
  GtkWidget* root_widget_ = nullptr;
  GdkDisplay* display_ = nullptr;
  GtkCssProvider* css_provider_ = nullptr;
  guint tick_id_ = 0u;
  int observed_w_ = 0;
  int observed_h_ = 0;
  int observed_scale_ = 0;
  WindowBinding window_binding_;
  std::shared_ptr<std::atomic<bool>> alive_ = std::make_shared<std::atomic<bool>>(true);
};

}  // namespace nve::gtk
