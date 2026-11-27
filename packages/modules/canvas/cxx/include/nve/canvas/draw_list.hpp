// `NVE_DRAW1` canvas draw lists: constants, op structs, and the strict zero-allocation streaming
// decoder. Byte-identical to `packages/modules/canvas/src/codec.rs`; a `nve.canvas` row's
// `draw_list` resource (`nve.res.draw_list`) holds these raw bytes.
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

namespace nve::canvas {

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

/// Wire opcodes matching `packages/modules/canvas/src/types.rs` `DrawOpcode` byte-for-byte.
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

}  // namespace nve::canvas
