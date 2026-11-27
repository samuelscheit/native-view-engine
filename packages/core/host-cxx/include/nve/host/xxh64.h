/* NVE host runtime: canonical XXH64, bit-identical to the Rust `nve-hash` crate.
 *
 * One source for C11 (`static inline`) and C++20 (`constexpr`, so C++ bakes table keys and schema
 * hashes at compile time; see <nve/host/xxh64.hpp> for the typed C++ API).
 */
#ifndef NVE_HOST_XXH64_H
#define NVE_HOST_XXH64_H

#include "api.h"
#include "utf8.h"

#define NVE_XXH64_PRIME1 UINT64_C(0x9E3779B185EBCA87)
#define NVE_XXH64_PRIME2 UINT64_C(0xC2B2AE3D27D4EB4F)
#define NVE_XXH64_PRIME3 UINT64_C(0x165667B19E3779F9)
#define NVE_XXH64_PRIME4 UINT64_C(0x85EBCA77C2B2AE63)
#define NVE_XXH64_PRIME5 UINT64_C(0x27D4EB2F165667C5)

/* Streaming XXH64 state; digests equal `nve_xxh64` for any chunking. */
typedef struct NveXxh64State {
  uint64_t total_len;
  uint64_t seed;
  uint64_t lanes[4];
  uint8_t buf[32];
  size_t buf_len;
} NveXxh64State;

/* 64-bit identity key (table keys, schema hashes, interface ids; Rust `nve_hash::Key64`). */
typedef struct NveKey64 {
  uint64_t value;
} NveKey64;

NVE_INLINE uint64_t nve_xxh64_rotl(uint64_t x, unsigned r) { return (x << r) | (x >> (64u - r)); }

NVE_INLINE uint64_t nve_xxh64_round(uint64_t acc, uint64_t lane) {
  return nve_xxh64_rotl(acc + lane * NVE_XXH64_PRIME2, 31u) * NVE_XXH64_PRIME1;
}

NVE_INLINE uint64_t nve_xxh64_merge_round(uint64_t acc, uint64_t val) {
  return (acc ^ nve_xxh64_round(0u, val)) * NVE_XXH64_PRIME1 + NVE_XXH64_PRIME4;
}

NVE_INLINE void nve_xxh64_stripe(uint64_t lanes[4], const uint8_t* p) {
  lanes[0] = nve_xxh64_round(lanes[0], nve_load_u64_le(p));
  lanes[1] = nve_xxh64_round(lanes[1], nve_load_u64_le(p + 8));
  lanes[2] = nve_xxh64_round(lanes[2], nve_load_u64_le(p + 16));
  lanes[3] = nve_xxh64_round(lanes[3], nve_load_u64_le(p + 24));
}

NVE_INLINE NveXxh64State nve_xxh64_init(uint64_t seed) {
  NveXxh64State s = {
      0u,
      seed,
      {seed + NVE_XXH64_PRIME1 + NVE_XXH64_PRIME2, seed + NVE_XXH64_PRIME2, seed, seed - NVE_XXH64_PRIME1},
      {0u},
      0u};
  return s;
}

/* Feeds `data[0..len)`; `data` may be NULL when `len == 0`. */
NVE_INLINE void nve_xxh64_update(NveXxh64State* s, const uint8_t* data, size_t len) {
  size_t i = 0u;
  s->total_len += (uint64_t)len;
  if (s->buf_len > 0u) {
    while (s->buf_len < 32u && i < len) s->buf[s->buf_len++] = data[i++];
    if (s->buf_len < 32u) return;
    nve_xxh64_stripe(s->lanes, s->buf);
    s->buf_len = 0u;
  }
  for (; i + 32u <= len; i += 32u) nve_xxh64_stripe(s->lanes, data + i);
  while (i < len) s->buf[s->buf_len++] = data[i++];
}

/* Digest of everything fed so far (the state stays usable). */
NVE_INLINE uint64_t nve_xxh64_digest(const NveXxh64State* s) {
  uint64_t h = s->seed + NVE_XXH64_PRIME5;
  size_t i = 0u;
  if (s->total_len >= 32u) {
    h = nve_xxh64_rotl(s->lanes[0], 1u) + nve_xxh64_rotl(s->lanes[1], 7u) +
        nve_xxh64_rotl(s->lanes[2], 12u) + nve_xxh64_rotl(s->lanes[3], 18u);
    h = nve_xxh64_merge_round(h, s->lanes[0]);
    h = nve_xxh64_merge_round(h, s->lanes[1]);
    h = nve_xxh64_merge_round(h, s->lanes[2]);
    h = nve_xxh64_merge_round(h, s->lanes[3]);
  }
  h += s->total_len;
  for (; i + 8u <= s->buf_len; i += 8u) {
    h ^= nve_xxh64_round(0u, nve_load_u64_le(s->buf + i));
    h = nve_xxh64_rotl(h, 27u) * NVE_XXH64_PRIME1 + NVE_XXH64_PRIME4;
  }
  if (i + 4u <= s->buf_len) {
    h ^= (uint64_t)nve_load_u32_le(s->buf + i) * NVE_XXH64_PRIME1;
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

/* One-shot canonical XXH64 of `data[0..len)` with `seed`. */
NVE_INLINE uint64_t nve_xxh64(const uint8_t* data, size_t len, uint64_t seed) {
  NveXxh64State s = nve_xxh64_init(seed);
  nve_xxh64_update(&s, data, len);
  return nve_xxh64_digest(&s);
}

NVE_INLINE uint32_t nve_key64_lo(NveKey64 key) { return (uint32_t)(key.value & 0xFFFFFFFFu); }
NVE_INLINE uint32_t nve_key64_hi(NveKey64 key) { return (uint32_t)(key.value >> 32); }

NVE_INLINE NveKey64 nve_key64_from_halves(uint32_t lo, uint32_t hi) {
  NveKey64 key = {((uint64_t)hi << 32) | (uint64_t)lo};
  return key;
}

NVE_INLINE int nve_key64_eq(NveKey64 a, NveKey64 b) { return a.value == b.value; }

/* Identity of a UTF-8 name: `xxh64(name, 0)` (Rust `nve_hash::const_id` / `Key64::of`).
 * Returns 0 and leaves `*out` untouched when `name[0..len)` is not well-formed UTF-8 (a Rust `&str`
 * can never hold such a name), else 1. */
#if defined(__cplusplus)
inline int nve_key64_of(const char* name, size_t len, NveKey64* out) {
  const uint8_t* bytes = reinterpret_cast<const uint8_t*>(name);
#else
static inline int nve_key64_of(const char* name, size_t len, NveKey64* out) {
  const uint8_t* bytes = (const uint8_t*)name;
#endif
  if (nve_utf8_valid(bytes, len) == 0) return 0;
  out->value = nve_xxh64(bytes, len, 0u);
  return 1;
}

#endif /* NVE_HOST_XXH64_H */
