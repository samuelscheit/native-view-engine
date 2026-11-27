#pragma once

#include "nve_abi.generated.h"
#include "nve_value.hpp"
#include "nve_wire_codecs.generated.hpp"

#include <array>
#include <bit>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <span>
#include <string_view>
#include <vector>

namespace nve {

/// Construct a zero-payload 64-byte `NveEvent`.
inline NveEvent make_base_event(
    uint64_t table,
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us,
    uint32_t event_id,
    uint32_t flags = 0u) noexcept {
  NveEvent ev{};
  ev.timestamp = ts_us;
  ev.table = table;
  ev.surface = surface_id;
  ev.node = node_id;
  ev.event = event_id;
  ev.flags = flags;
  return ev;
}

inline NveEvent make_press_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us,
    float x,
    float y) noexcept {
  NveEvent ev = make_base_event(
      NVE_TABLE_INTERACTION, surface_id, node_id, ts_us, NVE_INTERACTION_EVENT_PRESS, 0u);
  ev.payload[0] = std::bit_cast<uint32_t>(x);
  ev.payload[1] = std::bit_cast<uint32_t>(y);
  return ev;
}

inline NveEvent make_long_press_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us,
    float x,
    float y) noexcept {
  NveEvent ev = make_base_event(
      NVE_TABLE_INTERACTION, surface_id, node_id, ts_us, NVE_INTERACTION_EVENT_LONG_PRESS, 0u);
  ev.payload[0] = std::bit_cast<uint32_t>(x);
  ev.payload[1] = std::bit_cast<uint32_t>(y);
  return ev;
}

inline NveEvent make_focus_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us) noexcept {
  return make_base_event(
      NVE_TABLE_INTERACTION, surface_id, node_id, ts_us, NVE_INTERACTION_EVENT_FOCUS, 0u);
}

inline NveEvent make_blur_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us) noexcept {
  return make_base_event(
      NVE_TABLE_INTERACTION, surface_id, node_id, ts_us, NVE_INTERACTION_EVENT_BLUR, 0u);
}

inline NveEvent make_scroll_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us,
    float x,
    float y,
    float content_width,
    float content_height) noexcept {
  NveEvent ev = make_base_event(
      NVE_TABLE_SCROLL, surface_id, node_id, ts_us, NVE_SCROLL_EVENT_SCROLL, 0u);
  ev.payload[0] = std::bit_cast<uint32_t>(x);
  ev.payload[1] = std::bit_cast<uint32_t>(y);
  ev.payload[2] = std::bit_cast<uint32_t>(content_width);
  ev.payload[3] = std::bit_cast<uint32_t>(content_height);
  return ev;
}

inline NveEvent make_layout_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us,
    float x,
    float y,
    float width,
    float height) noexcept {
  NveEvent ev = make_base_event(
      NVE_TABLE_LAYOUT, surface_id, node_id, ts_us, NVE_LAYOUT_EVENT_LAYOUT, 0u);
  ev.payload[0] = std::bit_cast<uint32_t>(x);
  ev.payload[1] = std::bit_cast<uint32_t>(y);
  ev.payload[2] = std::bit_cast<uint32_t>(width);
  ev.payload[3] = std::bit_cast<uint32_t>(height);
  return ev;
}

inline NveEvent make_pointer_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us,
    NveInteractionEvent event_id,
    float x,
    float y,
    uint32_t flags,
    uint32_t pointer_id = 0u) noexcept {
  NveEvent ev = make_base_event(
      NVE_TABLE_INTERACTION, surface_id, node_id, ts_us, static_cast<uint32_t>(event_id), flags);
  ev.payload[0] = std::bit_cast<uint32_t>(x);
  ev.payload[1] = std::bit_cast<uint32_t>(y);
  ev.payload[2] = pointer_id;
  return ev;
}

inline NveEvent make_wheel_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us,
    float delta_x,
    float delta_y,
    uint32_t flags) noexcept {
  NveEvent ev = make_base_event(
      NVE_TABLE_INTERACTION, surface_id, node_id, ts_us, NVE_INTERACTION_EVENT_WHEEL, flags);
  ev.payload[0] = std::bit_cast<uint32_t>(delta_x);
  ev.payload[1] = std::bit_cast<uint32_t>(delta_y);
  return ev;
}

