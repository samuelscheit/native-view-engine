// NVE host runtime: one native host surface over `libnve_runtime`.
//
// `HostSession` owns a `RuntimeSession`, the app it runs (an in-process `nve_get_interface`, a
// runtime-loaded `cdylib` with state-preserving hot reload, or a paired remote via `pair_connect`),
// the wake marshaling to the host's UI thread, typed input-event dispatch, and app-state files
// (the runtime's checksummed, crash-atomic `StateFile`).
// Every call except `post_wake` runs on the thread that created the session (the runtime's owner
// thread); a host drives it with `pump()` from its UI loop whenever the wake callback fires.
#pragma once

#include <chrono>
#include <cstdint>
#include <cstring>
#include <functional>
#include <memory>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "../generated/nve_abi.generated.h"
#include "owned_events.hpp"
#include "runtime_session.hpp"

namespace nve::host {

/// Why loading an app module failed (`load_app_module` statuses; details in `last_error`).
enum class ModuleError : uint8_t {
  None = 0,
  Open = 1,            // NVE_RUNTIME_IO: missing / unreadable / not a loadable library
  MissingEntry = 2,    // NVE_RUNTIME_NOT_FOUND: no `nve_get_interface` export
  MissingSurface = 3,  // NVE_RUNTIME_UNSUPPORTED: no complete `NVE_IFACE_SURFACE`
  Attach = 4,          // any other status: the runtime refused the app (create failed, busy, ...)
};

[[nodiscard]] constexpr ModuleError module_error(NveRuntimeStatus status) noexcept {
  switch (status) {
    case NVE_RUNTIME_OK: return ModuleError::None;
    case NVE_RUNTIME_IO: return ModuleError::Open;
    case NVE_RUNTIME_NOT_FOUND: return ModuleError::MissingEntry;
    case NVE_RUNTIME_UNSUPPORTED: return ModuleError::MissingSurface;
    default: return ModuleError::Attach;
  }
}

/// Builds `NveDirectEvent`s for one surface (payloads follow `NveEventPayloadTag`).
struct EventFactory {
  uint32_t surface_id = 1u;

  [[nodiscard]] NveDirectEvent base(uint32_t node_id, NveEventKind kind, NveEventPayloadTag tag) const noexcept {
    NveDirectEvent ev{};
    ev.surface_id = surface_id;
    ev.node_id = node_id;
    ev.timestamp_us = static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(
                                                std::chrono::steady_clock::now().time_since_epoch())
                                                .count());
    ev.kind = static_cast<uint8_t>(kind);
    ev.payload_tag = static_cast<uint8_t>(tag);
    ev.value.tag = NVE_VAL_NULL;
    return ev;
  }
  /// Point event (`Press`, `Pointer*`) at node-local `(x, y)`.
  [[nodiscard]] NveDirectEvent pointer(uint32_t node, NveEventKind kind, float x, float y) const noexcept {
    NveDirectEvent ev = base(node, kind, NVE_EVENT_PAYLOAD_POINT);
    ev.x = x;
    ev.y = y;
    return ev;
  }
  [[nodiscard]] NveDirectEvent press(uint32_t node, float x, float y) const noexcept {
    return pointer(node, NVE_EVENT_PRESS, x, y);
  }
  /// `text` must outlive the dispatch.
  [[nodiscard]] NveDirectEvent text(uint32_t node, NveEventKind kind, std::string_view text) const noexcept {
    NveDirectEvent ev = base(node, kind, NVE_EVENT_PAYLOAD_TEXT);
    ev.text = NveStrSlice{reinterpret_cast<const uint8_t*>(text.data()), text.size()};
    return ev;
  }
  [[nodiscard]] NveDirectEvent focus(uint32_t node, bool focused) const noexcept {
    return base(node, focused ? NVE_EVENT_FOCUS : NVE_EVENT_BLUR, NVE_EVENT_PAYLOAD_EMPTY);
  }
  [[nodiscard]] NveDirectEvent scroll(uint32_t node, float x, float y, float content_w, float content_h) const noexcept {
    NveDirectEvent ev = base(node, NVE_EVENT_SCROLL, NVE_EVENT_PAYLOAD_SCROLL);
    ev.x = x;
    ev.y = y;
    ev.width = content_w;
    ev.height = content_h;
    return ev;
  }
};

class HostSession {
 public:
  /// Posts "call `pump()` soon" to the host UI thread. Invoked on ANY thread; must not block.
  using WakePoster = std::function<void()>;
  /// Receives the app's outbound (`NveComponentApi::drain_emitted_events`) events after each pump.
  using EventSink = std::function<void(std::span<const NveDirectEvent>)>;

