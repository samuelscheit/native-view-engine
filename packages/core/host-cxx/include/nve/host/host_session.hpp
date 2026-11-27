// NVE host runtime: one native host surface over `libnve_runtime`.
//
// `HostSession` owns a `RuntimeSession`, the app it runs (an in-process `nve_get_interface`, a
// loaded `cdylib` with state-preserving hot reload, or a paired remote via `pair_connect`), the
// wake marshaling to the host's UI thread, typed input-event dispatch, and app-state persistence.
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

#include "app_state.hpp"
#include "module_loader.hpp"
#include "../generated/nve_abi.generated.h"
#include "runtime_session.hpp"

namespace nve::host {

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
    (void)runtime_.clear_wake_callback();  // drains in-flight wakes before `wake_` dies
    (void)runtime_.detach_app();
    runtime_.reset();  // the runtime drops the app before its module unloads
  }
  HostSession(const HostSession&) = delete;
  HostSession& operator=(const HostSession&) = delete;

  [[nodiscard]] RuntimeSession& runtime() noexcept { return runtime_; }
  [[nodiscard]] const RuntimeSession& runtime() const noexcept { return runtime_; }
  [[nodiscard]] const EventFactory& events() const noexcept { return events_; }
  [[nodiscard]] uint32_t surface_id() const noexcept { return events_.surface_id; }
  [[nodiscard]] bool has_app() const noexcept { return attached_ || runtime_.pair_is_connected(); }
  [[nodiscard]] bool has_module() const noexcept { return module_.loaded(); }
  /// Bumped whenever the scene's source changes (attach, module load / reload, pair, unpair): the
  /// runtime then replaces the scene without removals, so `RetainedTree::sync` takes it to run a
  /// full pass.
  [[nodiscard]] uint64_t source_generation() const noexcept { return source_generation_; }
  [[nodiscard]] std::string last_error() const { return runtime_.last_error(); }

  /// Installs (or with `{}` removes) the UI-thread wake poster.
  void set_wake_poster(WakePoster poster) {
    (void)runtime_.clear_wake_callback();
    wake_ = std::move(poster);
    if (wake_) (void)runtime_.set_wake_callback(&HostSession::on_wake, this);
  }

  void set_event_sink(EventSink sink) { sink_ = std::move(sink); }

  /// Runs an in-process app (`nve_get_interface` linked into the host).
  bool attach(NveGetInterfaceFn get_interface) {
    if (get_interface == nullptr) return false;
    // A successful attach replaces any paired source; a failed one leaves everything running.
    if (runtime_.attach_app_interface(get_interface) != NVE_RUNTIME_OK) return false;
    module_.close();  // the previous module's app is gone
    attached_ = true;
    ++source_generation_;
    return true;
  }

  /// Loads and runs an app `cdylib`.
  bool load_module(std::string_view path, ModuleError* out_error = nullptr, std::string* out_message = nullptr) {
    AppModule next = AppModule::open(path, out_error, out_message);
    if (!next.loaded()) return false;
    if (runtime_.attach_app_interface(next.get_interface()) != NVE_RUNTIME_OK) {
      if (out_error != nullptr) *out_error = ModuleError::Attach;
      if (out_message != nullptr) *out_message = runtime_.last_error();
      return false;
    }
    module_ = std::move(next);
    attached_ = true;
    ++source_generation_;
    return true;
  }

  /// Hot-reloads the app `cdylib`, carrying its `NveDevApi` state across the swap.
  bool reload_module(std::string_view path, ModuleError* out_error = nullptr, std::string* out_message = nullptr) {
    std::vector<uint8_t> state = snapshot_state();
    if (!load_module(path, out_error, out_message)) return false;
    if (!state.empty()) (void)runtime_.restore_state(state);
    return true;
  }

  /// Thin-client mode: frames come from a remote `nve dev` server, input goes upstream.
  bool pair(std::string_view uri, std::string_view token, std::string* out_error = nullptr) {
    if (runtime_.pair_connect(uri, token) != NVE_RUNTIME_OK) {
      if (out_error != nullptr) *out_error = runtime_.last_error();
      return false;
    }
    module_.close();
    attached_ = false;
    ++source_generation_;
    return true;
  }

  /// Leaves paired mode (no-op when not paired).
  void unpair() {
    if (!runtime_.pair_is_connected()) return;
    (void)runtime_.pair_disconnect();
    ++source_generation_;
  }

  /// Advances the runtime (renders the app / applies paired frames) and forwards the app's
  /// outbound events; `true` when the scene changed.
  bool pump(float dt_s = 0.0f) {
    bool changed = false;
    const bool ok = runtime_.tick(dt_s, &changed) == NVE_RUNTIME_OK;
    NveEventBatch out{};
    if (sink_ && runtime_.drain_emitted_events(&out) == NVE_RUNTIME_OK && out.ptr != nullptr && out.len != 0u) {
      sink_(std::span<const NveDirectEvent>(out.ptr, out.len));
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

  /// Atomically persists the app state to `path` tagged with the host's `schema_version`
  /// (`app_state::write_file`); `false` when the app has no state or the write failed.
  bool save_state(const std::string& path, uint32_t schema_version) {
    const std::vector<uint8_t> state = snapshot_state();
    return !state.empty() && app_state::write_file(path, schema_version, state);
  }

  /// Restores state saved by `save_state` into the running app (`app_state::read_file` rules).
  bool restore_state_file(const std::string& path, uint32_t schema_version) {
    const std::optional<std::vector<uint8_t>> state = app_state::read_file(path, schema_version);
    return state && runtime_.restore_state(*state) == NVE_RUNTIME_OK;
  }

 private:
  HostSession(RuntimeSession rt, uint32_t surface_id) : runtime_(std::move(rt)) { events_.surface_id = surface_id; }

  static void on_wake(void* ctx) {
    auto* self = static_cast<HostSession*>(ctx);
    if (self->wake_) self->wake_();
  }

  RuntimeSession runtime_;  // reset in the destructor body, so `module_` outlives its app
  AppModule module_;
  WakePoster wake_;
  EventSink sink_;
  EventFactory events_;
  uint64_t source_generation_ = 0u;
  bool attached_ = false;
};

}  // namespace nve::host
