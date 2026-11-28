// The `libnve_runtime` surface behind both Qt hosts (`NveQtWidget`, `NveQtQuickItem`): one
// `HostSession` (in-process / `cdylib` with hot reload / paired app), UI-thread wakes, the
// `QFontMetrics` text measurer, app-state files, props, hot patches, and typed input dispatch.
// Each host derives from it and mirrors the scene in `sync_views`.
#pragma once

#include "NveQtImageCache.hpp"
#include "NveQtPaint.hpp"

#include <nve/host/controlled_text.hpp>
#include <nve/host/core_resources.hpp>
#include <nve/host/host_session.hpp>

#include <QPointF>

#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>

class QObject;

namespace nve::qt {

/// Resource ids a view last decoded (kept per `QtHost::resource_unchanged`), plus the image tint.
struct AppliedResources {
  uint32_t image_src = 0u;
  uint32_t image_tint = 0u;  // 0 = untinted
  uint32_t draw_list = 0u;
  uint32_t box_shadows = 0u;
};

struct QtHostConfig {
  uint32_t surface_id = 1u;
  float width = 640.0f;
  float height = 420.0f;
  float scale_factor = 1.0f;
};

class QtHost {
 public:
  using EventCallback = host::HostSession::EventSink;
  /// Custom text measurer (`text`, the node's resolved typography, max width: `INFINITY` = unbounded).
  using TextMeasurer = std::function<NveIntrinsicSize(std::string_view, const NveRuntimeTextStyle&, float)>;

  virtual ~QtHost();
  QtHost(const QtHost&) = delete;
  QtHost& operator=(const QtHost&) = delete;

  // --- App sources --------------------------------------------------------------------------
  bool attach_interface(NveGetInterfaceFn get_interface);
  bool load_dylib(std::string_view path, host::ModuleError* out_error = nullptr, std::string* out_message = nullptr);
  /// Hot-swaps the app `cdylib`, carrying `NveDevApi` state across the reload (views are kept).
  bool reload_dylib(std::string_view path, host::ModuleError* out_error = nullptr, std::string* out_message = nullptr);
  /// Thin-client mode against an `nve dev` server (`nve://`, `quic://`, `wt://`, `ws(s)://`).
  bool connect_paired(std::string_view uri, std::string_view token, std::string* out_error = nullptr);
  void disconnect_paired();
  [[nodiscard]] bool is_paired() const noexcept;
  /// Applies one `NVE2` frame directly (C++ apps without an `nve_get_interface` module).
  bool submit_frame(std::span<const uint8_t> nve2);

  // --- App state & dev loop -----------------------------------------------------------------
  /// App-state files (`state.nve`, written atomically by the runtime).
  bool save_state(std::string_view path, uint32_t schema_version);
  bool restore_state(std::string_view path, uint32_t schema_version);
  /// Deletes the state file (`--reset-state`); `true` when it is gone (also when it never existed).
  bool clear_state(std::string_view path);
  [[nodiscard]] std::string last_error() const { return session_->last_error(); }
  bool apply_props(std::span<const NveMapEntry> props);
  bool apply_hot_patch(std::span<const uint8_t> patch_nvp1);
  void set_event_callback(EventCallback callback);
  /// Replaces the `QFontMetrics` measurer (`{}` restores it); the next pump relayouts.
  void set_text_measurer(TextMeasurer measurer);

  /// Ticks the runtime and mirrors the scene; returns the views created or updated.
  size_t pump();

  // --- Input --------------------------------------------------------------------------------
  bool dispatch(const NveDirectEvent& ev);
  bool simulate_press(uint32_t node_id, float x = 0.0f, float y = 0.0f);
  bool dispatch_text_change(uint32_t node_id, std::string_view text);
  bool dispatch_submit(uint32_t node_id, std::string_view text);
  bool dispatch_focus(uint32_t node_id, bool focused);
  /// Moves the runtime's scroll offset of `node_id` to the native one, then dispatches Scroll.
  bool dispatch_scroll(uint32_t node_id, float offset_x, float offset_y, float content_width, float content_height);
  /// `NVE_EVENT_POINTER_*` at a node-local point.
  bool dispatch_pointer(uint32_t node_id, NveEventKind kind, float x, float y);
  /// Activation of a pressable or link node: Press when subscribed, else opens `href` natively.
  bool activate(uint32_t node_id, bool wants_press, std::string_view href, float x, float y);
  /// Scrolls the runtime offset of `node_id` by a delta (clamped to its content); the new offset,
  /// `nullopt` when the node is not a scroll container.
  std::optional<QPointF> scroll_by(uint32_t node_id, float dx, float dy);
  /// Sets the runtime offset of `node_id` (hit testing, clamping) without dispatching Scroll.
  std::optional<QPointF> set_scroll_offset(uint32_t node_id, float x, float y);

