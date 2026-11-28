/* NVE host runtime: word-table field decoding & encoding (C11 + constexpr C++20).
 *
 * A table row is `words` little-endian `u32`s. Generated per-table data supplies one
 * `NveFieldSpec {word, shift, bits, kind, opt, val_word, explicit_bit, inherits}` per field;
 * these helpers turn row words into typed values (required, optional, 64-bit, GridLine,
 * inherited explicit masks, and per-table `EventSet` words) exactly like Rust `nve_store`.
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
  NVE_FIELD_NODE = 9,
  NVE_FIELD_U64 = 10,
  NVE_FIELD_I64 = 11,
  NVE_FIELD_F64 = 12,
  NVE_FIELD_GRID_LINE = 13
} NveFieldKind;

/* Mirrors Rust `nve_abi::table::OptEncoding` (`#[repr(u8)]`). */
typedef enum NveOptEncoding {
  NVE_OPT_REQUIRED = 0,
  NVE_OPT_ZERO_NICHE = 1,
  NVE_OPT_BIT0_PRESENT = 2,
  NVE_OPT_PLUS_ONE = 3,
  NVE_OPT_TAG_XOR = 4,
  NVE_OPT_WIDE_BIT = 5
} NveOptEncoding;

#define NVE_NO_VAL_WORD 0xFFu
#define NVE_NO_EXPLICIT_BIT 0xFFu
#define NVE_NO_EVENTS_WORD 0xFFu

/* One field of a table row (matches `nve_tables.generated.h` 8-byte initializer layout). */
typedef struct NveFieldSpec {
  uint8_t word;         /* first word (for `NVE_OPT_WIDE_BIT`, word holding the 1-bit presence flag) */
  uint8_t shift;        /* bit offset inside `word` */
  uint8_t bits;         /* bit width inside `word` (32 = whole word; 2-word kinds span 2 words; 1 for `WideBit`) */
  uint8_t kind;         /* NveFieldKind */
  uint8_t opt;          /* NveOptEncoding */
  uint8_t val_word;     /* value word for `NVE_OPT_WIDE_BIT`, else `NVE_NO_VAL_WORD` */
  uint8_t explicit_bit; /* bit `0..31` in word 0 on inherited tables, else `NVE_NO_EXPLICIT_BIT` */
  uint8_t inherits;     /* 1 when an unset field on an inherited table resolves from the parent row, else 0 */
} NveFieldSpec;

/* Mirrors Rust `nve_abi::table::TableDesc` (without per-field metadata): everything an `NVE2`
 * `DeclareTables` entry carries. Generated per table as `NVE_TABLE_<NAME>_DESC`. */
typedef struct NveTableDesc {
  uint64_t key;
  uint64_t schema_hash;
  const char* name; /* UTF-8, not NUL-terminated in the wire; `name_len` bytes */
  uint32_t name_len;
  uint8_t storage; /* 0 = dense, 1 = sparse */
  uint8_t flags;   /* `table_flags` bits */
  uint8_t words;   /* row width, 1..=32 */
  uint8_t reserved;
  uint32_t str_words;          /* bit `i`: word `i` starts a two-word `Str` */
  uint32_t resource_words;     /* bit `i`: word `i` holds a `ResourceId` */
  const uint32_t* defaults;    /* `words` entries */
  const uint8_t* word_effects; /* `words` entries */
} NveTableDesc;

/* Mirrors Rust `nve_store::Length`: f32 bits with a 2-bit unit tag in the low mantissa bits. */
typedef enum NveLengthUnit { NVE_LENGTH_POINTS = 0, NVE_LENGTH_PERCENT = 1, NVE_LENGTH_AUTO = 2 } NveLengthUnit;

/* Mirrors Rust `nve_store::GridLine`: 16-bit CSS Grid line placement (`Auto`, `Line`, `Span`). */
typedef enum NveGridLineKind {
  NVE_GRID_LINE_AUTO = 0,
  NVE_GRID_LINE_LINE = 1,
  NVE_GRID_LINE_SPAN = 2
} NveGridLineKind;

