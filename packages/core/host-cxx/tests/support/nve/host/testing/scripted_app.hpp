// Test support shared by the native host suites (GTK, Qt, Windows): a C ABI app (`nve_get_interface`)
// that a test scripts directly. Ops written to `w` are flushed as one `NVE2` frame on the runtime's
// next render; dispatched events are recorded (companion arena bytes copied) and handed to `on_event`,
// which may write more ops (e.g. echo a controlled input's value). Like any C ABI app it is rendered
// only after it accepts an event: tests push ops written outside `on_event` by dispatching
// `flush_event()` (not recorded).
//
//   ScriptedApp app;                   // must outlive the attached runtime surface
//   app.w.create(1); ...               // first frame: ROOT + tree
//   host.attach_interface(&ScriptedApp::get_interface);  // binds `ScriptedApp::current()`
#pragma once

#include <cstdint>
#include <functional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include <nve/generated/nve_abi.generated.h>
#include <nve/host/nve2_writer.hpp>
#include <nve/host/owned_events.hpp>

namespace nve::host::testing {

struct RecordedEvent {
  NveEvent ev{};
  std::vector<uint8_t> bytes;  // owned copy of `event_bytes(ev, batch)`
  std::string text;            // owned copy of `event_text(ev, batch)`

  [[nodiscard]] NveEventBatch single_batch() const noexcept {
    return NveEventBatch{&ev, bytes.empty() ? nullptr : bytes.data(), 1u, static_cast<uint32_t>(bytes.size())};
  }
  [[nodiscard]] std::string_view key() const noexcept { return event_key(ev, single_batch()); }
  [[nodiscard]] std::string_view code() const noexcept { return event_code(ev, single_batch()); }
  [[nodiscard]] std::string_view custom_name() const noexcept { return event_custom_name(ev, single_batch()); }
  [[nodiscard]] std::span<const uint8_t> custom_data() const noexcept { return event_custom_data(ev, single_batch()); }
};

class ScriptedApp {
 public:
  ScriptedApp() { slot() = this; }
  ~ScriptedApp() {
    if (slot() == this) slot() = nullptr;
    // The runtime may destroy its app handle after this object (e.g. a host declared first):
    // detach every live handle so later callbacks see no app.
    for (Instance* inst : instances_) inst->app = nullptr;
  }
  ScriptedApp(const ScriptedApp&) = delete;
  ScriptedApp& operator=(const ScriptedApp&) = delete;

  /// The instance the next `create` binds (the most recently constructed live one).
  [[nodiscard]] static ScriptedApp*& slot() {
    static ScriptedApp* current = nullptr;
    return current;
  }

  static const void* get_interface(uint32_t interface_id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    return interface_id == NVE_IFACE_SURFACE ? &kSurface : nullptr;
  }

  /// Node id of the unrecorded "render my pending ops" event.
  static constexpr uint32_t kFlushNode = 0x00FFFFFFu;

  [[nodiscard]] static NveEvent flush_event(uint32_t surface_id = 1u) noexcept {
    NveEvent ev{};
    ev.table = NVE_TABLE_CUSTOM;
    ev.surface = surface_id;
    ev.node = kFlushNode;
    return ev;
  }

  Nve2Writer w;
  std::vector<RecordedEvent> events;
  std::function<void(ScriptedApp&, const RecordedEvent&)> on_event;
  uint32_t creates = 0u;
  uint32_t destroys = 0u;
  uint32_t renders = 0u;  // non-empty frames produced

