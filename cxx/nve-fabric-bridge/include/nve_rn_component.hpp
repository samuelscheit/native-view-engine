#pragma once

#include "nve_abi.generated.h"
#include "nve_scene_abi.generated.h"
#include "nve_value.hpp"

#include <cstddef>
#include <cstdint>
#include <functional>
#include <map>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <variant>
#include <vector>

namespace nve {

/// Computed layout rectangle (in logical points, parent-relative) for a shadow node.
struct LayoutRect {
  float x{0.0f};
  float y{0.0f};
  float width{0.0f};
  float height{0.0f};
};

/// Decoded native event kind matching `crates/nve-abi/src/events.rs` (`NativeEventKind`).
enum class EventKind : uint8_t {
  Press = 1,
  LongPress = 2,
  Scroll = 3,
  TextChange = 4,
  Focus = 5,
  Blur = 6,
  Layout = 7,
  Custom = 8,
};

/// Decoded event from an `EventRing` (`nve_abi::encode_events`).
struct DecodedEvent {
  uint32_t surface_id{0};
  uint32_t node_id{0};
  uint64_t timestamp_us{0};
  EventKind kind{EventKind::Custom};
  /// For EventKind::Custom this is the custom event name (e.g. "onIncrement");
  /// for built-in kinds it is the canonical RN name ("onPress", "onLongPress",
  /// "onScroll", "onChangeText", "onFocus", "onBlur", "onLayout").
  std::string name;
  /// Raw payload bytes (compact binary `NveBinValue` for custom events, UTF-8 text for TextChange).
  std::vector<uint8_t> payload_bytes;
  /// Structured fields populated for built-in event payload variants.
  float x{0.0f};
  float y{0.0f};
  float width{0.0f};
  float height{0.0f};
  std::string text;

