// `OwnedEvents` (64-byte-aligned `NveEvent` + flat byte arena + `NVEV` v2 wire codec) and
// `HostSession::pump`'s outbound event path: deep copy + out-of-bounds span sanitization, inline
// and split UTF-8 key packing, custom events, `NVEV` v2 wire round-trip and rejection of malformed
// frames, zero C++ allocations (counting both default and C++17 aligned `operator new`) for a
// reused buffer and for a steady-state `pump`, and a sink that destroys its session under ASan.
#include <algorithm>
#include <array>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <span>
#include <string>
#include <string_view>
#include <type_traits>
#include <vector>

#include "check.h"
#include "nve/host/bin_codec.hpp"
#include "nve/host/host_session.hpp"
#include "nve/host/owned_events.hpp"

// --- Counting global allocator (this test binary only; counts both default and aligned new) -----
namespace {
thread_local bool g_counting = false;
thread_local size_t g_allocs = 0u;

void* aligned_alloc_compat(size_t align, size_t n) {
  const size_t a = align < sizeof(void*) ? sizeof(void*) : align;
  const size_t sz = n != 0u ? n : a;
#if defined(_WIN32)
  return _aligned_malloc(sz, a);
#else
  void* p = nullptr;
  return posix_memalign(&p, a, sz) == 0 ? p : nullptr;
#endif
}
void aligned_free_compat(void* p) noexcept {
#if defined(_WIN32)
  _aligned_free(p);
#else
  std::free(p);
#endif
}
}  // namespace

void* operator new(std::size_t n) {
  if (g_counting) ++g_allocs;
  if (void* p = std::malloc(n != 0u ? n : 1u)) return p;
  throw std::bad_alloc();
}
void operator delete(void* p) noexcept { std::free(p); }
void operator delete(void* p, std::size_t) noexcept { std::free(p); }
void* operator new(std::size_t n, std::align_val_t al) {
  if (g_counting) ++g_allocs;
  if (void* p = aligned_alloc_compat(static_cast<size_t>(al), n)) return p;
  throw std::bad_alloc();
}
void operator delete(void* p, std::align_val_t) noexcept { aligned_free_compat(p); }
void operator delete(void* p, std::size_t, std::align_val_t) noexcept { aligned_free_compat(p); }

