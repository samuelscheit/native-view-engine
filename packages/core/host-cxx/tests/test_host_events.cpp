// `OwnedEvents` (the deep copy `HostSession::pump` hands its sink) and the pump's event path:
// every slice kind copied into flat slabs (text, STR, BYTES, nested ARRAY / MAP, NULL-with-len,
// the depth limit), slices valid after a move, zero C++ allocations for a reused buffer and for a
// steady-state `pump` (counting global `operator new`), and a sink that destroys its session.
#include <algorithm>
#include <array>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <memory>
#include <new>
#include <span>
#include <string>
#include <string_view>
#include <type_traits>
#include <vector>

#include "check.h"
#include "nve/host/host_session.hpp"
#include "nve/host/owned_events.hpp"

// --- Counting global allocator (this test binary only) -----------------------------------------
namespace {
thread_local bool g_counting = false;
thread_local size_t g_allocs = 0u;
}  // namespace

void* operator new(std::size_t n) {
  if (g_counting) ++g_allocs;
  if (void* p = std::malloc(n != 0u ? n : 1u)) return p;
  throw std::bad_alloc();
}
void operator delete(void* p) noexcept { std::free(p); }
void operator delete(void* p, std::size_t) noexcept { std::free(p); }

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

std::string_view sv(NveStrSlice s) { return str_view(s); }
std::span<const uint8_t> bytes(NveBufferSlice b) { return buffer_span(b); }

NveValue str_value(std::string_view s) {
  NveValue v{};
  v.tag = NVE_VAL_STR;
  v.data.str_val = str_slice(s);
  return v;
}
NveValue bytes_value(std::span<const uint8_t> b) {
  NveValue v{};
  v.tag = NVE_VAL_BYTES;
  v.data.bytes_val = buffer_slice(b);
  return v;
}
NveValue i64_value(int64_t n) {
  NveValue v{};
  v.tag = NVE_VAL_I64;
  v.data.i64_val = n;
  return v;
}
NveValue array_value(std::span<const NveValue> items) {
  NveValue v{};
  v.tag = NVE_VAL_ARRAY;
  v.data.array_val = NveValueArray{items.empty() ? nullptr : items.data(), items.size()};
  return v;
}
NveValue map_value(std::span<const NveMapEntry> entries) {
  NveValue v{};
  v.tag = NVE_VAL_MAP;
  v.data.map_val = NveValueMap{entries.empty() ? nullptr : entries.data(), entries.size()};
  return v;
}

/// `true` when `p` lies inside `owner`'s storage, i.e. not inside the source buffers.
bool owned_by(const void* p, const void* source_begin, size_t source_len) {
  const auto* b = static_cast<const uint8_t*>(source_begin);
  const auto* q = static_cast<const uint8_t*>(p);
  return p != nullptr && (q < b || q >= b + source_len);
}

