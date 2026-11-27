// NVE host runtime: validated C++20 RAII wrappers over `NveRuntimeApi` and `NveRuntimeSceneView`.
//
// `RuntimeSceneReader` validates `NVE_RUNTIME_SCHEMA_HASH`, every borrowed slab pointer/count pair,
// UTF-8 `text_slab`, and every published `NveTableView` once on `open()`.
// `RuntimeSceneGuard` pairs `scene_acquire` with automatic `scene_release` (and releases immediately
// if validation ever fails so reader locks can never leak).
// `RuntimeSession` wraps the `NveRuntimeApi` vtable with RAII `destroy` and move-only ownership.
// Borrowed outputs keep their C lifetimes: `RuntimeSceneGuard::resource` bytes live until the
// guard releases; `drain_emitted_events` / `snapshot_state` spans until the next session call.
// App `cdylib` loading and app-state files are the runtime's (`load_app_module`, `*_state_file`):
// hosts never `dlopen` apps or encode state files themselves.
#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>

#include "../generated/nve_runtime_abi.generated.h"
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

/// Validated zero-copy reader over a borrowed `NveRuntimeSceneView`.
class RuntimeSceneReader {
 public:
  RuntimeSceneReader() noexcept = default;

  /// Validates `view` once; `error` (optional) receives the reason on failure.
  [[nodiscard]] static std::optional<RuntimeSceneReader> open(
      const NveRuntimeSceneView* view,
      Key64 expected_schema = Key64{NVE_RUNTIME_SCHEMA_HASH},
      SceneError* error = nullptr) noexcept {
    const SceneError e = validate(view, expected_schema);
    if (error != nullptr) *error = e;
    if (e != SceneError::None) return std::nullopt;
    return RuntimeSceneReader(*view);
  }

  [[nodiscard]] uint64_t frame_id() const noexcept { return frame_id_; }
  /// Bumped by every runtime scene reset; ids and caches from another epoch are stale.
  [[nodiscard]] uint32_t scene_epoch() const noexcept { return scene_epoch_; }
  [[nodiscard]] uint32_t node_capacity() const noexcept { return static_cast<uint32_t>(rows_.size()); }
  [[nodiscard]] std::span<const NveNodeRow> rows() const noexcept { return rows_; }
  [[nodiscard]] std::span<const NveLayoutRect> rects() const noexcept { return rects_; }
  [[nodiscard]] std::span<const uint32_t> dirty_indices() const noexcept { return dirty_; }
  [[nodiscard]] std::span<const uint32_t> removed_ids() const noexcept { return removed_; }
  [[nodiscard]] std::span<const uint32_t> children_slab() const noexcept { return children_; }
  [[nodiscard]] std::string_view text_slab() const noexcept { return text_; }
  [[nodiscard]] uint32_t table_count() const noexcept { return static_cast<uint32_t>(tables_.size()); }
  /// Visual scroll shifts sorted by `node_id`; subtract from descendants' (pre-scroll) rects.
  [[nodiscard]] std::span<const NveScrollOffset> scroll_offsets() const noexcept { return scroll_; }
  /// Live resource ids (re)defined on this frame, strictly ascending (validated by `open`): drop
  /// host caches keyed by them.
  [[nodiscard]] std::span<const uint32_t> dirty_resources() const noexcept { return dirty_resources_; }

  /// Visual shift of scroll container `node_id` (`{0, 0}` when not scrolled; O(log n)).
  [[nodiscard]] NveScrollOffset scroll_offset(uint32_t node_id) const noexcept {
    const auto it = std::lower_bound(scroll_.begin(), scroll_.end(), node_id,
                                     [](const NveScrollOffset& s, uint32_t id) { return s.node_id < id; });
    return it != scroll_.end() && it->node_id == node_id ? *it : NveScrollOffset{node_id, 0.0f, 0.0f};
  }

  [[nodiscard]] const NveNodeRow* row(uint32_t index) const noexcept {
    return index < rows_.size() ? &rows_[index] : nullptr;
  }

  [[nodiscard]] const NveLayoutRect* rect(uint32_t index) const noexcept {
    return index < rects_.size() ? &rects_[index] : nullptr;
  }

  [[nodiscard]] bool is_live(uint32_t index) const noexcept {
    return index < rows_.size() && (rows_[index].flags & NVE_NODE_ROW_FLAG_LIVE) != 0u;
  }