namespace {

using namespace nve::host;

/// C++ allocations made while running `fn`.
template <class Fn>
size_t allocations(Fn&& fn) {
  g_allocs = 0u;
  g_counting = true;
  fn();
  g_counting = false;
  return g_allocs;
}

void test_owned_events_builders_and_copy() {
  OwnedEvents src;
  CHECK(src.push_press(1u, 7u, 100u, 12.5f, 24.0f));
  CHECK(src.push_key(1u, 8u, 101u, NVE_INTERACTION_EVENT_KEY_DOWN, "ArrowUp", "ArrowUp",
                     NVE_EVENT_FLAG_MOD_SHIFT, 38u));
  // > 16 B key exercises the `KEY_SPLIT_BYTES` arena path with multi-byte UTF-8.
  const std::string long_key = "MediaTrackNext\xC3\xA9""123";
  CHECK(long_key.size() > NVE_EVENT_INLINE_KEY_CAP);
  CHECK(src.push_key(1u, 9u, 102u, NVE_INTERACTION_EVENT_KEY_UP, long_key, "MediaTrackNext",
                     NVE_EVENT_FLAG_MOD_META, 176u));
  CHECK(src.push_text_change(1u, 10u, 103u, "h\xC3\xA9llo"));
  const std::array<uint8_t, 4> payload{9u, 8u, 7u, 6u};
  CHECK(src.push_custom(1u, 11u, 104u, "analytics.click", payload));
  // Invalid UTF-8 is rejected before touching the slabs.
  CHECK(!src.push_text_change(1u, 10u, 105u, std::string_view("\xFF", 1u)));
  CHECK(!src.push_key(1u, 10u, 106u, NVE_INTERACTION_EVENT_KEY_DOWN, std::string_view("\xC0\xAF", 2u)));
  CHECK(!src.push_custom(1u, 10u, 107u, std::string_view("\xED\xA0\x80", 3u), payload));

  const NveEventBatch src_batch = src.batch();
  CHECK(validate_event_batch(src_batch));
  CHECK(reinterpret_cast<uintptr_t>(src_batch.events) % NVE_EVENT_ALIGN == 0u);

  OwnedEvents owned;
  owned.assign(src_batch);
  src.clear();  // owned must not alias `src`

  const auto check = [&](const OwnedEvents& o) {
    const NveEventBatch b = o.batch();
    CHECK(o.size() == 5u && validate_event_batch(b));
    CHECK(reinterpret_cast<uintptr_t>(b.events) % NVE_EVENT_ALIGN == 0u);
    CHECK(is_interaction_event(b.events[0], NVE_INTERACTION_EVENT_PRESS));
    CHECK(event_payload_f32(b.events[0], 0u) == 12.5f && event_payload_f32(b.events[0], 1u) == 24.0f);
    CHECK(is_interaction_event(b.events[1], NVE_INTERACTION_EVENT_KEY_DOWN));
    CHECK((b.events[1].flags & NVE_EVENT_FLAG_INLINE_KEY_UTF8) != 0u);
    CHECK(event_key(b.events[1], b) == "ArrowUp" && event_code(b.events[1], b) == "ArrowUp");
    CHECK(is_interaction_event(b.events[2], NVE_INTERACTION_EVENT_KEY_UP));
    CHECK((b.events[2].flags & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) != 0u);
    CHECK(event_key(b.events[2], b) == long_key && event_code(b.events[2], b) == "MediaTrackNext");
    CHECK(is_text_input_event(b.events[3], NVE_TEXT_INPUT_EVENT_CHANGE));
    CHECK(event_text(b.events[3], b) == "h\xC3\xA9llo");
    CHECK(is_custom_event(b.events[4]) && event_custom_name(b.events[4], b) == "analytics.click");
    const std::span<const uint8_t> d = event_custom_data(b.events[4], b);
    CHECK(d.size() == 4u && d[0] == 9u && d[3] == 6u);
  };
  check(owned);

  // Moves keep the exact 64-byte-aligned slab pointer.
  const NveEvent* before = owned.events().data();
  OwnedEvents moved(std::move(owned));
  CHECK(moved.events().data() == before);
  check(moved);
  OwnedEvents assigned;
  assigned = std::move(moved);
  check(assigned);
  static_assert(std::is_nothrow_move_constructible_v<OwnedEvents> && std::is_nothrow_move_assignable_v<OwnedEvents>);

  // Hostile batch: NULL with non-zero counts, or out-of-bounds event byte span -> sanitized.
  assigned.assign(NveEventBatch{nullptr, nullptr, 5u, 9u});
  CHECK(assigned.empty());
  alignas(NVE_EVENT_ALIGN) NveEvent oob{};
  oob.table = NVE_TABLE_TEXT_INPUT;
  oob.flags = NVE_EVENT_FLAG_UTF8_BYTES;
  oob.bytes = RawEventBytes{4u, 20u};
  const std::array<uint8_t, 6> small_arena{'a', 'b', 'c', 'd', 'e', 'f'};
  assigned.assign(NveEventBatch{&oob, small_arena.data(), 1u, static_cast<uint32_t>(small_arena.size())});
  CHECK(assigned.size() == 1u && assigned.events()[0].bytes.len == 0u &&
        event_text(assigned.events()[0], assigned.batch()).empty());
}

/// `NVEV` v2 wire round-trip and hostile-frame rejection (magic, version, truncation, trailing,
/// conflicting flags, invalid UTF-8 in inline/split/custom/text payloads).
void test_nvev_v2_wire_codec() {
  OwnedEvents src;
  CHECK(src.push_press(2u, 3u, 55u, 1.0f, 2.0f));
  CHECK(src.push_key(2u, 4u, 56u, NVE_INTERACTION_EVENT_KEY_DOWN, "Enter", "Enter", 0u, 13u));
  CHECK(src.push_key(2u, 5u, 57u, NVE_INTERACTION_EVENT_KEY_UP, "VeryLongCustomKeyName", "CodeX", 0u, 0u));
  CHECK(src.push_text_change(2u, 6u, 58u, "caf\xC3\xA9"));
  const std::array<uint8_t, 3> raw_data{1u, 2u, 3u};
  CHECK(src.push_custom(2u, 7u, 59u, "ping", raw_data));

  const std::vector<uint8_t> wire = src.to_wire_bytes();
  CHECK(wire.size() == kEventWireHeaderSize + 5u * NVE_EVENT_SIZE + src.bytes().size());
  CHECK(wire[0] == 'N' && wire[1] == 'V' && wire[2] == 'E' && wire[3] == 'V');
  CHECK(nve_load_u16_le(wire.data() + 4u) == 2u);

  const std::optional<OwnedEvents> dec = OwnedEvents::decode_wire(wire);
  CHECK(dec.has_value() && dec->size() == 5u);
  if (dec) {
    const NveEventBatch b = dec->batch();
    CHECK(event_key(b.events[1], b) == "Enter" && event_code(b.events[1], b) == "Enter");
    CHECK(event_key(b.events[2], b) == "VeryLongCustomKeyName" && event_code(b.events[2], b) == "CodeX");
    CHECK(event_text(b.events[3], b) == "caf\xC3\xA9");
    CHECK(event_custom_name(b.events[4], b) == "ping" && event_custom_data(b.events[4], b).size() == 3u);
  }

  // Every truncated prefix and trailing byte must fail.
  for (size_t cut = 0u; cut < wire.size(); ++cut) {
    CHECK(!OwnedEvents::decode_wire(std::span<const uint8_t>(wire.data(), cut)).has_value());
  }
  std::vector<uint8_t> trailing = wire;
  trailing.push_back(0u);
  CHECK(!OwnedEvents::decode_wire(trailing).has_value());

  // Wrong magic / version v1 rejected.
  std::vector<uint8_t> bad_magic = wire;
  bad_magic[3] = 'X';
  CHECK(!OwnedEvents::decode_wire(bad_magic).has_value());
  std::vector<uint8_t> v1 = wire;
  v1[4] = 1u;
  CHECK(!OwnedEvents::decode_wire(v1).has_value());

  // Conflicting framing flags rejected.
  std::vector<uint8_t> conflict = wire;
  const uint32_t bad_flags = NVE_EVENT_FLAG_INLINE_KEY_UTF8 | NVE_EVENT_FLAG_KEY_SPLIT_BYTES;
  nve_store_u32_le(conflict.data() + kEventWireHeaderSize + 28u, bad_flags);
  CHECK(!OwnedEvents::decode_wire(conflict).has_value());

  // Invalid UTF-8 in text arena rejected.
  std::vector<uint8_t> bad_utf8 = wire;
  const size_t arena_off = kEventWireHeaderSize + 5u * NVE_EVENT_SIZE;
  const size_t text_off = src.events()[3].bytes.offset;
  bad_utf8[arena_off + text_off] = 0xFFu;
  CHECK(!OwnedEvents::decode_wire(bad_utf8).has_value());
}

/// A reused `OwnedEvents` allocates nothing once its slabs are sized.
void test_owned_events_reuse_allocates_nothing() {
  OwnedEvents src;
  for (uint32_t i = 0u; i < 3u; ++i) {
    CHECK(src.push_custom(1u, i + 1u, i, "evt", std::span<const uint8_t>(reinterpret_cast<const uint8_t*>("payload"), 7u)));
  }
  OwnedEvents owned;
  owned.assign(src.batch());  // sizes both slabs
  CHECK(allocations([&] {
          for (int i = 0; i < 100; ++i) {
            NveEventBatch sub = src.batch();
            sub.event_count = 1u + static_cast<uint32_t>(i % 3);
            owned.assign(sub);
          }
        }) == 0u);
  CHECK(owned.size() == 1u + 99u % 3u && event_custom_name(owned.events()[0], owned.batch()) == "evt");
}

/// A C ABI app that emits one custom event (name + `NveBinValue` payload) per dispatched event
/// without allocating, so `pump`'s own allocations are what the counter sees.
class Emitter {
 public:
  static const void* get_interface(uint32_t id) {
    static const NveSurfaceApi kSurface{&create, &destroy, &render, &dispatch_events};
    static const NveComponentApi kComponent{&set_props, &drain};
    if (id == NVE_IFACE_SURFACE) return &kSurface;
    if (id == NVE_IFACE_COMPONENT) return &kComponent;
    return nullptr;
  }

