#pragma once

#include "nve_abi.generated.h"
#include "nve_value.hpp"

#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <iostream>
#include <string>
#include <string_view>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace nve::qt_test {

inline bool approx_eq(double a, double b, double eps = 2.0) {
  return std::fabs(a - b) <= eps;
}

class BinaryCmdHelper {
 public:
  std::vector<uint8_t> buf;
  uint32_t ops{0};

  void begin(uint32_t surface_id, uint64_t frame_id) {
    buf.clear();
    ops = 0;
    push<uint32_t>(NVE_ABI_MAGIC);
    push<uint16_t>(NVE_ABI_VERSION);
    push<uint16_t>(0);
    push<uint32_t>(surface_id);
    push<uint64_t>(frame_id);
    push<uint32_t>(0);
  }

  void create_node(uint32_t id, uint16_t kind) {
    push<uint8_t>(NVE_OP_CREATE_NODE);
    push<uint32_t>(id);
    push<uint16_t>(kind);
    push<uint16_t>(0);
    ++ops;
  }

  void remove_node(uint32_t id) {
    push<uint8_t>(NVE_OP_REMOVE_NODE);
    push<uint32_t>(id);
    ++ops;
  }

  void set_children(uint32_t parent, std::initializer_list<uint32_t> kids) {
    push<uint8_t>(NVE_OP_SET_CHILDREN);
    push<uint32_t>(parent);
    push<uint32_t>(static_cast<uint32_t>(kids.size()));
    for (uint32_t k : kids) push<uint32_t>(k);
    ++ops;
  }

  void set_text(uint32_t id, const std::string& text) {
    push<uint8_t>(NVE_OP_SET_TEXT);
    push<uint32_t>(id);
    push<uint32_t>(static_cast<uint32_t>(text.size()));
    buf.insert(buf.end(), text.begin(), text.end());
    ++ops;
  }

  void set_event_mask(uint32_t id, uint64_t mask) {
    push<uint8_t>(NVE_OP_SET_EVENT_MASK);
    push<uint32_t>(id);
    push<uint64_t>(mask);
    ++ops;
  }

  void set_layout(
      uint32_t id,
      uint8_t dir,
      uint8_t wtag,
      float w,
      uint8_t htag,
      float h,
      float grow,
      float pad,
      float gap,
      float shrink = 1.0f) {
    push<uint8_t>(NVE_OP_SET_LAYOUT);
    push<uint32_t>(id);
    push_dim(wtag, w);
    push_dim(htag, h);
    push_dim(0, 0.0f);
    push_dim(0, 0.0f);
    push_dim(0, 0.0f);
    push_dim(0, 0.0f);
    push<uint8_t>(dir);
    push<uint8_t>(0);
    push<uint8_t>(1);
    push<float>(grow);
    push<float>(shrink);
    push<float>(gap);
    push_insets(pad, pad, pad, pad);
    push_insets(0.0f, 0.0f, 0.0f, 0.0f);
    ++ops;
  }

  void set_style(
      uint32_t id,
      uint32_t bg,
      uint32_t fg,
      uint32_t border_rgba,
      float border_w,
      float radius,
      float opacity,
      float font_size,
      int32_t z_index = 0) {
    push<uint8_t>(NVE_OP_SET_STYLE);
    push<uint32_t>(id);
    push<uint32_t>(bg);
    push<uint32_t>(fg);
    push<uint32_t>(border_rgba);
    push<float>(border_w);
    push<float>(radius);
    push<float>(opacity);
    push<float>(font_size);
    push<int32_t>(z_index);
    ++ops;
  }

  void set_raw_prop_string(uint32_t id, std::string_view key, std::string_view val) {
    push<uint8_t>(NVE_OP_SET_RAW_PROP);
    push<uint32_t>(id);
    push<uint16_t>(static_cast<uint16_t>(key.size()));
    buf.insert(buf.end(), key.begin(), key.end());
    push<uint8_t>(4u);
    push<uint32_t>(static_cast<uint32_t>(val.size()));
    buf.insert(buf.end(), val.begin(), val.end());
    ++ops;
  }

  void set_raw_prop_bool(uint32_t id, std::string_view key, bool val) {
    push<uint8_t>(NVE_OP_SET_RAW_PROP);
    push<uint32_t>(id);
    push<uint16_t>(static_cast<uint16_t>(key.size()));
    buf.insert(buf.end(), key.begin(), key.end());
    push<uint8_t>(1u);
    push<uint8_t>(val ? 1u : 0u);
    ++ops;
  }

  NveBufferSlice finish() {
    push<uint8_t>(NVE_OP_COMMIT);
    push<uint8_t>(1);
    ++ops;
    std::memcpy(buf.data() + 20, &ops, sizeof(uint32_t));
    return NveBufferSlice{buf.data(), buf.size()};
  }

 private:
  template <typename T>
  void push(T v) {
    const auto* p = reinterpret_cast<const uint8_t*>(&v);
    buf.insert(buf.end(), p, p + sizeof(T));
  }

  void push_dim(uint8_t tag, float val) {
    push<uint8_t>(tag);
    push<float>(val);
  }

  void push_insets(float t, float r, float b, float l) {
    push<float>(t);
    push<float>(r);
    push<float>(b);
    push<float>(l);
  }
};

struct DirectOutboundQueue {
  struct Pending {
    uint32_t surface_id{0};
    uint32_t node_id{0};
    uint64_t ts{0};
    std::string name;
    std::string payload;
  };
  std::vector<Pending> pending;
  std::vector<Pending> drained;
  std::vector<NveDirectEvent> batch_storage;

  void push_custom(uint32_t sid, uint32_t nid, uint64_t ts, std::string name, std::string payload) {
    pending.push_back(Pending{sid, nid, ts, std::move(name), std::move(payload)});
  }

  NveEventBatch drain() {
    drained.swap(pending);
    pending.clear();
    batch_storage.clear();
    batch_storage.reserve(drained.size());
    for (const auto& p : drained) {
      NveDirectEvent ev{};
      ev.surface_id = p.surface_id;
      ev.node_id = p.node_id;
      ev.timestamp_us = p.ts;
      ev.kind = NVE_EVENT_CUSTOM;
      ev.payload_tag = NVE_EVENT_PAYLOAD_CUSTOM;
      ev.name = nve::make_str_slice(p.name);
      ev.value = nve::make_bytes_value(NveBufferSlice{
          reinterpret_cast<const uint8_t*>(p.payload.data()),
          p.payload.size(),
      });
      batch_storage.push_back(ev);
    }
    return NveEventBatch{
        batch_storage.empty() ? nullptr : batch_storage.data(),
        static_cast<uint32_t>(batch_storage.size()),
        0u,
    };
  }
};

} // namespace nve::qt_test
