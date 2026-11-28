// NVE host runtime: validated C++20 RAII wrappers over `NveRuntimeApi` and `NveSceneView`.
//
// `SceneReader` (`scene_reader.hpp`) validates a borrowed scene once on `open()`.
// `RuntimeSceneGuard` pairs `scene_acquire` with automatic `scene_release` (and releases immediately
// if validation ever fails so reader locks can never leak).
// `RuntimeSession` wraps the `NveRuntimeApi` vtable with RAII `destroy` and move-only ownership.
// Borrowed outputs keep their C lifetimes: `RuntimeSceneGuard::resource` bytes live until the
// guard releases; `drain_emitted_events` / `snapshot_state` spans until the next session call.
// App `cdylib` loading and app-state files are the runtime's (`load_app_module`, `*_state_file`):
// hosts never `dlopen` apps or encode state files themselves.
#pragma once

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>

#include "../generated/nve_runtime_abi.generated.h"
#include "../generated/nve_tables.generated.h"
#include "scene_reader.hpp"
#include "table_reader.hpp"
#include "utf8.h"
#include "xxh64.hpp"

namespace nve::host {

/// Canonical payload of one published resource (`Resource::encode` bytes; codec-less module
/// kinds are their exact wire bytes), borrowed from a `RuntimeSceneGuard`.
struct RuntimeResource {
  Key64 kind;
  std::span<const uint8_t> bytes;
};

/// `NveBufferSlice` as a span (empty for NULL / zero length).
[[nodiscard]] inline std::span<const uint8_t> buffer_span(NveBufferSlice s) noexcept {
  return s.ptr != nullptr && s.len > 0u ? std::span<const uint8_t>(s.ptr, s.len)
                                         : std::span<const uint8_t>();
}

/// Span as an `NveBufferSlice` (NULL pointer when empty).
[[nodiscard]] inline NveBufferSlice buffer_slice(std::span<const uint8_t> bytes) noexcept {
  return NveBufferSlice{bytes.empty() ? nullptr : bytes.data(), bytes.size()};
}

/// UTF-8 text as an `NveStrSlice` (NULL pointer when empty).
[[nodiscard]] inline NveStrSlice str_slice(std::string_view text) noexcept {
  return NveStrSlice{text.empty() ? nullptr : reinterpret_cast<const uint8_t*>(text.data()), text.size()};
}

/// `NveStrSlice` as UTF-8 text (empty for NULL).
[[nodiscard]] inline std::string_view str_view(NveStrSlice s) noexcept {
  return s.ptr != nullptr && s.len > 0u ? std::string_view(reinterpret_cast<const char*>(s.ptr), s.len)
                                        : std::string_view();
}

/// The default typography at `font_size` / `font_weight` (host font, `normal` line height,
/// wrapping): what a node without `nve.typography` is measured with.
[[nodiscard]] inline NveRuntimeTextStyle text_style(float font_size, uint16_t font_weight = 400u) noexcept {
  NveRuntimeTextStyle s{};
  s.font_size = font_size;
  s.font_weight = font_weight;
  s.line_height_unit = NVE_LINE_HEIGHT_AUTO;
  return s;
}

/// Absolute line height in points (`0` for `normal`): points as is, percent as a multiplier of
/// `font_size` (CSS unit-less `line-height`). Hosts hand toolkits this absolute value: some
/// (GTK CSS) scale a unit-less factor by the font's natural line height instead of its size.
[[nodiscard]] inline double line_height_points(uint32_t unit, float value, float font_size) noexcept {
  if (!(value > 0.0f) || !std::isfinite(value)) return 0.0;
  if (unit == NVE_LINE_HEIGHT_PERCENT) return static_cast<double>(value) / 100.0 * static_cast<double>(font_size);
  if (unit == NVE_LINE_HEIGHT_POINTS) return static_cast<double>(value);
  return 0.0;
}

/// `white-space: nowrap | pre` never wrap at the available width.
[[nodiscard]] inline bool white_space_wraps(uint32_t white_space) noexcept {
  return white_space != NVE_WHITE_SPACE_NO_WRAP && white_space != NVE_WHITE_SPACE_PRE;
}

/// Move-only RAII read guard pairing `scene_acquire` with `scene_release`.
class RuntimeSceneGuard {
 public:
  RuntimeSceneGuard(const RuntimeSceneGuard&) = delete;
  RuntimeSceneGuard& operator=(const RuntimeSceneGuard&) = delete;

