//! C11 (`nve_hash.generated.h`, `static inline`) and constexpr C++20 (`nve_hash.generated.hpp`)
//! XXH64 mirrors.

use super::{banner, byte_list, HASH_VECTORS, STREAM_SPLITS};

const C_BODY: &str = r#"#ifndef NVE_HASH_GENERATED_H
#define NVE_HASH_GENERATED_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define NVE_XXH64_PRIME1 UINT64_C(0x9E3779B185EBCA87)
#define NVE_XXH64_PRIME2 UINT64_C(0xC2B2AE3D27D4EB4F)
#define NVE_XXH64_PRIME3 UINT64_C(0x165667B19E3779F9)
#define NVE_XXH64_PRIME4 UINT64_C(0x85EBCA77C2B2AE63)
#define NVE_XXH64_PRIME5 UINT64_C(0x27D4EB2F165667C5)

/** Streaming XXH64 state; digests equal `nve_xxh64` for any chunking. */
typedef struct NveXxh64State {
  uint64_t total_len;
  uint64_t seed;
  uint64_t lanes[4];
  uint8_t buf[32];
  size_t buf_len;
} NveXxh64State;

static inline uint64_t nve_xxh64_rotl(uint64_t x, unsigned r) {
  return (x << r) | (x >> (64u - r));
}

static inline uint64_t nve_xxh64_read64(const uint8_t* p) {
  uint64_t v = 0;
  for (int i = 7; i >= 0; --i) v = (v << 8) | p[i];
  return v;
}

