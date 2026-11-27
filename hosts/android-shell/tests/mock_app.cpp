// Mock `nve_get_interface` app for the JVM JNI suites: renders real `NVE2` fixture frames
// (`nve-host-fixtures`) and logs every dispatched `NveDirectEvent` to `$NVE_MOCK_EVENT_LOG`.
//
// Build-time knobs: `NVE_MOCK_FRAME` (first frame), optional `NVE_MOCK_NEXT_FRAME` (rendered after
// the first dispatched batch), `NVE_MOCK_BAD_IFACE` (exports an interface getter returning null).
#include <nve/generated/nve_abi.generated.h>

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

void log_event(const NveDirectEvent& ev) {
  const char* path = std::getenv("NVE_MOCK_EVENT_LOG");
  if (path == nullptr) return;
  if (FILE* f = std::fopen(path, "a")) {
    const size_t value_len = ev.value.tag == NVE_VAL_BYTES ? ev.value.data.bytes_val.len : 0u;
    const unsigned long long i64 = ev.value.tag == NVE_VAL_I64 ? static_cast<unsigned long long>(ev.value.data.i64_val) : 0u;
    std::fprintf(f, "surface=%u node=%u ts=%llu kind=%u tag=%u mods=%u flags=%u key=%u x=%g y=%g w=%g h=%g text=%s name=%s value=%zu i64=%llx\n",
                 ev.surface_id, ev.node_id, static_cast<unsigned long long>(ev.timestamp_us), ev.kind,
                 ev.payload_tag, ev.modifiers, ev.event_flags, ev.key_code, ev.x, ev.y, ev.width, ev.height,
                 slice(ev.text.ptr, ev.text.len).c_str(), slice(ev.name.ptr, ev.name.len).c_str(), value_len, i64);
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
  for (uint32_t i = 0u; i < batch.len; ++i) log_event(batch.ptr[i]);
  if (batch.len == 0u || app->next.empty()) return 0u;
  app->pending = 2;
  return 1u;
}

constexpr NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch};

}  // namespace

extern "C" __attribute__((visibility("default"))) const void* nve_get_interface(uint32_t id) {
  return id == NVE_IFACE_SURFACE ? &kSurface : nullptr;
}

#endif
