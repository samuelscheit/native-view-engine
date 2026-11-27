// NVE host runtime: hybrid FFI prelude shared by every generated C++ hybrid client.
#pragma once

#include <cstdint>
#include <optional>
#include <span>
#include <vector>

#include "bin_codec.hpp"
#include "xxh64.hpp"

namespace nve::host {

/// Status of a fallible hybrid FFI trampoline (Rust `NveHybridStatus`, `#[repr(i32)]`).
enum class HybridStatus : int32_t { Ok = 0, Err = 1, InvalidArg = 2, SchemaMismatch = 3, NullHandle = 4 };

/// Status of `NveHostCallbacks::invoke` / `channel_publish` (Rust `NveHostStatus`, `#[repr(i32)]`).
enum class HostStatus : int32_t {
  Ok = 0,
  Pending = 1,
  BufferTooSmall = -1,
  Unsupported = -2,
  PermissionDenied = -3,
  InvalidArg = -4,
  NotFound = -5,
  Cancelled = -6,
  IoError = -7,
  InternalError = -8,
};

/// `Ok` when a peer's schema hash matches the generated one, else `SchemaMismatch`.
[[nodiscard]] constexpr HybridStatus check_schema(Key64 expected, Key64 actual) noexcept {
  return expected == actual ? HybridStatus::Ok : HybridStatus::SchemaMismatch;
}

/// Owned dynamic value in the compact `NveBinValue` encoding; always holds exactly one valid value.
class DynamicValue {
 public:
  DynamicValue() : bytes_{static_cast<uint8_t>(BinTag::Null)} {}

  /// Validated copy of `bytes`, or `std::nullopt` when they are not exactly one valid value.
  [[nodiscard]] static std::optional<DynamicValue> from_bytes(std::span<const uint8_t> bytes) {
    if (BinReader::validate(bytes) != BinError::None) return std::nullopt;
    DynamicValue v;
    v.bytes_.assign(bytes.begin(), bytes.end());
    return v;
  }

  [[nodiscard]] std::span<const uint8_t> bytes() const noexcept { return bytes_; }
  [[nodiscard]] BinTag tag() const noexcept { return static_cast<BinTag>(bytes_.front()); }
  [[nodiscard]] bool is_null() const noexcept { return tag() == BinTag::Null; }
  [[nodiscard]] BinReader reader() const noexcept { return BinReader(bytes_); }

  friend bool operator==(const DynamicValue&, const DynamicValue&) = default;

 private:
  std::vector<uint8_t> bytes_;
};

}  // namespace nve::host