static inline uint32_t nve_xxh64_read32(const uint8_t* p) {
  return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static inline uint64_t nve_xxh64_round(uint64_t acc, uint64_t lane) {
  return nve_xxh64_rotl(acc + lane * NVE_XXH64_PRIME2, 31u) * NVE_XXH64_PRIME1;
}

static inline uint64_t nve_xxh64_merge_round(uint64_t acc, uint64_t val) {
  return (acc ^ nve_xxh64_round(0u, val)) * NVE_XXH64_PRIME1 + NVE_XXH64_PRIME4;
}

static inline void nve_xxh64_stripe(uint64_t lanes[4], const uint8_t* p) {
  for (int i = 0; i < 4; ++i) lanes[i] = nve_xxh64_round(lanes[i], nve_xxh64_read64(p + 8 * i));
}

static inline void nve_xxh64_init(NveXxh64State* s, uint64_t seed) {
  s->total_len = 0u;
  s->seed = seed;
  s->lanes[0] = seed + NVE_XXH64_PRIME1 + NVE_XXH64_PRIME2;
  s->lanes[1] = seed + NVE_XXH64_PRIME2;
  s->lanes[2] = seed;
  s->lanes[3] = seed - NVE_XXH64_PRIME1;
  s->buf_len = 0u;
}

static inline void nve_xxh64_update(NveXxh64State* s, const void* data, size_t len) {
  const uint8_t* p = (const uint8_t*)data;
  size_t i = 0u;
  s->total_len += (uint64_t)len;
  if (s->buf_len > 0u) {
    while (s->buf_len < 32u && i < len) s->buf[s->buf_len++] = p[i++];
    if (s->buf_len < 32u) return;
    nve_xxh64_stripe(s->lanes, s->buf);
    s->buf_len = 0u;
  }
  for (; i + 32u <= len; i += 32u) nve_xxh64_stripe(s->lanes, p + i);
  while (i < len) s->buf[s->buf_len++] = p[i++];
}

static inline uint64_t nve_xxh64_digest(const NveXxh64State* s) {
  uint64_t h;
  if (s->total_len >= 32u) {
    h = nve_xxh64_rotl(s->lanes[0], 1u) + nve_xxh64_rotl(s->lanes[1], 7u) +
        nve_xxh64_rotl(s->lanes[2], 12u) + nve_xxh64_rotl(s->lanes[3], 18u);
    for (int i = 0; i < 4; ++i) h = nve_xxh64_merge_round(h, s->lanes[i]);
  } else {
    h = s->seed + NVE_XXH64_PRIME5;
  }
  h += s->total_len;
  size_t i = 0u;
  for (; i + 8u <= s->buf_len; i += 8u) {
    h ^= nve_xxh64_round(0u, nve_xxh64_read64(s->buf + i));
    h = nve_xxh64_rotl(h, 27u) * NVE_XXH64_PRIME1 + NVE_XXH64_PRIME4;
  }
  if (i + 4u <= s->buf_len) {
    h ^= (uint64_t)nve_xxh64_read32(s->buf + i) * NVE_XXH64_PRIME1;
    h = nve_xxh64_rotl(h, 23u) * NVE_XXH64_PRIME2 + NVE_XXH64_PRIME3;
    i += 4u;
  }
  for (; i < s->buf_len; ++i) {
    h ^= (uint64_t)s->buf[i] * NVE_XXH64_PRIME5;
    h = nve_xxh64_rotl(h, 11u) * NVE_XXH64_PRIME1;
  }
  h ^= h >> 33;
  h *= NVE_XXH64_PRIME2;
  h ^= h >> 29;
  h *= NVE_XXH64_PRIME3;
  return h ^ (h >> 32);
}

/** One-shot canonical XXH64 of `data[0..len]` with `seed`. */
static inline uint64_t nve_xxh64(const void* data, size_t len, uint64_t seed) {
  NveXxh64State s;
  nve_xxh64_init(&s, seed);
  nve_xxh64_update(&s, data, len);
  return nve_xxh64_digest(&s);
}

/** Low / high `u32` halves of a 64-bit key (53-bit-safe JS / Dart-web transport). */
static inline uint32_t nve_key64_lo(uint64_t key) { return (uint32_t)key; }
static inline uint32_t nve_key64_hi(uint64_t key) { return (uint32_t)(key >> 32); }
static inline uint64_t nve_key64_from_halves(uint32_t lo, uint32_t hi) {
  return ((uint64_t)hi << 32) | lo;
}
"#;

/// Generates `cxx/nve-fabric-bridge/include/nve_hash.generated.h` (C11).
pub(super) fn generate_c_hash() -> String {
    let mut s = banner();
    s.push_str(C_BODY);
    s.push_str(
        "\n/** Returns 1 when one-shot and chunked digests of every golden vector match Rust. */\n\
         static inline int nve_hash_verify_test_vectors(void) {\n",
    );
    for (i, v) in HASH_VECTORS.iter().enumerate() {
        if !v.bytes.is_empty() {
            let bytes = byte_list(v.bytes, |b| format!("0x{b:02X}u"));
            writeln!(s, "  static const uint8_t v{i}[] = {{{bytes}}};");
        }
    }
    s.push_str(
        "  static const struct { const uint8_t* bytes; size_t len; uint64_t seed; uint64_t expected; } vs[] = {\n",
    );
    for (i, v) in HASH_VECTORS.iter().enumerate() {
        let ptr = if v.bytes.is_empty() { "NULL".to_owned() } else { format!("v{i}") };
        writeln!(
            s,
            "    {{{ptr}, {}u, UINT64_C(0x{:016X}), UINT64_C(0x{:016X})}}, /* {} */",
            v.bytes.len(),
            v.seed,
            v.expected(),
            v.label
        );
    }
    let [a, b, c] = STREAM_SPLITS;
    write!(
        s,
        r#"  }};
  static const size_t splits[] = {{{a}u, {b}u, {c}u}};
  for (size_t i = 0; i < sizeof vs / sizeof vs[0]; ++i) {{
    if (nve_xxh64(vs[i].bytes, vs[i].len, vs[i].seed) != vs[i].expected) return 0;
    NveXxh64State st;
    nve_xxh64_init(&st, vs[i].seed);
    size_t at = 0u;
    for (size_t k = 0; k < 3u; ++k) {{
      size_t end = splits[k] < vs[i].len ? splits[k] : vs[i].len;
      if (end > at) {{
        nve_xxh64_update(&st, vs[i].bytes + at, end - at);
        at = end;
      }}
    }}
    if (vs[i].len > at) nve_xxh64_update(&st, vs[i].bytes + at, vs[i].len - at);
    if (nve_xxh64_digest(&st) != vs[i].expected) return 0;
  }}
  return 1;
}}

#ifdef __cplusplus
}}
#endif

#endif /* NVE_HASH_GENERATED_H */
"#
    );
    s.into_string()
}

