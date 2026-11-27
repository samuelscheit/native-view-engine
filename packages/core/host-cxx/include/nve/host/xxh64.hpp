// NVE host runtime: typed constexpr C++20 XXH64 and `Key64` over <nve/host/xxh64.h>.
#pragma once

#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>
#include <type_traits>

#include "xxh64.h"

namespace nve::host {

/// Streaming canonical XXH64 (`constexpr`); digests equal `xxh64()` for any chunking.
class Xxh64Hasher {
 public:
  constexpr explicit Xxh64Hasher(uint64_t seed = 0u) noexcept : state_(nve_xxh64_init(seed)) {}

  constexpr Xxh64Hasher& update(std::span<const uint8_t> bytes) noexcept {
    nve_xxh64_update(&state_, bytes.data(), bytes.size());
    return *this;
  }

  /// Feeds the UTF-8 code units of `text` (also during constant evaluation).
  constexpr Xxh64Hasher& update(std::string_view text) noexcept {
    if (!std::is_constant_evaluated()) {
      nve_xxh64_update(&state_, reinterpret_cast<const uint8_t*>(text.data()), text.size());
      return *this;
    }
    uint8_t chunk[32] = {};
    for (size_t at = 0; at < text.size();) {
      size_t n = 0;
      for (; n < sizeof chunk && at + n < text.size(); ++n) chunk[n] = static_cast<uint8_t>(text[at + n]);
      nve_xxh64_update(&state_, chunk, n);
      at += n;
    }
    return *this;
  }

  constexpr Xxh64Hasher& update_u32(uint32_t v) noexcept {
    uint8_t b[4] = {};
    nve_store_u32_le(b, v);
    nve_xxh64_update(&state_, b, sizeof b);
    return *this;
  }

  constexpr Xxh64Hasher& update_u64(uint64_t v) noexcept {
    uint8_t b[8] = {};
    nve_store_u64_le(b, v);
    nve_xxh64_update(&state_, b, sizeof b);
    return *this;
  }

  [[nodiscard]] constexpr uint64_t digest() const noexcept { return nve_xxh64_digest(&state_); }

 private:
  NveXxh64State state_;
};

/// One-shot canonical XXH64 (bit-identical to Rust `nve_hash::xxh64`).
[[nodiscard]] constexpr uint64_t xxh64(std::span<const uint8_t> bytes, uint64_t seed = 0u) noexcept {
  return Xxh64Hasher(seed).update(bytes).digest();
}

[[nodiscard]] constexpr uint64_t xxh64(std::string_view text, uint64_t seed = 0u) noexcept {
  return Xxh64Hasher(seed).update(text).digest();
}

namespace detail {
/// Deliberately not `constexpr`: reaching it during constant evaluation is a compile error.
void name_is_not_valid_utf8();

consteval bool utf8_valid(std::string_view text) {
  uint8_t chunk[4] = {};
  for (size_t i = 0; i < text.size();) {
    const auto b0 = static_cast<uint8_t>(text[i]);
    const size_t n = b0 < 0x80u ? 1u : b0 < 0xE0u ? 2u : b0 < 0xF0u ? 3u : 4u;
    if (n > text.size() - i) return false;
    for (size_t k = 0; k < n; ++k) chunk[k] = static_cast<uint8_t>(text[i + k]);
    if (nve_utf8_valid(chunk, n) == 0) return false;
    i += n;
  }
  return true;
}
}  // namespace detail

/// Compile-time identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id`). A name that
/// is not well-formed UTF-8 (impossible for a Rust `&str`) fails to compile. Runtime names use the
/// validating `nve_key64_of`.
[[nodiscard]] consteval uint64_t const_id(std::string_view name) {
  if (!detail::utf8_valid(name)) detail::name_is_not_valid_utf8();
  return xxh64(name, 0u);
}

/// 64-bit identity key; crosses 53-bit hosts (JS / Dart-web) as `(lo, hi)` `u32` halves.
struct Key64 {
  uint64_t value = 0u;

  [[nodiscard]] static consteval Key64 of(std::string_view name) { return Key64{const_id(name)}; }
  [[nodiscard]] static constexpr Key64 from_halves(uint32_t lo, uint32_t hi) noexcept {
    return Key64{(static_cast<uint64_t>(hi) << 32) | lo};
  }
  [[nodiscard]] static constexpr Key64 from_c(NveKey64 key) noexcept { return Key64{key.value}; }

  [[nodiscard]] constexpr uint32_t lo() const noexcept { return static_cast<uint32_t>(value); }
  [[nodiscard]] constexpr uint32_t hi() const noexcept { return static_cast<uint32_t>(value >> 32); }
  [[nodiscard]] constexpr NveKey64 to_c() const noexcept { return NveKey64{value}; }

  friend constexpr bool operator==(Key64, Key64) noexcept = default;
};

}  // namespace nve::host
