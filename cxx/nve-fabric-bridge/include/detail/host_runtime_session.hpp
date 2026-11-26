#pragma once

#include "detail/host_runtime_dylib.hpp"
#include "nve_pair_client.hpp"
#include "nve_snapshot.hpp"

#include <cstdint>
#include <memory>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace nve {

/// Shared C++20 native host session owning a loaded dynamic library (optional),
/// `NveRnComponentBridge`, platform text measurer, outbound event callback, and surface config.
class NativeHostSession {
 public:
  NativeHostSession() = default;
  ~NativeHostSession() { reset(); }

  NativeHostSession(const NativeHostSession&) = delete;
  NativeHostSession& operator=(const NativeHostSession&) = delete;

  NativeHostSession(NativeHostSession&& other) noexcept
      : lib_(std::move(other.lib_)),
        bridge_(std::move(other.bridge_)),
        pair_client_(std::move(other.pair_client_)),
        paired_host_(other.paired_host_),
        paired_frame_count_(other.paired_frame_count_),
        frame0_host_(other.frame0_host_),
        pending_snapshot_state_(std::move(other.pending_snapshot_state_)),
        text_measurer_(std::move(other.text_measurer_)),
        event_callback_(std::move(other.event_callback_)),
        config_(other.config_) {
    other.paired_host_ = nullptr;
    other.paired_frame_count_ = 0u;
    other.frame0_host_ = nullptr;
  }

  NativeHostSession& operator=(NativeHostSession&& other) noexcept {
    if (this != &other) {
      // Destroy bridge & pair client before unloading our library, then transfer ownership.
      reset();
      lib_ = std::move(other.lib_);
      bridge_ = std::move(other.bridge_);
      pair_client_ = std::move(other.pair_client_);
      paired_host_ = other.paired_host_;
      paired_frame_count_ = other.paired_frame_count_;
      other.paired_host_ = nullptr;
      other.paired_frame_count_ = 0u;
      frame0_host_ = other.frame0_host_;
      other.frame0_host_ = nullptr;
      pending_snapshot_state_ = std::move(other.pending_snapshot_state_);
      text_measurer_ = std::move(other.text_measurer_);
      event_callback_ = std::move(other.event_callback_);
      config_ = other.config_;
    }
    return *this;
  }

  void reset() noexcept {
    disconnect_paired();
    bridge_.reset();
    destroy_frame0_host();
    pending_snapshot_state_.clear();
    lib_.close();
  }

  /// Connect in paired thin-client mode (zero local cdylib required).
  bool connect_paired(
      const PairClientConfig& pair_config,
      NveSurfaceConfig surface_config,
      PairClient::WakeCallback on_frame_ready = {},
      std::string* out_error = nullptr) {
    if (surface_config.surface_id == 0u) surface_config.surface_id = 1u;
    if (surface_config.width <= 0.0f) surface_config.width = 640.0f;
    if (surface_config.height <= 0.0f) surface_config.height = 420.0f;
    if (surface_config.scale_factor <= 0.0f) surface_config.scale_factor = 1.0f;

    NveFabricHostHandle next_host = nve_fabric_host_create(surface_config);
    if (next_host == nullptr) {
      if (out_error != nullptr) {
        *out_error = "Failed to create FabricHost for paired thin-client session";
      }
      return false;
    }
    if (text_measurer_) {
      fabric_host_set_text_measurer(next_host, text_measurer_);
    }

    auto next_client = std::make_unique<PairClient>();
    if (!next_client->connect(pair_config, std::move(on_frame_ready), out_error)) {
      nve_fabric_host_destroy(next_host);
      return false;
    }

    // Tear down any previous local or paired session before committing atomically.
    disconnect_paired();
    bridge_.reset();
    destroy_frame0_host();
    pending_snapshot_state_.clear();
    lib_.close();

    paired_host_ = next_host;
    pair_client_ = std::move(next_client);
    paired_frame_count_ = 0u;
    config_ = surface_config;

    (void)poll_paired_frames(nullptr);
    return true;
  }

  /// Convenience overload using the session's current/default `NveSurfaceConfig`.
  bool connect_paired(
      const PairClientConfig& pair_config,
      PairClient::WakeCallback on_frame_ready = {},
      std::string* out_error = nullptr) {
    return connect_paired(pair_config, effective_config(), std::move(on_frame_ready), out_error);
  }

