#pragma once

#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace nve {

/// 9-byte ASCII magic header identifying a binary `NVE_DRAW1` vector draw-list stream.
inline constexpr std::array<uint8_t, 9> kDrawListMagic = {
    'N', 'V', 'E', '_', 'D', 'R', 'A', 'W', '1'};

/// Current binary format version of `NVE_DRAW1`.
inline constexpr uint16_t kDrawListVersion = 1u;

/// Fixed byte length of the `NVE_DRAW1` header (`9 + 2 + 4`).
inline constexpr size_t kDrawListHeaderSize = 15u;

/// Strict upper bound on operations per draw list to guard against corrupt/hostile payloads.
inline constexpr size_t kMaxDrawOps = 65536u;

/// Strict upper bound on vertices inside a single `StrokePolyline` operation.
inline constexpr size_t kMaxPolylinePoints = 16384u;

/// Compact ASCII prefix used when transporting `NVE_DRAW1` bytes via UTF-8 string props.
inline constexpr std::string_view kDrawListAttrPrefix = "nve-draw1;b64,";

/// Wire opcodes matching `packages/core/tables/src/canvas_ir.rs` byte-for-byte.
enum class DrawOpcode : uint8_t {
  Clear = 1,
  FillRect = 2,
  FillRoundedRect = 3,
  FillCircle = 4,
  StrokeLine = 5,
  StrokePolyline = 6,
  ClipRect = 7,
  RestoreClip = 8,
};

/// Unpacked RGBA8888 color matching `nve_abi::ColorRgba` (`to_u32` packs `[r, g, b, a]` LE).
struct DrawColorRgba {
  uint8_t r{0};
  uint8_t g{0};
  uint8_t b{0};
  uint8_t a{0};

  static constexpr DrawColorRgba rgb(uint8_t red, uint8_t green, uint8_t blue) noexcept {
    return DrawColorRgba{red, green, blue, 255u};
  }

  static constexpr DrawColorRgba rgba(
      uint8_t red,
      uint8_t green,
      uint8_t blue,
      uint8_t alpha) noexcept {
    return DrawColorRgba{red, green, blue, alpha};
  }

  static constexpr DrawColorRgba from_u32(uint32_t v) noexcept {
    return DrawColorRgba{
        static_cast<uint8_t>(v & 0xFFu),
        static_cast<uint8_t>((v >> 8u) & 0xFFu),
        static_cast<uint8_t>((v >> 16u) & 0xFFu),
        static_cast<uint8_t>((v >> 24u) & 0xFFu),
    };
  }

  constexpr uint32_t to_u32() const noexcept {
    return static_cast<uint32_t>(r) | (static_cast<uint32_t>(g) << 8u) |
           (static_cast<uint32_t>(b) << 16u) | (static_cast<uint32_t>(a) << 24u);
  }

  constexpr bool operator==(const DrawColorRgba&) const noexcept = default;
};

struct DrawPoint {
  float x{0.0f};
  float y{0.0f};

  constexpr bool operator==(const DrawPoint&) const noexcept = default;
};

namespace detail {

inline uint16_t read_le_u16_unaligned(const uint8_t* p) noexcept {
  return static_cast<uint16_t>(
      static_cast<uint16_t>(p[0]) | (static_cast<uint16_t>(p[1]) << 8u));
}

inline uint32_t read_le_u32_unaligned(const uint8_t* p) noexcept {
  return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8u) |
         (static_cast<uint32_t>(p[2]) << 16u) | (static_cast<uint32_t>(p[3]) << 24u);
}

inline float read_le_f32_unaligned(const uint8_t* p) noexcept {
  const uint32_t bits = read_le_u32_unaligned(p);
  float val = 0.0f;
  std::memcpy(&val, &bits, sizeof(float));
  return val;
}

inline void write_le_u16(std::vector<uint8_t>& buf, uint16_t v) {
  buf.push_back(static_cast<uint8_t>(v & 0xFFu));
  buf.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
}

inline void write_le_u32(std::vector<uint8_t>& buf, uint32_t v) {
  buf.push_back(static_cast<uint8_t>(v & 0xFFu));
  buf.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
  buf.push_back(static_cast<uint8_t>((v >> 16u) & 0xFFu));
  buf.push_back(static_cast<uint8_t>((v >> 24u) & 0xFFu));
}