void test_owned_events_copy_every_slice_kind() {
  // Sources the copy must not depend on: overwritten after `assign`.
  std::array<char, 64> text_buf{};
  std::array<uint8_t, 4> blob{9u, 8u, 7u, 6u};
  std::string name_storage = "custom";
  std::array<char, 8> key_buf{'k', 'e', 'y', 0};
  std::array<NveValue, 2> deep{};
  std::array<NveValue, 3> items{};
  std::array<NveMapEntry, 3> entries{};
  const std::string_view text_src = "h\xC3\xA9llo";  // UTF-8, copied byte for byte
  std::copy(text_src.begin(), text_src.end(), text_buf.begin());

  deep[0] = i64_value(42);
  deep[1] = str_value("deep");
  items[0] = str_value(std::string_view(text_buf.data(), 3u));
  items[1] = bytes_value(blob);
  items[2] = array_value(deep);  // nested array
  entries[0] = NveMapEntry{str_slice(std::string_view(key_buf.data(), 3u)), array_value(items)};
  entries[1] = NveMapEntry{str_slice("null"), str_value({})};
  NveValue hostile{};  // NULL pointer with a non-zero length: copied as empty, never read
  hostile.tag = NVE_VAL_BYTES;
  hostile.data.bytes_val = NveBufferSlice{nullptr, 99u};
  entries[2] = NveMapEntry{NveStrSlice{nullptr, 5u}, hostile};

  std::array<NveDirectEvent, 2> evs{};
  evs[0].node_id = 7u;
  evs[0].kind = NVE_EVENT_CUSTOM;
  evs[0].name = str_slice(name_storage);
  evs[0].text = str_slice(std::string_view(text_buf.data(), text_src.size()));
  evs[0].value = map_value(entries);
  evs[1].node_id = 8u;
  evs[1].text = NveStrSlice{nullptr, 12u};
  evs[1].value = NveValue{};
  evs[1].value.tag = NVE_VAL_ARRAY;
  evs[1].value.data.array_val = NveValueArray{nullptr, 3u};

  OwnedEvents owned;
  owned.assign(evs);
  // Scribble every source: the copy must not alias any of them.
  text_buf.fill('#');
  blob.fill(0u);
  name_storage.assign(name_storage.size(), '#');
  key_buf.fill('#');
  deep.fill(NveValue{});
  items.fill(NveValue{});
  entries.fill(NveMapEntry{});
  evs.fill(NveDirectEvent{});

  const auto check = [&](const OwnedEvents& o) {
    CHECK(o.events().size() == 2u);
    const NveDirectEvent& e = o.events()[0];
    CHECK(e.node_id == 7u && e.kind == NVE_EVENT_CUSTOM);
    CHECK(sv(e.name) == "custom" && sv(e.text) == text_src);
    CHECK(owned_by(e.text.ptr, text_buf.data(), text_buf.size()));
    CHECK(e.value.tag == NVE_VAL_MAP && e.value.data.map_val.len == 3u);
    const NveMapEntry* m = e.value.data.map_val.ptr;
    CHECK(sv(m[0].key) == "key" && m[0].value.tag == NVE_VAL_ARRAY && m[0].value.data.array_val.len == 3u);
    const NveValue* a = m[0].value.data.array_val.ptr;
    CHECK(a[0].tag == NVE_VAL_STR && sv(a[0].data.str_val) == "h\xC3\xA9");
    CHECK(a[1].tag == NVE_VAL_BYTES && bytes(a[1].data.bytes_val).size() == 4u &&
          bytes(a[1].data.bytes_val)[0] == 9u && bytes(a[1].data.bytes_val)[3] == 6u);
    CHECK(a[2].tag == NVE_VAL_ARRAY && a[2].data.array_val.len == 2u);
    CHECK(a[2].data.array_val.ptr[0].data.i64_val == 42 && sv(a[2].data.array_val.ptr[1].data.str_val) == "deep");
    CHECK(sv(m[1].key) == "null" && m[1].value.tag == NVE_VAL_STR && m[1].value.data.str_val.ptr == nullptr);
    // NULL with a non-zero length: empty, never dereferenced.
    CHECK(m[2].key.ptr == nullptr && m[2].key.len == 0u);
    CHECK(m[2].value.data.bytes_val.ptr == nullptr && m[2].value.data.bytes_val.len == 0u);
    const NveDirectEvent& f = o.events()[1];
    CHECK(f.text.ptr == nullptr && f.text.len == 0u);
    CHECK(f.value.tag == NVE_VAL_ARRAY && f.value.data.array_val.ptr == nullptr && f.value.data.array_val.len == 0u);
  };
  check(owned);

  // Moves keep every slice valid (the slabs move, nothing is rebased).
  const NveDirectEvent* before = owned.events().data();
  OwnedEvents moved(std::move(owned));
  CHECK(moved.events().data() == before);
  check(moved);
  OwnedEvents assigned;
  assigned = std::move(moved);
  check(assigned);
  static_assert(std::is_nothrow_move_constructible_v<OwnedEvents> && std::is_nothrow_move_assignable_v<OwnedEvents>);

  assigned.assign({});
  CHECK(assigned.empty() && assigned.events().empty());
}

/// Values nested deeper than `kBinValueMaxDepth` copy as `Null`; everything above is kept.
void test_owned_events_depth_limit() {
  constexpr size_t kChain = kBinValueMaxDepth + 6u;
  std::vector<NveValue> chain(kChain + 1u);
  chain[kChain] = i64_value(1);
  for (size_t i = kChain; i-- > 0u;) chain[i] = array_value(std::span<const NveValue>(&chain[i + 1u], 1u));
  NveDirectEvent ev{};
  ev.value = chain[0];
  OwnedEvents owned;
  owned.assign(std::span<const NveDirectEvent>(&ev, 1u));
  size_t depth = 0u;
  const NveValue* v = &owned.events()[0].value;
  while (v->tag == NVE_VAL_ARRAY && v->data.array_val.len == 1u) {
    v = v->data.array_val.ptr;
    ++depth;
  }
  CHECK(depth == kBinValueMaxDepth);
  CHECK(v->tag == NVE_VAL_NULL);
}