 private:
  static NveAppHandle create(uint32_t) { return new Emitter(); }
  static void destroy(NveAppHandle h) { delete static_cast<Emitter*>(h); }
  static NveBufferSlice render(NveAppHandle, uint64_t) { return NveBufferSlice{nullptr, 0u}; }
  static uint32_t set_props(NveAppHandle, const NveMapEntry*, uint32_t) { return 0u; }

  static uint32_t dispatch_events(NveAppHandle h, NveEventBatch batch) {
    auto* self = static_cast<Emitter*>(h);
    for (uint32_t i = 0u; batch.events != nullptr && i < batch.event_count && self->len_ < self->pending_.size(); ++i) {
      ++self->count_;
      static constexpr char kName[] = "rich";
      uint8_t* dst = self->arena_.data() + self->arena_len_;
      std::memcpy(dst, kName, 4u);
      nve_store_u64_le(dst + 4u, static_cast<uint64_t>(self->count_));
      NveEvent& ev = self->pending_[self->len_++];
      ev = NveEvent{};
      ev.table = NVE_TABLE_CUSTOM;
      ev.surface = batch.events[i].surface;
      ev.node = batch.events[i].node;
      ev.flags = NVE_EVENT_FLAG_CUSTOM_NAMED;
      ev.payload[0] = 4u;
      ev.bytes = RawEventBytes{self->arena_len_, 12u};
      self->arena_len_ += 12u;
    }
    return 0u;
  }