inline void write_le_f32(std::vector<uint8_t>& buf, float v) {
  const float finite = std::isfinite(v) ? v : 0.0f;
  uint32_t bits = 0u;
  std::memcpy(&bits, &finite, sizeof(uint32_t));
  write_le_u32(buf, bits);
}

inline int decode_b64_digit(uint8_t b) noexcept {
  if (b >= 'A' && b <= 'Z') return static_cast<int>(b - 'A');
  if (b >= 'a' && b <= 'z') return static_cast<int>(b - 'a' + 26);
  if (b >= '0' && b <= '9') return static_cast<int>(b - '0' + 52);
  if (b == '+') return 62;
  if (b == '/') return 63;
  return -1;
}

} // namespace detail

/// Zero-allocation random-access view over validated little-endian `(f32, f32)` vertices inside
/// an `NVE_DRAW1` `StrokePolyline` payload. Safe on unaligned pointers.
class PolylinePointView {
 public:
  class Iterator {
   public:
    Iterator(const uint8_t* base, size_t idx) noexcept : base_(base), idx_(idx) {}

    DrawPoint operator*() const noexcept {
      const uint8_t* p = base_ + idx_ * 8u;
      return DrawPoint{
          detail::read_le_f32_unaligned(p),
          detail::read_le_f32_unaligned(p + 4u),
      };
    }

    Iterator& operator++() noexcept {
      ++idx_;
      return *this;
    }

    bool operator==(const Iterator& other) const noexcept { return idx_ == other.idx_; }
    bool operator!=(const Iterator& other) const noexcept { return idx_ != other.idx_; }

   private:
    const uint8_t* base_;
    size_t idx_;
  };

  constexpr PolylinePointView() noexcept = default;
  constexpr PolylinePointView(const uint8_t* data, uint32_t count) noexcept
      : data_(data), count_(count) {}

  constexpr size_t size() const noexcept { return static_cast<size_t>(count_); }
  constexpr bool empty() const noexcept { return count_ == 0u; }

  DrawPoint operator[](size_t index) const noexcept {
    const uint8_t* p = data_ + index * 8u;
    return DrawPoint{
        detail::read_le_f32_unaligned(p),
        detail::read_le_f32_unaligned(p + 4u),
    };
  }

  Iterator begin() const noexcept { return Iterator(data_, 0u); }
  Iterator end() const noexcept { return Iterator(data_, static_cast<size_t>(count_)); }

 private:
  const uint8_t* data_{nullptr};
  uint32_t count_{0u};
};

struct DrawClearOp {
  DrawColorRgba color{};
};

struct DrawFillRectOp {
  float x{0.0f};
  float y{0.0f};
  float w{0.0f};
  float h{0.0f};
  DrawColorRgba color{};
};

struct DrawFillRoundedRectOp {
  float x{0.0f};
  float y{0.0f};
  float w{0.0f};
  float h{0.0f};
  float radius{0.0f};
  DrawColorRgba color{};
};

struct DrawFillCircleOp {
  float cx{0.0f};
  float cy{0.0f};
  float radius{0.0f};
  DrawColorRgba color{};
};

struct DrawStrokeLineOp {
  float x0{0.0f};
  float y0{0.0f};
  float x1{0.0f};
  float y1{0.0f};
  float width{0.0f};
  DrawColorRgba color{};
};

struct DrawStrokePolylineOp {
  PolylinePointView points{};
  float width{0.0f};
  DrawColorRgba color{};
};

struct DrawClipRectOp {
  float x{0.0f};
  float y{0.0f};
  float w{0.0f};
  float h{0.0f};
};

struct DrawRestoreClipOp {};

enum class DrawListError : uint8_t {
  Ok = 0,
  Truncated,
  BadMagic,
  UnsupportedVersion,
  OpCountExceedsLimit,
  PolylinePointCountExceedsLimit,
  UnknownOpcode,
  NonFiniteFloat,
  TrailingBytes,
  InvalidBase64,
};

struct DrawListHeader {
  uint16_t version{0};
  uint32_t op_count{0};
};

struct DrawListDecodeResult {
  DrawListError error{DrawListError::Ok};
  uint32_t ops_decoded{0};
  size_t bytes_consumed{0};

