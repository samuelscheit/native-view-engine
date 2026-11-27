// Test support shared by the native host suites (GTK, Qt, Windows): a C ABI app (`nve_get_interface`)
// that a test scripts directly. Ops written to `w` are flushed as one `NVE2` frame on the runtime's
// next render; dispatched events are recorded (text payloads copied) and handed to `on_event`, which
// may write more ops (e.g. echo a controlled input's value). Like any C ABI app it is rendered only
// after it accepts an event: tests push ops written outside `on_event` by dispatching
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
#include <vector>

#include <nve/host/nve2_writer.hpp>
#include <nve/generated/nve_abi.generated.h>

namespace nve::host::testing {

struct RecordedEvent {
  NveDirectEvent ev{};
  std::string text;  // owned copy of `ev.text`
};

class ScriptedApp {
 public:
  ScriptedApp() { slot() = this; }
  ~ScriptedApp() {
    if (slot() == this) slot() = nullptr;
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

  [[nodiscard]] static NveDirectEvent flush_event(uint32_t surface_id = 1u) {
    NveDirectEvent ev{};
    ev.surface_id = surface_id;
    ev.node_id = kFlushNode;
    ev.kind = NVE_EVENT_CUSTOM;
    ev.payload_tag = NVE_EVENT_PAYLOAD_EMPTY;
    return ev;
  }

  Nve2Writer w;
  std::vector<RecordedEvent> events;
  std::function<void(ScriptedApp&, const RecordedEvent&)> on_event;
  uint32_t creates = 0u;
  uint32_t destroys = 0u;
  uint32_t renders = 0u;  // non-empty frames produced

  [[nodiscard]] size_t count(NveEventKind kind, uint32_t node = 0u) const {
    size_t n = 0u;
    for (const RecordedEvent& e : events) n += e.ev.kind == kind && (node == 0u || e.ev.node_id == node) ? 1u : 0u;
    return n;
  }
  [[nodiscard]] const RecordedEvent* last(NveEventKind kind) const {
    for (auto it = events.rbegin(); it != events.rend(); ++it) {
      if (it->ev.kind == kind) return &*it;
    }
    return nullptr;
  }

 private:
  static NveAppHandle create(uint32_t) {
    ScriptedApp* self = slot();
    if (self != nullptr) ++self->creates;
    return self;
  }
  static void destroy(NveAppHandle h) {
    if (h != nullptr) ++static_cast<ScriptedApp*>(h)->destroys;
  }
  static NveBufferSlice render(NveAppHandle h, uint64_t frame_id) {
    auto* self = static_cast<ScriptedApp*>(h);
    if (self == nullptr || !self->w.pending()) return NveBufferSlice{nullptr, 0u};
    const std::span<const uint8_t> frame = self->w.finish(frame_id);
    ++self->renders;
    return NveBufferSlice{frame.data(), frame.size()};
  }
  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<ScriptedApp*>(h);
    if (self == nullptr) return 0u;
    for (uint32_t i = 0; batch.ptr != nullptr && i < batch.len; ++i) {
      if (batch.ptr[i].node_id == kFlushNode) continue;
      RecordedEvent rec{batch.ptr[i], {}};
      if (rec.ev.text.ptr != nullptr) rec.text.assign(reinterpret_cast<const char*>(rec.ev.text.ptr), rec.ev.text.len);
      rec.ev.text = NveStrSlice{nullptr, 0u};  // borrowed for the call only
      rec.ev.name = NveStrSlice{nullptr, 0u};
      self->events.push_back(rec);
      if (self->on_event) self->on_event(*self, self->events.back());
    }
    return self->w.pending() ? 1u : 0u;
  }
};

}  // namespace nve::host::testing
