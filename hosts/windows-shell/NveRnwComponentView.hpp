#pragma once

#include "NveWinTextMeasurer.hpp"
#include "nve_host_runtime.hpp"

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#if defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC
#include <winrt/Microsoft.ReactNative.Composition.Input.h>
#include <winrt/Microsoft.ReactNative.Composition.h>
#include <winrt/Microsoft.ReactNative.h>
#include <winrt/Microsoft.UI.Composition.h>
#include <winrt/Microsoft.UI.Content.h>
#include <winrt/Windows.Foundation.h>
#endif

namespace nve::win {

/// Retained visual node reconciled from `nve::RenderedNodeSnapshot` via
/// `nve::RetainedViewReconciler`. Shared between the standalone Win32/Direct2D host
/// (`NveWinHostWindow`) and the `react-native-windows` Fabric host (`NveRnwComponentHost`).
struct NveWinNodeVisual {
  uint32_t node_id{0};
  uint16_t kind{0};
  uint32_t view_tag{0};
  bool is_text_leaf{false};
  bool interactive{false};
  std::string custom_name;
  std::string text;
  nve::LayoutRect layout{};
  uint32_t background_rgba{0};
  uint32_t text_color_rgba{0};
  uint32_t border_color_rgba{0};
  float border_width{0.0f};
  float border_radius{0.0f};
  float opacity{1.0f};
  float font_size{16.0f};
  int32_t z_index{0};
  uint64_t event_mask{0};
  uint32_t parent_id{0};
  std::vector<uint32_t> children;
};

/// Flattened surface-space node entry produced in paint order (z-index + tree order).
struct FlattenedVisualNode {
  uint32_t node_id{0};
  float abs_x{0.0f};
  float abs_y{0.0f};
  float effective_opacity{1.0f};
};

/// Result of interactive hit-testing in surface DIP coordinates.
struct HitTestResult {
  uint32_t node_id{0};
  float local_x{0.0f};
  float local_y{0.0f};
};

/// Reconciled retained visual tree backed directly by `nve::RetainedViewReconciler`.
class NveWinVisualTree {
 public:
  void sync(const std::vector<nve::RenderedNodeSnapshot>& snapshots);
  void clear();

  NveWinNodeVisual* find(uint32_t node_id) noexcept { return reconciler_.find(node_id); }
  const NveWinNodeVisual* find(uint32_t node_id) const noexcept {
    return reconciler_.find(node_id);
  }

  size_t size() const noexcept { return reconciler_.size(); }
  bool empty() const noexcept { return reconciler_.empty(); }

  uint32_t root_background_rgba() const noexcept { return adapter_.root_background_rgba; }
  const nve::LayoutRect& root_layout() const noexcept { return adapter_.root_layout; }
  const std::vector<uint32_t>& root_children() const noexcept { return adapter_.root_children; }

  uint64_t created_count() const noexcept { return adapter_.created_count; }
  uint64_t updated_count() const noexcept { return adapter_.updated_count; }
  uint64_t destroyed_count() const noexcept { return adapter_.destroyed_count; }

  /// Compute a node's surface-space rectangle by accumulating parent-relative layout offsets.
  std::optional<nve::LayoutRect> absolute_rect(uint32_t node_id) const;

  /// Collect all live non-root nodes in paint order (children sorted stably by `z_index`).
  std::vector<FlattenedVisualNode> build_draw_order() const;

  /// Find the topmost interactive node (`NVE_COMP_PRESSABLE` or `kEventMaskPress`) in reverse
  /// z/tree order containing `(surface_x, surface_y)`.
  std::optional<HitTestResult> hit_test_interactive(float surface_x, float surface_y) const;

 private:
  struct Adapter {
    uint32_t root_background_rgba{0};
    nve::LayoutRect root_layout{};
    std::vector<uint32_t> root_children;
    uint64_t created_count{0};
    uint64_t updated_count{0};
    uint64_t destroyed_count{0};

    void update_root(const nve::RenderedNodeSnapshot& snap);
    NveWinNodeVisual create_view(const nve::RenderedNodeSnapshot& snap);
    void update_view(NveWinNodeVisual& view, const nve::RenderedNodeSnapshot& snap);
    void destroy_view(uint32_t node_id, NveWinNodeVisual& view);