  constexpr bool ok() const noexcept { return error == DrawListError::Ok; }
  explicit constexpr operator bool() const noexcept { return ok(); }
};

/// Parses and validates only the fixed 15-byte `NVE_DRAW1` header.
inline DrawListError decode_draw_list_header(
    std::span<const uint8_t> bytes,
    DrawListHeader& out_header) noexcept {
  out_header = DrawListHeader{};
  if (bytes.size() < kDrawListHeaderSize) {
    return DrawListError::Truncated;
  }
  if (std::memcmp(bytes.data(), kDrawListMagic.data(), kDrawListMagic.size()) != 0) {
    return DrawListError::BadMagic;
  }
  const uint16_t version = detail::read_le_u16_unaligned(bytes.data() + 9u);
  if (version != kDrawListVersion) {
    return DrawListError::UnsupportedVersion;
  }
  const uint32_t op_count = detail::read_le_u32_unaligned(bytes.data() + 11u);
  if (static_cast<size_t>(op_count) > kMaxDrawOps) {
    return DrawListError::OpCountExceedsLimit;
  }
  out_header.version = version;
  out_header.op_count = op_count;
  return DrawListError::Ok;
}

/// Zero-allocation streaming visitor decoder for `NVE_DRAW1` binary draw lists.
///
/// Validates header magic, version, strict op/vertex limits, unaligned-safe bounds,
/// finite IEEE-754 floats (rejects NaN/Inf), and exact byte consumption (no trailing bytes).
///
/// `Visitor` must expose:
/// - `on_clear(const DrawClearOp&)`
/// - `on_fill_rect(const DrawFillRectOp&)`
/// - `on_fill_rounded_rect(const DrawFillRoundedRectOp&)`
/// - `on_fill_circle(const DrawFillCircleOp&)`
/// - `on_stroke_line(const DrawStrokeLineOp&)`
/// - `on_stroke_polyline(const DrawStrokePolylineOp&)`
/// - `on_clip_rect(const DrawClipRectOp&)`
/// - `on_restore_clip(const DrawRestoreClipOp&)`
template <typename Visitor>
inline DrawListDecodeResult decode_draw_list(
    std::span<const uint8_t> bytes,
    Visitor&& visitor) {
  DrawListHeader header{};
  const DrawListError header_err = decode_draw_list_header(bytes, header);
  if (header_err != DrawListError::Ok) {
    return DrawListDecodeResult{header_err, 0u, 0u};
  }

  size_t pos = kDrawListHeaderSize;
  const size_t total = bytes.size();
  const uint32_t op_count = header.op_count;
  // Every op consumes at least 1 byte (its opcode tag).
  if (total - pos < static_cast<size_t>(op_count)) {
    return DrawListDecodeResult{DrawListError::Truncated, 0u, pos};
  }

  auto take = [&](size_t n, const uint8_t*& out_ptr) noexcept -> bool {
    if (total - pos < n) {
      return false;
    }
    out_ptr = bytes.data() + pos;
    pos += n;
    return true;
  };

  auto read_f32 = [&](float& out_val) noexcept -> DrawListError {
    const uint8_t* p = nullptr;
    if (!take(4u, p)) {
      return DrawListError::Truncated;
    }
    const float v = detail::read_le_f32_unaligned(p);
    if (!std::isfinite(v)) {
      return DrawListError::NonFiniteFloat;
    }
    out_val = v;
    return DrawListError::Ok;
  };

  auto read_u32 = [&](uint32_t& out_val) noexcept -> bool {
    const uint8_t* p = nullptr;
    if (!take(4u, p)) {
      return false;
    }
    out_val = detail::read_le_u32_unaligned(p);
    return true;
  };

  auto read_color = [&](DrawColorRgba& out_color) noexcept -> bool {
    uint32_t raw = 0u;
    if (!read_u32(raw)) {
      return false;
    }
    out_color = DrawColorRgba::from_u32(raw);
    return true;
  };

  uint32_t decoded_ops = 0u;
  for (uint32_t i = 0u; i < op_count; ++i) {
    const uint8_t* tag_ptr = nullptr;
    if (!take(1u, tag_ptr)) {
      return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
    }
    const uint8_t tag = *tag_ptr;
    switch (static_cast<DrawOpcode>(tag)) {
      case DrawOpcode::Clear: {
        DrawClearOp op{};
        if (!read_color(op.color)) {
          return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
        }
        visitor.on_clear(op);
        break;
      }
      case DrawOpcode::FillRect: {
        DrawFillRectOp op{};
        if (const auto err = read_f32(op.x); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.y); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.w); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.h); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (!read_color(op.color)) {
          return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
        }
        visitor.on_fill_rect(op);
        break;
      }
      case DrawOpcode::FillRoundedRect: {
        DrawFillRoundedRectOp op{};
        if (const auto err = read_f32(op.x); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.y); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.w); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.h); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.radius); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (!read_color(op.color)) {
          return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
        }
        visitor.on_fill_rounded_rect(op);
        break;
      }
      case DrawOpcode::FillCircle: {
        DrawFillCircleOp op{};
        if (const auto err = read_f32(op.cx); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.cy); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.radius); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (!read_color(op.color)) {
          return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
        }
        visitor.on_fill_circle(op);
        break;
      }
      case DrawOpcode::StrokeLine: {
        DrawStrokeLineOp op{};
        if (const auto err = read_f32(op.x0); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.y0); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.x1); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.y1); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.width); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (!read_color(op.color)) {
          return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
        }
        visitor.on_stroke_line(op);
        break;
      }
      case DrawOpcode::StrokePolyline: {
        DrawStrokePolylineOp op{};
        if (const auto err = read_f32(op.width); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (!read_color(op.color)) {
          return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
        }
        uint32_t pt_count = 0u;
        if (!read_u32(pt_count)) {
          return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
        }
        if (static_cast<size_t>(pt_count) > kMaxPolylinePoints) {
          return DrawListDecodeResult{DrawListError::PolylinePointCountExceedsLimit, decoded_ops, pos};
        }
        const size_t needed_bytes = static_cast<size_t>(pt_count) * 8u;
        const uint8_t* pts_ptr = nullptr;
        if (!take(needed_bytes, pts_ptr)) {
          return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
        }
        // Validate every vertex coordinate before invoking the visitor (zero heap allocations).
        for (uint32_t p = 0u; p < pt_count; ++p) {
          const uint8_t* pair = pts_ptr + static_cast<size_t>(p) * 8u;
          const float px = detail::read_le_f32_unaligned(pair);
          const float py = detail::read_le_f32_unaligned(pair + 4u);
          if (!std::isfinite(px) || !std::isfinite(py)) {
            return DrawListDecodeResult{DrawListError::NonFiniteFloat, decoded_ops, pos};
          }
        }
        op.points = PolylinePointView(pts_ptr, pt_count);
        visitor.on_stroke_polyline(op);
        break;
      }
      case DrawOpcode::ClipRect: {
        DrawClipRectOp op{};
        if (const auto err = read_f32(op.x); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.y); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.w); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        if (const auto err = read_f32(op.h); err != DrawListError::Ok) {
          return DrawListDecodeResult{err, decoded_ops, pos};
        }
        visitor.on_clip_rect(op);
        break;
      }
      case DrawOpcode::RestoreClip: {
        visitor.on_restore_clip(DrawRestoreClipOp{});
        break;
      }
      default:
        return DrawListDecodeResult{DrawListError::UnknownOpcode, decoded_ops, pos};
    }
    ++decoded_ops;
  }

  if (pos != total) {
    return DrawListDecodeResult{DrawListError::TrailingBytes, decoded_ops, pos};
  }
  return DrawListDecodeResult{DrawListError::Ok, decoded_ops, pos};
}