  std::string_view payload_as_string() const noexcept {
    if (payload_bytes.empty()) {
      return {};
    }
    return std::string_view(
        reinterpret_cast<const char*>(payload_bytes.data()), payload_bytes.size());
  }
};

/// Typed prop value for structured `NveRnComponentBridge::update_props(PropMap)`.
using PropValue = std::variant<std::nullptr_t, bool, int64_t, double, std::string>;
using PropMap = std::map<std::string, PropValue>;

/// Raw prop value decoded from `NVE_OP_SET_RAW_PROP` (matches `nve_abi::RawPropValue`).
using RawPropValue =
    std::variant<std::nullptr_t, bool, int64_t, double, std::string, std::vector<uint8_t>>;
using RawPropMap = std::unordered_map<std::string, RawPropValue>;

/// Callback invoked for each outbound event drained from `NveComponentApi::drain_emitted_events`.
using EventEmitterCallback = std::function<void(const DecodedEvent&)>;

/// Descriptor registered by `import_rn`-generated C++20 adapters for custom 3rd-party RN components.
struct CustomComponentDescriptor {
  std::string component_name;
  std::function<void(uint32_t node_id, const RawPropMap& props)> on_update_props;
  std::function<void(uint32_t node_id, std::string_view command_name, NveBufferSlice payload)>
      on_dispatch_command;
};

/// Snapshot of a rendered shadow node (including computed layout) for UIKit/Android view mounting.
struct RenderedNodeSnapshot {
  uint32_t node_id{0};
  uint16_t kind{0};
  std::string custom_name;
  std::string text;
  LayoutRect layout{};
  uint32_t background_rgba{0};
  uint32_t text_color_rgba{0};
  uint32_t border_color_rgba{0};
  float border_width{0.0f};
  float border_radius{0.0f};
  float opacity{1.0f};
  float font_size{16.0f};
  int32_t z_index{0};
  uint64_t event_mask{0};
  std::vector<uint32_t> children;
  RawPropMap raw_props;
};

/// Decode an `EventRing` byte slice (`nve_abi::encode_events`) into `DecodedEvent` records.
bool decode_event_ring(NveBufferSlice slice, std::vector<DecodedEvent>& out_events);

/// Host inspection & control helpers implemented by `nve_fabric_bridge`.
bool fabric_host_get_node_layout(NveFabricHostHandle host, uint32_t node_id, LayoutRect* out_rect);
bool fabric_host_has_yoga_layout() noexcept;
bool fabric_host_set_surface_size(
    NveFabricHostHandle host,
    float width,
    float height,
    float scale_factor);
void fabric_host_enqueue_press(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    float x,
    float y);
void fabric_host_enqueue_text_change(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    std::string_view text);
void fabric_host_enqueue_custom_event(
    NveFabricHostHandle host,
    uint32_t node_id,
    uint64_t timestamp_us,
    std::string_view name,
    const uint8_t* data,
    size_t len);
/// Platform text measurement callback invoked by Yoga (`measure_text_node`) when registered.
/// Given UTF-8 text, font size in points, and max width (<= 0 or non-finite means unconstrained),
/// returns exact native glyph bounds `(width, height)` in points.
using TextMeasurerCallback =
    std::function<NveIntrinsicSize(std::string_view text, float font_size, float max_width)>;

void fabric_host_set_text_measurer(
    NveFabricHostHandle host,
    TextMeasurerCallback measurer);
void fabric_host_register_custom_component(
    NveFabricHostHandle host,
    CustomComponentDescriptor descriptor);
std::vector<RenderedNodeSnapshot> fabric_host_snapshot_nodes(NveFabricHostHandle host);

/// Return a zero-copy, incrementally updated 64-byte `NveSceneView` pointing directly into
/// `host`'s internal node/text/children/dirty/removed slabs (zero snapshot materialization).
NveSceneView fabric_host_scene_view(NveFabricHostHandle host) noexcept;

/// Freeze-dry the live shadow tree inside `host` into a complete, self-contained Frame-0
/// `CommandBuffer` suitable for zero-copy instant-boot `MmapSnapshot` persistence.
std::vector<uint8_t> fabric_host_serialize_frame0(
    NveFabricHostHandle host,
    uint64_t frame_id = 1u);

/// Embeds any NVE Rust component (`nve_get_interface` / `NveModule`) inside a React Native Fabric
/// component (iOS/macOS `RCTViewComponentView`, Android `ViewGroupManager`, or headless C++ host).
///
/// Bridges:
/// - RN props (`update_props` / `update_props_json`) -> `NveComponentApi::set_props`
/// - Render pass (`render`) -> `NveSurfaceApi::render` -> `nve_fabric_host_submit_frame`
/// - Intrinsic sizing & hit testing -> `NveLayoutApi::measure` / `hit_test` (zero-serialization)
/// - Outbound events (`drain_emitted_events`) -> `NveComponentApi::drain_emitted_events` -> callback
class NveRnComponentBridge {
 public:
  NveRnComponentBridge(
      NveModule module,
      NveSurfaceConfig config,
      EventEmitterCallback on_event = {});
  NveRnComponentBridge(
      NveGetInterfaceFn get_interface,
      NveSurfaceConfig config,
      EventEmitterCallback on_event = {});
  ~NveRnComponentBridge();

  NveRnComponentBridge(const NveRnComponentBridge&) = delete;
  NveRnComponentBridge& operator=(const NveRnComponentBridge&) = delete;
  NveRnComponentBridge(NveRnComponentBridge&& other) noexcept;
  NveRnComponentBridge& operator=(NveRnComponentBridge&& other) noexcept;

  /// Replace or attach the outbound event callback (forwarding to RN EventEmitter).
  void set_event_callback(EventEmitterCallback callback);

  /// Update viewport dimensions (e.g. from Fabric `updateLayoutMetrics:`).
  void set_surface_size(float width, float height, float scale_factor = 0.0f);

  /// Push a raw encoded prop payload to `NveComponentApi::set_props`.
  /// Returns true if the Rust component signaled a re-render is needed (auto-renders when `auto_render` is true).
  bool update_props_json(std::string_view props_json, bool auto_render = true);