    template <typename LookupFn>
    void sync_children(
        uint32_t parent_id,
        NveWinNodeVisual* parent_view,
        const std::vector<uint32_t>& child_ids,
        LookupFn&& lookup_fn) {
      if (parent_id == NVE_ROOT_NODE_ID) {
        root_children = child_ids;
      } else if (parent_view != nullptr) {
        parent_view->children = child_ids;
      }
      for (uint32_t cid : child_ids) {
        if (NveWinNodeVisual* child = lookup_fn(cid)) {
          child->parent_id = parent_id;
        }
      }
    }
  };

  void append_draw_order(
      uint32_t node_id,
      float origin_x,
      float origin_y,
      float inherited_opacity,
      std::vector<FlattenedVisualNode>& out,
      uint32_t depth) const;

  Adapter adapter_{};
  nve::RetainedViewReconciler<NveWinNodeVisual, Adapter> reconciler_{};
};

/// Portable C++20 component controller wrapping `nve::NativeHostSession`,
/// `DirectWriteTextMeasurer`, and `NveWinVisualTree` for both `react-native-windows` Fabric
/// component views and the standalone Win32/Direct2D host shell.
class NveRnwComponentHost {
 public:
  explicit NveRnwComponentHost(
      std::shared_ptr<DirectWriteTextMeasurer> text_measurer = nullptr);
  ~NveRnwComponentHost() = default;

  NveRnwComponentHost(const NveRnwComponentHost&) = delete;
  NveRnwComponentHost& operator=(const NveRnwComponentHost&) = delete;
  NveRnwComponentHost(NveRnwComponentHost&&) noexcept = default;
  NveRnwComponentHost& operator=(NveRnwComponentHost&&) noexcept = default;

  /// Attach an in-process `nve_get_interface` entry point and reconcile initial visuals.
  bool attach_interface(NveGetInterfaceFn get_interface, NveSurfaceConfig config);

  /// Open an NVE `.dll`/`.dylib`, mount the surface, and reconcile initial visuals.
  /// Error codes match `nve::load_nve_dylib`: `0` OK, `1` open/empty path failed,
  /// `2` missing `nve_get_interface`, `3` missing `NVE_IFACE_SURFACE` / init failure.
  bool load_dylib(
      std::string_view path,
      NveSurfaceConfig config,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr);

  /// Atomically hot-reload a replacement NVE `.dll`/`.dylib`, preserving Rust component state
  /// across the reload and keeping the current visual tree intact if the reload fails.
  bool reload_dylib(
      std::string_view path,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr);

  /// Zero-copy instant-boot Frame-0 replay from an `NVE_SNAP` file before any dylib loads.
  /// Reconciles `visual_tree()` immediately and caches state for the next `load_dylib()` /
  /// `attach_interface()`.
  bool replay_frame0_snapshot(
      std::string_view snapshot_path,
      uint32_t expected_schema_version,
      NveSurfaceConfig config,
      uint64_t* out_restore_us = nullptr);

  /// Captures a complete Frame-0 `CommandBuffer` + `snapshot_state()` and persists it atomically.
  bool save_snapshot(std::string_view snapshot_path, uint32_t schema_version) const;

  /// Tear down the active session and clear all retained visuals.
  void reset() noexcept;

  /// Register outbound component event callback (e.g. RNW `EventEmitter::DispatchEvent`).
  void set_event_callback(nve::EventCallback cb);

  /// Register notification invoked whenever the reconciled `NveWinVisualTree` changes.
  void set_on_visual_tree_changed(std::function<void()> cb);

  /// Update viewport dimensions (in DIPs) and scale factor (e.g. from RNW `LayoutMetrics`
  /// or `WM_SIZE` / `WM_DPICHANGED`), triggering re-layout and visual reconciliation.
  void set_surface_size(float width, float height, float scale_factor = 0.0f);

  /// Apply a UTF-8 JSON props payload via `NveComponentApi::set_props`, re-rendering and
  /// reconciling visuals if dirty, and draining outbound events.
  bool update_props_json(std::string_view props_json);

  /// Apply a Tier-1 hot-patch JSON payload (`NveDevApi::apply_hot_patch`).
  bool apply_hot_patch_json(std::string_view patch_json);

