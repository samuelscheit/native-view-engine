// Mock `nve_get_interface` app for the JVM JNI suites: renders real `NVE2` fixture frames
// (`nve-host-fixtures`), logs every dispatched `NveEvent` to `$NVE_MOCK_EVENT_LOG`, and implements
// `NVE_IFACE_COMPONENT`, `NVE_IFACE_DEV`, and `NVE_IFACE_HOST` so ABI v6 JNI bindings are tested
// end-to-end against real `libnve_runtime`.
//
// Build-time knobs: `NVE_MOCK_FRAME` (first frame), optional `NVE_MOCK_NEXT_FRAME` (rendered after
// the first dispatched batch), `NVE_MOCK_BAD_IFACE` (exports an interface getter returning null).
#include <nve/generated/nve_abi.generated.h>
#include <nve/host/bin_codec.hpp>
#include <nve/host/owned_events.hpp>

#include <cstdio>
#include <cstdlib>
#include <cstring>
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
  uint32_t surface_id = 1u;
  std::vector<uint8_t> first = load(NVE_MOCK_FRAME);
#if defined(NVE_MOCK_NEXT_FRAME)
  std::vector<uint8_t> next = load(NVE_MOCK_NEXT_FRAME);
#else
  std::vector<uint8_t> next;
#endif
  int pending = 1;  // 1: first frame due, 2: next frame due, 0: clean
  nve::host::OwnedEvents emitted;
  nve::host::OwnedEvents drained;
  std::vector<uint8_t> state{'i', 'n', 'i', 't'};
  NveHostCallbacks host_cb{};
  bool has_host_cb = false;
};

#if !defined(NVE_MOCK_NO_COMPONENT)
void write_nve_value(nve::host::BinWriter& w, const NveValue& v) {
  switch (v.tag) {
    case NVE_VAL_NULL:
      w.write_null();
      return;
    case NVE_VAL_BOOL:
      w.write_bool(v.data.bool_val != 0u);
      return;
    case NVE_VAL_I64:
      w.write_i64(v.data.i64_val);
      return;
    case NVE_VAL_F64:
      w.write_f64(v.data.f64_val);
      return;
    case NVE_VAL_STR:
      w.write_str(v.data.str_val.ptr != nullptr
                      ? std::string_view(reinterpret_cast<const char*>(v.data.str_val.ptr), v.data.str_val.len)
                      : std::string_view());
      return;
    case NVE_VAL_BYTES:
      w.write_bytes(v.data.bytes_val.ptr != nullptr
                        ? std::span<const uint8_t>(v.data.bytes_val.ptr, v.data.bytes_val.len)
                        : std::span<const uint8_t>());
      return;
    case NVE_VAL_ARRAY: {
      const auto m = w.begin_array(static_cast<uint32_t>(v.data.array_val.len));
      for (size_t i = 0u; i < v.data.array_val.len; ++i) write_nve_value(w, v.data.array_val.ptr[i]);
      w.end(m);
      return;
    }
    case NVE_VAL_MAP: {
      const auto m = w.begin_map(static_cast<uint32_t>(v.data.map_val.len));
      for (size_t i = 0u; i < v.data.map_val.len; ++i) {
        const NveMapEntry& e = v.data.map_val.ptr[i];
        w.write_map_key(e.key.ptr != nullptr
                            ? std::string_view(reinterpret_cast<const char*>(e.key.ptr), e.key.len)
                            : std::string_view());
        write_nve_value(w, e.value);
      }
      w.end(m);
      return;
    }
  }
  w.write_null();
}
#endif

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

