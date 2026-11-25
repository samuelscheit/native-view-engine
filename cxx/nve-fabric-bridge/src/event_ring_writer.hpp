#pragma once

#include "nve_abi.generated.h"
#include "nve_value.hpp"

#include <bit>
#include <cstdint>
#include <cstring>
#include <limits>
#include <string_view>
#include <type_traits>
#include <vector>

namespace nve {

/**
 * Lightweight binary EventRing writer matching `crates/nve-abi/src/events.rs`.
 *
 * When compiled with React Native Fabric (`NVE_WITH_RN_FABRIC=1`), our `EventListener`
 * intercepts `facebook::react::RawEvent` in C++ before JSI (returning `true` to suppress JS
 * dispatch). Built-in events downcast `EventPayload::getType()` directly; arbitrary 3rd-party
 * RN Fabric components whose `ValueFactoryEventPayload` only exposes `asJSIValue(jsi::Runtime&)`
 * are captured via `MockJsiRecorderRuntime` directly into `nve::BumpArena` for zero-copy direct
 * FFI dispatch (`dispatch_events_direct`), or encoded in-place into compact binary `NveBinValue`
 * inside this `EventRingWriter` for non-shared boundaries — zero Hermes, zero JSON.
 */
class EventRingWriter {
 public:
  void clear() {
    count_ = 0;
    buf_.clear();
    buf_.resize(4, 0);
  }

  uint32_t count() const noexcept { return count_; }

  void push_empty(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, uint8_t kind) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(surface_id, node_id, ts_us, kind, 0u /* EventPayload::Empty */);
    ++count_;
  }

