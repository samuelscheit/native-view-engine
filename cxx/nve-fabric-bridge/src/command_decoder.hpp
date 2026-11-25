#pragma once

#include "nve_abi.h"

#include <cstring>
#include <string>
#include <string_view>
#include <vector>

namespace nve {

struct Header {
  uint32_t magic{};
  uint16_t version{};
  uint16_t flags{};
  uint32_t surface_id{};
  uint64_t frame_id{};
  uint32_t op_count{};
};

struct Dimension {
  uint8_t tag{}; /* 0 = Auto, 1 = Points, 2 = Percent */
  float value{};
};

struct EdgeInsets {
  float top{};
  float right{};
  float bottom{};
  float left{};
};

struct LayoutProps {
  Dimension width, height, min_width, min_height, max_width, max_height;
  uint8_t flex_direction{};
  uint8_t justify_content{};
  uint8_t align_items{};
  float flex_grow{};
  float flex_shrink{1.0f};
  float gap{};
  EdgeInsets padding;
  EdgeInsets margin;
};

struct StyleProps {
  uint32_t background_rgba{0u};
  uint32_t text_color_rgba{0xFF000000u};
  uint32_t border_color_rgba{0u};
  float border_width{0.0f};
  float border_radius{0.0f};
  float opacity{1.0f};
  float font_size{16.0f};
  int32_t z_index{0};
};

inline bool is_valid_component_kind(uint16_t kind) noexcept {
  return kind <= NVE_COMP_SKIA_CANVAS || kind == NVE_COMP_CUSTOM;
}

inline bool is_valid_dimension(const Dimension& d) noexcept {
  return d.tag <= 2;
}

inline bool is_valid_layout(const LayoutProps& l) noexcept {
  return is_valid_dimension(l.width) && is_valid_dimension(l.height) &&
         is_valid_dimension(l.min_width) && is_valid_dimension(l.min_height) &&
         is_valid_dimension(l.max_width) && is_valid_dimension(l.max_height) &&
         l.flex_direction <= 3 && l.justify_content <= 5 && l.align_items <= 4;
}

class BinaryReader {
 public:
  BinaryReader(const uint8_t* ptr, size_t len) : ptr_(ptr), len_(len), pos_(0) {}

  size_t remaining() const noexcept { return len_ - pos_; }
  size_t position() const noexcept { return pos_; }
  bool ok_pointer() const noexcept { return ptr_ != nullptr || len_ == 0; }

  template <typename T>
  bool read(T& out) {
    if (!ok_pointer() || remaining() < sizeof(T)) return false;
    std::memcpy(&out, ptr_ + pos_, sizeof(T));
    pos_ += sizeof(T);
    return true;
  }

  bool read_string(size_t len, std::string_view& out) {
    if (!ok_pointer() || remaining() < len) return false;
    out = std::string_view(reinterpret_cast<const char*>(ptr_ + pos_), len);
    pos_ += len;
    return true;
  }

  bool read_bytes(size_t len, std::vector<uint8_t>& out) {
    if (!ok_pointer() || remaining() < len) return false;
    out.assign(ptr_ + pos_, ptr_ + pos_ + len);
    pos_ += len;
    return true;
  }

  bool skip(size_t len) {
    if (!ok_pointer() || remaining() < len) return false;
    pos_ += len;
    return true;
  }

  bool read_dim(Dimension& d) { return read(d.tag) && read(d.value); }

  bool read_insets(EdgeInsets& e) {
    return read(e.top) && read(e.right) && read(e.bottom) && read(e.left);
  }

  bool read_layout(LayoutProps& l) {
    return read_dim(l.width) && read_dim(l.height) && read_dim(l.min_width) &&
           read_dim(l.min_height) && read_dim(l.max_width) && read_dim(l.max_height) &&
           read(l.flex_direction) && read(l.justify_content) && read(l.align_items) &&
           read(l.flex_grow) && read(l.flex_shrink) && read(l.gap) &&
           read_insets(l.padding) && read_insets(l.margin);
  }

  bool read_style(StyleProps& s) {
    return read(s.background_rgba) && read(s.text_color_rgba) && read(s.border_color_rgba) &&
           read(s.border_width) && read(s.border_radius) && read(s.opacity) &&
           read(s.font_size) && read(s.z_index);
  }

 private:
  const uint8_t* ptr_;
  size_t len_;
  size_t pos_;
};

} // namespace nve