  /// Disconnect any active paired thin-client session and destroy its `FabricHost`.
  void disconnect_paired() noexcept {
    if (pair_client_ != nullptr) {
      pair_client_->disconnect();
      pair_client_.reset();
    }
    destroy_paired_host();
    paired_frame_count_ = 0u;
  }

  /// Drain all reconstructed `NVE1` frames queued by `PairClient` in strict sequence order and
  /// submit them into `paired_host_`. Returns the number of frames successfully applied.
  size_t poll_paired_frames(NveFrameStats* out_last_stats = nullptr) {
    if (!is_paired()) {
      return 0u;
    }
    std::vector<std::pair<uint32_t, std::vector<uint8_t>>> frames;
    if (pair_client_->drain_pending_frames(frames) == 0u) {
      return 0u;
    }
    size_t applied = 0u;
    NveFrameStats last_stats{};
    for (const auto& [seq, cmd_buf] : frames) {
      (void)seq;
      if (cmd_buf.empty()) {
        continue;
      }
      NveFrameStats stats{};
      if (nve_fabric_host_submit_frame(
              paired_host_, NveBufferSlice{cmd_buf.data(), cmd_buf.size()}, &stats) == 0) {
        ++applied;
        ++paired_frame_count_;
        last_stats = stats;
        const NveBufferSlice ev = nve_fabric_host_drain_events(paired_host_);
        if (ev.ptr != nullptr && ev.len > 4u) {
          (void)pair_client_->send_events(std::span<const uint8_t>(ev.ptr, ev.len));
        }
      }
    }
    if (applied > 0u && out_last_stats != nullptr) {
      *out_last_stats = last_stats;
    }
    return applied;
  }

  /// Stream a raw binary `EventRing` batch upstream to the paired host (`InputEvents = 11`).
  bool send_paired_event_ring(NveBufferSlice events, std::string* out_error = nullptr) {
    if (!is_paired() || events.ptr == nullptr || events.len < 4u) {
      return false;
    }
    return pair_client_->send_events(std::span<const uint8_t>(events.ptr, events.len), out_error);
  }

  /// Attach an in-process `nve_get_interface` function pointer directly (no dylib owned).
  bool attach_interface(NveGetInterfaceFn get_interface, NveSurfaceConfig config) {
    const NveModule mod = nve_resolve_module(get_interface);
    if (mod.surface == nullptr || mod.surface->create == nullptr) {
      return false;
    }
    return install_module(mod, DynamicLibrary{}, config, pending_snapshot_state_);
  }

  /// Open an NVE `cdylib`, mount the surface, and perform an initial render.
  bool load_dylib(
      std::string_view path,
      NveSurfaceConfig config,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr) {
    LoadedModule loaded = load_nve_dylib(path);
    if (!loaded.ok()) {
      if (out_err_code != nullptr) *out_err_code = loaded.error_code;
      if (out_err_msg != nullptr) *out_err_msg = std::move(loaded.error_message);
      return false;
    }
    if (!install_module(loaded.module, std::move(loaded.lib), config, pending_snapshot_state_)) {
      if (out_err_code != nullptr) *out_err_code = 3;
      if (out_err_msg != nullptr) *out_err_msg = "Failed to initialize NVE surface instance";
      return false;
    }
    if (out_err_code != nullptr) *out_err_code = 0;
    if (out_err_msg != nullptr) out_err_msg->clear();
    return true;
  }

  /// Atomically hot-reload a new NVE `cdylib`, preserving state via `snapshot_state`/`restore_state`.
  bool reload_dylib(
      std::string_view path,
      int* out_err_code = nullptr,
      std::string* out_err_msg = nullptr) {
    LoadedModule loaded = load_nve_dylib(path);
    if (!loaded.ok()) {
      if (out_err_code != nullptr) *out_err_code = loaded.error_code;
      if (out_err_msg != nullptr) *out_err_msg = std::move(loaded.error_message);
      return false;
    }

    std::vector<uint8_t> saved_state;
    if (is_valid()) {
      saved_state = bridge_->snapshot_state();
    } else if (!pending_snapshot_state_.empty()) {
      saved_state = pending_snapshot_state_;
    }

    if (!install_module(loaded.module, std::move(loaded.lib), config_, saved_state)) {
      if (out_err_code != nullptr) *out_err_code = 3;
      if (out_err_msg != nullptr) {
        *out_err_msg = "Failed to initialize replacement NVE surface instance";
      }
      return false;
    }

    if (out_err_code != nullptr) *out_err_code = 0;
    if (out_err_msg != nullptr) out_err_msg->clear();
    return true;
  }