inline NveEvent make_text_event(
    uint32_t surface_id,
    uint32_t node_id,
    uint64_t ts_us,
    NveTextInputEvent event_id,
    uint32_t bytes_len,
    uint32_t bytes_offset = 0u) noexcept {
  NveEvent ev = make_base_event(
      NVE_TABLE_TEXT_INPUT,
      surface_id,
      node_id,
      ts_us,
      static_cast<uint32_t>(event_id),
      NVE_EVENT_FLAG_UTF8_BYTES);
  ev.bytes.offset = bytes_len > 0u ? bytes_offset : 0u;
  ev.bytes.len = bytes_len;
  return ev;
}

inline NveEventBatch make_single_event_batch(
    const NveEvent& ev,
    const uint8_t* bytes = nullptr,
    uint32_t bytes_len = 0u) noexcept {
  return NveEventBatch{&ev, bytes_len > 0u ? bytes : nullptr, 1u, bytes_len};
}

inline float event_payload_f32(const NveEvent& ev, size_t idx) noexcept {
  return std::bit_cast<float>(ev.payload[idx]);
}

/// Extract custom event name & payload bytes from a `NVE_EVENT_FLAG_CUSTOM_NAMED` `NveEvent`.
inline bool event_custom_parts(
    const NveEvent& ev,
    const uint8_t* arena,
    size_t arena_len,
    std::string_view& out_name,
    std::span<const uint8_t>& out_payload) noexcept {
  out_name = {};
  out_payload = {};
  if (ev.bytes.len > 0u &&
      (arena == nullptr || ev.bytes.offset > arena_len ||
       ev.bytes.len > arena_len - ev.bytes.offset)) {
    return false;
  }
  const uint8_t* base = ev.bytes.len > 0u ? (arena + ev.bytes.offset) : nullptr;
  const size_t total = ev.bytes.len;
  if ((ev.flags & NVE_EVENT_FLAG_CUSTOM_NAMED) != 0u) {
    const uint32_t nlen = ev.payload[0];
    if (nlen > total) {
      return false;
    }
    out_name = nlen > 0u ? std::string_view(reinterpret_cast<const char*>(base), nlen)
                         : std::string_view{};
    out_payload = (total > nlen) ? std::span<const uint8_t>(base + nlen, total - nlen)
                                 : std::span<const uint8_t>{};
    return true;
  }
  out_payload = total > 0u ? std::span<const uint8_t>(base, total) : std::span<const uint8_t>{};
  return true;
}

/// Serialize one 64-byte `NveEvent` in little-endian wire layout.
inline void append_event_le(std::vector<uint8_t>& out, const NveEvent& ev) {
  wire::append_le_u64(out, ev.timestamp);
  wire::append_le_u64(out, ev.table);
  wire::append_le_u32(out, ev.surface);
  wire::append_le_u32(out, ev.node);
  wire::append_le_u32(out, ev.event);
  wire::append_le_u32(out, ev.flags);
  for (uint32_t w : ev.payload) {
    wire::append_le_u32(out, w);
  }
  wire::append_le_u32(out, ev.bytes.offset);
  wire::append_le_u32(out, ev.bytes.len);
}

/// Reusable builder for both zero-copy in-process `NveEventBatch` and `NVEV` v2 wire bytes.
class EventBatchBuilder {
 public:
  void clear() noexcept {
    events_.clear();
    arena_.clear();
    wire_buf_.clear();
  }

  void reserve(size_t event_cap, size_t byte_cap = 256u) {
    events_.reserve(event_cap);
    arena_.reserve(byte_cap);
  }

  uint32_t count() const noexcept { return static_cast<uint32_t>(events_.size()); }
  bool empty() const noexcept { return events_.empty(); }
  const std::vector<NveEvent>& events() const noexcept { return events_; }
  const std::vector<uint8_t>& arena() const noexcept { return arena_; }