const CPP_BODY: &str = r#"#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>

namespace nve {

inline constexpr uint64_t kXxh64Prime1 = 0x9E3779B185EBCA87ULL;
inline constexpr uint64_t kXxh64Prime2 = 0xC2B2AE3D27D4EB4FULL;
inline constexpr uint64_t kXxh64Prime3 = 0x165667B19E3779F9ULL;
inline constexpr uint64_t kXxh64Prime4 = 0x85EBCA77C2B2AE63ULL;
inline constexpr uint64_t kXxh64Prime5 = 0x27D4EB2F165667C5ULL;

namespace hash_detail {

constexpr uint64_t rotl(uint64_t x, unsigned r) noexcept { return (x << r) | (x >> (64u - r)); }

template <class Byte>
constexpr uint64_t read64(const Byte* p) noexcept {
  uint64_t v = 0;
  for (int i = 7; i >= 0; --i) v = (v << 8) | static_cast<uint8_t>(p[i]);
  return v;
}

template <class Byte>
constexpr uint64_t read32(const Byte* p) noexcept {
  uint64_t v = 0;
  for (int i = 3; i >= 0; --i) v = (v << 8) | static_cast<uint8_t>(p[i]);
  return v;
}

constexpr uint64_t round(uint64_t acc, uint64_t lane) noexcept {
  return rotl(acc + lane * kXxh64Prime2, 31u) * kXxh64Prime1;
}

constexpr uint64_t merge_round(uint64_t acc, uint64_t val) noexcept {
  return (acc ^ round(0u, val)) * kXxh64Prime1 + kXxh64Prime4;
}

}  // namespace hash_detail

/// Streaming canonical XXH64 (`constexpr`); digests equal `xxh64` for any chunking.
class Xxh64Hasher {
 private:
  template <class Byte>
  constexpr void stripe(const Byte* p) noexcept {
    for (size_t k = 0; k < 4u; ++k) lanes_[k] = hash_detail::round(lanes_[k], hash_detail::read64(p + 8u * k));
  }


  template <class Byte>
  constexpr void feed(const Byte* p, size_t len) noexcept {
    total_len_ += len;
    size_t i = 0u;
    if (buf_len_ > 0u) {
      while (buf_len_ < 32u && i < len) buf_[buf_len_++] = static_cast<uint8_t>(p[i++]);
      if (buf_len_ < 32u) return;
      stripe(buf_.data());
      buf_len_ = 0u;
    }
    for (; i + 32u <= len; i += 32u) stripe(p + i);
    while (i < len) buf_[buf_len_++] = static_cast<uint8_t>(p[i++]);
  }


 public:
  constexpr explicit Xxh64Hasher(uint64_t seed = 0u) noexcept
      : seed_(seed),
        lanes_{seed + kXxh64Prime1 + kXxh64Prime2, seed + kXxh64Prime2, seed, seed - kXxh64Prime1} {}

  constexpr void update(std::span<const uint8_t> bytes) noexcept { feed(bytes.data(), bytes.size()); }
  constexpr void update(const uint8_t* data, size_t len) noexcept { feed(data, len); }
  constexpr void update(std::string_view text) noexcept { feed(text.data(), text.size()); }

  [[nodiscard]] constexpr uint64_t digest() const noexcept {
    using namespace hash_detail;
    uint64_t h = total_len_ >= 32u
                     ? rotl(lanes_[0], 1u) + rotl(lanes_[1], 7u) + rotl(lanes_[2], 12u) +
                           rotl(lanes_[3], 18u)
                     : seed_ + kXxh64Prime5;
    if (total_len_ >= 32u) {
      for (uint64_t lane : lanes_) h = merge_round(h, lane);
    }
    h += total_len_;
    size_t i = 0u;
    for (; i + 8u <= buf_len_; i += 8u) {
      h ^= round(0u, read64(buf_.data() + i));
      h = rotl(h, 27u) * kXxh64Prime1 + kXxh64Prime4;
    }
    if (i + 4u <= buf_len_) {
      h ^= read32(buf_.data() + i) * kXxh64Prime1;
      h = rotl(h, 23u) * kXxh64Prime2 + kXxh64Prime3;
      i += 4u;
    }
    for (; i < buf_len_; ++i) {
      h ^= static_cast<uint64_t>(buf_[i]) * kXxh64Prime5;
      h = rotl(h, 11u) * kXxh64Prime1;
    }
    h ^= h >> 33;
    h *= kXxh64Prime2;
    h ^= h >> 29;
    h *= kXxh64Prime3;
    return h ^ (h >> 32);
  }

