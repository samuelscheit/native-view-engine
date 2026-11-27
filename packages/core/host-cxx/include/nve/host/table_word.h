/* NVE host runtime: word-table field decoding (C11 + constexpr C++20).
 *
 * A table row is `words` little-endian `u32`s. Generated per-table data supplies one
 * `NveFieldSpec {word, shift, bits, kind}` per field; these helpers turn a row word into a typed
 * value exactly like the Rust `nve_store::WordValue` impls.
 */
#ifndef NVE_HOST_TABLE_WORD_H
#define NVE_HOST_TABLE_WORD_H

#include "api.h"

/* Mirrors Rust `nve_abi::table::FieldKind` (`#[repr(u8)]`). */
typedef enum NveFieldKind {
  NVE_FIELD_U32 = 0,
  NVE_FIELD_I32 = 1,
  NVE_FIELD_F32 = 2,
  NVE_FIELD_BOOL = 3,
  NVE_FIELD_ENUM = 4,
  NVE_FIELD_COLOR = 5,
  NVE_FIELD_LENGTH = 6,
  NVE_FIELD_STR = 7,
  NVE_FIELD_RESOURCE = 8,
  NVE_FIELD_NODE = 9
} NveFieldKind;

/* One field of a table row (Rust `nve_abi::table::FieldDesc` without name / effects). */
typedef struct NveFieldSpec {
  uint8_t word;  /* first word */
  uint8_t shift; /* bit offset inside `word` */
  uint8_t bits;  /* bit width inside `word` (32 = whole word; `Str` spans two words) */
  uint8_t kind;  /* NveFieldKind */
} NveFieldSpec;

/* Mirrors Rust `nve_store::Length`: f32 bits with a 2-bit unit tag in the low mantissa bits. */
typedef enum NveLengthUnit { NVE_LENGTH_POINTS = 0, NVE_LENGTH_PERCENT = 1, NVE_LENGTH_AUTO = 2 } NveLengthUnit;

#define NVE_MAX_TABLE_WORDS 32u
#define NVE_LENGTH_TAG_MASK 3u

/* Field bits of `word` (unsigned). */
NVE_INLINE uint32_t nve_field_bits(uint32_t word, NveFieldSpec f) {
  return nve_bits_extract(word, f.shift, f.bits);
}

/* Field bits of `word` as two's-complement `i32` (sign-extended from `bits`). */
NVE_INLINE int32_t nve_field_i32(uint32_t word, NveFieldSpec f) {
  return nve_bits_sign_extend(nve_field_bits(word, f), f.bits);
}

/* Tag `11` is unused by Rust and normalizes to `auto`, so every bit pattern is valid. */
NVE_INLINE NveLengthUnit nve_length_unit(uint32_t word) {
  const uint32_t tag = word & NVE_LENGTH_TAG_MASK;
  return tag == 0u ? NVE_LENGTH_POINTS : tag == 1u ? NVE_LENGTH_PERCENT : NVE_LENGTH_AUTO;
}

/* Raw `f32` bits of a length's magnitude (tag bits cleared; the unused tag `11` decodes to 0). */
NVE_INLINE uint32_t nve_length_value_bits(uint32_t word) {
  return (word & NVE_LENGTH_TAG_MASK) == NVE_LENGTH_TAG_MASK ? 0u : word & ~NVE_LENGTH_TAG_MASK;
}

/* Packed RGBA8888 channel `0 = r, 1 = g, 2 = b, 3 = a` (`r | g<<8 | b<<16 | a<<24`). */
NVE_INLINE uint8_t nve_color_channel(uint32_t rgba, uint32_t channel) {
  return (uint8_t)((rgba >> (channel * 8u)) & 0xFFu);
}

/* 1 when `f` is well-formed: known kind, a valid bit range, inside a maximal row (32 words). */
NVE_INLINE int nve_field_valid(NveFieldSpec f) {
  const uint32_t span = f.kind == (uint8_t)NVE_FIELD_STR ? 2u : 1u;
  return f.kind <= (uint8_t)NVE_FIELD_NODE && nve_bits_range_valid(f.shift, f.bits) &&
         (uint32_t)f.word + span <= NVE_MAX_TABLE_WORDS;
}

/* 1 when `f` is well-formed and lies inside a row of `words` words (`Str` needs its second word). */
NVE_INLINE int nve_field_fits(NveFieldSpec f, uint32_t words) {
  const uint32_t span = f.kind == (uint8_t)NVE_FIELD_STR ? 2u : 1u;
  return nve_field_valid(f) && (uint32_t)f.word + span <= words;
}

#endif /* NVE_HOST_TABLE_WORD_H */