  void push_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(
        surface_id, node_id, ts_us, 1u /* NativeEventKind::Press */, 1u /* EventPayload::Point */);
    write_le<float>(x);
    write_le<float>(y);
    ++count_;
  }

  void push_long_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        2u /* NativeEventKind::LongPress */,
        1u /* EventPayload::Point */);
    write_le<float>(x);
    write_le<float>(y);
    ++count_;
  }

  void push_scroll(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      float x,
      float y,
      float content_width,
      float content_height) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        3u /* NativeEventKind::Scroll */,
        2u /* EventPayload::Scroll */);
    write_le<float>(x);
    write_le<float>(y);
    write_le<float>(content_width);
    write_le<float>(content_height);
    ++count_;
  }

  bool push_text_change(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view text) {
    if (count_ == std::numeric_limits<uint32_t>::max() ||
        text.size() > static_cast<size_t>(std::numeric_limits<uint32_t>::max()) ||
        (!text.empty() && text.data() == nullptr)) {
      return false;
    }
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        4u /* NativeEventKind::TextChange */,
        3u /* EventPayload::Text */);
    write_le<uint32_t>(static_cast<uint32_t>(text.size()));
    if (!text.empty()) {
      buf_.insert(
          buf_.end(),
          reinterpret_cast<const uint8_t*>(text.data()),
          reinterpret_cast<const uint8_t*>(text.data()) + text.size());
    }
    ++count_;
    return true;
  }

  void push_layout(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      float x,
      float y,
      float width,
      float height) {
    if (count_ == std::numeric_limits<uint32_t>::max()) {
      return;
    }
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        7u /* NativeEventKind::Layout */,
        4u /* EventPayload::Layout */);
    write_le<float>(x);
    write_le<float>(y);
    write_le<float>(width);
    write_le<float>(height);
    ++count_;
  }

  bool push_custom(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const uint8_t* data,
      size_t len) {
    if ((data == nullptr && len > 0u) ||
        len > static_cast<size_t>(std::numeric_limits<uint32_t>::max())) {
      return false;
    }
    return write_custom_event(surface_id, node_id, ts_us, name, [&](std::vector<uint8_t>& buf) {
      if (len > 0u) {
        buf.insert(buf.end(), data, data + len);
      }
      return true;
    });
  }

  /// Encodes `value` directly into this `EventRingWriter`'s preallocated byte buffer using compact
  /// binary `NveBinValue` (zero intermediate string/vector allocation, zero JSON).
  bool push_custom_value(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      const NveValue& value) {
    return write_custom_event(surface_id, node_id, ts_us, name, [&](std::vector<uint8_t>& buf) {
      return encode_bin_value(buf, value);
    });
  }

  /// Encode a single `NveDirectEvent` into this binary `EventRingWriter` (for non-shared boundaries).
  /// Rejects unknown `payload_tag` values cleanly instead of silently mapping to Custom.
  bool push_direct_event(const NveDirectEvent& ev) {
    switch (ev.payload_tag) {
      case 0u:
        push_empty(ev.surface_id, ev.node_id, ev.timestamp_us, ev.kind);
        return true;
      case 1u:
        if (ev.kind == 2u) {
          push_long_press(ev.surface_id, ev.node_id, ev.timestamp_us, ev.x, ev.y);
        } else {
          push_press(ev.surface_id, ev.node_id, ev.timestamp_us, ev.x, ev.y);
        }
        return true;
      case 2u:
        push_scroll(ev.surface_id, ev.node_id, ev.timestamp_us, ev.x, ev.y, ev.width, ev.height);
        return true;
      case 3u:
        return push_text_change(ev.surface_id, ev.node_id, ev.timestamp_us, str_slice_view(ev.text));
      case 4u:
        push_layout(ev.surface_id, ev.node_id, ev.timestamp_us, ev.x, ev.y, ev.width, ev.height);
        return true;
      case 5u:
        return push_custom_value(
            ev.surface_id, ev.node_id, ev.timestamp_us, str_slice_view(ev.name), ev.value);
      default:
        return false;
    }
  }

  const std::vector<uint8_t>& finalize() {
    ensure_header();
    std::memcpy(buf_.data(), &count_, sizeof(uint32_t));
    return buf_;
  }

 private:
  void ensure_header() {
    if (buf_.size() < 4u) {
      buf_.resize(4u, 0u);
    }
  }

  template <typename T>
  void write_le(T v) {
    static_assert(std::is_trivially_copyable_v<T>);
    if constexpr (std::is_same_v<T, float>) {
      write_le<uint32_t>(std::bit_cast<uint32_t>(v));
    } else if constexpr (std::is_integral_v<T>) {
      using U = std::make_unsigned_t<T>;
      const U u = static_cast<U>(v);
      for (size_t i = 0u; i < sizeof(U); ++i) {
        buf_.push_back(static_cast<uint8_t>((u >> (i * 8u)) & 0xFFu));
      }
    } else {
      const auto* p = reinterpret_cast<const uint8_t*>(&v);
      buf_.insert(buf_.end(), p, p + sizeof(T));
    }
  }

  void write_event_header(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      uint8_t kind,
      uint8_t payload_tag) {
    ensure_header();
    write_le<uint32_t>(surface_id);
    write_le<uint32_t>(node_id);
    write_le<uint64_t>(ts_us);
    buf_.push_back(kind);
    buf_.push_back(payload_tag);
  }

  template <typename PayloadWriter>
  bool write_custom_event(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      std::string_view name,
      PayloadWriter&& write_payload) {
    if (count_ == std::numeric_limits<uint32_t>::max() ||
        name.size() > static_cast<size_t>(std::numeric_limits<uint16_t>::max()) ||
        (!name.empty() && name.data() == nullptr)) {
      return false;
    }
    ensure_header();
    const size_t rollback = buf_.size();
    write_event_header(
        surface_id,
        node_id,
        ts_us,
        8u /* NativeEventKind::Custom */,
        5u /* EventPayload::Custom */);
    const uint16_t nlen = static_cast<uint16_t>(name.size());
    write_le<uint16_t>(nlen);
    if (nlen > 0u) {
      buf_.insert(
          buf_.end(),
          reinterpret_cast<const uint8_t*>(name.data()),
          reinterpret_cast<const uint8_t*>(name.data()) + nlen);
    }
    const size_t len_offset = buf_.size();
    write_le<uint32_t>(0u);
    const size_t payload_start = buf_.size();
    if (!write_payload(buf_)) {
      buf_.resize(rollback);
      return false;
    }
    const size_t payload_bytes = buf_.size() - payload_start;
    if (payload_bytes > static_cast<size_t>(std::numeric_limits<uint32_t>::max())) {
      buf_.resize(rollback);
      return false;
    }
    const uint32_t payload_len = static_cast<uint32_t>(payload_bytes);
    buf_[len_offset] = static_cast<uint8_t>(payload_len & 0xFFu);
    buf_[len_offset + 1u] = static_cast<uint8_t>((payload_len >> 8u) & 0xFFu);
    buf_[len_offset + 2u] = static_cast<uint8_t>((payload_len >> 16u) & 0xFFu);
    buf_[len_offset + 3u] = static_cast<uint8_t>((payload_len >> 24u) & 0xFFu);
    ++count_;
    return true;
  }

  uint32_t count_{0};
  std::vector<uint8_t> buf_;
};

} // namespace nve
