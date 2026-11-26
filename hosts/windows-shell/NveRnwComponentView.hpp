#pragma once

#include "NveWinTextMeasurer.hpp"
#include "nve_host_runtime.hpp"

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
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

/// Image content-fit mode matching React Native's `ImageResizeMode`.
enum class ImageResizeMode : uint8_t {
  Cover = 0,
  Contain,
  Stretch,
  Center,
};

/// Parse `ImageResizeMode` from UTF-8 string (`cover`, `contain`, `stretch`, `center`).
ImageResizeMode parse_image_resize_mode(std::string_view raw) noexcept;

/// Retained visual node reconciled in O(changed) time from `NveSceneView` via
/// `nve::IncrementalSceneReconciler`. Shared between the standalone Win32/Direct2D host
/// (`NveWinHostWindow`) and the `react-native-windows` Fabric host (`NveRnwComponentHost`).
struct NveWinNodeVisual {
  uint32_t node_id{0};
  uint16_t kind{0};
  uint32_t view_tag{0};
  bool is_text_leaf{false};
  bool interactive{false};
  bool clip_bounds{false};
  std::string text;
  std::string image_src;
  std::vector<uint8_t> draw_list;
  ImageResizeMode resize_mode{ImageResizeMode::Cover};
  nve::LayoutRect layout{};
  uint32_t background_rgba{0};
  uint32_t text_color_rgba{0};
  uint32_t border_color_rgba{0};
  float border_width{0.0f};
  float border_radius{0.0f};
  float opacity{1.0f};
  float font_size{16.0f};
  int32_t z_index{0};
  float scroll_offset_x{0.0f};
  float scroll_offset_y{0.0f};
  float content_width{0.0f};
  float content_height{0.0f};
  uint32_t parent_id{0};
  std::vector<uint32_t> children;
};

/// Flattened surface-space node entry produced in paint order (z-index + tree order).
struct FlattenedVisualNode {
  uint32_t node_id{0};
  float abs_x{0.0f};
  float abs_y{0.0f};
  float effective_opacity{1.0f};
  bool has_clip{false};
  nve::LayoutRect clip_rect{};
};

/// Result of interactive hit-testing in surface DIP coordinates.
struct HitTestResult {
  uint32_t node_id{0};
  float local_x{0.0f};
  float local_y{0.0f};
};

/// Reconciled retained visual tree backed directly by `nve::IncrementalSceneReconciler`.
class NveWinVisualTree {
 public:
  using RawPropByNode = std::unordered_map<uint32_t, nve::RawPropMap>;

  /// Incrementally reconcile retained visuals directly from a zero-copy `NveSceneView`.
  bool sync(const NveSceneView& view, const RawPropByNode* raw_props = nullptr);
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

  /// Update and clamp scroll offset for a `NVE_NODE_KIND_SCROLL_VIEW` node.
  bool set_scroll_offset(uint32_t node_id, float offset_x, float offset_y);

  /// Query current scroll offset `(x, y)` for a scroll node.
  std::optional<std::pair<float, float>> scroll_offset(uint32_t node_id) const noexcept;

  /// Compute a node's surface-space rectangle (incorporating parent offsets and ancestor scrolls).
  std::optional<nve::LayoutRect> absolute_rect(uint32_t node_id) const;

  /// Collect all live non-root nodes in paint order (children sorted stably by `z_index`).
  std::vector<FlattenedVisualNode> build_draw_order() const;

  /// Find the topmost interactive node in reverse z/tree order containing `(surface_x, surface_y)`.
  std::optional<HitTestResult> hit_test_interactive(float surface_x, float surface_y) const;

  /// Find the deepest `NVE_NODE_KIND_SCROLL_VIEW` enclosing `(surface_x, surface_y)`.
  std::optional<uint32_t> hit_test_scrollable(float surface_x, float surface_y) const;

 private:
  struct Adapter {
    uint32_t root_background_rgba{0};
    nve::LayoutRect root_layout{};
    std::vector<uint32_t> root_children;
    const RawPropByNode* raw_props{nullptr};
    uint64_t created_count{0};
    uint64_t updated_count{0};
    uint64_t destroyed_count{0};

    void update_root(const NveSceneView& view, const NveRenderNode& node);
    NveWinNodeVisual create_view(const NveSceneView& view, const NveRenderNode& node);
    void update_view(const NveSceneView& view, NveWinNodeVisual& visual, const NveRenderNode& node);
    void destroy_view(uint32_t node_id, NveWinNodeVisual& visual);

    template <typename LookupFn>
    void sync_children(
        uint32_t parent_id,
        NveWinNodeVisual* parent_view,
        std::span<const uint32_t> child_ids,
        LookupFn&& lookup_fn) {
      if (parent_id == NVE_ROOT_NODE_ID) {
        root_children.assign(child_ids.begin(), child_ids.end());
      } else if (parent_view != nullptr) {
        parent_view->children.assign(child_ids.begin(), child_ids.end());
      }

      float max_w = (parent_view != nullptr) ? parent_view->layout.width : 0.0f;
      float max_h = (parent_view != nullptr) ? parent_view->layout.height : 0.0f;
      for (uint32_t cid : child_ids) {
        if (NveWinNodeVisual* child = lookup_fn(cid)) {
          child->parent_id = parent_id;
          if (parent_view != nullptr && parent_view->kind == NVE_NODE_KIND_SCROLL_VIEW) {
            max_w = std::max(max_w, child->layout.x + child->layout.width);
            max_h = std::max(max_h, child->layout.y + child->layout.height);
          }
        }
      }

      if (parent_view != nullptr && parent_view->kind == NVE_NODE_KIND_SCROLL_VIEW) {
        parent_view->content_width = max_w;
        parent_view->content_height = max_h;
        const float max_sx = std::max(0.0f, max_w - parent_view->layout.width);
        const float max_sy = std::max(0.0f, max_h - parent_view->layout.height);
        parent_view->scroll_offset_x = std::clamp(parent_view->scroll_offset_x, 0.0f, max_sx);
        parent_view->scroll_offset_y = std::clamp(parent_view->scroll_offset_y, 0.0f, max_sy);
      }
    }
  };

