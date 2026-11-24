#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace nve {

/**
 * Lightweight binary EventRing writer matching `crates/nve-abi/src/events.rs`.
 *
 * When compiled with React Native Fabric (`NVE_WITH_RN_FABRIC=1`), our `EventListener`
 * intercepts `facebook::react::RawEvent` in C++ before JSI (returning `true` to suppress JS
 * dispatch). Built-in events downcast `EventPayload::getType()` directly; arbitrary 3rd-party
 * RN Fabric components whose `ValueFactoryEventPayload` only exposes `asJSIValue(jsi::Runtime&)`
 * are captured via a ~200-line mock `jsi::Runtime` recorder directly into this EventRing —
 * requiring zero Hermes runtime and zero JS heap.
 */
class EventRingWriter {
 public:
  void clear() {
    count_ = 0;
    buf_.clear();
    buf_.resize(4, 0);
  }

  void push_press(uint32_t surface_id, uint32_t node_id, uint64_t ts_us, float x, float y) {
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(1); /* NativeEventKind::Press */
    buf_.push_back(1); /* EventPayload::Point */
    write_f32(x);
    write_f32(y);
    ++count_;
  }

  void push_custom(
      uint32_t surface_id,
      uint32_t node_id,
      uint64_t ts_us,
      const std::string& name,
      const uint8_t* data,
      size_t len) {
    ensure_header();
    write_u32(surface_id);
    write_u32(node_id);
    write_u64(ts_us);
    buf_.push_back(8); /* NativeEventKind::Custom */
    buf_.push_back(5); /* EventPayload::Custom */
    uint16_t nlen = static_cast<uint16_t>(name.size());
    write_u16(nlen);
    buf_.insert(buf_.end(), name.begin(), name.end());
    write_u32(static_cast<uint32_t>(len));
    if (len > 0 && data != nullptr) {
      buf_.insert(buf_.end(), data, data + len);
    }
    ++count_;
  }

  const std::vector<uint8_t>& finalize() {
    ensure_header();
    std::memcpy(buf_.data(), &count_, sizeof(uint32_t));
    return buf_;
  }

 private:
  void ensure_header() {
    if (buf_.size() < 4) {
      buf_.resize(4, 0);
    }
  }
  void write_u16(uint16_t v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf_.insert(buf_.end(), p, p + 2);
  }
  void write_u32(uint32_t v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf_.insert(buf_.end(), p, p + 4);
  }
  void write_u64(uint64_t v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf_.insert(buf_.end(), p, p + 8);
  }
  void write_f32(float v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf_.insert(buf_.end(), p, p + 4);
  }

  uint32_t count_{0};
  std::vector<uint8_t> buf_;
};

} // namespace nve
