// NVE host runtime: uniform 64-byte `NveEvent` batch ownership (`OwnedEvents`), builders,
// zero-copy field accessors, ingress validation, and `NVEV` v2 binary wire codec.
//
// Mirrors Rust `nve_abi::event::{NveEvent, NveEventBatch, EventBatchBuf, ValidatedEvents, wire}`:
// - `NveEvent` is pointer-free (64 B): every variable-length string/byte payload lives inside the
//   companion byte arena (`NveEventBatch::bytes[offset..offset + len]`), or inline in
//   `payload[2..6]` for W3C keys `<= 16 B`.
// - `OwnedEvents` stores 64-byte-aligned `NveEvent` records plus one flat byte arena so Rust
//   `ValidatedEvents::try_from_raw` takes its zero-copy `Aligned` fast path and steady-state
//   reuses (`clear()` + `assign()` / `push_*()`) make zero heap allocations.
#pragma once

#include <bit>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <new>
#include <optional>
#include <span>
#include <string_view>
#include <type_traits>
#include <utility>
#include <vector>

#include "../generated/nve_abi.generated.h"
#include "api.h"
#include "utf8.h"

namespace nve::host {

inline constexpr uint32_t kEventWireMagic = 0x5645564Eu;  // "NVEV" little-endian
inline constexpr uint16_t kEventWireVersion = 2u;
inline constexpr size_t kEventWireHeaderSize = 14u;  // magic[4] + version:u16 + count:u32 + bytes_len:u32

inline constexpr uint32_t kEventInternalFramingMask =
    NVE_EVENT_FLAG_UTF8_BYTES | NVE_EVENT_FLAG_INLINE_KEY_UTF8 | NVE_EVENT_FLAG_CUSTOM_NAMED |
    NVE_EVENT_FLAG_KEY_SPLIT_BYTES;

/// Stateless C++20 allocator guaranteeing `Align`-byte alignment for Rust FFI slabs (`align(64)`).
template <class T, size_t Align>
struct AlignedAllocator {
  using value_type = T;
  using is_always_equal = std::true_type;

  template <class U>
  struct rebind {
    using other = AlignedAllocator<U, Align>;
  };

  constexpr AlignedAllocator() noexcept = default;
  template <class U>
  constexpr AlignedAllocator(const AlignedAllocator<U, Align>&) noexcept {}

  [[nodiscard]] T* allocate(size_t n) {
    if (n > std::numeric_limits<size_t>::max() / sizeof(T)) throw std::bad_array_new_length();
    const size_t bytes = n * sizeof(T);
    return static_cast<T*>(::operator new(bytes != 0u ? bytes : Align, std::align_val_t{Align}));
  }

  void deallocate(T* p, size_t) noexcept { ::operator delete(p, std::align_val_t{Align}); }