  NveEventBatch as_batch() const noexcept {
    return NveEventBatch{
        events_.empty() ? nullptr : events_.data(),
        arena_.empty() ? nullptr : arena_.data(),
        static_cast<uint32_t>(events_.size()),
        static_cast<uint32_t>(arena_.size()),
    };
  }

  bool push_event(
      uint64_t table,
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      uint32_t event_id,
      uint32_t flags,
      const std::array<uint32_t, 6>& payload,
      const uint8_t* data = nullptr,
      size_t len = 0u) {
    if (events_.size() == static_cast<size_t>(std::numeric_limits<uint32_t>::max()) ||
        (data == nullptr && len > 0u) ||
        len > static_cast<size_t>(std::numeric_limits<uint32_t>::max()) - arena_.size()) {
      return false;
    }
    const uint32_t off = len > 0u ? static_cast<uint32_t>(arena_.size()) : 0u;
    if (len > 0u) {
      arena_.insert(arena_.end(), data, data + len);
    }
    NveEvent ev = make_base_event(table, surface_id, node_id, ts_us, event_id, flags);
    for (size_t i = 0u; i < 6u; ++i) {
      ev.payload[i] = payload[i];
    }
    ev.bytes.offset = off;
    ev.bytes.len = static_cast<uint32_t>(len);
    events_.push_back(ev);
    return true;
  }

  bool push_nve_event(const NveEvent& ev, std::span<const uint8_t> src_arena = {}) {
    if (ev.bytes.len > 0u) {
      if (src_arena.data() == nullptr || ev.bytes.offset > src_arena.size() ||
          ev.bytes.len > src_arena.size() - ev.bytes.offset) {
        return false;
      }
    }
    const std::array<uint32_t, 6> payload{
        ev.payload[0], ev.payload[1], ev.payload[2], ev.payload[3], ev.payload[4], ev.payload[5]};
    const uint8_t* data = ev.bytes.len > 0u ? (src_arena.data() + ev.bytes.offset) : nullptr;
    return push_event(
        ev.table, ev.surface, ev.node, ev.timestamp, ev.event, ev.flags, payload, data, ev.bytes.len);
  }

  bool push_batch(NveEventBatch batch) {
    if ((batch.event_count > 0u && batch.events == nullptr) ||
        (batch.bytes_len > 0u && batch.bytes == nullptr)) {
      return false;
    }
    const size_t ev_rollback = events_.size();
    const size_t arena_rollback = arena_.size();
    const std::span<const uint8_t> src_arena(batch.bytes, batch.bytes_len);
    for (uint32_t i = 0u; i < batch.event_count; ++i) {
      if (!push_nve_event(batch.events[i], src_arena)) {
        events_.resize(ev_rollback);
        arena_.resize(arena_rollback);
        return false;
      }
    }
    return true;
  }