  [[nodiscard]] bool has_table(uint32_t index, uint8_t slot) const noexcept {
    if (index >= rows_.size() || slot >= 128u) return false;
    const size_t word = static_cast<size_t>(slot) >> 5u;
    const uint32_t bit = static_cast<uint32_t>(slot) & 31u;
    return (rows_[index].table_set[word] & (uint32_t{1} << bit)) != 0u;
  }

  /// `text_slab[offset .. offset + len)`; `nullopt` when out of range or not on UTF-8 boundaries.
  [[nodiscard]] std::optional<std::string_view> text(uint32_t offset, uint32_t len) const noexcept {
    if (offset > text_.size() || len > text_.size() - offset) return std::nullopt;
    if (!is_boundary(offset) || !is_boundary(static_cast<size_t>(offset) + len)) return std::nullopt;
    return text_.substr(offset, len);
  }

  /// `children_slab[offset .. offset + count)`; `nullopt` when out of range.
  [[nodiscard]] std::optional<std::span<const uint32_t>> children(uint32_t offset, uint32_t count) const noexcept {
    if (offset > children_.size() || count > children_.size() - offset) return std::nullopt;
    return children_.subspan(offset, count);
  }

  /// Convenience child-ID slice for node `index` (empty when out of range or invalid span).
  [[nodiscard]] std::optional<std::span<const uint32_t>> node_children(uint32_t index) const noexcept {
    if (index >= rows_.size()) return std::nullopt;
    return children(rows_[index].child_off, rows_[index].child_len);
  }

  /// Visits `(index, row, rect)` for every in-range dirty node index in O(dirty).
  template <class Fn>
  void for_each_dirty(Fn&& visit) const {
    for (const uint32_t index : dirty_) {
      if (index < rows_.size()) visit(index, rows_[index], rects_[index]);
    }
  }

  [[nodiscard]] std::optional<TableReader> table(uint32_t index) const noexcept {
    if (index >= tables_.size()) return std::nullopt;
    return TableReader::from_view(tables_[index]);
  }

  /// Published table with `key`, if any (O(tables)).
  [[nodiscard]] std::optional<TableReader> find_table(Key64 key) const noexcept {
    for (const NveTableView& t : tables_) {
      if (Key64::from_halves(t.key_lo, t.key_hi) == key) return TableReader::from_view(t);
    }
    return std::nullopt;
  }

 private:
  explicit RuntimeSceneReader(const NveRuntimeSceneView& v) noexcept
      : frame_id_(v.frame_id),
        scene_epoch_(v.scene_epoch),
        rows_(span_of(v.rows, v.node_capacity)),
        rects_(span_of(v.rects, v.node_capacity)),
        text_(v.text_slab_len > 0u
                  ? std::string_view(reinterpret_cast<const char*>(v.text_slab), v.text_slab_len)
                  : std::string_view()),
        children_(span_of(v.children_slab, v.children_slab_len)),
        dirty_(span_of(v.dirty_indices, v.dirty_count)),
        removed_(span_of(v.removed_ids, v.removed_count)),
        tables_(span_of(v.tables, v.table_count)),
        scroll_(span_of(v.scroll_offsets, v.scroll_count)),
        dirty_resources_(span_of(v.dirty_resources, v.dirty_resource_count)) {}

  template <class T>
  static std::span<const T> span_of(const T* p, uint32_t count) noexcept {
    return count > 0u ? std::span<const T>(p, static_cast<size_t>(count)) : std::span<const T>();
  }

  template <class T>
  static SceneError check_slab(const T* p, uint32_t count) noexcept {
    if (count == 0u) return SceneError::None;
    if (p == nullptr || !count_fits_host(count, sizeof(T))) return SceneError::NullSlab;
    if ((reinterpret_cast<uintptr_t>(p) % alignof(T)) != 0u) return SceneError::MisalignedSlab;
    return SceneError::None;
  }