typedef struct NveGridLine {
  uint8_t kind;  /* NveGridLineKind */
  int16_t line;  /* valid when `kind == NVE_GRID_LINE_LINE` (`-8192..=8191`) */
  uint16_t span; /* valid when `kind == NVE_GRID_LINE_SPAN` (`0..=16383`) */
} NveGridLine;

#define NVE_MAX_TABLE_WORDS 32u
#define NVE_LENGTH_TAG_MASK 3u
#define NVE_GRID_LINE_TAG_SHIFT 14u
#define NVE_GRID_LINE_TAG_NICHE 0xC000u
#define NVE_GRID_LINE_PAYLOAD_MASK 0x3FFFu
#define NVE_GRID_LINE_MIN_LINE (-8192)
#define NVE_GRID_LINE_MAX_LINE 8191
#define NVE_GRID_LINE_MAX_SPAN 16383u

/* 1 when `kind` spans two consecutive 32-bit words (`Str`, `U64`, `I64`, `F64`). */
NVE_INLINE int nve_field_is_two_word(uint8_t kind) {
  return kind == (uint8_t)NVE_FIELD_STR || kind == (uint8_t)NVE_FIELD_U64 ||
         kind == (uint8_t)NVE_FIELD_I64 || kind == (uint8_t)NVE_FIELD_F64;
}

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

/* Decodes a 16-bit packed `GridLine` word (unused tag `11` normalizes to `Auto`). */
NVE_INLINE NveGridLine nve_grid_line_decode(uint32_t bits) {
  const uint32_t tag = (bits >> NVE_GRID_LINE_TAG_SHIFT) & 3u;
  const uint32_t payload = bits & NVE_GRID_LINE_PAYLOAD_MASK;
  NveGridLine out = {(uint8_t)NVE_GRID_LINE_AUTO, 0, 0u};
  if (tag == 1u) {
    out.kind = (uint8_t)NVE_GRID_LINE_LINE;
    out.line = (int16_t)nve_bits_sign_extend(payload, 14u);
  } else if (tag == 2u) {
    out.kind = (uint8_t)NVE_GRID_LINE_SPAN;
    out.span = (uint16_t)payload;
  }
  return out;
}

/* Encodes a `GridLine` into 16 bits, saturating out-of-range lines and spans like Rust. */
NVE_INLINE uint32_t nve_grid_line_encode(NveGridLine g) {
  if (g.kind == (uint8_t)NVE_GRID_LINE_LINE) {
    int32_t v = (int32_t)g.line;
    if (v < NVE_GRID_LINE_MIN_LINE) v = NVE_GRID_LINE_MIN_LINE;
    if (v > NVE_GRID_LINE_MAX_LINE) v = NVE_GRID_LINE_MAX_LINE;
    return (1u << NVE_GRID_LINE_TAG_SHIFT) | ((uint32_t)v & NVE_GRID_LINE_PAYLOAD_MASK);
  }
  if (g.kind == (uint8_t)NVE_GRID_LINE_SPAN) {
    uint32_t v = (uint32_t)g.span;
    if (v > NVE_GRID_LINE_MAX_SPAN) v = NVE_GRID_LINE_MAX_SPAN;
    return (2u << NVE_GRID_LINE_TAG_SHIFT) | v;
  }
  return 0u;
}

/* Packed RGBA8888 channel `0 = r, 1 = g, 2 = b, 3 = a` (`r | g<<8 | b<<16 | a<<24`). */
/* Any other channel reads 0 (never a shift by >= 32). */
NVE_INLINE uint8_t nve_color_channel(uint32_t rgba, uint32_t channel) {
  if (channel >= 4u) return 0u;
  return (uint8_t)((rgba >> (channel * 8u)) & 0xFFu);
}