  /// Push a structured key/value prop map directly via `NveComponentApi::set_props_direct`
  /// using an internal zero-allocation `BumpArena` (falling back to binary `NveBinValue` via
  /// `set_props` if `set_props_direct` is NULL).
  bool update_props(const PropMap& props, bool auto_render = true);

  /// Zero-copy direct props update via `NveComponentApi::set_props_direct` (falls back to
  /// binary `NveBinValue` via `set_props` when `set_props_direct` is NULL).
  bool update_props_direct(const NveMapEntry* entries, uint32_t count, bool auto_render = true);

  /// Invoke `NveSurfaceApi::render(app, next_frame_id)` and submit the binary CommandBuffer to the
  /// internal `NveFabricHostHandle`. Returns 0 on success or a negative error code.
  int32_t render(NveFrameStats* out_stats = nullptr);

  /// Dispatch an inbound binary EventRing (e.g. native touch/press) into `NveSurfaceApi::dispatch_events`,
  /// re-rendering if the Rust component requests it and `auto_render` is true.
  uint32_t dispatch_native_events(NveBufferSlice events, bool auto_render = true);

  /// Zero-copy direct event batch dispatch via `NveSurfaceApi::dispatch_events_direct`
  /// (falls back to binary `EventRing` via `dispatch_events` if `dispatch_events_direct` is NULL).
  uint32_t dispatch_direct_events(NveEventBatch batch, bool auto_render = true);

  /// Query whether the attached module exposes zero-copy direct event / prop FFI entry points.
  bool has_direct_events() const noexcept {
    return module_.surface != nullptr && module_.surface->dispatch_events_direct != nullptr;
  }
  bool has_direct_props() const noexcept {
    return module_.component != nullptr && module_.component->set_props_direct != nullptr;
  }

  /// Zero-cost scalar layout measurement via `NveLayoutApi::measure` (if provided).
  std::optional<NveIntrinsicSize> measure(NveSizeConstraints constraints) const;

  /// Zero-cost scalar gesture hit-test via `NveLayoutApi::hit_test` (defaults to PassThrough).
  NveGestureDisposition hit_test(float x, float y, uint8_t pointer_phase = 0) const;

  /// Drain outbound events emitted by the Rust component via `NveComponentApi::drain_emitted_events`,
  /// decode them, invoke the registered `EventEmitterCallback` for each, and return them.
  std::vector<DecodedEvent> drain_emitted_events();

  /// Apply a Tier-1 hot-patch JSON payload (`NveDevApi::apply_hot_patch`).
  bool apply_hot_patch_json(std::string_view patch_json, bool auto_render = true);

  /// Snapshot & restore component state across Tier-2 dylib hot reloads (`NveDevApi`).
  std::vector<uint8_t> snapshot_state() const;
  void restore_state(NveBufferSlice state, bool auto_render = true);

  /// Accessors for host inspection & layout queries.
  NveFabricHostHandle fabric_host() const noexcept { return fabric_host_; }
  NveAppHandle app_handle() const noexcept { return app_handle_; }
  const NveSurfaceConfig& surface_config() const noexcept { return config_; }
  uint64_t frame_count() const noexcept { return next_frame_id_ > 0 ? next_frame_id_ - 1 : 0; }
  uint32_t node_count() const noexcept;
  std::optional<LayoutRect> node_layout(uint32_t node_id) const;
  bool is_valid() const noexcept { return fabric_host_ != nullptr && app_handle_ != nullptr; }

 private:
  void destroy_handles() noexcept;

  NveModule module_{};
  NveSurfaceConfig config_{};
  NveFabricHostHandle fabric_host_{nullptr};
  NveAppHandle app_handle_{nullptr};
  uint64_t next_frame_id_{1};
  EventEmitterCallback on_event_;
  BumpArena prop_arena_{8192u};
  std::vector<uint8_t> fallback_props_buf_;
};

} // namespace nve