struct NoopDrawListVisitor {
  void on_clear(const DrawClearOp&) const noexcept {}
  void on_fill_rect(const DrawFillRectOp&) const noexcept {}
  void on_fill_rounded_rect(const DrawFillRoundedRectOp&) const noexcept {}
  void on_fill_circle(const DrawFillCircleOp&) const noexcept {}
  void on_stroke_line(const DrawStrokeLineOp&) const noexcept {}
  void on_stroke_polyline(const DrawStrokePolylineOp&) const noexcept {}
  void on_clip_rect(const DrawClipRectOp&) const noexcept {}
  void on_restore_clip(const DrawRestoreClipOp&) const noexcept {}
};

/// Zero-allocation full structural validation of an `NVE_DRAW1` payload.
inline DrawListDecodeResult validate_draw_list(std::span<const uint8_t> bytes) noexcept {
  return decode_draw_list(bytes, NoopDrawListVisitor{});
}

/// Decodes a prefixed (`nve-draw1;b64,...`) or bare RFC4648 base64 string into raw `NVE_DRAW1`
/// bytes and validates its 15-byte header.
inline DrawListError decode_draw_list_attr_bytes(
    std::string_view raw,
    std::vector<uint8_t>& out_bytes) {
  out_bytes.clear();
  std::string_view b64 = raw;
  if (b64.size() >= kDrawListAttrPrefix.size() &&
      b64.substr(0u, kDrawListAttrPrefix.size()) == kDrawListAttrPrefix) {
    b64.remove_prefix(kDrawListAttrPrefix.size());
  }
  if (b64.empty() || (b64.size() % 4u) != 0u) {
    return DrawListError::InvalidBase64;
  }

  out_bytes.reserve((b64.size() / 4u) * 3u);
  for (size_t i = 0u; i < b64.size(); i += 4u) {
    const bool is_last = (i + 4u == b64.size());
    const uint8_t c0 = static_cast<uint8_t>(b64[i]);
    const uint8_t c1 = static_cast<uint8_t>(b64[i + 1u]);
    const uint8_t c2 = static_cast<uint8_t>(b64[i + 2u]);
    const uint8_t c3 = static_cast<uint8_t>(b64[i + 3u]);

    const int a = detail::decode_b64_digit(c0);
    const int b = detail::decode_b64_digit(c1);
    if (a < 0 || b < 0) {
      out_bytes.clear();
      return DrawListError::InvalidBase64;
    }
    if (c2 == '=') {
      if (!is_last || c3 != '=') {
        out_bytes.clear();
        return DrawListError::InvalidBase64;
      }
      out_bytes.push_back(static_cast<uint8_t>((a << 2) | (b >> 4)));
    } else {
      const int c = detail::decode_b64_digit(c2);
      if (c < 0) {
        out_bytes.clear();
        return DrawListError::InvalidBase64;
      }
      out_bytes.push_back(static_cast<uint8_t>((a << 2) | (b >> 4)));
      if (c3 == '=') {
        if (!is_last) {
          out_bytes.clear();
          return DrawListError::InvalidBase64;
        }
        out_bytes.push_back(static_cast<uint8_t>(((b & 0x0F) << 4) | (c >> 2)));
      } else {
        const int d = detail::decode_b64_digit(c3);
        if (d < 0) {
          out_bytes.clear();
          return DrawListError::InvalidBase64;
        }
        out_bytes.push_back(static_cast<uint8_t>(((b & 0x0F) << 4) | (c >> 2)));
        out_bytes.push_back(static_cast<uint8_t>(((c & 0x03) << 6) | d));
      }
    }
  }

  DrawListHeader header{};
  const DrawListError hdr_err = decode_draw_list_header(out_bytes, header);
  if (hdr_err != DrawListError::Ok) {
    out_bytes.clear();
    return hdr_err;
  }
  return DrawListError::Ok;
}

