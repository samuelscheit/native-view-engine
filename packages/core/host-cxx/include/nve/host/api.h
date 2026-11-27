/* NVE host runtime: portable C11 / C++20 macros and little-endian byte loaders.
 *
 * Every NVE binary ABI is little-endian. The loaders below assemble values byte by byte, so they
 * are alignment-free, strict-aliasing-safe and `constexpr` in C++20 (compilers fold them to one
 * unaligned load on little-endian targets).
 */
#ifndef NVE_HOST_API_H
#define NVE_HOST_API_H

#include <stddef.h>
#include <stdint.h>

#if defined(__cplusplus)
#define NVE_EXTERN_C_BEGIN extern "C" {
#define NVE_EXTERN_C_END }
/* `constexpr` functions are implicitly `inline` and usable in constant expressions. */
#define NVE_INLINE constexpr inline
#ifndef NVE_STATIC_ASSERT
#define NVE_STATIC_ASSERT(cond, msg) static_assert(cond, msg)
#endif
#define NVE_ALIGNOF(type) alignof(type)
#else
#define NVE_EXTERN_C_BEGIN
#define NVE_EXTERN_C_END
#define NVE_INLINE static inline
#ifndef NVE_STATIC_ASSERT
#define NVE_STATIC_ASSERT(cond, msg) _Static_assert(cond, msg)
#endif
#define NVE_ALIGNOF(type) _Alignof(type)
#endif

#define NVE_OFFSETOF(type, member) offsetof(type, member)

/* Symbol visibility for functions exported across the C ABI (`nve_get_interface`, host shims). */
#ifndef NVE_API
#if defined(_WIN32)
#if defined(NVE_BUILDING_LIBRARY)
#define NVE_API __declspec(dllexport)
#else
#define NVE_API __declspec(dllimport)
#endif
#elif defined(__GNUC__) || defined(__clang__)
#define NVE_API __attribute__((visibility("default")))
#else
#define NVE_API
#endif
#endif

/* The binary ABI is little-endian only; refuse to compile anywhere else. */
#if defined(__BYTE_ORDER__) && defined(__ORDER_LITTLE_ENDIAN__)
NVE_STATIC_ASSERT(__BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__, "NVE requires a little-endian target");
#endif

NVE_STATIC_ASSERT(sizeof(float) == 4, "NVE requires IEEE-754 binary32 float");
NVE_STATIC_ASSERT(sizeof(double) == 8, "NVE requires IEEE-754 binary64 double");

NVE_INLINE uint16_t nve_load_u16_le(const uint8_t* p) {
  return (uint16_t)((uint16_t)p[0] | (uint16_t)((uint16_t)p[1] << 8));
}

NVE_INLINE uint32_t nve_load_u32_le(const uint8_t* p) {
  return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

NVE_INLINE uint64_t nve_load_u64_le(const uint8_t* p) {
  return (uint64_t)nve_load_u32_le(p) | ((uint64_t)nve_load_u32_le(p + 4) << 32);
}

NVE_INLINE void nve_store_u16_le(uint8_t* p, uint16_t v) {
  p[0] = (uint8_t)(v & 0xFFu);
  p[1] = (uint8_t)(v >> 8);
}

NVE_INLINE void nve_store_u32_le(uint8_t* p, uint32_t v) {
  p[0] = (uint8_t)(v & 0xFFu);
  p[1] = (uint8_t)((v >> 8) & 0xFFu);
  p[2] = (uint8_t)((v >> 16) & 0xFFu);
  p[3] = (uint8_t)(v >> 24);
}

NVE_INLINE void nve_store_u64_le(uint8_t* p, uint64_t v) {
  nve_store_u32_le(p, (uint32_t)(v & 0xFFFFFFFFu));
  nve_store_u32_le(p + 4, (uint32_t)(v >> 32));
}

/* 1 when `[shift, shift + width)` is a non-empty bit range inside one 32-bit word. */
NVE_INLINE int nve_bits_range_valid(uint32_t shift, uint32_t width) {
  return width >= 1u && width <= 32u && shift < 32u && shift + width <= 32u;
}

/* `bits[shift .. shift + width)` of `word`; total: an invalid range yields 0 (never a UB shift). */
NVE_INLINE uint32_t nve_bits_extract(uint32_t word, uint32_t shift, uint32_t width) {
  if (!nve_bits_range_valid(shift, width)) return 0u;
  return width == 32u ? word : (word >> shift) & ((1u << width) - 1u);
}

/* Sign-extends the low `width` bits of `bits`; total: `width` outside `1..=32` yields 0. */
NVE_INLINE int32_t nve_bits_sign_extend(uint32_t bits, uint32_t width) {
  if (width < 1u || width > 32u) return 0;
  const uint32_t sign = 1u << (width - 1u);
  const uint32_t masked = width >= 32u ? bits : bits & ((1u << width) - 1u);
  /* `masked - 2^width` in 64-bit: no implementation-defined unsigned -> signed narrowing. */
  return (masked & sign) != 0u ? (int32_t)((int64_t)masked - (int64_t)((uint64_t)1u << width))
                               : (int32_t)masked;
}

#endif /* NVE_HOST_API_H */