  /// Render a frame from the Rust surface, reconcile retained visuals, and drain outbound events.
  int32_t render_frame(NveFrameStats* out_stats = nullptr);

  /// Dispatch a press event directly to `node_id` at local DIP coordinates `(local_x, local_y)`.
  uint32_t dispatch_press(uint32_t node_id, float local_x, float local_y);

  /// Hit-test `(surface_x, surface_y)` in DIPs against the topmost interactive node and dispatch
  /// a press event if hit. Returns non-zero if the Rust component requested a re-render.
  uint32_t dispatch_click_at(float surface_x, float surface_y);

  /// Zero-serialization intrinsic size measurement via `NveLayoutApi::measure` (if exposed).
  std::optional<NveIntrinsicSize> measure(NveSizeConstraints constraints) const;

  /// Drain outbound events emitted by the Rust component.
  std::vector<nve::DecodedEvent> drain_events();

  bool is_valid() const noexcept { return session_.is_valid(); }
  bool has_replayed_frame0() const noexcept { return session_.has_replayed_frame0(); }
  uint32_t node_count() const noexcept { return session_.node_count(); }
  uint32_t surface_id() const noexcept { return session_.surface_config().surface_id; }
  const NveSurfaceConfig& surface_config() const noexcept { return session_.surface_config(); }

  nve::NativeHostSession& session() noexcept { return session_; }
  const nve::NativeHostSession& session() const noexcept { return session_; }

  NveWinVisualTree& visual_tree() noexcept { return visual_tree_; }
  const NveWinVisualTree& visual_tree() const noexcept { return visual_tree_; }

  const std::shared_ptr<DirectWriteTextMeasurer>& text_measurer() const noexcept {
    return text_measurer_;
  }

 private:
  void sync_visuals_and_notify();

  std::shared_ptr<DirectWriteTextMeasurer> text_measurer_;
  nve::NativeHostSession session_;
  NveWinVisualTree visual_tree_;
  std::function<void()> on_visual_tree_changed_;
};

#if defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

/// Configuration for registering an NVE Rust component inside a modern `react-native-windows`
/// 0.83+/0.88+ Composition Fabric application via `IReactPackageBuilderFabric::AddViewComponent`
/// and `IReactCompositionViewComponentBuilder`.
struct NveRnwComponentRegistrationConfig {
  /// React Native component name exposed to JS (e.g. `L"NveNativeView"`).
  std::wstring component_name{L"NveNativeView"};
  /// Optional statically linked NVE component interface (`nve_get_interface`).
  NveGetInterfaceFn default_interface{nullptr};
  /// Optional path to a Rust NVE `.dll` loaded automatically on mount.
  std::string default_dylib_path;
  /// When true, registers via `SetContentIslandComponentViewInitializer` so a WinUI 3 /
  /// Direct2D `Microsoft::UI::Content::ContentIsland` can be connected to the Fabric view.
  bool use_content_island{false};
  /// Optional hook to build and return a `ContentIsland` connected via
  /// `ContentIslandComponentView::Connect(...)`.
  std::function<winrt::Microsoft::UI::Content::ContentIsland(
      winrt::Microsoft::ReactNative::Composition::ContentIslandComponentView const&,
      std::shared_ptr<NveRnwComponentHost>)>
      create_content_island;
  /// Optional hook invoked once a `ViewComponentView` has initialized its `NveRnwComponentHost`.
  std::function<void(
      winrt::Microsoft::ReactNative::Composition::ViewComponentView const&,
      std::shared_ptr<NveRnwComponentHost>)>
      on_view_initialized;
};

/// Retrieve the `NveRnwComponentHost` bound to a live RNW Fabric `ComponentView`.
std::shared_ptr<NveRnwComponentHost> get_nve_host_from_component_view(
    winrt::Microsoft::ReactNative::ComponentView const& view);

/// Register an NVE Rust component with `react-native-windows` Composition Fabric
/// (`IReactPackageBuilderFabric` + `IReactCompositionViewComponentBuilder`).
bool register_nve_rnw_component(
    winrt::Microsoft::ReactNative::IReactPackageBuilder const& package_builder,
    NveRnwComponentRegistrationConfig config = {});

#endif // defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

} // namespace nve::win