 private:
  uint64_t total_len_{0u};
  uint64_t seed_;
  std::array<uint64_t, 4> lanes_;
  std::array<uint8_t, 32> buf_{};
  size_t buf_len_{0u};
};

/// One-shot canonical XXH64 (bit-identical to Rust `nve_hash::xxh64`).
constexpr uint64_t xxh64(std::span<const uint8_t> bytes, uint64_t seed = 0u) noexcept {
  Xxh64Hasher h(seed);
  h.update(bytes);
  return h.digest();
}

constexpr uint64_t xxh64(const uint8_t* data, size_t len, uint64_t seed = 0u) noexcept {
  Xxh64Hasher h(seed);
  h.update(data, len);
  return h.digest();
}

constexpr uint64_t xxh64(std::string_view text, uint64_t seed = 0u) noexcept {
  Xxh64Hasher h(seed);
  h.update(text);
  return h.digest();
}

/// Compile-time identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id`).
constexpr uint64_t const_id(std::string_view name) noexcept { return xxh64(name, 0u); }

/// 64-bit identity key crossing 53-bit hosts as `(lo, hi)` `u32` halves (Rust `nve_hash::Key64`).
struct Key64 {
  uint64_t value{0u};

  static constexpr Key64 of(std::string_view name) noexcept { return Key64{const_id(name)}; }
  static constexpr Key64 from_halves(uint32_t lo, uint32_t hi) noexcept {
    return Key64{(static_cast<uint64_t>(hi) << 32) | lo};
  }
  [[nodiscard]] constexpr uint32_t lo() const noexcept { return static_cast<uint32_t>(value); }
  [[nodiscard]] constexpr uint32_t hi() const noexcept { return static_cast<uint32_t>(value >> 32); }
  friend constexpr bool operator==(Key64, Key64) = default;
};
"#;

/// Generates `cxx/nve-fabric-bridge/include/nve_hash.generated.hpp` (constexpr C++20).
pub(super) fn generate_cpp_hash() -> String {
    let mut s = banner();
    s.push_str(CPP_BODY);
    s.push_str("\n// Golden vectors, verified at compile time (one-shot and chunked streaming).\nnamespace hash_vectors {\n\n");
    let [a, b, c] = STREAM_SPLITS;
    writeln!(
        s,
        r#"template <size_t N>
constexpr uint64_t streamed(const std::array<uint8_t, N>& bytes, uint64_t seed) noexcept {{
  Xxh64Hasher h(seed);
  size_t at = 0u;
  for (size_t split : {{size_t{{{a}u}}, size_t{{{b}u}}, size_t{{{c}u}}}}) {{
    const size_t end = split < N ? split : N;
    if (end > at) {{
      h.update(std::span<const uint8_t>(bytes).subspan(at, end - at));
      at = end;
    }}
  }}
  h.update(std::span<const uint8_t>(bytes).subspan(at));
  return h.digest();
}}
"#
    );
    for v in HASH_VECTORS {
        let bytes = byte_list(v.bytes, |b| format!("0x{b:02X}u"));
        let (label, n, seed, exp) = (v.label, v.bytes.len(), v.seed, v.expected());
        writeln!(s, "inline constexpr std::array<uint8_t, {n}> k_{label} = {{{bytes}}};");
        writeln!(
            s,
            "static_assert(xxh64(std::span<const uint8_t>(k_{label}), 0x{seed:016X}ULL) == 0x{exp:016X}ULL, \"XXH64 vector `{label}`\");"
        );
        writeln!(
            s,
            "static_assert(streamed(k_{label}, 0x{seed:016X}ULL) == 0x{exp:016X}ULL, \"XXH64 streamed vector `{label}`\");\n"
        );
    }
    s.push_str("}  // namespace hash_vectors\n\n}  // namespace nve\n");
    s.into_string()
}