  void append_draw_order(
      uint32_t node_id,
      float origin_x,
      float origin_y,
      float inherited_opacity,
      bool has_clip,
      nve::LayoutRect clip_rect,
      std::vector<FlattenedVisualNode>& out,
      uint32_t depth) const;

  Adapter adapter_{};
  nve::IncrementalSceneReconciler<NveWinNodeVisual, Adapter> reconciler_{};
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

  bool attach_interface(NveGetInterfaceFn get_interface, NveSurfaceConfig config);

  bool load_dylib(
      std::string_view path,
      NveSurfaceConfig config,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr);

  bool reload_dylib(
      std::string_view path,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr);

  /// Connect in zero-cdylib paired thin-client mode using a parsed `PairClientConfig`.
  bool connect_paired(
      const nve::PairClientConfig& pair_config,
      NveSurfaceConfig surface_config,
      nve::PairClient::WakeCallback on_frame_ready = {},
      std::string* out_error = nullptr);

  /// Connect in zero-cdylib paired thin-client mode from `--pair-connect` endpoint/token/transport.
  bool connect_paired(
      std::string_view endpoint,
      std::string_view token,
      std::string_view transport_override,
      NveSurfaceConfig surface_config,
      nve::PairClient::WakeCallback on_frame_ready = {},
      std::string* out_error = nullptr);

  /// Drain queued paired `NVD1` frames, apply them into `FabricHost`, and sync the visual tree.
  size_t poll_paired_frames(NveFrameStats* out_last_stats = nullptr);

  /// Disconnect any active paired thin-client session and clear the visual tree.
  void disconnect_paired() noexcept;

  bool replay_frame0_snapshot(
      std::string_view snapshot_path,
      uint32_t expected_schema_version,
      NveSurfaceConfig config,
      uint64_t* out_restore_us = nullptr);

  bool save_snapshot(std::string_view snapshot_path, uint32_t schema_version) const;
  void reset() noexcept;

  void set_event_callback(nve::EventCallback cb);
  void set_on_visual_tree_changed(std::function<void()> cb);
  void set_surface_size(float width, float height, float scale_factor = 0.0f);

  bool update_props(const nve::PropMap& props);
  bool update_props(const NveMapEntry* entries, uint32_t count);
  bool apply_hot_patch(std::span<const uint8_t> patch_nvp1);
  int32_t render_frame(NveFrameStats* out_stats = nullptr);

  uint32_t dispatch_press(uint32_t node_id, float local_x, float local_y);
  uint32_t dispatch_click_at(float surface_x, float surface_y);

  /// Dispatch a zero-copy `EventKind::Scroll` event directly into `NativeHostSession`.
  uint32_t dispatch_scroll(
      uint32_t node_id,
      float offset_x,
      float offset_y,
      float content_width,
      float content_height);

  /// Hit-test `(surface_x, surface_y)` for the deepest `ScrollView`, apply clamped `(delta_x, delta_y)`
  /// scroll offset, notify listeners, and dispatch zero-copy `EventKind::Scroll`.
  uint32_t dispatch_wheel_at(float surface_x, float surface_y, float delta_x, float delta_y);

  std::optional<NveIntrinsicSize> measure(NveSizeConstraints constraints) const;
  std::vector<nve::DecodedEvent> drain_events();

  bool is_valid() const noexcept { return session_.is_valid(); }
  bool is_paired() const noexcept { return session_.is_paired(); }
  bool is_paired_connected() const noexcept { return session_.is_paired_connected(); }
  uint64_t paired_frame_count() const noexcept { return session_.paired_frame_count(); }
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
  void sync_visuals_and_notify(bool check_raw_props = false);

  std::shared_ptr<DirectWriteTextMeasurer> text_measurer_;
  nve::NativeHostSession session_;
  NveWinVisualTree visual_tree_;
  std::function<void()> on_visual_tree_changed_;
  bool raw_props_initialized_{false};
};

#if defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

struct NveRnwComponentRegistrationConfig {
  std::wstring component_name{L"NveNativeView"};
  NveGetInterfaceFn default_interface{nullptr};
  std::string default_dylib_path;
  bool use_content_island{false};
  std::function<winrt::Microsoft::UI::Content::ContentIsland(
      winrt::Microsoft::ReactNative::Composition::ContentIslandComponentView const&,
      std::shared_ptr<NveRnwComponentHost>)>
      create_content_island;
  std::function<void(
      winrt::Microsoft::ReactNative::Composition::ViewComponentView const&,
      std::shared_ptr<NveRnwComponentHost>)>
      on_view_initialized;
};

std::shared_ptr<NveRnwComponentHost> get_nve_host_from_component_view(
    winrt::Microsoft::ReactNative::ComponentView const& view);

bool register_nve_rnw_component(
    winrt::Microsoft::ReactNative::IReactPackageBuilder const& package_builder,
    NveRnwComponentRegistrationConfig config = {});

#endif // defined(_WIN32) && defined(NVE_WITH_RNW_FABRIC) && NVE_WITH_RNW_FABRIC

} // namespace nve::win