  RuntimeSceneGuard(RuntimeSceneGuard&& other) noexcept
      : api_(std::exchange(other.api_, nullptr)),
        handle_(std::exchange(other.handle_, nullptr)),
        raw_(std::exchange(other.raw_, NveSceneView{})),
        reader_(std::exchange(other.reader_, SceneReader{})) {}

  RuntimeSceneGuard& operator=(RuntimeSceneGuard&& other) noexcept {
    if (this != &other) {
      release();
      api_ = std::exchange(other.api_, nullptr);
      handle_ = std::exchange(other.handle_, nullptr);
      raw_ = std::exchange(other.raw_, NveSceneView{});
      reader_ = std::exchange(other.reader_, SceneReader{});
    }
    return *this;
  }

  ~RuntimeSceneGuard() noexcept { release(); }

  void release() noexcept {
    if (api_ != nullptr && handle_ != nullptr) {
      (void)api_->scene_release(handle_);
      api_ = nullptr;
      handle_ = nullptr;
    }
    raw_ = NveSceneView{};
    reader_ = SceneReader{};
  }

  [[nodiscard]] bool active() const noexcept { return handle_ != nullptr; }
  [[nodiscard]] const SceneReader& reader() const noexcept { return reader_; }
  [[nodiscard]] const SceneReader* operator->() const noexcept { return &reader_; }
  [[nodiscard]] const NveSceneView& raw() const noexcept { return raw_; }

  /// Published resource `id`, borrowed until this guard releases. `out_status` (optional) gets
  /// `NVE_RUNTIME_NOT_FOUND` for stale / NONE ids and `NVE_RUNTIME_INVALID_ARG` when inactive.
  [[nodiscard]] std::optional<RuntimeResource> resource(uint32_t id,
                                                        NveRuntimeStatus* out_status = nullptr) const noexcept {
    NveRuntimeStatus st = NVE_RUNTIME_INVALID_ARG;
    uint64_t kind = 0u;
    NveBufferSlice payload{nullptr, 0u};
    if (active()) st = static_cast<NveRuntimeStatus>(api_->resource_payload(handle_, id, &kind, &payload));
    if (out_status != nullptr) *out_status = st;
    if (st != NVE_RUNTIME_OK) return std::nullopt;
    return RuntimeResource{Key64{kind}, buffer_span(payload)};
  }

 private:
  friend class RuntimeSession;

  RuntimeSceneGuard(const NveRuntimeApi* api, NveRuntimeHandle handle,
                    const NveSceneView& raw, SceneReader reader) noexcept
      : api_(api), handle_(handle), raw_(raw), reader_(reader) {}

  const NveRuntimeApi* api_ = nullptr;
  NveRuntimeHandle handle_ = nullptr;
  NveSceneView raw_{};
  SceneReader reader_;
};

/// Move-only RAII host session over a live `NveRuntimeHandle` and validated `NveRuntimeApi`.
class RuntimeSession {
 public:
  [[nodiscard]] static constexpr NveRuntimeConfig default_config(
      uint32_t surface_id = 1u, float width = 800.0f, float height = 600.0f,
      float scale_factor = 1.0f) noexcept {
    return NveRuntimeConfig{surface_id, width, height, scale_factor, 0u, 0u};
  }

  /// Verifies ABI version, struct size, schema hash, and every vtable slot before calling `create`.
  [[nodiscard]] static bool validate_api(const NveRuntimeApi* api) noexcept {
    return nve_runtime_api_slots_complete(api) != 0 && api->abi_version == NVE_RUNTIME_ABI_VERSION &&
           api->struct_size == sizeof(NveRuntimeApi) && api->schema_hash == NVE_RUNTIME_SCHEMA_HASH;
  }