  // --- Surface ------------------------------------------------------------------------------
  void set_surface_size(float width, float height, float scale_factor = 0.0f);
  /// Recreates the surface under another id (the running app is dropped).
  void set_surface_id(uint32_t surface_id);

  // --- Introspection ------------------------------------------------------------------------
  [[nodiscard]] bool is_loaded() const noexcept { return session_->has_app(); }
  [[nodiscard]] uint32_t surface_id() const noexcept { return config_.surface_id; }
  [[nodiscard]] const QtHostConfig& config() const noexcept { return config_; }
  [[nodiscard]] host::HostSession& session() noexcept { return *session_; }
  [[nodiscard]] NveQtImageCache& images() noexcept { return images_; }

 protected:
  QtHost(QObject* ui, QtHostConfig config);

  /// Derived destructors call this first: stops wakes and drops every view while `this` is whole.
  void shutdown() noexcept;
  /// Drops every view (shutdown, surface recreation).
  virtual void clear_views() = 0;
  /// Mirrors `scene` (held for the call) through the host's `RetainedTree`; returns the views
  /// created or updated.
  virtual size_t sync_views(const host::RuntimeSceneGuard& scene) = 0;
  /// The tree's `resource_unchanged(scene, id, applied)` for the scene being synced.
  [[nodiscard]] virtual bool tree_resource_unchanged(const host::SceneReader& scene, uint32_t id,
                                                     uint32_t applied) const noexcept = 0;
  /// Runs after each sync with the scene released (may dispatch / pump again).
  virtual void after_sync() {}

  /// Payload of published resource `id` of `kind` (empty when absent); valid only in `sync_views`.
  [[nodiscard]] std::span<const uint8_t> scene_resource(uint32_t id, host::Key64 kind) const noexcept;
  [[nodiscard]] bool syncing() const noexcept { return syncing_; }
  /// In `sync_views`: whether a view may keep what it decoded from resource `applied` now that
  /// its node reads `id` (same id, incremental sync, bytes not redefined this frame).
  [[nodiscard]] bool resource_unchanged(uint32_t id, uint32_t applied) const noexcept;
  /// Resolves `node`'s content into `paint` (in `sync_views`): a `nve.canvas` draw list (redrawn
  /// on new bytes), a decoded (content-keyed cache, tinted) image, or leaf text; the content kind
  /// repainted, `nullopt` when unchanged.
  std::optional<NodeContent> resolve_content(NodePaint& paint, AppliedResources& applied, const host::NodeData& node,
                                             const canvas::CanvasTable& canvas);
  /// `node`'s box visuals; box shadows are re-decoded only when their resource changed (else
  /// taken from `previous`).
  [[nodiscard]] BoxVisual resolve_visual(const host::NodeData& node, AppliedResources& applied,
                                         const BoxVisual& previous) const;
  /// Mirrors the current scene without ticking.
  size_t sync();
  /// A controlled field of node `id` now shows `current`: the shared edit path
  /// (`host::dispatch_controlled_edit`). `refind(id)` returns the field after the dispatch
  /// (`ControlledAnswer<Field>`); an in-process app's transformed or rejected answer is shown
  /// through `write(field, value)`. May destroy the field (and this host).
  template <class Field, class Refind, class Write>
  void edit_text(host::ControlledText& text, std::string_view current, uint32_t id, Refind&& refind, Write&& write) {
    const auto alive = alive_;
    (void)host::dispatch_controlled_edit<Field>(
        text, current, [&](std::string_view sent) { return dispatch_text_change(id, sent); },
        [&]() -> host::ControlledAnswer<Field> {
          // Paired answers are asynchronous and arrive through `app_published`.
          if (!alive->load(std::memory_order_acquire) || is_paired()) return {};
          return refind(id);
        },
        std::forward<Write>(write));
  }

 private:
  void create_session();
  static void measure_trampoline(void* ctx, NveStrSlice text, const NveRuntimeTextStyle* style, float max_width,
                                 float* out_width, float* out_height);

  QObject* ui_;
  QtHostConfig config_;
  std::unique_ptr<host::HostSession> session_;
  EventCallback sink_;
  TextMeasurer measurer_;
  const host::RuntimeSceneGuard* scene_ = nullptr;  // set during `sync_views`
  bool syncing_ = false;
  NveQtImageCache images_{64u};
  std::shared_ptr<std::atomic<bool>> alive_ = std::make_shared<std::atomic<bool>>(true);
};

}  // namespace nve::qt
