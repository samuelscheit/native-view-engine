// Canonical 4-byte-aligned `FlatResource` (`nve.res.draw_list`) canvas draw lists: constants,
// op structs, and the strict zero-allocation streaming decoder. Byte-identical to
// `packages/modules/canvas/src/flat.rs` (`[op_count: u32, (opcode: u32, args...)*]`).
#pragma once

#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <span>

namespace nve::canvas {

/// Fixed byte length of the flat draw-list header (1 little-endian `u32` word: `op_count`).
inline constexpr size_t kDrawListHeaderSize = 4u;

/// Strict upper bound on operations per draw list to guard against corrupt/hostile payloads.
inline constexpr size_t kMaxDrawOps = 65536u;

/// Strict upper bound on vertices inside a single `StrokePolyline` operation.
inline constexpr size_t kMaxPolylinePoints = 16384u;

/// Wire opcodes matching `packages/modules/canvas/src/flat.rs` (`OP_*` `u32` words) byte-for-byte.
enum class DrawOpcode : uint32_t {
  Clear = 1u,
  FillRect = 2u,
  FillRoundedRect = 3u,
  FillCircle = 4u,
  StrokeLine = 5u,
  StrokePolyline = 6u,
  ClipRect = 7u,
  RestoreClip = 8u,
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

}  // namespace detail

/// Zero-allocation random-access view over validated little-endian `(f32, f32)` vertices inside
/// a flat `StrokePolyline` payload. Safe on unaligned byte pointers.
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
  Unaligned,
  OpCountExceedsLimit,
  PolylinePointCountExceedsLimit,
  UnknownOpcode,
  NonFiniteFloat,
  TrailingBytes,
};

struct DrawListHeader {
  uint32_t op_count{0};
};

struct DrawListDecodeResult {
  DrawListError error{DrawListError::Ok};
  uint32_t ops_decoded{0};
  size_t bytes_consumed{0};

  constexpr bool ok() const noexcept { return error == DrawListError::Ok; }
  explicit constexpr operator bool() const noexcept { return ok(); }
};

/// Parses and validates the leading `u32` `op_count` header word and 4-byte stream alignment.
inline DrawListError decode_draw_list_header(
    std::span<const uint8_t> bytes,
    DrawListHeader& out_header) noexcept {
  out_header = DrawListHeader{};
  if (bytes.size() < kDrawListHeaderSize) {
    return DrawListError::Truncated;
  }
  if ((bytes.size() & 3u) != 0u) {
    return DrawListError::Unaligned;
  }
  const uint32_t op_count = detail::read_le_u32_unaligned(bytes.data());
  if (static_cast<size_t>(op_count) > kMaxDrawOps) {
    return DrawListError::OpCountExceedsLimit;
  }
  out_header.op_count = op_count;
  return DrawListError::Ok;
}

/// Zero-allocation streaming visitor decoder for canonical flat `u32`-word `nve.res.draw_list`
/// payloads (`packages/modules/canvas/src/flat.rs`).
///
/// Validates 4-byte length alignment, strict op/vertex limits, unaligned-safe bounds,
/// finite IEEE-754 floats (rejects NaN/Inf), and exact word consumption (no trailing words).
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
  // Every op consumes at least 1 u32 word (its opcode word).
  if ((total - pos) / 4u < static_cast<size_t>(op_count)) {
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

  auto read_u32 = [&](uint32_t& out_val) noexcept -> bool {
    const uint8_t* p = nullptr;
    if (!take(4u, p)) {
      return false;
    }
    out_val = detail::read_le_u32_unaligned(p);
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
    uint32_t raw_opcode = 0u;
    if (!read_u32(raw_opcode)) {
      return DrawListDecodeResult{DrawListError::Truncated, decoded_ops, pos};
    }
    switch (static_cast<DrawOpcode>(raw_opcode)) {
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

/// Zero-allocation full structural validation of a flat `nve.res.draw_list` payload.
inline DrawListDecodeResult validate_draw_list(std::span<const uint8_t> bytes) noexcept {
  return decode_draw_list(bytes, NoopDrawListVisitor{});
}

}  // namespace nve::canvas
