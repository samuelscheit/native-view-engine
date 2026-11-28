// Mock `nve_get_interface` app for the JVM JNI suites: renders real `NVE2` fixture frames
// (`nve-host-fixtures`) and logs every dispatched `NveEvent` to `$NVE_MOCK_EVENT_LOG`.
//
// Build-time knobs: `NVE_MOCK_FRAME` (first frame), optional `NVE_MOCK_NEXT_FRAME` (rendered after
// the first dispatched batch), `NVE_MOCK_BAD_IFACE` (exports an interface getter returning null).
#include <nve/generated/nve_abi.generated.h>
#include <nve/host/owned_events.hpp>

#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

#if defined(NVE_MOCK_BAD_IFACE)

extern "C" __attribute__((visibility("default"))) const void* nve_get_interface(uint32_t) { return nullptr; }

#else

namespace {

std::vector<uint8_t> load(const char* path) {
  std::ifstream in(path, std::ios::binary);
  return std::vector<uint8_t>(std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>());
}

struct MockApp {
  std::vector<uint8_t> first = load(NVE_MOCK_FRAME);
#if defined(NVE_MOCK_NEXT_FRAME)
  std::vector<uint8_t> next = load(NVE_MOCK_NEXT_FRAME);
#else
  std::vector<uint8_t> next;
#endif
  int pending = 1;  // 1: first frame due, 2: next frame due, 0: clean
};

/// Printable copy of borrowed bytes (`\xNN` for control bytes, so binary table payloads stay on one line).
std::string slice(const uint8_t* p, size_t n) {
  std::string out;
  for (size_t i = 0u; p != nullptr && i < n; ++i) {
    if (p[i] < 0x20u || p[i] == 0x7Fu) {
      char hex[5];
      std::snprintf(hex, sizeof hex, "\\x%02X", p[i]);
      out += hex;
    } else {
      out.push_back(static_cast<char>(p[i]));
    }
  }
  return out;
}

void log_event(const NveEvent& ev, NveEventBatch batch) {
  const char* path = std::getenv("NVE_MOCK_EVENT_LOG");
  if (path == nullptr) return;
  if (FILE* f = std::fopen(path, "a")) {
    const std::string_view text = nve::host::event_text(ev, batch);
    const std::string_view key = nve::host::event_key(ev, batch);
    const std::string_view code = nve::host::event_code(ev, batch);
    const std::string_view cname = nve::host::event_custom_name(ev, batch);
    const std::span<const uint8_t> cdata = nve::host::event_custom_data(ev, batch);
    const std::span<const uint8_t> raw = nve::host::event_bytes(ev, batch);
    std::fprintf(
        f,
        "surface=%u node=%u ts=%llu table=%016llx event=%u flags=%u "
        "p0=%u p1=%u p2=%u f0=%g f1=%g f2=%g f3=%g "
        "text=%s key=%s code=%s cname=%s cdata=%zu bytes=%s\n",
        ev.surface, ev.node, static_cast<unsigned long long>(ev.timestamp),
        static_cast<unsigned long long>(ev.table), ev.event, ev.flags, ev.payload[0], ev.payload[1], ev.payload[2],
        nve::host::event_payload_f32(ev, 0u), nve::host::event_payload_f32(ev, 1u),
        nve::host::event_payload_f32(ev, 2u), nve::host::event_payload_f32(ev, 3u),
        slice(reinterpret_cast<const uint8_t*>(text.data()), text.size()).c_str(),
        slice(reinterpret_cast<const uint8_t*>(key.data()), key.size()).c_str(),
        slice(reinterpret_cast<const uint8_t*>(code.data()), code.size()).c_str(),
        slice(reinterpret_cast<const uint8_t*>(cname.data()), cname.size()).c_str(), cdata.size(),
        slice(raw.data(), raw.size()).c_str());
    std::fclose(f);
  }
}

NveAppHandle create(uint32_t) { return new MockApp(); }

void destroy(NveAppHandle h) { delete static_cast<MockApp*>(h); }

NveBufferSlice render(NveAppHandle h, uint64_t) {
  auto* app = static_cast<MockApp*>(h);
  const std::vector<uint8_t>* frame = app->pending == 1 ? &app->first : app->pending == 2 ? &app->next : nullptr;
  app->pending = 0;
  if (frame == nullptr || frame->empty()) return NveBufferSlice{nullptr, 0u};
  return NveBufferSlice{frame->data(), frame->size()};
}

uint32_t dispatch(NveAppHandle h, NveEventBatch batch) {
  auto* app = static_cast<MockApp*>(h);
  for (uint32_t i = 0u; i < batch.event_count; ++i) log_event(batch.events[i], batch);
  if (batch.event_count == 0u || app->next.empty()) return 0u;
  app->pending = 2;
  return 1u;
}

constexpr NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch};

}  // namespace

extern "C" __attribute__((visibility("default"))) const void* nve_get_interface(uint32_t id) {
  return id == NVE_IFACE_SURFACE ? &kSurface : nullptr;
}

#endif