  void push_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    (void)push_nve_event(make_press_event(surface_id, node_id, ts_us, x, y));
  }

  void push_long_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    (void)push_nve_event(make_long_press_event(surface_id, node_id, ts_us, x, y));
  }

  void push_focus(uint32_t surface_id, uint32_t node_id, uint64_t ts_us) {
    (void)push_nve_event(make_focus_event(surface_id, node_id, ts_us));
  }

  void push_blur(uint32_t surface_id, uint32_t node_id, uint64_t ts_us) {
    (void)push_nve_event(make_blur_event(surface_id, node_id, ts_us));
  }

  void push_scroll(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      float x,
      float y,
      float content_width,
      float content_height) {
    (void)push_nve_event(
        make_scroll_event(surface_id, node_id, ts_us, x, y, content_width, content_height));
  }

  void push_layout(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      float x,
      float y,
      float width,
      float height) {
    (void)push_nve_event(make_layout_event(surface_id, node_id, ts_us, x, y, width, height));
  }

  bool push_text_event(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      NveTextInputEvent event_id,
      std::string_view text) {
    if (!text.empty() && text.data() == nullptr) {
      return false;
    }
    return push_event(
        NVE_TABLE_TEXT_INPUT,
        surface_id,
        node_id,
        ts_us,
        static_cast<uint32_t>(event_id),
        NVE_EVENT_FLAG_UTF8_BYTES,
        {0u, 0u, 0u, 0u, 0u, 0u},
        reinterpret_cast<const uint8_t*>(text.data()),
        text.size());
  }

  bool push_text_change(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view text) {
    return push_text_event(surface_id, node_id, ts_us, NVE_TEXT_INPUT_EVENT_CHANGE, text);
  }

  bool push_key_event(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      NveInteractionEvent event_id,
      std::string_view key,
      std::string_view code,
      uint32_t flags,
      uint32_t key_code) {
    if ((!key.empty() && key.data() == nullptr) || (!code.empty() && code.data() == nullptr) ||
        key.size() > static_cast<size_t>(std::numeric_limits<uint32_t>::max())) {
      return false;
    }
    uint32_t f = flags & ~(NVE_EVENT_FLAG_UTF8_BYTES | NVE_EVENT_FLAG_INLINE_KEY_UTF8 |
                           NVE_EVENT_FLAG_CUSTOM_NAMED | NVE_EVENT_FLAG_KEY_SPLIT_BYTES);
    std::array<uint32_t, 6> payload{key_code, static_cast<uint32_t>(key.size()), 0u, 0u, 0u, 0u};
    if (key.size() <= NVE_EVENT_INLINE_KEY_CAP) {
      std::array<uint8_t, NVE_EVENT_INLINE_KEY_CAP> kb{};
      for (size_t i = 0u; i < key.size(); ++i) {
        kb[i] = static_cast<uint8_t>(key[i]);
      }
      for (size_t w = 0u; w < 4u; ++w) {
        payload[2u + w] = wire::read_le_u32(kb.data() + w * 4u);
      }
      f |= NVE_EVENT_FLAG_INLINE_KEY_UTF8;
      if (!code.empty()) {
        f |= NVE_EVENT_FLAG_UTF8_BYTES;
      }
      return push_event(
          NVE_TABLE_INTERACTION,
          surface_id,
          node_id,
          ts_us,
          static_cast<uint32_t>(event_id),
          f,
          payload,
          reinterpret_cast<const uint8_t*>(code.data()),
          code.size());
    }
    if (events_.size() == static_cast<size_t>(std::numeric_limits<uint32_t>::max()) ||
        key.size() > static_cast<size_t>(std::numeric_limits<uint32_t>::max()) - arena_.size() ||
        code.size() >
            static_cast<size_t>(std::numeric_limits<uint32_t>::max()) - arena_.size() - key.size()) {
      return false;
    }
    const uint32_t off = static_cast<uint32_t>(arena_.size());
    arena_.insert(
        arena_.end(),
        reinterpret_cast<const uint8_t*>(key.data()),
        reinterpret_cast<const uint8_t*>(key.data()) + key.size());
    if (!code.empty()) {
      arena_.insert(
          arena_.end(),
          reinterpret_cast<const uint8_t*>(code.data()),
          reinterpret_cast<const uint8_t*>(code.data()) + code.size());
    }
    f |= NVE_EVENT_FLAG_KEY_SPLIT_BYTES | NVE_EVENT_FLAG_UTF8_BYTES;
    NveEvent ev = make_base_event(
        NVE_TABLE_INTERACTION, surface_id, node_id, ts_us, static_cast<uint32_t>(event_id), f);
    for (size_t i = 0u; i < 6u; ++i) {
      ev.payload[i] = payload[i];
    }
    ev.bytes.offset = off;
    ev.bytes.len = static_cast<uint32_t>(key.size() + code.size());
    events_.push_back(ev);
    return true;
  }

  void push_pointer_event(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      NveInteractionEvent event_id,
      float x,
      float y,
      uint32_t flags,
      uint32_t pointer_id = 0u) {
    (void)push_nve_event(
        make_pointer_event(surface_id, node_id, ts_us, event_id, x, y, flags, pointer_id));
  }

  void push_wheel_event(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      float delta_x,
      float delta_y,
      uint32_t flags) {
    (void)push_nve_event(make_wheel_event(surface_id, node_id, ts_us, delta_x, delta_y, flags));
  }

  bool push_custom(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const uint8_t* data,
      size_t len) {
    if ((data == nullptr && len > 0u) || (!name.empty() && name.data() == nullptr) ||
        events_.size() == static_cast<size_t>(std::numeric_limits<uint32_t>::max()) ||
        name.size() > static_cast<size_t>(std::numeric_limits<uint32_t>::max()) - arena_.size() ||
        len >
            static_cast<size_t>(std::numeric_limits<uint32_t>::max()) - arena_.size() - name.size()) {
      return false;
    }
    const size_t total = name.size() + len;
    const uint32_t off = total > 0u ? static_cast<uint32_t>(arena_.size()) : 0u;
    if (!name.empty()) {
      arena_.insert(
          arena_.end(),
          reinterpret_cast<const uint8_t*>(name.data()),
          reinterpret_cast<const uint8_t*>(name.data()) + name.size());
    }
    if (len > 0u) {
      arena_.insert(arena_.end(), data, data + len);
    }
    NveEvent ev = make_base_event(
        NVE_TABLE_CUSTOM, surface_id, node_id, ts_us, 0u, NVE_EVENT_FLAG_CUSTOM_NAMED);
    ev.payload[0] = static_cast<uint32_t>(name.size());
    ev.bytes.offset = off;
    ev.bytes.len = static_cast<uint32_t>(total);
    events_.push_back(ev);
    return true;
  }

  /// Encodes `value` directly into this builder's reusable byte arena as compact `NveBinValue`.
  bool push_custom_value(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const NveValue& value) {
    if ((!name.empty() && name.data() == nullptr) ||
        events_.size() == static_cast<size_t>(std::numeric_limits<uint32_t>::max()) ||
        name.size() > static_cast<size_t>(std::numeric_limits<uint32_t>::max()) - arena_.size()) {
      return false;
    }
    const size_t rollback = arena_.size();
    if (!name.empty()) {
      arena_.insert(
          arena_.end(),
          reinterpret_cast<const uint8_t*>(name.data()),
          reinterpret_cast<const uint8_t*>(name.data()) + name.size());
    }
    if (!encode_bin_value(arena_, value)) {
      arena_.resize(rollback);
      return false;
    }
    const size_t total = arena_.size() - rollback;
    if (total > static_cast<size_t>(std::numeric_limits<uint32_t>::max())) {
      arena_.resize(rollback);
      return false;
    }
    NveEvent ev = make_base_event(
        NVE_TABLE_CUSTOM, surface_id, node_id, ts_us, 0u, NVE_EVENT_FLAG_CUSTOM_NAMED);
    ev.payload[0] = static_cast<uint32_t>(name.size());
    ev.bytes.offset = total > 0u ? static_cast<uint32_t>(rollback) : 0u;
    ev.bytes.len = static_cast<uint32_t>(total);
    events_.push_back(ev);
    return true;
  }

  /// Serialize into a compact `NVEV` v2 wire buffer (14 B header + 64*N B events + arena bytes).
  const std::vector<uint8_t>& finalize() {
    wire_buf_.clear();
    wire_buf_.reserve(
        wire::kEventWireHeaderSize + events_.size() * wire::kEventRecordSize + arena_.size());
    wire_buf_.insert(
        wire_buf_.end(), wire::kEventWireMagicBytes.begin(), wire::kEventWireMagicBytes.end());
    wire::append_le_u16(wire_buf_, wire::kEventWireVersion);
    wire::append_le_u32(wire_buf_, static_cast<uint32_t>(events_.size()));
    wire::append_le_u32(wire_buf_, static_cast<uint32_t>(arena_.size()));
    for (const NveEvent& ev : events_) {
      append_event_le(wire_buf_, ev);
    }
    wire_buf_.insert(wire_buf_.end(), arena_.begin(), arena_.end());
    return wire_buf_;
  }

 private:
  std::vector<NveEvent> events_;
  std::vector<uint8_t> arena_;
  std::vector<uint8_t> wire_buf_;
};

} // namespace nve