  template <class U, size_t A>
  friend constexpr bool operator==(const AlignedAllocator&, const AlignedAllocator<U, A>&) noexcept {
    return Align == A;
  }
};

// --- Zero-copy `NveEvent` / `NveEventBatch` helpers ---------------------------------------------

[[nodiscard]] constexpr float event_payload_f32(const NveEvent& ev, size_t idx) noexcept {
  return idx < 6u ? std::bit_cast<float>(ev.payload[idx]) : 0.0f;
}
[[nodiscard]] constexpr uint64_t event_payload_u64(const NveEvent& ev, size_t idx) noexcept {
  return idx + 1u < 6u ? uint64_t{ev.payload[idx]} | (uint64_t{ev.payload[idx + 1u]} << 32u) : 0u;
}
[[nodiscard]] constexpr int64_t event_payload_i64(const NveEvent& ev, size_t idx) noexcept {
  return static_cast<int64_t>(event_payload_u64(ev, idx));
}
[[nodiscard]] constexpr double event_payload_f64(const NveEvent& ev, size_t idx) noexcept {
  return std::bit_cast<double>(event_payload_u64(ev, idx));
}

[[nodiscard]] constexpr bool is_interaction_event(const NveEvent& ev, NveInteractionEvent kind) noexcept {
  return ev.table == NVE_TABLE_INTERACTION && ev.event == static_cast<uint32_t>(kind);
}
[[nodiscard]] constexpr bool is_text_input_event(const NveEvent& ev, NveTextInputEvent kind) noexcept {
  return ev.table == NVE_TABLE_TEXT_INPUT && ev.event == static_cast<uint32_t>(kind);
}
[[nodiscard]] constexpr bool is_scroll_event(const NveEvent& ev, NveScrollEvent kind) noexcept {
  return ev.table == NVE_TABLE_SCROLL && ev.event == static_cast<uint32_t>(kind);
}
[[nodiscard]] constexpr bool is_layout_event(const NveEvent& ev, NveLayoutEvent kind) noexcept {
  return ev.table == NVE_TABLE_LAYOUT && ev.event == static_cast<uint32_t>(kind);
}
[[nodiscard]] constexpr bool is_custom_event(const NveEvent& ev) noexcept {
  return ev.table == NVE_TABLE_CUSTOM && (ev.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) != 0u;
}
[[nodiscard]] constexpr bool event_bubbles(const NveEvent& ev) noexcept {
  return (ev.flags & NVE_EVENT_FLAG_NO_BUBBLE) == 0u;
}

[[nodiscard]] inline std::span<const NveEvent> batch_events(NveEventBatch batch) noexcept {
  return batch.events != nullptr && batch.event_count != 0u
             ? std::span<const NveEvent>(batch.events, batch.event_count)
             : std::span<const NveEvent>();
}

[[nodiscard]] inline std::span<const uint8_t> batch_bytes(NveEventBatch batch) noexcept {
  return batch.bytes != nullptr && batch.bytes_len != 0u
             ? std::span<const uint8_t>(batch.bytes, batch.bytes_len)
             : std::span<const uint8_t>();
}

/// Bounds-checked companion byte slice of `ev` inside `batch` (empty if absent or out of bounds).
[[nodiscard]] inline std::span<const uint8_t> event_bytes(const NveEvent& ev, NveEventBatch batch) noexcept {
  if (ev.bytes.len == 0u || batch.bytes == nullptr) return {};
  if (ev.bytes.offset > batch.bytes_len || ev.bytes.len > batch.bytes_len - ev.bytes.offset) return {};
  return {batch.bytes + ev.bytes.offset, ev.bytes.len};
}

/// Canonicalizes a generational handle (`SurfaceId` / `NodeId`): index 0 with any generation -> 0.
[[nodiscard]] constexpr uint32_t canonical_event_handle(uint32_t raw) noexcept {
  return (raw & 0x00FFFFFFu) == 0u ? 0u : raw;
}

/// UTF-8 text payload (`TextInput` Change / Submit / Composition); mirrors `ValidatedEvent::text`.
[[nodiscard]] inline std::string_view event_text(const NveEvent& ev, NveEventBatch batch) noexcept {
  constexpr uint32_t kNonTextMask =
      NVE_EVENT_FLAG_INLINE_KEY_UTF8 | NVE_EVENT_FLAG_KEY_SPLIT_BYTES | NVE_EVENT_FLAG_CUSTOM_NAMED;
  if ((ev.flags & NVE_EVENT_FLAG_UTF8_BYTES) == 0u || (ev.flags & kNonTextMask) != 0u) return {};
  const std::span<const uint8_t> b = event_bytes(ev, batch);
  return b.empty() ? std::string_view() : std::string_view(reinterpret_cast<const char*>(b.data()), b.size());
}

/// W3C key string (`<= 16 B` inline fast path in `ev.payload[2..6]`, or `KEY_SPLIT_BYTES` arena).
/// Borrows `ev` (for inline keys) or `batch.bytes` (for split keys).
[[nodiscard]] inline std::string_view event_key(const NveEvent& ev, NveEventBatch batch) noexcept {
  if ((ev.flags & NVE_EVENT_FLAG_INLINE_KEY_UTF8) != 0u) {
    const uint32_t klen = ev.payload[1];
    if (klen == 0u || klen > NVE_EVENT_INLINE_KEY_CAP) return {};
    return std::string_view(reinterpret_cast<const char*>(&ev.payload[2]), klen);
  }
  if ((ev.flags & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) != 0u) {
    const std::span<const uint8_t> b = event_bytes(ev, batch);
    const uint32_t klen = ev.payload[1];
    if (klen == 0u || klen > b.size()) return {};
    return std::string_view(reinterpret_cast<const char*>(b.data()), klen);
  }
  return {};
}

/// Physical key code string (`code` in `bytes` or suffix of `KEY_SPLIT_BYTES`).
[[nodiscard]] inline std::string_view event_code(const NveEvent& ev, NveEventBatch batch) noexcept {
  if ((ev.flags & NVE_EVENT_FLAG_INLINE_KEY_UTF8) != 0u && (ev.flags & NVE_EVENT_FLAG_UTF8_BYTES) != 0u) {
    const std::span<const uint8_t> b = event_bytes(ev, batch);
    return b.empty() ? std::string_view() : std::string_view(reinterpret_cast<const char*>(b.data()), b.size());
  }
  if ((ev.flags & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) != 0u) {
    const std::span<const uint8_t> b = event_bytes(ev, batch);
    const uint32_t klen = ev.payload[1];
    if (klen > b.size()) return {};
    const size_t clen = b.size() - klen;
    return clen != 0u ? std::string_view(reinterpret_cast<const char*>(b.data() + klen), clen) : std::string_view();
  }
  return {};
}

/// Custom event name (`NVE_TABLE_CUSTOM` + `NVE_EVENT_FLAG_CUSTOM_NAMED`).
[[nodiscard]] inline std::string_view event_custom_name(const NveEvent& ev, NveEventBatch batch) noexcept {
  if ((ev.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) == 0u) return {};
  const std::span<const uint8_t> b = event_bytes(ev, batch);
  const uint32_t nlen = ev.payload[0];
  if (nlen == 0u || nlen > b.size()) return {};
  return std::string_view(reinterpret_cast<const char*>(b.data()), nlen);
}

/// Custom event raw payload bytes following the name prefix.
[[nodiscard]] inline std::span<const uint8_t> event_custom_data(const NveEvent& ev, NveEventBatch batch) noexcept {
  const std::span<const uint8_t> b = event_bytes(ev, batch);
  if ((ev.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) == 0u) return b;
  const uint32_t nlen = ev.payload[0];
  if (nlen > b.size()) return {};
  return b.subspan(nlen);
}

/// Validates one 64-byte event record against `arena` (exact mirror of Rust `validate_one_event`).
[[nodiscard]] inline bool validate_event_record(const NveEvent& ev, std::span<const uint8_t> arena) noexcept {
  if (ev.bytes.offset > arena.size() || ev.bytes.len > arena.size() - ev.bytes.offset) return false;
  const std::span<const uint8_t> slice =
      ev.bytes.len != 0u ? arena.subspan(ev.bytes.offset, ev.bytes.len) : std::span<const uint8_t>();
  const bool has_utf8 = (ev.flags & NVE_EVENT_FLAG_UTF8_BYTES) != 0u;
  const bool has_inline = (ev.flags & NVE_EVENT_FLAG_INLINE_KEY_UTF8) != 0u;
  const bool has_split = (ev.flags & NVE_EVENT_FLAG_KEY_SPLIT_BYTES) != 0u;
  const bool has_custom = (ev.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) != 0u;
  if ((has_inline && has_split) || (has_custom && (has_inline || has_split))) return false;
  if (has_utf8 && !slice.empty() && nve_utf8_valid(slice.data(), slice.size()) == 0) return false;
  if (has_inline) {
    const uint32_t klen = ev.payload[1];
    if (klen > NVE_EVENT_INLINE_KEY_CAP) return false;
    if (klen != 0u && nve_utf8_valid(reinterpret_cast<const uint8_t*>(&ev.payload[2]), klen) == 0) return false;
  }
  if (has_split) {
    const uint32_t klen = ev.payload[1];
    if (klen > slice.size()) return false;
    if (has_utf8) {
      if (klen < slice.size() && (slice[klen] & 0xC0u) == 0x80u) return false;  // not a UTF-8 char boundary
    } else {
      if ((klen != 0u && nve_utf8_valid(slice.data(), klen) == 0) ||
          (klen < slice.size() && nve_utf8_valid(slice.data() + klen, slice.size() - klen) == 0)) {
        return false;
      }
    }
  }
  if (has_custom) {
    const uint32_t nlen = ev.payload[0];
    if (nlen > slice.size()) return false;
    if (nlen != 0u && nve_utf8_valid(slice.data(), nlen) == 0) return false;
  }
  return true;
}

/// Validates an entire FFI `NveEventBatch` in a single linear pass.
[[nodiscard]] inline bool validate_event_batch(NveEventBatch batch) noexcept {
  if (batch.event_count != 0u && batch.events == nullptr) return false;
  if (batch.bytes_len != 0u && batch.bytes == nullptr) return false;
  const std::span<const uint8_t> arena = batch_bytes(batch);
  for (uint32_t i = 0u; i < batch.event_count; ++i) {
    if (!validate_event_record(batch.events[i], arena)) return false;
  }
  return true;
}

/// Packs up to `NVE_EVENT_INLINE_KEY_CAP` (16) UTF-8 key bytes into `payload[2..6]` with length
/// in `payload[1]` (exact mirror of Rust `NveEvent::pack_inline_key`).
[[nodiscard]] inline bool pack_inline_key(uint32_t (&payload)[6], std::string_view key) noexcept {
  if (key.size() > NVE_EVENT_INLINE_KEY_CAP) return false;
  uint8_t buf[NVE_EVENT_INLINE_KEY_CAP]{};
  if (!key.empty()) std::memcpy(buf, key.data(), key.size());
  payload[1] = static_cast<uint32_t>(key.size());
  payload[2] = nve_load_u32_le(buf + 0u);
  payload[3] = nve_load_u32_le(buf + 4u);
  payload[4] = nve_load_u32_le(buf + 8u);
  payload[5] = nve_load_u32_le(buf + 12u);
  return true;
}

/// Reusable owned batch of 64-byte-aligned `NveEvent` records + flat byte arena.
///
/// Steady-state `assign` / `push_*` after `clear` make zero heap allocations. Moving transfers
/// both buffers with no pointer rebasing.
class OwnedEvents {
 public:
  OwnedEvents() noexcept = default;
  OwnedEvents(const OwnedEvents&) = delete;
  OwnedEvents& operator=(const OwnedEvents&) = delete;
  OwnedEvents(OwnedEvents&&) noexcept = default;
  OwnedEvents& operator=(OwnedEvents&&) noexcept = default;