  /// Zero-copy instant-boot Frame-0 replay from an `NVERELOC` file on disk.
  bool replay_frame0_snapshot(
      std::string_view snapshot_path,
      uint32_t expected_schema_version,
      NveFrameStats* out_stats = nullptr,
      SnapshotStatus* out_status = nullptr) {
    MmapSnapshot snap;
    if (!MmapSnapshot::open_with_schema(
            snapshot_path, expected_schema_version, snap, /*verify_checksum=*/true, out_status)) {
      return false;
    }

    const NveBufferSlice frame0 = snap.frame0_slice();
    if (frame0.ptr == nullptr || frame0.len == 0u) {
      if (out_status != nullptr) *out_status = SnapshotStatus::Truncated;
      return false;
    }

    const NveBufferSlice state = snap.state_slice();
    if (is_valid()) {
      if (nve_fabric_host_submit_frame(bridge_->fabric_host(), frame0, out_stats) != 0) {
        if (out_status != nullptr) *out_status = SnapshotStatus::IoError;
        return false;
      }
      if (state.ptr != nullptr && state.len > 0u) {
        bridge_->restore_state(state, /*auto_render=*/false);
      }
      return true;
    }

    NveSurfaceConfig cfg = effective_config();
    NveFabricHostHandle next_host = nve_fabric_host_create(cfg);
    if (next_host == nullptr) {
      if (out_status != nullptr) *out_status = SnapshotStatus::IoError;
      return false;
    }
    if (text_measurer_) {
      fabric_host_set_text_measurer(next_host, text_measurer_);
    }

    if (nve_fabric_host_submit_frame(next_host, frame0, out_stats) != 0) {
      nve_fabric_host_destroy(next_host);
      if (out_status != nullptr) *out_status = SnapshotStatus::IoError;
      return false;
    }

    destroy_frame0_host();
    frame0_host_ = next_host;
    config_ = cfg;
    if (state.ptr != nullptr && state.len > 0u) {
      pending_snapshot_state_.assign(state.ptr, state.ptr + state.len);
    } else {
      pending_snapshot_state_.clear();
    }
    return true;
  }

  /// Captures a complete Frame-0 `CommandBuffer` + `snapshot_state()` and persists atomically.
  bool save_snapshot(
      std::string_view snapshot_path,
      uint32_t schema_version,
      std::string_view meta = {},
      SnapshotStatus* out_status = nullptr) const {
    NveFabricHostHandle host = active_fabric_host();
    if (host == nullptr) {
      if (out_status != nullptr) *out_status = SnapshotStatus::IoError;
      return false;
    }

    const uint64_t frame_id =
        (is_valid() && bridge_->frame_count() > 0u) ? bridge_->frame_count() : 1u;
    const std::vector<uint8_t> frame0 = fabric_host_serialize_frame0(host, frame_id);
    if (frame0.empty()) {
      if (out_status != nullptr) *out_status = SnapshotStatus::IoError;
      return false;
    }

    std::vector<uint8_t> state = is_valid() ? bridge_->snapshot_state() : pending_snapshot_state_;
    const NveBufferSlice frame0_slice{frame0.data(), frame0.size()};
    const NveBufferSlice state_slice{state.empty() ? nullptr : state.data(), state.size()};
    const NveBufferSlice meta_slice{
        meta.empty() ? nullptr : reinterpret_cast<const uint8_t*>(meta.data()),
        meta.size(),
    };
    return write_snapshot_atomic(
        snapshot_path, schema_version, frame0_slice, state_slice, meta_slice, 0u, out_status);
  }