  [[nodiscard]] size_t count(uint64_t table, uint32_t event_id, uint32_t node = 0u) const noexcept {
    size_t n = 0u;
    for (const RecordedEvent& e : events) {
      n += e.ev.table == table && e.ev.event == event_id && (node == 0u || e.ev.node == node) ? 1u : 0u;
    }
    return n;
  }
  [[nodiscard]] size_t count(NveInteractionEvent kind, uint32_t node = 0u) const noexcept {
    return count(NVE_TABLE_INTERACTION, static_cast<uint32_t>(kind), node);
  }
  [[nodiscard]] size_t count(NveTextInputEvent kind, uint32_t node = 0u) const noexcept {
    return count(NVE_TABLE_TEXT_INPUT, static_cast<uint32_t>(kind), node);
  }
  [[nodiscard]] size_t count(NveScrollEvent kind, uint32_t node = 0u) const noexcept {
    return count(NVE_TABLE_SCROLL, static_cast<uint32_t>(kind), node);
  }
  [[nodiscard]] size_t count(NveLayoutEvent kind, uint32_t node = 0u) const noexcept {
    return count(NVE_TABLE_LAYOUT, static_cast<uint32_t>(kind), node);
  }

  [[nodiscard]] const RecordedEvent* last(uint64_t table, uint32_t event_id) const noexcept {
    for (auto it = events.rbegin(); it != events.rend(); ++it) {
      if (it->ev.table == table && it->ev.event == event_id) return &*it;
    }
    return nullptr;
  }
  [[nodiscard]] const RecordedEvent* last(NveInteractionEvent kind) const noexcept {
    return last(NVE_TABLE_INTERACTION, static_cast<uint32_t>(kind));
  }
  [[nodiscard]] const RecordedEvent* last(NveTextInputEvent kind) const noexcept {
    return last(NVE_TABLE_TEXT_INPUT, static_cast<uint32_t>(kind));
  }
  [[nodiscard]] const RecordedEvent* last(NveScrollEvent kind) const noexcept {
    return last(NVE_TABLE_SCROLL, static_cast<uint32_t>(kind));
  }
  [[nodiscard]] const RecordedEvent* last(NveLayoutEvent kind) const noexcept {
    return last(NVE_TABLE_LAYOUT, static_cast<uint32_t>(kind));
  }

 private:
  /// One runtime app handle: owned by the runtime, pointing at its (possibly gone) app.
  struct Instance {
    ScriptedApp* app;
  };

  [[nodiscard]] static ScriptedApp* app_of(NveAppHandle h) noexcept {
    return h != nullptr ? static_cast<Instance*>(h)->app : nullptr;
  }

  static NveAppHandle create(uint32_t) {
    ScriptedApp* self = slot();
    if (self == nullptr) return nullptr;
    ++self->creates;
    self->instances_.push_back(new Instance{self});
    return self->instances_.back();
  }
  static void destroy(NveAppHandle h) {
    auto* inst = static_cast<Instance*>(h);
    if (inst == nullptr) return;
    if (ScriptedApp* self = inst->app) {
      ++self->destroys;
      std::erase(self->instances_, inst);
    }
    delete inst;
  }
  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    ScriptedApp* self = app_of(h);
    if (self == nullptr || !self->w.pending()) return NveBufferSlice{nullptr, 0u};
    const std::span<const uint8_t> frame = self->w.finish(frame_id);
    ++self->renders;
    return NveBufferSlice{frame.data(), frame.size()};
  }
  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    ScriptedApp* self = app_of(h);
    if (self == nullptr) return 0u;
    for (uint32_t i = 0u; batch.events != nullptr && i < batch.event_count; ++i) {
      const NveEvent& raw = batch.events[i];
      if (raw.node == kFlushNode) continue;
      const std::span<const uint8_t> b = event_bytes(raw, batch);
      RecordedEvent rec{};
      rec.ev = raw;
      rec.bytes.assign(b.begin(), b.end());
      rec.ev.bytes = RawEventBytes{0u, static_cast<uint32_t>(rec.bytes.size())};
      rec.text = std::string(event_text(raw, batch));
      self->events.push_back(std::move(rec));
      if (self->on_event) self->on_event(*self, self->events.back());
    }
    return self->w.pending() ? 1u : 0u;
  }

  std::vector<Instance*> instances_;
};

}  // namespace nve::host::testing