  void swap(OwnedEvents& other) noexcept {
    events_.swap(other.events_);
    bytes_.swap(other.bytes_);
  }

  /// Replaces contents with an owned copy of `batch`. Null pointers with non-zero counts copy as
  /// empty; any out-of-bounds event byte span is zeroed so reads stay memory-safe.
  void assign(NveEventBatch batch) {
    clear();
    if (batch.events == nullptr || batch.event_count == 0u) return;
    const uint32_t blen = batch.bytes != nullptr ? batch.bytes_len : 0u;
    events_.assign(batch.events, batch.events + batch.event_count);
    if (blen != 0u) bytes_.assign(batch.bytes, batch.bytes + blen);
    for (NveEvent& ev : events_) {
      ev.surface = canonical_event_handle(ev.surface);
      ev.node = canonical_event_handle(ev.node);
      if (ev.bytes.offset > blen || ev.bytes.len > blen - ev.bytes.offset) {
        ev.bytes = RawEventBytes{0u, 0u};
        ev.flags &= ~kEventInternalFramingMask;
      }
    }
  }

  /// Empties the batch while keeping both slabs' capacities.
  void clear() noexcept {
    events_.clear();
    bytes_.clear();
  }

  void reserve(size_t event_cap, size_t byte_cap) {
    events_.reserve(event_cap);
    bytes_.reserve(byte_cap);
  }