  void set_text_measurer(TextMeasurerCallback cb) {
    text_measurer_ = std::move(cb);
    if (is_valid()) {
      fabric_host_set_text_measurer(bridge_->fabric_host(), text_measurer_);
    } else if (paired_host_ != nullptr) {
      fabric_host_set_text_measurer(paired_host_, text_measurer_);
    } else if (frame0_host_ != nullptr) {
      fabric_host_set_text_measurer(frame0_host_, text_measurer_);
    }
  }

  void set_event_callback(EventCallback cb) {
    event_callback_ = std::move(cb);
    if (bridge_ != nullptr) {
      bridge_->set_event_callback(event_callback_);
    }
  }

  void set_surface_config(NveSurfaceConfig config) {
    config_ = config;
    if (bridge_ != nullptr) {
      bridge_->set_surface_size(config.width, config.height, config.scale_factor);
    } else if (paired_host_ != nullptr) {
      fabric_host_set_surface_size(paired_host_, config.width, config.height, config.scale_factor);
    } else if (frame0_host_ != nullptr) {
      fabric_host_set_surface_size(frame0_host_, config.width, config.height, config.scale_factor);
    }
  }

  void set_surface_size(float width, float height, float scale_factor = 0.0f) {
    config_.width = width;
    config_.height = height;
    if (scale_factor > 0.0f) {
      config_.scale_factor = scale_factor;
    }
    if (bridge_ != nullptr) {
      bridge_->set_surface_size(width, height, scale_factor);
    } else if (paired_host_ != nullptr) {
      fabric_host_set_surface_size(
          paired_host_, config_.width, config_.height, config_.scale_factor);
    } else if (frame0_host_ != nullptr) {
      fabric_host_set_surface_size(
          frame0_host_, config_.width, config_.height, config_.scale_factor);
    }
  }

  bool update_props(const PropMap& props, bool auto_render = true) {
    return is_valid() && bridge_->update_props(props, auto_render);
  }

  bool update_props(const NveMapEntry* entries, uint32_t count, bool auto_render = true) {
    return is_valid() && bridge_->update_props(entries, count, auto_render);
  }

  int32_t render(NveFrameStats* out_stats = nullptr) {
    if (is_valid()) {
      return bridge_->render(out_stats);
    }
    if (is_paired()) {
      (void)poll_paired_frames(out_stats);
      return 0;
    }
    return -100;
  }

  uint32_t dispatch_press(uint32_t node_id, float x, float y, bool auto_render = true) {
    if (is_valid()) {
      NveDirectEvent ev{};
      ev.surface_id = config_.surface_id;
      ev.node_id = node_id;
      ev.timestamp_us = current_timestamp_us();
      ev.kind = static_cast<uint8_t>(EventKind::Press);
      ev.payload_tag = NVE_EVENT_PAYLOAD_POINT;
      ev.x = x;
      ev.y = y;
      ev.value = make_null_value();
      return bridge_->dispatch_events(NveEventBatch{&ev, 1u, 0u}, auto_render);
    }
    if (is_paired()) {
      fabric_host_enqueue_press(paired_host_, node_id, current_timestamp_us(), x, y);
      const NveBufferSlice inbound = nve_fabric_host_drain_events(paired_host_);
      return send_paired_event_ring(inbound) ? 1u : 0u;
    }
    return 0;
  }

  uint32_t dispatch_events(NveEventBatch batch, bool auto_render = true) {
    if (is_valid()) {
      return bridge_->dispatch_events(batch, auto_render);
    }
    if (is_paired() && batch.ptr != nullptr && batch.len > 0u) {
      std::vector<uint8_t> ring;
      if (!encode_direct_event_batch(batch, ring)) {
        return 0u;
      }
      return pair_client_->send_events(ring) ? batch.len : 0u;
    }
    return 0;
  }

  std::vector<DecodedEvent> drain_events() {
    return is_valid() ? bridge_->drain_emitted_events() : std::vector<DecodedEvent>{};
  }

  std::vector<RenderedNodeSnapshot> snapshot_nodes() const {
    NveFabricHostHandle host = active_fabric_host();
    return host != nullptr ? fabric_host_snapshot_nodes(host)
                           : std::vector<RenderedNodeSnapshot>{};
  }

