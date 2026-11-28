// NVE host runtime: decoders (and encoders, for C++ apps / fixtures) of the core `FlatResource`
// kinds a native view host renders, word-identical to `nve_tables::tables::resources`:
//
//   nve.res.image_src   u32 tag (0 = Url, 1 = EncodedBytes) | u32 byte_len | payload padded to 4 B
//   nve.res.box_shadows u32 count (<= 4096); per shadow 6 u32 words:
//                       f32 offset_x, offset_y, blur, spread, u32 RGBA, u32 inset (0 | 1)
//
// Decoders return views into the given bytes (no allocation except the shadow list) and reject
// unaligned lengths, trailing bytes, over-limit counts, and invalid UTF-8 exactly like Rust.
#pragma once

#include <algorithm>
#include <bit>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <optional>
#include <span>
#include <string>
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

inline void pad4(std::vector<uint8_t>& out) {
  while ((out.size() & 3u) != 0u) out.push_back(0u);
}
}  // namespace detail

/// Decoded `ImageSrc`: `Kind::Url` (`url`) or `Kind::EncodedBytes` (`encoded`).
struct ImageSource {
  enum class Kind : uint8_t { Url = 0u, EncodedBytes = 1u };
  Kind kind = Kind::Url;
  std::string_view url;
  std::span<const uint8_t> encoded;
  [[nodiscard]] constexpr bool is_url() const noexcept { return kind == Kind::Url; }
  [[nodiscard]] constexpr bool is_encoded_bytes() const noexcept { return kind == Kind::EncodedBytes; }
  [[nodiscard]] static constexpr ImageSource from_url(std::string_view u) noexcept {
    return ImageSource{Kind::Url, u, {}};
  }
  [[nodiscard]] static constexpr ImageSource from_encoded(std::span<const uint8_t> bytes) noexcept {
    return ImageSource{Kind::EncodedBytes, {}, bytes};
  }
};

/// Stable decoded-image cache key shared by every host: the URL itself, or
/// `bytes:<xxh64 hex>:<len>` for inline bytes (never collides with `Url("")`).
[[nodiscard]] inline std::string image_cache_key(const ImageSource& src) {
  if (src.is_url()) return std::string(src.url);
  char key[48];
  std::snprintf(key, sizeof(key), "bytes:%016llx:%zu", static_cast<unsigned long long>(xxh64(src.encoded)),
                src.encoded.size());
  return key;
}

[[nodiscard]] inline std::optional<ImageSource> decode_image_src(std::span<const uint8_t> bytes) noexcept {
  constexpr uint32_t kMaxString = 256u * 1024u;
  constexpr uint32_t kMaxImage = 32u * 1024u * 1024u;
  if ((bytes.size() & 3u) != 0u) return std::nullopt;
  detail::ResourceCursor r(bytes);
  const auto tag = r.u32();
  if (!tag || *tag > 1u) return std::nullopt;
  const auto len = r.count(*tag == 0u ? kMaxString : kMaxImage);
  if (!len) return std::nullopt;
  const size_t padded = (static_cast<size_t>(*len) + 3u) & ~size_t{3u};
  const auto raw = r.take(padded);
  if (!raw || !r.done()) return std::nullopt;
  for (const uint8_t pad : raw->subspan(*len)) {
    if (pad != 0u) return std::nullopt;
  }
  const std::span<const uint8_t> body = raw->first(*len);
  if (*tag == 1u) return ImageSource::from_encoded(body);
  if (!body.empty() && nve_utf8_valid(body.data(), body.size()) == 0) return std::nullopt;
  return ImageSource::from_url(std::string_view(reinterpret_cast<const char*>(body.data()), body.size()));
}

[[nodiscard]] inline std::vector<uint8_t> encode_image_url(std::string_view url) {
  std::vector<uint8_t> out;
  detail::put_u32(out, 0u);
  detail::put_u32(out, static_cast<uint32_t>(url.size()));
  out.insert(out.end(), url.begin(), url.end());
  detail::pad4(out);
  return out;
}

[[nodiscard]] inline std::vector<uint8_t> encode_image_bytes(std::span<const uint8_t> encoded) {
  std::vector<uint8_t> out;
  detail::put_u32(out, 1u);
  detail::put_u32(out, static_cast<uint32_t>(encoded.size()));
  out.insert(out.end(), encoded.begin(), encoded.end());
  detail::pad4(out);
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
  if ((bytes.size() & 3u) != 0u) return std::nullopt;
  detail::ResourceCursor r(bytes);
  const auto n = r.count(4096u);
  if (!n) return std::nullopt;
  std::vector<BoxShadow> out;
  out.reserve(std::min<uint32_t>(*n, 16u));
  for (uint32_t i = 0u; i < *n; ++i) {
    const auto x = r.f32(), y = r.f32(), blur = r.f32(), spread = r.f32();
    const auto color = r.u32();
    const auto inset = r.u32();
    if (!x || !y || !blur || !spread || !color || !inset || *inset > 1u) return std::nullopt;
    // Sealed like `read_node` floats: NaN / Inf never reach a painter (an infinite D2D geometry
    // fails the whole frame); blur is a non-negative extent, spread may be negative.
    const auto finite = [](float v) { return std::isfinite(v) ? v : 0.0f; };
    out.push_back(BoxShadow{finite(*x), finite(*y), std::max(finite(*blur), 0.0f), finite(*spread), *color,
                            *inset == 1u});
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
    detail::put_u32(out, s.inset ? 1u : 0u);
  }
  return out;
}

}  // namespace nve::host
