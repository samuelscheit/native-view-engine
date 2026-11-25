#pragma once

#include "command_decoder.hpp"
#include "nve_abi.h"

#include <cstddef>
#include <cstdint>
#include <cstring>
#include <string_view>
#include <vector>

namespace nve {

/// Zero-dependency binary CommandBuffer encoder matching `crates/nve-abi/src/commands.rs`
/// and `cxx/nve-fabric-bridge/src/fabric_adapter.cpp`.
class CommandBufferEncoder {
 public:
  CommandBufferEncoder(uint32_t surface_id, uint64_t frame_id) {
    reset(surface_id, frame_id);
  }

  void reset(uint32_t surface_id, uint64_t frame_id) {
    op_count_ = 0;
    buf_.clear();
    push<uint32_t>(NVE_ABI_MAGIC);
    push<uint16_t>(NVE_ABI_VERSION);
    push<uint16_t>(0);
    push<uint32_t>(surface_id);
    push<uint64_t>(frame_id);
    push<uint32_t>(0); /* op_count patched in finish() */
  }

  void reserve(size_t bytes) { buf_.reserve(bytes); }

  void create_node(uint32_t id, uint16_t kind, std::string_view custom_name = {}) {
    push<uint8_t>(NVE_OP_CREATE_NODE);
    push<uint32_t>(id);
    push<uint16_t>(kind);
    push<uint16_t>(static_cast<uint16_t>(custom_name.size()));
    push_bytes(custom_name.data(), custom_name.size());
    ++op_count_;
  }

  void remove_node(uint32_t id) {
    push<uint8_t>(NVE_OP_REMOVE_NODE);
    push<uint32_t>(id);
    ++op_count_;
  }

  void set_children(uint32_t parent_id, const std::vector<uint32_t>& children) {
    push<uint8_t>(NVE_OP_SET_CHILDREN);
    push<uint32_t>(parent_id);
    push<uint32_t>(static_cast<uint32_t>(children.size()));
    for (uint32_t c : children) {
      push<uint32_t>(c);
    }
    ++op_count_;
  }

  void set_text(uint32_t id, std::string_view text) {
    push<uint8_t>(NVE_OP_SET_TEXT);
    push<uint32_t>(id);
    push<uint32_t>(static_cast<uint32_t>(text.size()));
    push_bytes(text.data(), text.size());
    ++op_count_;
  }

  void set_layout(uint32_t id, const LayoutProps& l) {
    push<uint8_t>(NVE_OP_SET_LAYOUT);
    push<uint32_t>(id);
    push_dim(l.width);
    push_dim(l.height);
    push_dim(l.min_width);
    push_dim(l.min_height);
    push_dim(l.max_width);
    push_dim(l.max_height);
    push<uint8_t>(l.flex_direction);
    push<uint8_t>(l.justify_content);
    push<uint8_t>(l.align_items);
    push<float>(l.flex_grow);
    push<float>(l.flex_shrink);
    push<float>(l.gap);
    push_insets(l.padding);
    push_insets(l.margin);
    ++op_count_;
  }

  void set_style(uint32_t id, const StyleProps& s) {
    push<uint8_t>(NVE_OP_SET_STYLE);
    push<uint32_t>(id);
    push<uint32_t>(s.background_rgba);
    push<uint32_t>(s.text_color_rgba);
    push<uint32_t>(s.border_color_rgba);
    push<float>(s.border_width);
    push<float>(s.border_radius);
    push<float>(s.opacity);
    push<float>(s.font_size);
    push<int32_t>(s.z_index);
    ++op_count_;
  }

  void set_raw_prop_null(uint32_t id, std::string_view key) {
    write_raw_prop_header(id, key, 0);
  }

  void set_raw_prop_bool(uint32_t id, std::string_view key, bool v) {
    write_raw_prop_header(id, key, 1);
    push<uint8_t>(v ? 1 : 0);
  }

  void set_raw_prop_i64(uint32_t id, std::string_view key, int64_t v) {
    write_raw_prop_header(id, key, 2);
    push<int64_t>(v);
  }

  void set_raw_prop_f64(uint32_t id, std::string_view key, double v) {
    write_raw_prop_header(id, key, 3);
    push<double>(v);
  }

  void set_raw_prop_string(uint32_t id, std::string_view key, std::string_view v) {
    write_raw_prop_header(id, key, 4);
    push<uint32_t>(static_cast<uint32_t>(v.size()));
    push_bytes(v.data(), v.size());
  }

  void set_raw_prop_bytes(uint32_t id, std::string_view key, const std::vector<uint8_t>& v) {
    write_raw_prop_header(id, key, 5);
    push<uint32_t>(static_cast<uint32_t>(v.size()));
    push_bytes(v.data(), v.size());
  }

  void set_event_mask(uint32_t id, uint64_t mask) {
    push<uint8_t>(NVE_OP_SET_EVENT_MASK);
    push<uint32_t>(id);
    push<uint64_t>(mask);
    ++op_count_;
  }

  void dispatch_command(uint32_t id, std::string_view name, const std::vector<uint8_t>& payload) {
    push<uint8_t>(NVE_OP_DISPATCH_COMMAND);
    push<uint32_t>(id);
    push<uint16_t>(static_cast<uint16_t>(name.size()));
    push_bytes(name.data(), name.size());
    push<uint32_t>(static_cast<uint32_t>(payload.size()));
    push_bytes(payload.data(), payload.size());
    ++op_count_;
  }

  void commit(bool sync_mount) {
    push<uint8_t>(NVE_OP_COMMIT);
    push<uint8_t>(sync_mount ? 1 : 0);
    ++op_count_;
  }

  NveBufferSlice finish() {
    std::memcpy(buf_.data() + 20, &op_count_, sizeof(uint32_t));
    return NveBufferSlice{buf_.data(), buf_.size()};
  }

  std::vector<uint8_t>& raw() { return buf_; }
  const std::vector<uint8_t>& raw() const { return buf_; }
  uint32_t op_count() const noexcept { return op_count_; }

 private:
  template <typename T>
  void push(T value) {
    const auto* p = reinterpret_cast<const uint8_t*>(&value);
    buf_.insert(buf_.end(), p, p + sizeof(T));
  }

  void push_bytes(const void* data, size_t len) {
    if (len == 0 || data == nullptr) return;
    const auto* p = static_cast<const uint8_t*>(data);
    buf_.insert(buf_.end(), p, p + len);
  }

  void push_dim(const Dimension& d) {
    push<uint8_t>(d.tag);
    push<float>(d.value);
  }

  void push_insets(const EdgeInsets& e) {
    push<float>(e.top);
    push<float>(e.right);
    push<float>(e.bottom);
    push<float>(e.left);
  }

  void write_raw_prop_header(uint32_t id, std::string_view key, uint8_t tag) {
    push<uint8_t>(NVE_OP_SET_RAW_PROP);
    push<uint32_t>(id);
    push<uint16_t>(static_cast<uint16_t>(key.size()));
    push_bytes(key.data(), key.size());
    push<uint8_t>(tag);
    ++op_count_;
  }

  uint32_t op_count_{0};
  std::vector<uint8_t> buf_;
};

} // namespace nve