NveAppHandle create(uint32_t surface_id) {
  auto* app = new MockApp();
  app->surface_id = surface_id;
  return app;
}

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
  for (uint32_t i = 0u; i < batch.event_count; ++i) {
    const NveEvent& ev = batch.events[i];
    log_event(ev, batch);
    if (nve::host::is_custom_event(ev)) {
      const std::string_view cname = nve::host::event_custom_name(ev, batch);
      const std::span<const uint8_t> cdata = nve::host::event_custom_data(ev, batch);
      if (cname == "host_call_sync" && app->has_host_cb && app->host_cb.invoke != nullptr) {
        uint8_t reply[256];
        size_t out_len = 0u;
        const NveBufferSlice req{cdata.empty() ? nullptr : cdata.data(), cdata.size()};
        const int32_t rc = app->host_cb.invoke(app->host_cb.host_ctx, 0x0010u, 1u, 101u, req, reply, sizeof reply,
                                               &out_len);
        nve::host::BinWriter w;
        const auto m = w.begin_map(2u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.write_map_key("reply");
        w.write_bytes(rc == NVE_HOST_OK && out_len <= sizeof reply ? std::span<const uint8_t>(reply, out_len)
                                                                   : std::span<const uint8_t>());
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostSyncReply", w.bytes());
      } else if (cname == "host_call_async" && app->has_host_cb && app->host_cb.invoke != nullptr) {
        uint8_t reply[16];
        size_t out_len = 0u;
        const NveBufferSlice req{cdata.empty() ? nullptr : cdata.data(), cdata.size()};
        const int32_t rc = app->host_cb.invoke(app->host_cb.host_ctx, 0x0010u, 2u, 202u, req, reply, sizeof reply,
                                               &out_len);
        nve::host::BinWriter w;
        const auto m = w.begin_map(1u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostAsyncStart", w.bytes());
      } else if (cname == "host_publish" && app->has_host_cb && app->host_cb.channel_publish != nullptr) {
        const NveBufferSlice pay{cdata.empty() ? nullptr : cdata.data(), cdata.size()};
        const int32_t rc = app->host_cb.channel_publish(app->host_cb.host_ctx, 42u, 1u, pay);
        nve::host::BinWriter w;
        const auto m = w.begin_map(1u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostPublish", w.bytes());
      } else if (cname == "host_call_small_buf" && app->has_host_cb && app->host_cb.invoke != nullptr) {
        uint8_t small_reply[4];
        size_t out_len = 0u;
        const NveBufferSlice req{cdata.empty() ? nullptr : cdata.data(), cdata.size()};
        const int32_t rc = app->host_cb.invoke(app->host_cb.host_ctx, 0x0010u, 3u, 303u, req, small_reply,
                                               sizeof small_reply, &out_len);
        nve::host::BinWriter w;
        const auto m = w.begin_map(2u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.write_map_key("outLen");
        w.write_i64(static_cast<int64_t>(out_len));
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostSmallBuf", w.bytes());
      } else if (cname == "host_call_null_reply" && app->has_host_cb && app->host_cb.invoke != nullptr) {
        uint8_t reply[64];
        size_t out_len = 999u;
        const NveBufferSlice req{cdata.empty() ? nullptr : cdata.data(), cdata.size()};
        const int32_t rc =
            app->host_cb.invoke(app->host_cb.host_ctx, 0x0010u, 4u, 404u, req, reply, sizeof reply, &out_len);
        nve::host::BinWriter w;
        const auto m = w.begin_map(2u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.write_map_key("outLen");
        w.write_i64(static_cast<int64_t>(out_len));
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostNullReply", w.bytes());
      } else if (cname == "host_call_throw" && app->has_host_cb && app->host_cb.invoke != nullptr) {
        uint8_t reply[64];
        size_t out_len = 0u;
        const NveBufferSlice req{cdata.empty() ? nullptr : cdata.data(), cdata.size()};
        const int32_t rc =
            app->host_cb.invoke(app->host_cb.host_ctx, 0x0010u, 5u, 505u, req, reply, sizeof reply, &out_len);
        nve::host::BinWriter w;
        const auto m = w.begin_map(1u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostThrow", w.bytes());
      } else if (cname == "host_publish_throw" && app->has_host_cb && app->host_cb.channel_publish != nullptr) {
        const NveBufferSlice pay{cdata.empty() ? nullptr : cdata.data(), cdata.size()};
        const int32_t rc = app->host_cb.channel_publish(app->host_cb.host_ctx, 99u, 0u, pay);
        nve::host::BinWriter w;
        const auto m = w.begin_map(1u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostPublishThrow", w.bytes());
      } else if (cname == "host_call_null_ptr" && app->has_host_cb && app->host_cb.invoke != nullptr) {
        uint8_t reply[64];
        size_t out_len = 0u;
        const NveBufferSlice bad_req{nullptr, 8u};
        const int32_t rc =
            app->host_cb.invoke(app->host_cb.host_ctx, 0x0010u, 1u, 606u, bad_req, reply, sizeof reply, &out_len);
        nve::host::BinWriter w;
        const auto m = w.begin_map(1u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostNullPtr", w.bytes());
      } else if (cname == "host_publish_null_ptr" && app->has_host_cb && app->host_cb.channel_publish != nullptr) {
        const NveBufferSlice bad_pay{nullptr, 8u};
        const int32_t rc = app->host_cb.channel_publish(app->host_cb.host_ctx, 42u, 0u, bad_pay);
        nve::host::BinWriter w;
        const auto m = w.begin_map(1u);
        w.write_map_key("status");
        w.write_i64(rc);
        w.end(m);
        (void)app->emitted.push_custom(app->surface_id, 0u, ev.timestamp, "onHostPublishNullPtr", w.bytes());
      }
    }
  }
  if (batch.event_count == 0u || app->next.empty()) return 0u;
  app->pending = 2;
  return 1u;
}

#if !defined(NVE_MOCK_NO_COMPONENT)
uint32_t set_props(NveAppHandle h, const NveMapEntry* entries, uint32_t count) {
  auto* app = static_cast<MockApp*>(h);
  nve::host::BinWriter w;
  const auto m = w.begin_map(count);
  for (uint32_t i = 0u; i < count; ++i) {
    const NveMapEntry& e = entries[i];
    w.write_map_key(e.key.ptr != nullptr ? std::string_view(reinterpret_cast<const char*>(e.key.ptr), e.key.len)
                                         : std::string_view());
    write_nve_value(w, e.value);
  }
  w.end(m);
  (void)app->emitted.push_custom(app->surface_id, 0u, 99u, "onPropsEcho", w.bytes());
  if (!app->next.empty()) app->pending = 2;
  return 1u;
}

NveEventBatch drain_emitted_events(NveAppHandle h) {
  auto* app = static_cast<MockApp*>(h);
  app->drained.clear();
  app->drained.swap(app->emitted);
  return app->drained.batch();
}
#endif

uint32_t apply_hot_patch(NveAppHandle h, NveBufferSlice patch_nvp1) {
  auto* app = static_cast<MockApp*>(h);
  nve::host::BinWriter w;
  const auto m = w.begin_map(1u);
  w.write_map_key("patchBytes");
  w.write_i64(static_cast<int64_t>(patch_nvp1.len));
  w.end(m);
  (void)app->emitted.push_custom(app->surface_id, 0u, 100u, "onHotPatch", w.bytes());
  if (!app->next.empty()) app->pending = 2;
  return 1u;
}

NveBufferSlice snapshot_state(NveAppHandle h) {
  auto* app = static_cast<MockApp*>(h);
  return NveBufferSlice{app->state.empty() ? nullptr : app->state.data(), app->state.size()};
}

void restore_state(NveAppHandle h, NveBufferSlice state) {
  auto* app = static_cast<MockApp*>(h);
  app->state.clear();
  if (state.ptr != nullptr && state.len > 0u) app->state.assign(state.ptr, state.ptr + state.len);
  if (!app->next.empty()) app->pending = 2;
}

void bind_host(NveAppHandle h, const NveHostCallbacks* callbacks) {
  auto* app = static_cast<MockApp*>(h);
  if (callbacks == nullptr) {
    app->host_cb = NveHostCallbacks{};
    app->has_host_cb = false;
  } else {
    app->host_cb = *callbacks;
    app->has_host_cb = true;
  }
}

uint32_t dispatch_host_events(NveAppHandle h, NveBufferSlice batch) {
  auto* app = static_cast<MockApp*>(h);
  nve::host::BinWriter w;
  const auto m = w.begin_map(1u);
  w.write_map_key("batchBytes");
  w.write_i64(static_cast<int64_t>(batch.len));
  w.end(m);
  (void)app->emitted.push_custom(app->surface_id, 0u, 101u, "onHostEvents", w.bytes());
  return 1u;
}

#if defined(NVE_MOCK_WITH_LAYOUT)
NveIntrinsicSize mock_measure(NveAppHandle, NveSizeConstraints c) {
  const float w = c.max_w < 123.5f ? c.max_w : 123.5f;
  const float h = c.max_h < 45.25f ? c.max_h : 45.25f;
  return NveIntrinsicSize{w < c.min_w ? c.min_w : w, h < c.min_h ? c.min_h : h};
}

uint8_t mock_hit_test(NveAppHandle, float, float, uint8_t) { return NVE_GESTURE_PASS_THROUGH; }

constexpr NveLayoutApi kLayout{&mock_measure, &mock_hit_test};
#endif

constexpr NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch};
#if !defined(NVE_MOCK_NO_COMPONENT)
constexpr NveComponentApi kComponent{&set_props, &drain_emitted_events};
#endif
constexpr NveDevApi kDev{&apply_hot_patch, &snapshot_state, &restore_state};
constexpr NveHostApi kHost{&bind_host, &dispatch_host_events};

}  // namespace

extern "C" __attribute__((visibility("default"))) const void* nve_get_interface(uint32_t id) {
  if (id == NVE_IFACE_SURFACE) return &kSurface;
#if defined(NVE_MOCK_WITH_LAYOUT)
  if (id == NVE_IFACE_LAYOUT) return &kLayout;
#endif
#if !defined(NVE_MOCK_NO_COMPONENT)
  if (id == NVE_IFACE_COMPONENT) return &kComponent;
#endif
  if (id == NVE_IFACE_DEV) return &kDev;
  if (id == NVE_IFACE_HOST) return &kHost;
  return nullptr;
}

#endif