  /// Zero-copy incrementally updated `NveSceneView` directly from the active `FabricHost`.
  NveSceneView scene_view() const noexcept {
    return fabric_host_scene_view(active_fabric_host());
  }

  NveFabricHostHandle active_fabric_host() const noexcept {
    if (is_valid()) return bridge_->fabric_host();
    if (paired_host_ != nullptr) return paired_host_;
    return frame0_host_;
  }

  uint32_t node_count() const noexcept {
    NveFabricHostHandle host = active_fabric_host();
    return host != nullptr ? nve_fabric_host_node_count(host) : 0u;
  }

  NveRnComponentBridge* bridge() noexcept { return bridge_.get(); }
  const NveRnComponentBridge* bridge() const noexcept { return bridge_.get(); }
  PairClient* pair_client() noexcept { return pair_client_.get(); }
  const PairClient* pair_client() const noexcept { return pair_client_.get(); }
  const NveSurfaceConfig& surface_config() const noexcept { return config_; }
  bool is_valid() const noexcept { return bridge_ != nullptr && bridge_->is_valid(); }
  bool is_paired() const noexcept { return pair_client_ != nullptr && paired_host_ != nullptr; }
  bool is_paired_connected() const noexcept {
    return pair_client_ != nullptr && pair_client_->is_connected();
  }
  uint64_t paired_frame_count() const noexcept { return paired_frame_count_; }
  bool has_replayed_frame0() const noexcept { return frame0_host_ != nullptr; }
  bool has_pending_snapshot_state() const noexcept { return !pending_snapshot_state_.empty(); }
  const std::vector<uint8_t>& pending_snapshot_state() const noexcept {
    return pending_snapshot_state_;
  }
  void clear_pending_snapshot_state() noexcept { pending_snapshot_state_.clear(); }

 private:
  void destroy_frame0_host() noexcept {
    if (frame0_host_ != nullptr) {
      nve_fabric_host_destroy(frame0_host_);
      frame0_host_ = nullptr;
    }
  }

  void destroy_paired_host() noexcept {
    if (paired_host_ != nullptr) {
      nve_fabric_host_destroy(paired_host_);
      paired_host_ = nullptr;
    }
  }

  NveSurfaceConfig effective_config() const noexcept {
    NveSurfaceConfig cfg = config_;
    if (cfg.surface_id == 0u) cfg.surface_id = 1u;
    if (cfg.width <= 0.0f) cfg.width = 640.0f;
    if (cfg.height <= 0.0f) cfg.height = 420.0f;
    if (cfg.scale_factor <= 0.0f) cfg.scale_factor = 1.0f;
    return cfg;
  }

  bool install_module(
      NveModule mod,
      DynamicLibrary new_lib,
      NveSurfaceConfig config,
      const std::vector<uint8_t>& saved_state) {
    auto next_bridge = std::make_unique<NveRnComponentBridge>(mod, config, event_callback_);
    if (!next_bridge->is_valid()) {
      return false;
    }
    if (text_measurer_) {
      fabric_host_set_text_measurer(next_bridge->fabric_host(), text_measurer_);
    }
    if (!saved_state.empty()) {
      next_bridge->restore_state(
          NveBufferSlice{saved_state.data(), saved_state.size()}, /*auto_render=*/false);
    }

    // Tear down old bridge, paired client & Frame-0 preview before unloading old dylib, then commit atomically.
    disconnect_paired();
    bridge_.reset();
    destroy_frame0_host();
    pending_snapshot_state_.clear();
    lib_.close();
    lib_ = std::move(new_lib);
    config_ = config;
    bridge_ = std::move(next_bridge);
    (void)bridge_->render(nullptr);
    return true;
  }

  // Declaration order matters: `bridge_` & `pair_client_` are destroyed before `lib_` is closed.
  DynamicLibrary lib_;
  std::unique_ptr<NveRnComponentBridge> bridge_;
  std::unique_ptr<PairClient> pair_client_;
  NveFabricHostHandle paired_host_{nullptr};
  uint64_t paired_frame_count_{0};
  NveFabricHostHandle frame0_host_{nullptr};
  std::vector<uint8_t> pending_snapshot_state_;
  TextMeasurerCallback text_measurer_;
  EventCallback event_callback_;
  NveSurfaceConfig config_{};
};

} // namespace nve