/// Encodes raw `NVE_DRAW1` bytes as a prefixed `nve-draw1;b64,...` attribute string.
inline std::string encode_draw_list_attr_base64(std::span<const uint8_t> bytes) {
  static constexpr char kAlphabet[65] =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  std::string out;
  out.reserve(kDrawListAttrPrefix.size() + ((bytes.size() + 2u) / 3u) * 4u);
  out.append(kDrawListAttrPrefix);

  size_t i = 0u;
  while (i + 3u <= bytes.size()) {
    const uint32_t n = (static_cast<uint32_t>(bytes[i]) << 16u) |
                       (static_cast<uint32_t>(bytes[i + 1u]) << 8u) |
                       static_cast<uint32_t>(bytes[i + 2u]);
    out.push_back(kAlphabet[(n >> 18u) & 0x3Fu]);
    out.push_back(kAlphabet[(n >> 12u) & 0x3Fu]);
    out.push_back(kAlphabet[(n >> 6u) & 0x3Fu]);
    out.push_back(kAlphabet[n & 0x3Fu]);
    i += 3u;
  }
  const size_t rem = bytes.size() - i;
  if (rem == 1u) {
    const uint32_t n = static_cast<uint32_t>(bytes[i]) << 16u;
    out.push_back(kAlphabet[(n >> 18u) & 0x3Fu]);
    out.push_back(kAlphabet[(n >> 12u) & 0x3Fu]);
    out.push_back('=');
    out.push_back('=');
  } else if (rem == 2u) {
    const uint32_t n =
        (static_cast<uint32_t>(bytes[i]) << 16u) | (static_cast<uint32_t>(bytes[i + 1u]) << 8u);
    out.push_back(kAlphabet[(n >> 18u) & 0x3Fu]);
    out.push_back(kAlphabet[(n >> 12u) & 0x3Fu]);
    out.push_back(kAlphabet[(n >> 6u) & 0x3Fu]);
    out.push_back('=');
  }
  return out;
}