/* 1 when `f` is well-formed: known kind/opt, valid bit range, inside a maximal row (32 words). */
NVE_INLINE int nve_field_valid(NveFieldSpec f) {
  const int two_word = nve_field_is_two_word(f.kind);
  const uint32_t span = two_word ? 2u : 1u;
  if (f.kind > (uint8_t)NVE_FIELD_GRID_LINE || f.opt > (uint8_t)NVE_OPT_WIDE_BIT) return 0;
  if (!nve_bits_range_valid(f.shift, f.bits)) return 0;
  if (two_word && (f.shift != 0u || f.bits != 32u || f.opt != (uint8_t)NVE_OPT_REQUIRED)) return 0;
  if ((uint32_t)f.word + span > NVE_MAX_TABLE_WORDS) return 0;
  if (f.opt == (uint8_t)NVE_OPT_WIDE_BIT) {
    if (f.bits != 1u || f.val_word >= NVE_MAX_TABLE_WORDS || f.val_word == f.word) return 0;
  } else if (f.val_word != NVE_NO_VAL_WORD) {
    return 0;
  }
  if (f.opt == (uint8_t)NVE_OPT_BIT0_PRESENT && f.bits < 2u) return 0;
  if (f.opt == (uint8_t)NVE_OPT_TAG_XOR && f.kind != (uint8_t)NVE_FIELD_LENGTH &&
      f.kind != (uint8_t)NVE_FIELD_GRID_LINE) {
    return 0;
  }
  if (f.explicit_bit != NVE_NO_EXPLICIT_BIT && f.explicit_bit >= 32u) return 0;
  if (f.inherits > 1u || (f.inherits != 0u && f.explicit_bit == NVE_NO_EXPLICIT_BIT)) return 0;
  return 1;
}

/* 1 when `f` is well-formed and all of its words lie inside a row of `words` words. */
NVE_INLINE int nve_field_fits(NveFieldSpec f, uint32_t words) {
  const uint32_t span = nve_field_is_two_word(f.kind) ? 2u : 1u;
  return nve_field_valid(f) && words <= NVE_MAX_TABLE_WORDS && (uint32_t)f.word + span <= words &&
         (f.opt != (uint8_t)NVE_OPT_WIDE_BIT || (uint32_t)f.val_word < words);
}

/* 1 when `f` is explicitly set in `row` (non-inherited fields are always explicit; inherited
 * tables store their explicit bitmask in word 0). */
NVE_INLINE int nve_field_explicit(const uint32_t* row, uint32_t words, NveFieldSpec f) {
  if (f.explicit_bit == NVE_NO_EXPLICIT_BIT) return 1;
  if (row == NULL || words == 0u || f.explicit_bit >= 32u) return 0;
  return ((row[0] >> f.explicit_bit) & 1u) != 0u ? 1 : 0;
}

/* Per-table `EventSet` listener bitmask word (`0` when `events_word` is `NVE_NO_EVENTS_WORD` or OOB). */
NVE_INLINE uint32_t nve_row_events(const uint32_t* row, uint32_t words, uint8_t events_word) {
  if (row == NULL || events_word == NVE_NO_EVENTS_WORD || (uint32_t)events_word >= words) return 0u;
  return row[events_word];
}

/* Reads a two-word 64-bit unsigned payload (`lo | hi<<32`); 0 if invalid or out of range. */
NVE_INLINE uint64_t nve_field_u64(const uint32_t* row, uint32_t words, NveFieldSpec f) {
  if (row == NULL || !nve_field_is_two_word(f.kind) || !nve_field_fits(f, words)) return 0u;
  return (uint64_t)row[f.word] | ((uint64_t)row[f.word + 1u] << 32u);
}

/* Decodes a required or optional scalar field from `row[0..words)`. Returns 1 if present (writing
 * the decoded unsigned/word bits to `*out_bits` when non-NULL) and 0 if absent or invalid. */