  [[nodiscard]] static std::optional<RuntimeSession> create(
      const NveRuntimeApi* api,
      const NveRuntimeConfig& cfg = default_config()) noexcept {
    if (!validate_api(api)) return std::nullopt;
    NveRuntimeHandle handle = api->create(&cfg);
    if (handle == nullptr) return std::nullopt;
    return RuntimeSession(api, handle);
  }

  RuntimeSession() noexcept = default;
  RuntimeSession(const RuntimeSession&) = delete;
  RuntimeSession& operator=(const RuntimeSession&) = delete;

  RuntimeSession(RuntimeSession&& other) noexcept
      : api_(std::exchange(other.api_, nullptr)),
        handle_(std::exchange(other.handle_, nullptr)) {}

  RuntimeSession& operator=(RuntimeSession&& other) noexcept {
    if (this != &other) {
      reset();
      api_ = std::exchange(other.api_, nullptr);
      handle_ = std::exchange(other.handle_, nullptr);
    }
    return *this;
  }

  ~RuntimeSession() noexcept { reset(); }

  void reset() noexcept {
    if (api_ != nullptr && handle_ != nullptr) {
      api_->destroy(handle_);
      api_ = nullptr;
      handle_ = nullptr;
    }
  }

  [[nodiscard]] bool valid() const noexcept { return api_ != nullptr && handle_ != nullptr; }
  [[nodiscard]] NveRuntimeHandle handle() const noexcept { return handle_; }
  [[nodiscard]] const NveRuntimeApi* api() const noexcept { return api_; }