  [[nodiscard]] std::span<const NveEvent> events() const noexcept { return events_; }
  [[nodiscard]] std::span<const uint8_t> bytes() const noexcept { return bytes_; }
  [[nodiscard]] bool empty() const noexcept { return events_.empty(); }
  [[nodiscard]] size_t size() const noexcept { return events_.size(); }

  /// Borrowed FFI view (64-byte-aligned `events` pointer; valid until the next mutation).
  [[nodiscard]] NveEventBatch batch() const noexcept {
    return NveEventBatch{
        events_.empty() ? nullptr : events_.data(),
        bytes_.empty() ? nullptr : bytes_.data(),
        static_cast<uint32_t>(events_.size()),
        static_cast<uint32_t>(bytes_.size()),
    };
  }

  /// Appends a raw event and its arena slice (rejecting overflow past 32-bit wire limits).
  bool push_raw(NveEvent ev, std::span<const uint8_t> data = {}) {
    if (events_.size() >= std::numeric_limits<uint32_t>::max()) return false;
    RawEventBytes span{0u, 0u};
    if (!append_bytes(data, &span)) return false;
    ev.surface = canonical_event_handle(ev.surface);
    ev.node = canonical_event_handle(ev.node);
    ev.bytes = span;
    events_.push_back(ev);
    return true;
  }

