// Canonical 4-byte-aligned `FlatResource` (`nve.res.draw_list`) canvas draw-list codec facade
// over the generated zero-allocation decoder (`nve/generated/nve_draw_list.generated.hpp`).
#pragma once

#include <cstddef>
#include <cstdint>
#include <span>

#include <nve/generated/nve_draw_list.generated.hpp>

namespace nve::canvas {

/// Fixed byte length of the flat draw-list header (1 little-endian `u32` word: `op_count`).
inline constexpr size_t kDrawListHeaderSize = generated::draw_list::kHeaderSize;

/// Strict upper bound on operations per draw list to guard against corrupt/hostile payloads.
inline constexpr size_t kMaxDrawOps = generated::draw_list::kMaxDrawOps;

/// Strict upper bound on vertices inside a single `StrokePolyline` operation.
inline constexpr size_t kMaxPolylinePoints = generated::draw_list::kMaxPolylinePoints;

/// Maximum live nested `ClipRect` depth passed to sinks before overflow balancing absorbs deeper clips.
inline constexpr uint32_t kMaxClipDepth = generated::draw_list::kMaxClipDepth;

using DrawOpcode = generated::draw_list::Opcode;
using DrawColorRgba = generated::draw_list::ColorRgba;
using DrawPoint = generated::draw_list::Point2;
using PolylinePointView = generated::draw_list::Point2View;

using DrawClearOp = generated::draw_list::ClearOp;
using DrawFillRectOp = generated::draw_list::FillRectOp;
using DrawFillRoundedRectOp = generated::draw_list::FillRoundedRectOp;
using DrawFillCircleOp = generated::draw_list::FillCircleOp;
using DrawStrokeLineOp = generated::draw_list::StrokeLineOp;
using DrawStrokePolylineOp = generated::draw_list::StrokePolylineOp;
using DrawClipRectOp = generated::draw_list::ClipRectOp;
using DrawRestoreClipOp = generated::draw_list::RestoreClipOp;
using DrawListHeader = generated::draw_list::Header;

enum class DrawListError : uint8_t {
  Ok = 0,
  Truncated,
  Unaligned,
  OpCountExceedsLimit,
  PolylinePointCountExceedsLimit,
  UnknownOpcode,
  NonFiniteFloat,
  NegativeFloat,
  TrailingBytes,
};

struct DrawListDecodeResult {
  DrawListError error{DrawListError::Ok};
  uint32_t ops_decoded{0};
  size_t bytes_consumed{0};

  constexpr bool ok() const noexcept { return error == DrawListError::Ok; }
  explicit constexpr operator bool() const noexcept { return ok(); }
};

namespace detail {

constexpr DrawListError map_error(generated::draw_list::Error err) noexcept {
  using E = generated::draw_list::Error;
  switch (err) {
    case E::Ok: return DrawListError::Ok;
    case E::Truncated: return DrawListError::Truncated;
    case E::Unaligned: return DrawListError::Unaligned;
    case E::OpCountExceedsLimit: return DrawListError::OpCountExceedsLimit;
    case E::SliceLenExceedsLimit: return DrawListError::PolylinePointCountExceedsLimit;
    case E::UnknownOpcode: return DrawListError::UnknownOpcode;
    case E::NonFiniteFloat: return DrawListError::NonFiniteFloat;
    case E::NegativeFloat: return DrawListError::NegativeFloat;
    case E::TrailingBytes: return DrawListError::TrailingBytes;
  }
  return DrawListError::UnknownOpcode;
}

constexpr DrawListDecodeResult map_result(generated::draw_list::DecodeResult res) noexcept {
  return DrawListDecodeResult{map_error(res.error), res.ops_decoded, res.bytes_consumed};
}

}  // namespace detail

/// Parses and validates the leading `u32` `op_count` header word and 4-byte stream alignment.
inline DrawListError decode_draw_list_header(
    std::span<const uint8_t> bytes,
    DrawListHeader& out_header) noexcept {
  return detail::map_error(generated::draw_list::decode_header(bytes, out_header));
}

/// Zero-allocation full structural and float validation of a flat `nve.res.draw_list` payload.
inline DrawListDecodeResult validate_draw_list(std::span<const uint8_t> bytes) noexcept {
  return detail::map_result(generated::draw_list::validate(bytes));
}

/// Streams a previously validated `nve.res.draw_list` buffer into `visitor` with `kMaxClipDepth`
/// overflow balancing and RAII clip unwind.
template <typename Visitor>
inline uint32_t stream_validated_draw_list(std::span<const uint8_t> bytes, Visitor&& visitor) {
  return generated::draw_list::stream_validated(bytes, visitor);
}

/// Validates `bytes` whole, then streams every op into `visitor` with balanced clips (`kMaxClipDepth`
/// cap + overflow tracking + RAII unwind). Malformed streams never invoke any visitor callback.
template <typename Visitor>
inline DrawListDecodeResult decode_draw_list(std::span<const uint8_t> bytes, Visitor&& visitor) {
  return detail::map_result(generated::draw_list::decode(bytes, visitor));
}

}  // namespace nve::canvas