/// Compact C++20 `NVE_DRAW1` binary encoder mirroring Rust `nve_tables::DrawListBuilder`.
class DrawListEncoder {
 public:
  explicit DrawListEncoder(size_t capacity_bytes = 256u) {
    buf_.reserve(std::max(capacity_bytes, kDrawListHeaderSize));
    buf_.insert(buf_.end(), kDrawListMagic.begin(), kDrawListMagic.end());
    detail::write_le_u16(buf_, kDrawListVersion);
    detail::write_le_u32(buf_, 0u);
  }

  uint32_t op_count() const noexcept { return op_count_; }

  DrawListEncoder& clear(DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::Clear));
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& fill_rect(float x, float y, float w, float h, DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::FillRect));
    detail::write_le_f32(buf_, x);
    detail::write_le_f32(buf_, y);
    detail::write_le_f32(buf_, w);
    detail::write_le_f32(buf_, h);
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& fill_rounded_rect(
      float x,
      float y,
      float w,
      float h,
      float radius,
      DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::FillRoundedRect));
    detail::write_le_f32(buf_, x);
    detail::write_le_f32(buf_, y);
    detail::write_le_f32(buf_, w);
    detail::write_le_f32(buf_, h);
    detail::write_le_f32(buf_, radius);
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& fill_circle(float cx, float cy, float radius, DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::FillCircle));
    detail::write_le_f32(buf_, cx);
    detail::write_le_f32(buf_, cy);
    detail::write_le_f32(buf_, radius);
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& stroke_line(
      float x0,
      float y0,
      float x1,
      float y1,
      float width,
      DrawColorRgba color) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::StrokeLine));
    detail::write_le_f32(buf_, x0);
    detail::write_le_f32(buf_, y0);
    detail::write_le_f32(buf_, x1);
    detail::write_le_f32(buf_, y1);
    detail::write_le_f32(buf_, width);
    detail::write_le_u32(buf_, color.to_u32());
    ++op_count_;
    return *this;
  }

  DrawListEncoder& stroke_polyline(
      std::span<const DrawPoint> points,
      float width,
      DrawColorRgba color) {
    const size_t clamped = std::min(points.size(), kMaxPolylinePoints);
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::StrokePolyline));
    detail::write_le_f32(buf_, width);
    detail::write_le_u32(buf_, color.to_u32());
    detail::write_le_u32(buf_, static_cast<uint32_t>(clamped));
    for (size_t i = 0u; i < clamped; ++i) {
      detail::write_le_f32(buf_, points[i].x);
      detail::write_le_f32(buf_, points[i].y);
    }
    ++op_count_;
    return *this;
  }

  DrawListEncoder& clip_rect(float x, float y, float w, float h) {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::ClipRect));
    detail::write_le_f32(buf_, x);
    detail::write_le_f32(buf_, y);
    detail::write_le_f32(buf_, w);
    detail::write_le_f32(buf_, h);
    ++op_count_;
    return *this;
  }

  DrawListEncoder& restore_clip() {
    buf_.push_back(static_cast<uint8_t>(DrawOpcode::RestoreClip));
    ++op_count_;
    return *this;
  }

  /// Appends a Studio-style telemetry sparkline matching `CanvasOp::sparkline`.
  DrawListEncoder& sparkline(
      std::span<const float> samples,
      float width,
      float height,
      DrawColorRgba stroke_color,
      DrawColorRgba background_color) {
    const float w = (std::isfinite(width) && width > 0.0f) ? width : 16.0f;
    const float h = (std::isfinite(height) && height > 0.0f) ? height : 16.0f;
    const float pad_x = std::clamp(w * 0.04f, 4.0f, 12.0f);
    const float pad_y = std::clamp(h * 0.14f, 4.0f, 12.0f);
    const float plot_w = std::max(w - pad_x * 2.0f, 1.0f);
    const float plot_h = std::max(h - pad_y * 2.0f, 1.0f);

    const DrawColorRgba grid_color = DrawColorRgba::rgba(148u, 163u, 184u, 52u);
    const DrawColorRgba baseline_color = DrawColorRgba::rgba(148u, 163u, 184u, 90u);

    clear(DrawColorRgba::rgba(0u, 0u, 0u, 0u));
    fill_rounded_rect(0.0f, 0.0f, w, h, std::min(8.0f, h * 0.25f), background_color);
    clip_rect(1.0f, 1.0f, std::max(w - 2.0f, 1.0f), std::max(h - 2.0f, 1.0f));

    const float mid_y = pad_y + plot_h * 0.5f;
    stroke_line(pad_x, pad_y, pad_x + plot_w, pad_y, 1.0f, grid_color);
    stroke_line(pad_x, mid_y, pad_x + plot_w, mid_y, 1.0f, grid_color);
    stroke_line(pad_x, pad_y + plot_h, pad_x + plot_w, pad_y + plot_h, 1.0f, baseline_color);

    std::vector<float> clean;
    clean.reserve(std::min(samples.size(), kMaxPolylinePoints));
    for (float s : samples) {
      if (std::isfinite(s)) {
        clean.push_back(s);
        if (clean.size() >= kMaxPolylinePoints) break;
      }
    }

    if (clean.empty()) {
      restore_clip();
      return *this;
    }

    if (clean.size() == 1u) {
      const float cy = pad_y + plot_h * 0.5f;
      stroke_line(pad_x, cy, pad_x + plot_w, cy, 2.0f, stroke_color);
      fill_circle(pad_x + plot_w, cy, 3.5f, stroke_color);
      restore_clip();
      return *this;
    }

    float min_v = std::numeric_limits<float>::infinity();
    float max_v = -std::numeric_limits<float>::infinity();
    for (float v : clean) {
      min_v = std::min(min_v, v);
      max_v = std::max(max_v, v);
    }
    const float span = max_v - min_v;
    const float denom = static_cast<float>(clean.size() - 1u);

    std::vector<DrawPoint> pts;
    pts.reserve(clean.size());
    for (size_t idx = 0u; idx < clean.size(); ++idx) {
      const float t = static_cast<float>(idx) / denom;
      const float norm = (std::fabs(span) <= std::numeric_limits<float>::epsilon())
                             ? 0.5f
                             : std::clamp((clean[idx] - min_v) / span, 0.0f, 1.0f);
      pts.push_back(DrawPoint{pad_x + t * plot_w, pad_y + (1.0f - norm) * plot_h});
    }

    const DrawPoint last = pts.back();
    const float bar_w = std::clamp(plot_w / static_cast<float>(clean.size()), 3.0f, 10.0f);
    const float bar_h = std::max(pad_y + plot_h - last.y, 2.0f);
    fill_rect(
        std::max(last.x - bar_w * 0.5f, pad_x),
        last.y,
        bar_w,
        bar_h,
        DrawColorRgba::rgba(stroke_color.r, stroke_color.g, stroke_color.b, 48u));
    stroke_polyline(pts, 2.0f, stroke_color);
    fill_circle(last.x, last.y, 3.5f, stroke_color);
    restore_clip();
    return *this;
  }

  std::vector<uint8_t> finish() {
    buf_[11u] = static_cast<uint8_t>(op_count_ & 0xFFu);
    buf_[12u] = static_cast<uint8_t>((op_count_ >> 8u) & 0xFFu);
    buf_[13u] = static_cast<uint8_t>((op_count_ >> 16u) & 0xFFu);
    buf_[14u] = static_cast<uint8_t>((op_count_ >> 24u) & 0xFFu);
    return buf_;
  }

  std::string to_attr_string() {
    const std::vector<uint8_t> bytes = finish();
    return encode_draw_list_attr_base64(bytes);
  }

 private:
  std::vector<uint8_t> buf_;
  uint32_t op_count_{0u};
};

} // namespace nve