  bool push_press(uint32_t surface, uint32_t node, uint64_t ts, float x, float y) {
    return push_interaction(surface, node, ts, NVE_INTERACTION_EVENT_PRESS, 0u,
                            {std::bit_cast<uint32_t>(x), std::bit_cast<uint32_t>(y), 0u, 0u, 0u, 0u});
  }

  bool push_long_press(uint32_t surface, uint32_t node, uint64_t ts, float x, float y) {
    return push_interaction(surface, node, ts, NVE_INTERACTION_EVENT_LONG_PRESS, 0u,
                            {std::bit_cast<uint32_t>(x), std::bit_cast<uint32_t>(y), 0u, 0u, 0u, 0u});
  }

  bool push_focus(uint32_t surface, uint32_t node, uint64_t ts) {
    return push_interaction(surface, node, ts, NVE_INTERACTION_EVENT_FOCUS, 0u, {});
  }

  bool push_blur(uint32_t surface, uint32_t node, uint64_t ts) {
    return push_interaction(surface, node, ts, NVE_INTERACTION_EVENT_BLUR, 0u, {});
  }

  bool push_pointer(uint32_t surface, uint32_t node, uint64_t ts, NveInteractionEvent kind, float x, float y,
                    uint32_t pointer_id = 0u, uint32_t flags = 0u) {
    return push_interaction(surface, node, ts, kind, flags & ~kEventInternalFramingMask,
                            {std::bit_cast<uint32_t>(x), std::bit_cast<uint32_t>(y), pointer_id, 0u, 0u, 0u});
  }

  bool push_wheel(uint32_t surface, uint32_t node, uint64_t ts, float dx, float dy, uint32_t flags = 0u) {
    return push_interaction(surface, node, ts, NVE_INTERACTION_EVENT_WHEEL, flags & ~kEventInternalFramingMask,
                            {std::bit_cast<uint32_t>(dx), std::bit_cast<uint32_t>(dy), 0u, 0u, 0u, 0u});
  }

  /// Appends a keyboard event (`KeyDown` / `KeyUp`), rejecting non-UTF-8 `key` or `code`.
  bool push_key(uint32_t surface, uint32_t node, uint64_t ts, NveInteractionEvent kind, std::string_view key,
                std::string_view code = {}, uint32_t flags = 0u, uint32_t key_code = 0u) {
    if (!utf8_ok(key) || !utf8_ok(code)) return false;
    NveEvent ev = make_base(NVE_TABLE_INTERACTION, surface, node, ts, static_cast<uint32_t>(kind));
    ev.payload[0] = key_code;
    const uint32_t base_flags = flags & ~kEventInternalFramingMask;
    if (pack_inline_key(ev.payload, key)) {
      ev.flags = base_flags | NVE_EVENT_FLAG_INLINE_KEY_UTF8 | (code.empty() ? 0u : NVE_EVENT_FLAG_UTF8_BYTES);
      return push_raw(ev, str_bytes(code));
    }
    if (events_.size() >= std::numeric_limits<uint32_t>::max()) return false;
    if (key.size() > std::numeric_limits<uint32_t>::max()) return false;
    RawEventBytes span{0u, 0u};
    if (!append_pair(str_bytes(key), str_bytes(code), &span)) return false;
    ev.flags = base_flags | NVE_EVENT_FLAG_KEY_SPLIT_BYTES | NVE_EVENT_FLAG_UTF8_BYTES;
    ev.payload[1] = static_cast<uint32_t>(key.size());
    ev.bytes = span;
    events_.push_back(ev);
    return true;
  }

  bool push_text_input(uint32_t surface, uint32_t node, uint64_t ts, NveTextInputEvent kind, std::string_view text) {
    if (!utf8_ok(text)) return false;
    NveEvent ev = make_base(NVE_TABLE_TEXT_INPUT, surface, node, ts, static_cast<uint32_t>(kind));
    ev.flags = NVE_EVENT_FLAG_UTF8_BYTES;
    return push_raw(ev, str_bytes(text));
  }