  static SceneError validate(const NveRuntimeSceneView* v, Key64 expected_schema) noexcept {
    if (v == nullptr) return SceneError::NullView;
    if (v->schema_hash != expected_schema.value) return SceneError::SchemaMismatch;
    if (const SceneError e = check_slab(v->rows, v->node_capacity); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->rects, v->node_capacity); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->text_slab, v->text_slab_len); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->children_slab, v->children_slab_len); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->dirty_indices, v->dirty_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->removed_ids, v->removed_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->tables, v->table_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->scroll_offsets, v->scroll_count); e != SceneError::None) return e;
    if (const SceneError e = check_slab(v->dirty_resources, v->dirty_resource_count); e != SceneError::None) return e;
    if (v->text_slab_len > 0u && nve_utf8_valid(v->text_slab, v->text_slab_len) == 0) {
      return SceneError::InvalidUtf8;
    }
    for (uint32_t i = 0u; i < v->table_count; ++i) {
      if (!TableReader::from_view(v->tables[i])) return SceneError::InvalidTable;
    }
    // `scroll_offset()` binary-searches, so ids must be strictly ascending (no duplicates).
    for (uint32_t i = 1u; i < v->scroll_count; ++i) {
      if (v->scroll_offsets[i - 1u].node_id >= v->scroll_offsets[i].node_id) {
        return SceneError::UnsortedScrollOffsets;
      }
    }
    // Documented sorted + deduplicated: hosts may binary-search or merge it.
    for (uint32_t i = 1u; i < v->dirty_resource_count; ++i) {
      if (v->dirty_resources[i - 1u] >= v->dirty_resources[i]) return SceneError::UnsortedDirtyResources;
    }
    return SceneError::None;
  }

  [[nodiscard]] bool is_boundary(size_t at) const noexcept {
    return at == text_.size() || (static_cast<uint8_t>(text_[at]) & 0xC0u) != 0x80u;
  }

  uint64_t frame_id_ = 0u;
  uint32_t scene_epoch_ = 0u;
  std::span<const NveNodeRow> rows_;
  std::span<const NveLayoutRect> rects_;
  std::string_view text_;
  std::span<const uint32_t> children_;
  std::span<const uint32_t> dirty_;
  std::span<const uint32_t> removed_;
  std::span<const NveTableView> tables_;
  std::span<const NveScrollOffset> scroll_;
  std::span<const uint32_t> dirty_resources_;
};

/// Move-only RAII read guard pairing `scene_acquire` with `scene_release`.
class RuntimeSceneGuard {
 public:
  RuntimeSceneGuard(const RuntimeSceneGuard&) = delete;
  RuntimeSceneGuard& operator=(const RuntimeSceneGuard&) = delete;

  RuntimeSceneGuard(RuntimeSceneGuard&& other) noexcept
      : api_(std::exchange(other.api_, nullptr)),
        handle_(std::exchange(other.handle_, nullptr)),
        raw_(std::exchange(other.raw_, NveRuntimeSceneView{})),
        reader_(std::exchange(other.reader_, RuntimeSceneReader{})) {}

  RuntimeSceneGuard& operator=(RuntimeSceneGuard&& other) noexcept {
    if (this != &other) {
      release();
      api_ = std::exchange(other.api_, nullptr);
      handle_ = std::exchange(other.handle_, nullptr);
      raw_ = std::exchange(other.raw_, NveRuntimeSceneView{});
      reader_ = std::exchange(other.reader_, RuntimeSceneReader{});
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
    raw_ = NveRuntimeSceneView{};
    reader_ = RuntimeSceneReader{};
  }

  [[nodiscard]] bool active() const noexcept { return handle_ != nullptr; }
  [[nodiscard]] const RuntimeSceneReader& reader() const noexcept { return reader_; }
  [[nodiscard]] const RuntimeSceneReader* operator->() const noexcept { return &reader_; }
  [[nodiscard]] const NveRuntimeSceneView& raw() const noexcept { return raw_; }

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
                    const NveRuntimeSceneView& raw, RuntimeSceneReader reader) noexcept
      : api_(api), handle_(handle), raw_(raw), reader_(reader) {}

  const NveRuntimeApi* api_ = nullptr;
  NveRuntimeHandle handle_ = nullptr;
  NveRuntimeSceneView raw_{};
  RuntimeSceneReader reader_;
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

  [[nodiscard]] std::optional<RuntimeSceneGuard> acquire_scene(
      NveRuntimeStatus* out_status = nullptr,
      SceneError* out_scene_error = nullptr) const noexcept {
    if (!valid()) {
      if (out_status != nullptr) *out_status = NVE_RUNTIME_INVALID_ARG;
      if (out_scene_error != nullptr) *out_scene_error = SceneError::NullView;
      return std::nullopt;
    }
    NveRuntimeSceneView raw{};
    const auto st = static_cast<NveRuntimeStatus>(api_->scene_acquire(handle_, &raw));
    if (out_status != nullptr) *out_status = st;
    if (st != NVE_RUNTIME_OK) {
      if (out_scene_error != nullptr) *out_scene_error = SceneError::NullView;
      return std::nullopt;
    }
    SceneError serr = SceneError::None;
    std::optional<RuntimeSceneReader> reader =
        RuntimeSceneReader::open(&raw, Key64{NVE_RUNTIME_SCHEMA_HASH}, &serr);
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