NVE_INLINE int nve_field_opt_bits(const uint32_t* row, uint32_t words, NveFieldSpec f, uint32_t* out_bits) {
  uint32_t raw = 0u;
  uint32_t val = 0u;
  if (row == NULL || nve_field_is_two_word(f.kind) || !nve_field_fits(f, words)) return 0;
  raw = nve_field_bits(row[f.word], f);
  switch (f.opt) {
    case NVE_OPT_REQUIRED: val = raw; break;
    case NVE_OPT_ZERO_NICHE:
      if (raw == 0u) return 0;
      val = raw;
      break;
    case NVE_OPT_BIT0_PRESENT:
      if ((raw & 1u) == 0u) return 0;
      val = raw >> 1u;
      break;
    case NVE_OPT_PLUS_ONE:
      if (raw == 0u) return 0;
      val = raw - 1u;
      break;
    case NVE_OPT_TAG_XOR:
      if (f.kind == (uint8_t)NVE_FIELD_LENGTH) {
        if ((raw & NVE_LENGTH_TAG_MASK) == 0u) return 0;
        val = raw ^ NVE_LENGTH_TAG_MASK;
      } else {
        if ((raw & NVE_GRID_LINE_TAG_NICHE) == 0u) return 0;
        val = raw ^ NVE_GRID_LINE_TAG_NICHE;
      }
      break;
    case NVE_OPT_WIDE_BIT:
      if ((raw & 1u) == 0u) return 0;
      val = row[f.val_word];
      break;
    default: return 0;
  }
  if (out_bits != NULL) *out_bits = val;
  return 1;
}

/* Decodes a required or optional signed integer field from `row[0..words)`, sign-extending from
 * its payload width. Returns 1 if present and 0 if absent or invalid. */
NVE_INLINE int nve_field_opt_i32(const uint32_t* row, uint32_t words, NveFieldSpec f, int32_t* out_value) {
  uint32_t bits = 0u;
  uint32_t width = f.bits;
  if (!nve_field_opt_bits(row, words, f, &bits)) return 0;
  if (f.opt == (uint8_t)NVE_OPT_BIT0_PRESENT) {
    width = (uint32_t)f.bits - 1u;
  } else if (f.opt == (uint8_t)NVE_OPT_WIDE_BIT) {
    width = 32u;
  }
  if (out_value != NULL) *out_value = nve_bits_sign_extend(bits, width);
  return 1;
}

/* Encodes a required or optional scalar field value into `f`'s primary bit slice (`*out_word_bits`)
 * and, for `NVE_OPT_WIDE_BIT`, its value word (`*out_val_word`). Returns 1 on success, 0 if invalid. */
NVE_INLINE int nve_field_opt_encode(NveFieldSpec f, int present, uint32_t value, uint32_t* out_word_bits,
                                    uint32_t* out_val_word) {
  uint32_t wb = 0u;
  uint32_t vw = 0u;
  if (nve_field_is_two_word(f.kind) || !nve_field_valid(f)) return 0;
  switch (f.opt) {
    case NVE_OPT_REQUIRED: wb = value; break;
    case NVE_OPT_ZERO_NICHE: wb = present ? value : 0u; break;
    case NVE_OPT_BIT0_PRESENT: wb = present ? ((value << 1u) | 1u) : 0u; break;
    case NVE_OPT_PLUS_ONE: wb = present ? (value + 1u) : 0u; break;
    case NVE_OPT_TAG_XOR:
      if (f.kind == (uint8_t)NVE_FIELD_LENGTH) {
        const uint32_t norm =
            (value & NVE_LENGTH_TAG_MASK) == NVE_LENGTH_TAG_MASK ? (uint32_t)NVE_LENGTH_AUTO : value;
        wb = present ? (norm ^ NVE_LENGTH_TAG_MASK) : 0u;
      } else {
        const uint32_t norm =
            (value & NVE_GRID_LINE_TAG_NICHE) == NVE_GRID_LINE_TAG_NICHE ? 0u : (value & 0xFFFFu);
        wb = present ? (norm ^ NVE_GRID_LINE_TAG_NICHE) : 0u;
      }
      break;
    case NVE_OPT_WIDE_BIT:
      wb = present ? 1u : 0u;
      vw = present ? value : 0u;
      break;
    default: return 0;
  }
  if (out_word_bits != NULL) *out_word_bits = wb;
  if (out_val_word != NULL) *out_val_word = vw;
  return 1;
}

#endif /* NVE_HOST_TABLE_WORD_H */