  bool push_text_change(uint32_t surface, uint32_t node, uint64_t ts, std::string_view text) {
    return push_text_input(surface, node, ts, NVE_TEXT_INPUT_EVENT_CHANGE, text);
  }

  bool push_submit(uint32_t surface, uint32_t node, uint64_t ts, std::string_view text) {
    return push_text_input(surface, node, ts, NVE_TEXT_INPUT_EVENT_SUBMIT, text);
  }

  bool push_scroll(uint32_t surface, uint32_t node, uint64_t ts, float x, float y, float content_w, float content_h) {
    NveEvent ev = make_base(NVE_TABLE_SCROLL, surface, node, ts, NVE_SCROLL_EVENT_SCROLL);
    ev.payload[0] = std::bit_cast<uint32_t>(x);
    ev.payload[1] = std::bit_cast<uint32_t>(y);
    ev.payload[2] = std::bit_cast<uint32_t>(content_w);
    ev.payload[3] = std::bit_cast<uint32_t>(content_h);
    return push_raw(ev);
  }

  bool push_layout(uint32_t surface, uint32_t node, uint64_t ts, float x, float y, float width, float height) {
    NveEvent ev = make_base(NVE_TABLE_LAYOUT, surface, node, ts, NVE_LAYOUT_EVENT_LAYOUT);
    ev.payload[0] = std::bit_cast<uint32_t>(x);
    ev.payload[1] = std::bit_cast<uint32_t>(y);
    ev.payload[2] = std::bit_cast<uint32_t>(width);
    ev.payload[3] = std::bit_cast<uint32_t>(height);
    return push_raw(ev);
  }

  /// Appends an `nve.custom` named event (`name` + raw binary `data`), rejecting non-UTF-8 `name`.
  bool push_custom(uint32_t surface, uint32_t node, uint64_t ts, std::string_view name,
                   std::span<const uint8_t> data = {}, uint32_t flags = 0u) {
    if (!utf8_ok(name) || events_.size() >= std::numeric_limits<uint32_t>::max()) return false;
    if (name.size() > std::numeric_limits<uint32_t>::max()) return false;
    RawEventBytes span{0u, 0u};
    if (!append_pair(str_bytes(name), data, &span)) return false;
    NveEvent ev = make_base(NVE_TABLE_CUSTOM, surface, node, ts, 0u);
    ev.flags = (flags & ~kEventInternalFramingMask) | NVE_EVENT_FLAG_CUSTOM_NAMED;
    ev.payload[0] = static_cast<uint32_t>(name.size());
    ev.bytes = span;
    events_.push_back(ev);
    return true;
  }

  /// Appends this batch as a little-endian `NVEV` v2 wire frame into `out`.
  void encode_wire(std::vector<uint8_t>& out) const {
    const size_t ev_bytes = events_.size() * NVE_EVENT_SIZE;
    const size_t start = out.size();
    out.resize(start + kEventWireHeaderSize + ev_bytes + bytes_.size());
    uint8_t* p = out.data() + start;
    nve_store_u32_le(p + 0u, kEventWireMagic);
    nve_store_u16_le(p + 4u, kEventWireVersion);
    nve_store_u32_le(p + 6u, static_cast<uint32_t>(events_.size()));
    nve_store_u32_le(p + 10u, static_cast<uint32_t>(bytes_.size()));
    if (ev_bytes != 0u) std::memcpy(p + kEventWireHeaderSize, events_.data(), ev_bytes);
    if (!bytes_.empty()) std::memcpy(p + kEventWireHeaderSize + ev_bytes, bytes_.data(), bytes_.size());
  }

  [[nodiscard]] std::vector<uint8_t> to_wire_bytes() const {
    std::vector<uint8_t> out;
    encode_wire(out);
    return out;
  }