  /// Creates a runtime surface (`libnve_runtime` linked into the host).
  [[nodiscard]] static std::unique_ptr<HostSession> create(const NveRuntimeConfig& cfg, std::string* out_error = nullptr) {
    std::optional<RuntimeSession> rt =
        RuntimeSession::create(nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION), cfg);
    if (!rt) {
      if (out_error != nullptr) *out_error = "libnve_runtime rejected the ABI or surface config";
      return nullptr;
    }
    return std::unique_ptr<HostSession>(new HostSession(std::move(*rt), cfg.surface_id));
  }

  ~HostSession() {
    *alive_ = false;  // a `pump` whose sink destroys the session stops touching it
    (void)runtime_.clear_wake_callback();  // drains in-flight wakes before `wake_` dies
  }
  HostSession(const HostSession&) = delete;
  HostSession& operator=(const HostSession&) = delete;

  [[nodiscard]] RuntimeSession& runtime() noexcept { return runtime_; }
  [[nodiscard]] const RuntimeSession& runtime() const noexcept { return runtime_; }
  [[nodiscard]] const EventFactory& events() const noexcept { return events_; }
  [[nodiscard]] uint32_t surface_id() const noexcept { return events_.surface_id; }
  [[nodiscard]] bool has_app() const noexcept { return attached_ || runtime_.pair_is_connected(); }
  /// The running app came from `load_module` / `reload_module`.
  [[nodiscard]] bool has_module() const noexcept { return module_; }
  [[nodiscard]] std::string last_error() const { return runtime_.last_error(); }

  /// Installs (or with `{}` removes) the UI-thread wake poster.
  void set_wake_poster(WakePoster poster) {
    (void)runtime_.clear_wake_callback();
    wake_ = std::move(poster);
    if (wake_) (void)runtime_.set_wake_callback(&HostSession::on_wake, this);
  }

  void set_event_sink(EventSink sink) {
    sink_ = sink ? std::make_shared<const EventSink>(std::move(sink)) : nullptr;
  }

  /// Runs an in-process app (`nve_get_interface` linked into the host).
  bool attach(NveGetInterfaceFn get_interface) {
    if (get_interface == nullptr) return false;
    // A successful attach replaces any paired source; a failed one leaves everything running.
    if (runtime_.attach_app_interface(get_interface) != NVE_RUNTIME_OK) return false;
    on_source_changed(true, false);
    return true;
  }

  /// Loads and runs an app `cdylib` (the runtime loads a private copy, so a rebuilt file always
  /// runs its new code). A failure leaves the running app untouched.
  bool load_module(std::string_view path, ModuleError* out_error = nullptr, std::string* out_message = nullptr) {
    return load(path, false, out_error, out_message);
  }

  /// Hot-reloads the app `cdylib` at `path` (typically just rebuilt), carrying its `NveDevApi`
  /// state into the new app before it goes live.
  bool reload_module(std::string_view path, ModuleError* out_error = nullptr, std::string* out_message = nullptr) {
    return load(path, true, out_error, out_message);
  }

  /// Thin-client mode: frames come from a remote `nve dev` server, input goes upstream.
  bool pair(std::string_view uri, std::string_view token, std::string* out_error = nullptr) {
    if (runtime_.pair_connect(uri, token) != NVE_RUNTIME_OK) {
      if (out_error != nullptr) *out_error = runtime_.last_error();
      return false;
    }
    on_source_changed(false, false, true);
    return true;
  }

  /// Leaves paired mode, stopping the pairing worker whether it is connected, still connecting,
  /// or reconnecting (no-op when not paired).
  void unpair() {
    if (!paired_) return;
    paired_ = false;
    (void)runtime_.pair_disconnect();
  }

  /// Advances the runtime (renders the app / applies paired frames) and forwards the app's
  /// outbound events; `true` when the scene changed.
  ///
  /// The sink may re-enter the session (dispatch, pump, attach, reload, `set_event_sink`): it
  /// gets an owned deep copy of the batch that no nested pump reuses, through a copy of the sink
  /// that replacing it cannot destroy mid-call. It may even destroy the session: `pump` (and the
  /// `dispatch` / `set_props` / `apply_hot_patch` that called it) then touches no member again.
  /// Steady state allocates nothing: the batch slabs are reused.
  bool pump(float dt_s = 0.0f) {
    bool changed = false;
    const bool ok = runtime_.tick(dt_s, &changed) == NVE_RUNTIME_OK;
    NveEventBatch out{};
    if (sink_ && runtime_.drain_emitted_events(&out) == NVE_RUNTIME_OK && out.ptr != nullptr && out.len != 0u) {
      OwnedEvents batch = std::exchange(drained_, OwnedEvents{});  // a nested pump gets its own
      batch.assign(std::span<const NveDirectEvent>(out.ptr, out.len));
      const std::shared_ptr<bool> alive = alive_;
      const std::shared_ptr<const EventSink> sink = sink_;  // survives `set_event_sink` / `~HostSession`
      (*sink)(batch.events());
      if (!*alive) return ok && changed;  // the sink destroyed the session
      batch.clear();
      drained_ = std::move(batch);  // keeps the slabs' capacity for the next batch
    }
    return ok && changed;
  }

  /// Embedded-component props (`NveComponentApi::set_props`), then pump.
  bool set_props(std::span<const NveMapEntry> props) {
    bool changed = false;
    if (runtime_.set_props(props.data(), static_cast<uint32_t>(props.size()), &changed) != NVE_RUNTIME_OK) {
      return false;
    }
    (void)pump();
    return true;
  }

  /// One `NVP1` Tier-1 hot patch (`NveDevApi::apply_hot_patch`), then pump.
  bool apply_hot_patch(std::span<const uint8_t> patch_nvp1) {
    bool changed = false;
    if (runtime_.apply_hot_patch(patch_nvp1, &changed) != NVE_RUNTIME_OK) return false;
    (void)pump();
    return changed;
  }

  /// Dispatches one event and pumps; `true` when the scene changed.
  bool dispatch(const NveDirectEvent& ev) {
    bool changed = false;
    if (runtime_.dispatch_events(NveEventBatch{&ev, 1u, 0u}, &changed) != NVE_RUNTIME_OK) return false;
    return pump() || changed;
  }

  /// The app's `NveDevApi::snapshot_state` bytes (empty when unsupported).
  [[nodiscard]] std::vector<uint8_t> snapshot_state() {
    std::span<const uint8_t> state;
    if (runtime_.snapshot_state(&state) != NVE_RUNTIME_OK) return {};
    return std::vector<uint8_t>(state.begin(), state.end());
  }

  /// Persists the app state to `path` tagged with the host's `schema_version` (runtime
  /// `save_state_file`); `false` (reason in `last_error`) when the app has no state or the write
  /// failed.
  bool save_state(std::string_view path, uint32_t schema_version) {
    return runtime_.save_state_file(path, schema_version) == NVE_RUNTIME_OK;
  }

  /// Restores state saved by `save_state` into the running app; `false` when there is no usable
  /// file (missing, corrupt -> quarantined, other schema) or the app takes no state.
  bool restore_state_file(std::string_view path, uint32_t schema_version) {
    return runtime_.restore_state_file(path, schema_version) == NVE_RUNTIME_OK;
  }

  /// Deletes the state file at `path` (cold start, e.g. `--reset-state`).
  bool clear_state_file(std::string_view path) { return runtime_.clear_state_file(path) == NVE_RUNTIME_OK; }

 private:
  HostSession(RuntimeSession rt, uint32_t surface_id) : runtime_(std::move(rt)) { events_.surface_id = surface_id; }

  bool load(std::string_view path, bool carry_state, ModuleError* out_error, std::string* out_message) {
    const ModuleError e = module_error(runtime_.load_app_module(path, carry_state));
    if (out_error != nullptr) *out_error = e;
    if (e != ModuleError::None) {
      if (out_message != nullptr) *out_message = runtime_.last_error();
      return false;
    }
    on_source_changed(true, true);
    return true;
  }

  /// Any successful source switch replaces the previous source, pairing included.
  void on_source_changed(bool attached, bool module, bool paired = false) noexcept {
    attached_ = attached;
    module_ = module;
    paired_ = paired;
  }

  static void on_wake(void* ctx) {
    auto* self = static_cast<HostSession*>(ctx);
    if (self->wake_) self->wake_();
  }

  RuntimeSession runtime_;  // owns the app and its staged module
  WakePoster wake_;
  std::shared_ptr<const EventSink> sink_;
  OwnedEvents drained_;
  EventFactory events_;
  /// `false` once the destructor ran; `pump` holds a copy across its sink call.
  std::shared_ptr<bool> alive_ = std::make_shared<bool>(true);
  bool attached_ = false;
  bool module_ = false;
  bool paired_ = false;  // a pairing worker runs (connected, connecting, or reconnecting)
};

}  // namespace nve::host
