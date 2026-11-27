// NVE host runtime: decoders (and encoders, for C++ apps / fixtures) of the core resource kinds
// a native view host renders, byte-identical to `nve_components::tables::resources`:
//
//   nve.res.image_src   u8 tag; 0 = Url (u32 len + UTF-8), 1 = EncodedBytes (u32 len + bytes)
//   nve.res.box_shadows u32 count (<= 4096); per shadow f32 offset_x, offset_y, blur, spread,
//                       u32 RGBA, u8 inset
//
// Decoders return views into the given bytes (no allocation except the shadow list) and reject
// trailing bytes, over-limit counts, and invalid UTF-8 exactly like Rust.
#pragma once

#include <algorithm>
#include <bit>
#include <cstdint>
#include <optional>
#include <span>
#include <string_view>
#include <vector>

#include "api.h"
#include "utf8.h"
#include "xxh64.hpp"

namespace nve::host {

inline constexpr Key64 kImageSrcKind = Key64::of("nve.res.image_src");
inline constexpr Key64 kBoxShadowsKind = Key64::of("nve.res.box_shadows");

namespace detail {
class ResourceCursor {
 public:
  explicit ResourceCursor(std::span<const uint8_t> b) noexcept : b_(b) {}
  [[nodiscard]] std::optional<std::span<const uint8_t>> take(size_t n) noexcept {
    if (n > b_.size() - at_) return std::nullopt;
    const auto s = b_.subspan(at_, n);
    at_ += n;
    return s;
  }
  [[nodiscard]] std::optional<uint8_t> u8() noexcept {
    const auto s = take(1u);
    return s ? std::optional<uint8_t>((*s)[0]) : std::nullopt;
  }
  [[nodiscard]] std::optional<uint32_t> u32() noexcept {
    const auto s = take(4u);
    return s ? std::optional<uint32_t>(nve_load_u32_le(s->data())) : std::nullopt;
  }
  [[nodiscard]] std::optional<float> f32() noexcept {
    const auto v = u32();
    return v ? std::optional<float>(std::bit_cast<float>(*v)) : std::nullopt;
  }
  [[nodiscard]] std::optional<uint32_t> count(uint32_t max) noexcept {
    const auto n = u32();
    return n && *n <= max ? n : std::nullopt;
  }
  [[nodiscard]] bool done() const noexcept { return at_ == b_.size(); }

 private:
  std::span<const uint8_t> b_;
  size_t at_ = 0u;
};

inline void put_u32(std::vector<uint8_t>& out, uint32_t v) {
  for (uint32_t s = 0; s < 32u; s += 8u) out.push_back(static_cast<uint8_t>(v >> s));
}
}  // namespace detail

/// Decoded `ImageSrc`: exactly one of `url` / `encoded` is set.
struct ImageSource {
  std::string_view url;
  std::span<const uint8_t> encoded;
  [[nodiscard]] bool is_url() const noexcept { return encoded.empty(); }
};

[[nodiscard]] inline std::optional<ImageSource> decode_image_src(std::span<const uint8_t> bytes) noexcept {
  constexpr uint32_t kMaxString = 256u * 1024u;
  constexpr uint32_t kMaxImage = 32u * 1024u * 1024u;
  detail::ResourceCursor r(bytes);
  const auto tag = r.u8();
  if (!tag || *tag > 1u) return std::nullopt;
  const auto len = r.count(*tag == 0u ? kMaxString : kMaxImage);
  if (!len) return std::nullopt;
  const auto body = r.take(*len);
  if (!body || !r.done()) return std::nullopt;
  if (*tag == 1u) return ImageSource{{}, *body};
  if (!body->empty() && nve_utf8_valid(body->data(), body->size()) == 0) return std::nullopt;
  return ImageSource{std::string_view(reinterpret_cast<const char*>(body->data()), body->size()), {}};
}

[[nodiscard]] inline std::vector<uint8_t> encode_image_url(std::string_view url) {
  std::vector<uint8_t> out{0u};
  detail::put_u32(out, static_cast<uint32_t>(url.size()));
  out.insert(out.end(), url.begin(), url.end());
  return out;
}

[[nodiscard]] inline std::vector<uint8_t> encode_image_bytes(std::span<const uint8_t> encoded) {
  std::vector<uint8_t> out{1u};
  detail::put_u32(out, static_cast<uint32_t>(encoded.size()));
  out.insert(out.end(), encoded.begin(), encoded.end());
  return out;
}

struct BoxShadow {
  float offset_x = 0.0f;
  float offset_y = 0.0f;
  float blur_radius = 0.0f;
  float spread_radius = 0.0f;
  uint32_t color = 0u;  // packed RGBA8888
  bool inset = false;
  friend bool operator==(const BoxShadow&, const BoxShadow&) noexcept = default;
};

[[nodiscard]] inline std::optional<std::vector<BoxShadow>> decode_box_shadows(std::span<const uint8_t> bytes) {
  detail::ResourceCursor r(bytes);
  const auto n = r.count(4096u);
  if (!n) return std::nullopt;
  std::vector<BoxShadow> out;
  out.reserve(std::min<uint32_t>(*n, 16u));
  for (uint32_t i = 0; i < *n; ++i) {
    const auto x = r.f32(), y = r.f32(), blur = r.f32(), spread = r.f32();
    const auto color = r.u32();
    const auto inset = r.u8();
    if (!x || !y || !blur || !spread || !color || !inset || *inset > 1u) return std::nullopt;
    out.push_back(BoxShadow{*x, *y, *blur, *spread, *color, *inset == 1u});
  }
  if (!r.done()) return std::nullopt;
  return out;
}

[[nodiscard]] inline std::vector<uint8_t> encode_box_shadows(std::span<const BoxShadow> shadows) {
  std::vector<uint8_t> out;
  detail::put_u32(out, static_cast<uint32_t>(shadows.size()));
  for (const BoxShadow& s : shadows) {
    for (const float f : {s.offset_x, s.offset_y, s.blur_radius, s.spread_radius}) {
      detail::put_u32(out, std::bit_cast<uint32_t>(f));
    }
    detail::put_u32(out, s.color);
    out.push_back(s.inset ? 1u : 0u);
  }
  return out;
}

}  // namespace nve::host