/// A reused `OwnedEvents` allocates only when a batch outgrows every earlier one.
void test_owned_events_reuse_allocates_nothing() {
  std::array<NveValue, 4> items{str_value("aaaaaaaa"), str_value("bb"), i64_value(3), str_value("c")};
  std::array<NveMapEntry, 2> map{NveMapEntry{str_slice("items"), array_value(items)},
                                 NveMapEntry{str_slice("text"), str_value("payload")}};
  std::array<NveDirectEvent, 3> evs{};
  for (NveDirectEvent& e : evs) {
    e.name = str_slice("evt");
    e.text = str_slice("some text");
    e.value = map_value(map);
  }
  OwnedEvents owned;
  owned.assign(evs);  // sizes the slabs
  CHECK(allocations([&] {
          for (int i = 0; i < 100; ++i) {
            owned.assign(std::span<const NveDirectEvent>(evs).first(1u + static_cast<size_t>(i) % 3u));
          }
        }) == 0u);
  CHECK(owned.events().size() == 1u + 99u % 3u && sv(owned.events()[0].value.data.map_val.ptr[1].value.data.str_val) == "payload");
}

/// A C ABI app that emits one rich event (text + nested MAP / ARRAY / BYTES) per dispatched
/// event without allocating, so `pump`'s own allocations are what the counter sees.
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
    for (uint32_t i = 0u; batch.ptr != nullptr && i < batch.len && self->len_ < self->pending_.size(); ++i) {
      ++self->count_;
      self->deep_[0] = i64_value(self->count_);
      self->deep_[1] = bytes_value(kBlob);
      self->items_[0] = str_value("item");
      self->items_[1] = array_value(self->deep_);
      self->entries_[0] = NveMapEntry{str_slice("count"), i64_value(self->count_)};
      self->entries_[1] = NveMapEntry{str_slice("items"), array_value(self->items_)};
      NveDirectEvent& ev = self->pending_[self->len_++];
      ev = NveDirectEvent{};
      ev.node_id = batch.ptr[i].node_id;
      ev.kind = NVE_EVENT_CUSTOM;
      ev.name = str_slice("rich");
      ev.text = str_slice("h\xC3\xA9llo");
      ev.value = map_value(self->entries_);
    }
    return 0u;
  }

  static NveEventBatch drain(NveAppHandle h) {
    auto* self = static_cast<Emitter*>(h);
    const NveEventBatch out{self->len_ != 0u ? self->pending_.data() : nullptr, self->len_, 0u};
    self->len_ = 0u;  // the batch stays readable until the next call into the app
    return out;
  }

  static constexpr std::array<uint8_t, 3> kBlob{1u, 2u, 3u};
  std::array<NveDirectEvent, 8> pending_{};
  uint32_t len_ = 0u;
  int64_t count_ = 0;
  std::array<NveValue, 2> deep_{};
  std::array<NveValue, 2> items_{};
  std::array<NveMapEntry, 2> entries_{};
};

bool rich_event_ok(const NveDirectEvent& ev, int64_t expected_count) {
  if (sv(ev.name) != "rich" || sv(ev.text) != "h\xC3\xA9llo" || ev.value.tag != NVE_VAL_MAP) return false;
  const NveMapEntry* m = ev.value.data.map_val.ptr;
  if (ev.value.data.map_val.len != 2u || sv(m[0].key) != "count" || m[0].value.data.i64_val != expected_count) return false;
  const NveValue* items = m[1].value.data.array_val.ptr;
  if (m[1].value.data.array_val.len != 2u || sv(items[0].data.str_val) != "item") return false;
  const NveValue* deep = items[1].data.array_val.ptr;
  return items[1].data.array_val.len == 2u && deep[0].data.i64_val == expected_count &&
         bytes(deep[1].data.bytes_val).size() == 3u && bytes(deep[1].data.bytes_val)[2] == 3u;
}

/// Steady-state `pump` with a sink: the deep copy reuses its slabs and the sink is shared, not
/// copied, so a dispatch + pump makes no C++ allocation at all.
void test_steady_state_pump_allocates_nothing() {
  auto host = HostSession::create(RuntimeSession::default_config());
  CHECK(host != nullptr);
  if (!host) return;
  CHECK(host->attach(&Emitter::get_interface));
  int64_t expected = 0;
  size_t delivered = 0u;
  bool all_ok = true;
  host->set_event_sink([&](std::span<const NveDirectEvent> evs) {
    for (const NveDirectEvent& ev : evs) {
      all_ok = all_ok && rich_event_ok(ev, ++expected);
      ++delivered;
    }
  });
  const NveDirectEvent press = host->events().press(3u, 0.0f, 0.0f);
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
  host->set_event_sink([&](std::span<const NveDirectEvent> evs) {
    host.reset();  // destroys the session, its runtime, the app, and the event buffer
    batch_ok = evs.size() == 1u && rich_event_ok(evs[0], 1);
  });
  HostSession* raw = host.get();
  (void)raw->dispatch(raw->events().press(3u, 0.0f, 0.0f));
  CHECK(host == nullptr && batch_ok);
}

}  // namespace

int main() {
  test_owned_events_copy_every_slice_kind();
  test_owned_events_depth_limit();
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