  static NveEventBatch drain(NveAppHandle h) {
    auto* self = static_cast<Emitter*>(h);
    const NveEventBatch out{self->len_ != 0u ? self->pending_.data() : nullptr,
                            self->arena_len_ != 0u ? self->arena_.data() : nullptr, self->len_, self->arena_len_};
    self->len_ = 0u;
    self->arena_len_ = 0u;
    return out;
  }

  alignas(NVE_EVENT_ALIGN) std::array<NveEvent, 8> pending_{};
  std::array<uint8_t, 128> arena_{};
  uint32_t len_ = 0u;
  uint32_t arena_len_ = 0u;
  int64_t count_ = 0;
};

bool rich_event_ok(const NveEvent& ev, NveEventBatch batch, int64_t expected_count) {
  if (!is_custom_event(ev) || event_custom_name(ev, batch) != "rich") return false;
  const std::span<const uint8_t> d = event_custom_data(ev, batch);
  return d.size() == 8u && static_cast<int64_t>(nve_load_u64_le(d.data())) == expected_count;
}

/// Steady-state `pump` with a sink: the copy reuses its 64-byte-aligned slabs and the sink is
/// shared, not copied, so a dispatch + pump makes zero C++ allocations (default OR aligned).
void test_steady_state_pump_allocates_nothing() {
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  CHECK(host->attach(&Emitter::get_interface));
  int64_t expected = 0;
  size_t delivered = 0u;
  bool all_ok = true;
  host->set_event_sink([&](NveEventBatch batch) {
    for (const NveEvent& ev : batch_events(batch)) {
      all_ok = all_ok && rich_event_ok(ev, batch, ++expected);
      ++delivered;
    }
  });
  const NveEvent press = host->events().press(3u, 0.0f, 0.0f);
  for (int i = 0; i < 4; ++i) (void)host->dispatch(press);  // warm-up sizes every slab
  CHECK(allocations([&] {
          for (int i = 0; i < 200; ++i) (void)host->dispatch(press);
        }) == 0u);
  CHECK(all_ok && delivered == 204u);
}

/// A sink may destroy its session: `pump` (and the `dispatch` that called it) never touch the
/// destroyed members afterwards (ASan proves it), and the sink still reads its whole batch.
void test_sink_destroying_the_session() {
  std::unique_ptr<HostSession> host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  CHECK(host->attach(&Emitter::get_interface));
  bool batch_ok = false;
  host->set_event_sink([&](NveEventBatch batch) {
    host.reset();  // destroys the session, its runtime, the app, and the event buffer
    batch_ok = batch.event_count == 1u && rich_event_ok(batch.events[0], batch, 1);
  });
  HostSession* raw = host.get();
  (void)raw->dispatch(raw->events().press(3u, 0.0f, 0.0f));
  CHECK(host == nullptr && batch_ok);
}

}  // namespace

int main() {
  test_owned_events_builders_and_copy();
  test_nvev_v2_wire_codec();
  test_owned_events_reuse_allocates_nothing();
  test_steady_state_pump_allocates_nothing();
  test_sink_destroying_the_session();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_events_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_events_test: ok\n");
  return 0;
}