  /// Decodes and validates an `NVEV` v2 wire frame (exact mirror of Rust `EventBatchBuf::decode_wire`).
  [[nodiscard]] static std::optional<OwnedEvents> decode_wire(std::span<const uint8_t> raw) {
    if (raw.size() < kEventWireHeaderSize) return std::nullopt;
    if (nve_load_u32_le(raw.data() + 0u) != kEventWireMagic) return std::nullopt;
    if (nve_load_u16_le(raw.data() + 4u) != kEventWireVersion) return std::nullopt;
    const uint32_t count = nve_load_u32_le(raw.data() + 6u);
    const uint32_t blen = nve_load_u32_le(raw.data() + 10u);
    const uint64_t ev_bytes = uint64_t{count} * NVE_EVENT_SIZE;
    const uint64_t expected = kEventWireHeaderSize + ev_bytes + uint64_t{blen};
    if (expected != raw.size()) return std::nullopt;
    const std::span<const uint8_t> arena = raw.subspan(static_cast<size_t>(kEventWireHeaderSize + ev_bytes), blen);
    OwnedEvents out;
    out.events_.resize(count);
    for (uint32_t i = 0u; i < count; ++i) {
      std::memcpy(&out.events_[i], raw.data() + kEventWireHeaderSize + size_t{i} * NVE_EVENT_SIZE, NVE_EVENT_SIZE);
      out.events_[i].surface = canonical_event_handle(out.events_[i].surface);
      out.events_[i].node = canonical_event_handle(out.events_[i].node);
      if (!validate_event_record(out.events_[i], arena)) return std::nullopt;
    }
    if (blen != 0u) out.bytes_.assign(arena.begin(), arena.end());
    return out;
  }

 private:
  [[nodiscard]] static bool utf8_ok(std::string_view s) noexcept {
    return s.empty() || nve_utf8_valid(reinterpret_cast<const uint8_t*>(s.data()), s.size()) != 0;
  }

  [[nodiscard]] static std::span<const uint8_t> str_bytes(std::string_view s) noexcept {
    return s.empty() ? std::span<const uint8_t>()
                     : std::span<const uint8_t>(reinterpret_cast<const uint8_t*>(s.data()), s.size());
  }

  [[nodiscard]] static NveEvent make_base(uint64_t table, uint32_t surface, uint32_t node, uint64_t ts,
                                          uint32_t event_id) noexcept {
    NveEvent ev{};
    ev.timestamp = ts;
    ev.table = table;
    ev.surface = canonical_event_handle(surface);
    ev.node = canonical_event_handle(node);
    ev.event = event_id;
    return ev;
  }

  bool push_interaction(uint32_t surface, uint32_t node, uint64_t ts, NveInteractionEvent kind, uint32_t flags,
                        const uint32_t (&payload)[6]) {
    NveEvent ev = make_base(NVE_TABLE_INTERACTION, surface, node, ts, static_cast<uint32_t>(kind));
    ev.flags = flags;
    for (size_t i = 0u; i < 6u; ++i) ev.payload[i] = payload[i];
    return push_raw(ev);
  }

  bool append_bytes(std::span<const uint8_t> data, RawEventBytes* out) {
    if (data.empty()) {
      *out = RawEventBytes{0u, 0u};
      return true;
    }
    const uint64_t max32 = std::numeric_limits<uint32_t>::max();
    if (bytes_.size() > max32 || data.size() > max32 - bytes_.size()) return false;
    const auto off = static_cast<uint32_t>(bytes_.size());
    bytes_.insert(bytes_.end(), data.begin(), data.end());
    *out = RawEventBytes{off, static_cast<uint32_t>(data.size())};
    return true;
  }

  bool append_pair(std::span<const uint8_t> first, std::span<const uint8_t> second, RawEventBytes* out) {
    const uint64_t max32 = std::numeric_limits<uint32_t>::max();
    if (first.size() > max32 || second.size() > max32 - first.size()) return false;
    const uint64_t total = first.size() + second.size();
    if (total == 0u) {
      *out = RawEventBytes{0u, 0u};
      return true;
    }
    if (bytes_.size() > max32 || total > max32 - bytes_.size()) return false;
    const auto off = static_cast<uint32_t>(bytes_.size());
    if (!first.empty()) bytes_.insert(bytes_.end(), first.begin(), first.end());
    if (!second.empty()) bytes_.insert(bytes_.end(), second.begin(), second.end());
    *out = RawEventBytes{off, static_cast<uint32_t>(total)};
    return true;
  }

  std::vector<NveEvent, AlignedAllocator<NveEvent, NVE_EVENT_ALIGN>> events_;
  std::vector<uint8_t> bytes_;
};

static_assert(std::is_nothrow_move_constructible_v<OwnedEvents> && std::is_nothrow_move_assignable_v<OwnedEvents>,
              "HostSession::pump hands batches around by move");

}  // namespace nve::host