  [[nodiscard]] NveRuntimeStatus set_wake_callback(NveRuntimeWakeFn cb, void* ctx) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->set_wake_callback(handle_, cb, ctx));
  }

  [[nodiscard]] NveRuntimeStatus clear_wake_callback() noexcept {
    return set_wake_callback(nullptr, nullptr);
  }

  [[nodiscard]] NveRuntimeStatus set_viewport(float width, float height, float scale_factor) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->set_viewport(handle_, width, height, scale_factor));
  }

  [[nodiscard]] NveRuntimeStatus attach_app_interface(NveGetInterfaceFn get_iface) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->attach_app_interface(handle_, get_iface));
  }

  [[nodiscard]] NveRuntimeStatus detach_app() noexcept {
    return attach_app_interface(nullptr);
  }

  /// Loads and attaches the app `cdylib` at `utf8_path` from a private staged copy, so a rebuilt
  /// file always runs its new code; `carry_state` moves the running app's state into the new app
  /// before it goes live. `IO`: cannot open, `NOT_FOUND`: no `nve_get_interface`, `UNSUPPORTED`:
  /// no complete `NVE_IFACE_SURFACE`. A failure leaves the running app untouched.
  [[nodiscard]] NveRuntimeStatus load_app_module(std::string_view utf8_path, bool carry_state = false) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(
        api_->load_app_module(handle_, str_slice(utf8_path), carry_state ? 1u : 0u));
  }

  /// Installs a native text measurer (`ctx` must outlive it); the next `tick` relayouts.
  [[nodiscard]] NveRuntimeStatus set_text_measurer(NveRuntimeMeasureTextFn measure, void* ctx) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->set_text_measurer(handle_, measure, ctx));
  }

  /// Restores the built-in heuristic text measurer.
  [[nodiscard]] NveRuntimeStatus clear_text_measurer() noexcept {
    return set_text_measurer(nullptr, nullptr);
  }

  /// Forwards `count` embedding-host props (`NVE_RUNTIME_UNSUPPORTED` without
  /// `NVE_IFACE_COMPONENT`). Pointer + count: `NveMapEntry` is complete only where the caller
  /// includes `nve_abi.generated.h`.
  [[nodiscard]] NveRuntimeStatus set_props(const NveMapEntry* entries, uint32_t count,
                                           bool* out_changed = nullptr) noexcept {
    if (!valid() || (entries == nullptr && count != 0u)) return NVE_RUNTIME_INVALID_ARG;
    uint32_t changed = 0u;
    const auto st = static_cast<NveRuntimeStatus>(
        api_->set_props(handle_, count == 0u ? nullptr : entries, count, &changed));
    if (out_changed != nullptr) *out_changed = (changed != 0u);
    return st;
  }

  /// Drains app-emitted events into `out` (`ptr` NULL when empty; borrowed until the next session
  /// call).
  [[nodiscard]] NveRuntimeStatus drain_emitted_events(NveEventBatch* out) noexcept {
    if (!valid() || out == nullptr) return NVE_RUNTIME_INVALID_ARG;
    NveEventBatch batch{nullptr, 0u, 0u};
    const auto st = static_cast<NveRuntimeStatus>(api_->drain_emitted_events(handle_, &batch));
    *out = st == NVE_RUNTIME_OK && batch.len > 0u ? batch : NveEventBatch{nullptr, 0u, 0u};
    return st;
  }

  /// App state snapshot into `out` (borrowed until the next session call).
  [[nodiscard]] NveRuntimeStatus snapshot_state(std::span<const uint8_t>* out) noexcept {
    if (!valid() || out == nullptr) return NVE_RUNTIME_INVALID_ARG;
    NveBufferSlice state{nullptr, 0u};
    const auto st = static_cast<NveRuntimeStatus>(api_->snapshot_state(handle_, &state));
    *out = st == NVE_RUNTIME_OK ? buffer_span(state) : std::span<const uint8_t>();
    return st;
  }

  /// Restores a snapshot; the runtime wakes the host to re-render.
  [[nodiscard]] NveRuntimeStatus restore_state(std::span<const uint8_t> state) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->restore_state(handle_, buffer_slice(state)));
  }

  /// Writes the app state as a crash-atomic, checksummed runtime `StateFile` tagged
  /// `schema_version` (`NOT_FOUND` when the app has no state).
  [[nodiscard]] NveRuntimeStatus save_state_file(std::string_view utf8_path, uint32_t schema_version) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->save_state_file(handle_, str_slice(utf8_path), schema_version));
  }

  /// Restores a `save_state_file` file (`NOT_FOUND` when missing, quarantined as corrupt, or
  /// rotated as an older schema; `IO` for a newer schema).
  [[nodiscard]] NveRuntimeStatus restore_state_file(std::string_view utf8_path, uint32_t schema_version) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(
        api_->restore_state_file(handle_, str_slice(utf8_path), schema_version));
  }

  /// Deletes a state file (cold start; `OK` when already absent).
  [[nodiscard]] NveRuntimeStatus clear_state_file(std::string_view utf8_path) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->clear_state_file(handle_, str_slice(utf8_path)));
  }

  /// Validates and forwards one `NVP1` hot patch (`NVE_RUNTIME_UNSUPPORTED` without `NVE_IFACE_DEV`).
  [[nodiscard]] NveRuntimeStatus apply_hot_patch(std::span<const uint8_t> patch_nvp1,
                                                 bool* out_changed = nullptr) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    uint32_t changed = 0u;
    const auto st = static_cast<NveRuntimeStatus>(
        api_->apply_hot_patch(handle_, buffer_slice(patch_nvp1), &changed));
    if (out_changed != nullptr) *out_changed = (changed != 0u);
    return st;
  }

  [[nodiscard]] NveRuntimeStatus submit_nve2(std::span<const uint8_t> bytes) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->submit_nve2(handle_, buffer_slice(bytes)));
  }

  [[nodiscard]] NveRuntimeStatus tick(float dt_s, bool* out_changed = nullptr) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    uint32_t changed = 0u;
    const auto st = static_cast<NveRuntimeStatus>(api_->tick(handle_, dt_s, &changed));
    if (out_changed != nullptr) *out_changed = (changed != 0u);
    return st;
  }

  [[nodiscard]] NveRuntimeStatus dispatch_events(NveEventBatch batch,
                                                 bool* out_changed = nullptr) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    uint32_t changed = 0u;
    const auto st = static_cast<NveRuntimeStatus>(api_->dispatch_events(handle_, batch, &changed));
    if (out_changed != nullptr) *out_changed = (changed != 0u);
    return st;
  }

  [[nodiscard]] NveRuntimeStatus hit_test(float x, float y, uint32_t* out_node_id) const noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->hit_test(handle_, x, y, out_node_id));
  }

  [[nodiscard]] NveRuntimeStatus scroll_by(uint32_t node_id, float dx, float dy,
                                           float* out_x = nullptr,
                                           float* out_y = nullptr) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->scroll_by(handle_, node_id, dx, dy, out_x, out_y));
  }

  /// Scrolls `node_id` to the absolute offset `(x, y)` (non-finite: keep that axis), clamped by the
  /// runtime; `out_x` / `out_y` receive the applied offset. (A host-side composition of
  /// `scroll_by`: the runtime keeps one relative scroll slot.)
  [[nodiscard]] NveRuntimeStatus scroll_to(uint32_t node_id, float x, float y, float* out_x = nullptr,
                                           float* out_y = nullptr) noexcept {
    float cur_x = 0.0f, cur_y = 0.0f;
    if (const NveRuntimeStatus st = scroll_by(node_id, 0.0f, 0.0f, &cur_x, &cur_y); st != NVE_RUNTIME_OK) return st;
    const float dx = std::isfinite(x) ? x - cur_x : 0.0f;
    const float dy = std::isfinite(y) ? y - cur_y : 0.0f;
    return scroll_by(node_id, dx, dy, out_x, out_y);
  }

  [[nodiscard]] std::optional<RuntimeSceneGuard> acquire_scene(
      NveRuntimeStatus* out_status = nullptr,
      SceneError* out_scene_error = nullptr) const noexcept {
    if (!valid()) {
      if (out_status != nullptr) *out_status = NVE_RUNTIME_INVALID_ARG;
      if (out_scene_error != nullptr) *out_scene_error = SceneError::NullView;
      return std::nullopt;
    }
    NveSceneView raw{};
    const auto st = static_cast<NveRuntimeStatus>(api_->scene_acquire(handle_, &raw));
    if (out_status != nullptr) *out_status = st;
    if (st != NVE_RUNTIME_OK) {
      if (out_scene_error != nullptr) *out_scene_error = SceneError::NullView;
      return std::nullopt;
    }
    SceneError serr = SceneError::None;
    std::optional<SceneReader> reader =
        SceneReader::open(&raw, Key64{NVE_RUNTIME_SCHEMA_HASH}, &serr);
    if (out_scene_error != nullptr) *out_scene_error = serr;
    if (!reader.has_value()) {
      (void)api_->scene_release(handle_);
      return std::nullopt;
    }
    return RuntimeSceneGuard(api_, handle_, raw, *reader);
  }

  [[nodiscard]] NveRuntimeStatus metrics(NveRuntimeMetrics* out_metrics) const noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->get_metrics(handle_, out_metrics));
  }

  [[nodiscard]] NveRuntimeStatus pair_connect(std::string_view uri,
                                              std::string_view token = {}) noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->pair_connect(handle_, str_slice(uri), str_slice(token)));
  }

  [[nodiscard]] NveRuntimeStatus pair_disconnect() noexcept {
    if (!valid()) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<NveRuntimeStatus>(api_->pair_disconnect(handle_));
  }

  [[nodiscard]] bool pair_is_connected() const noexcept {
    return valid() && api_->pair_is_connected(handle_) != 0u;
  }

  [[nodiscard]] std::string last_error() const {
    if (api_ == nullptr) return {};
    const size_t needed = api_->last_error(nullptr, 0u);
    if (needed == 0u) return {};
    std::string out(needed + 1u, '\0');
    (void)api_->last_error(reinterpret_cast<uint8_t*>(out.data()), out.size());
    const size_t nul = out.find('\0');
    if (nul != std::string::npos) out.resize(nul);
    return out;
  }

 private:
  RuntimeSession(const NveRuntimeApi* api, NveRuntimeHandle handle) noexcept
      : api_(api), handle_(handle) {}

  const NveRuntimeApi* api_ = nullptr;
  NveRuntimeHandle handle_ = nullptr;
};

}  // namespace nve::host
