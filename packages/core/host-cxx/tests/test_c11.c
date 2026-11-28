/* C11 suite: <nve/host/api.h>, <nve/host/xxh64.h>, <nve/host/utf8.h>, <nve/host/table_word.h>, and
 * the generated <nve/generated/nve_abi.generated.h> / <nve/generated/nve_tables.generated.h>
 * (compiled strictly: -Wall -Wextra -Werror -pedantic). */
#include "nve/generated/nve_abi.generated.h"
#include "nve/generated/nve_tables.generated.h"
#include "nve/host/api.h"
#include "nve/host/table_word.h"
#include "nve/host/utf8.h"
#include "nve/host/xxh64.h"
#include "vectors.h"

typedef struct LayoutProbe {
  uint32_t a;
  uint64_t b;
} LayoutProbe;

NVE_STATIC_ASSERT(NVE_OFFSETOF(LayoutProbe, b) == 8, "u64 field aligned to 8");
NVE_STATIC_ASSERT(NVE_ALIGNOF(NveKey64) == NVE_ALIGNOF(uint64_t), "NveKey64 is one u64");
NVE_STATIC_ASSERT(sizeof(NveFieldSpec) == 8, "NveFieldSpec packs into 8 bytes");
NVE_STATIC_ASSERT(sizeof(NveEvent) == NVE_EVENT_SIZE, "NveEvent is 64 bytes");

static void test_xxh64_vectors(void) {
  static uint8_t buf[NVE_HASH_VECTOR_MAX_LEN];
  static const size_t steps[] = {1, 3, 7, 13, 24, 31, 32, 33, 64, 100};
  for (size_t v = 0; v < NVE_HASH_VECTOR_COUNT; ++v) {
    const NveHashVector* hv = &NVE_HASH_VECTORS[v];
    const size_t len = vector_bytes(hv, buf);
    CHECK(nve_xxh64(buf, len, hv->seed) == hv->expected);
    for (size_t s = 0; s < sizeof steps / sizeof steps[0]; ++s) {
      NveXxh64State st = nve_xxh64_init(hv->seed);
      for (size_t at = 0; at < len; at += steps[s]) {
        const size_t n = len - at < steps[s] ? len - at : steps[s];
        nve_xxh64_update(&st, buf + at, n);
      }
      CHECK(nve_xxh64_digest(&st) == hv->expected);
    }
  }
  /* NULL with zero length is the empty input. */
  CHECK(nve_xxh64(NULL, 0, 0) == UINT64_C(0xEF46DB3751D8E999));
  /* Digest does not consume the state. */
  NveXxh64State st = nve_xxh64_init(0);
  nve_xxh64_update(&st, (const uint8_t*)"ab", 2);
  CHECK(nve_xxh64_digest(&st) == nve_xxh64_digest(&st));
  nve_xxh64_update(&st, (const uint8_t*)"c", 1);
  CHECK(nve_xxh64_digest(&st) == UINT64_C(0x44BC2CF5AD770999));
}

static void test_key64(void) {
  NveKey64 k = {0};
  CHECK(nve_key64_of("nve.visual", 10, &k) == 1);
  CHECK(k.value == UINT64_C(0x33E73115985A4FD6));
  /* A name that is not well-formed UTF-8 (lone surrogate / overlong) has no identity. */
  NveKey64 untouched = {UINT64_C(7)};
  CHECK(nve_key64_of("\xED\xA0\x80", 3, &untouched) == 0 && untouched.value == 7u);
  CHECK(nve_key64_of("\xC0\xAF", 2, &untouched) == 0 && untouched.value == 7u);
  /* NULL handling is total (UBSan): empty NULL name is the empty name; NULL with a length or NULL out is rejected. */
  NveKey64 empty = {0};
  CHECK(nve_key64_of(NULL, 0, &empty) == 1 && empty.value == UINT64_C(0xEF46DB3751D8E999));
  CHECK(nve_key64_of(NULL, 5, &untouched) == 0 && untouched.value == 7u);
  CHECK(nve_key64_of("a", 1, NULL) == 0);
  CHECK(nve_key64_lo(k) == UINT32_C(0x985A4FD6));
  CHECK(nve_key64_hi(k) == UINT32_C(0x33E73115));
  CHECK(nve_key64_eq(nve_key64_from_halves(nve_key64_lo(k), nve_key64_hi(k)), k));
  CHECK(nve_key64_from_halves(UINT32_MAX, UINT32_MAX).value == UINT64_MAX);
}

static void test_le_loaders(void) {
  uint8_t b[9] = {0};
  nve_store_u64_le(b + 1, UINT64_C(0x0102030405060708)); /* unaligned on purpose */
  CHECK(b[1] == 0x08 && b[8] == 0x01);
  CHECK(nve_load_u64_le(b + 1) == UINT64_C(0x0102030405060708));
  CHECK(nve_load_u32_le(b + 1) == UINT32_C(0x05060708));
  CHECK(nve_load_u16_le(b + 1) == 0x0708);
  nve_store_u32_le(b, UINT32_C(0xFFFFFFFF));
  CHECK(nve_load_u32_le(b) == UINT32_MAX);
  nve_store_u16_le(b, 0xBEEF);
  CHECK(b[0] == 0xEF && b[1] == 0xBE);
}

static void test_bits(void) {
  CHECK(nve_bits_extract(UINT32_C(0xABCD1234), 0, 32) == UINT32_C(0xABCD1234));
  CHECK(nve_bits_extract(UINT32_C(0xABCD1234), 8, 8) == 0x12u);
  CHECK(nve_bits_extract(UINT32_C(0x80000000), 31, 1) == 1u);
  CHECK(nve_bits_sign_extend(0x7u, 3) == -1);
  CHECK(nve_bits_sign_extend(0x3u, 3) == 3);
  CHECK(nve_bits_sign_extend(0x4u, 3) == -4);
  CHECK(nve_bits_sign_extend(UINT32_C(0x80000000), 32) == INT32_MIN);
  CHECK(nve_bits_sign_extend(UINT32_C(0xFFFFFFFF), 32) == -1);
  CHECK(nve_bits_sign_extend(UINT32_C(0x7FFFFFFF), 32) == INT32_MAX);
  CHECK(nve_bits_sign_extend(1u, 1) == -1);
  /* Invalid ranges are total (0), never a UB shift (the suite runs under UBSan). */
  CHECK(nve_bits_extract(UINT32_MAX, 0, 0) == 0u);
  CHECK(nve_bits_extract(UINT32_MAX, 0, 33) == 0u);
  CHECK(nve_bits_extract(UINT32_MAX, 32, 1) == 0u);
  CHECK(nve_bits_extract(UINT32_MAX, 31, 2) == 0u);
  CHECK(nve_bits_extract(UINT32_MAX, UINT32_MAX, 2) == 0u);
  CHECK(nve_bits_sign_extend(1u, 0) == 0);
  CHECK(nve_bits_sign_extend(1u, 40) == 0);
}

static void test_table_word(void) {
  const NveFieldSpec opacity = {2, 8, 8, NVE_FIELD_U32, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec delta = {0, 4, 4, NVE_FIELD_I32, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec label = {3, 0, 32, NVE_FIELD_STR, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec big_u64 = {1, 0, 32, NVE_FIELD_U64, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec grid_col = {0, 0, 16, NVE_FIELD_GRID_LINE, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  CHECK(nve_field_bits(UINT32_C(0x0000AB00), opacity) == 0xABu);
  CHECK(nve_field_i32(UINT32_C(0x000000F0), delta) == -1);
  CHECK(nve_field_fits(opacity, 3));
  CHECK(!nve_field_fits(opacity, 2));
  CHECK(nve_field_fits(label, 5));
  CHECK(!nve_field_fits(label, 4)); /* second string word missing */
  CHECK(nve_field_fits(big_u64, 3) && !nve_field_fits(big_u64, 2));
  CHECK(nve_field_fits(grid_col, 1));
  const NveFieldSpec too_wide = {0, 24, 16, NVE_FIELD_U32, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  CHECK(!nve_field_fits(too_wide, 1));
  const NveFieldSpec zero_bits = {0, 0, 0, NVE_FIELD_U32, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  CHECK(!nve_field_fits(zero_bits, 1));
  const NveFieldSpec past_row = {32, 0, 32, NVE_FIELD_U32, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec str_past_row = {31, 0, 32, NVE_FIELD_STR, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec u64_past_row = {31, 0, 32, NVE_FIELD_U64, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec bad_kind = {0, 0, 32, 14, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec bad_opt = {0, 0, 32, NVE_FIELD_U32, 6, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec bad_wide = {0, 0, 1, NVE_FIELD_F32, NVE_OPT_WIDE_BIT, 0, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec bad_tag_xor = {0, 0, 32, NVE_FIELD_U32, NVE_OPT_TAG_XOR, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  CHECK(!nve_field_valid(past_row) && !nve_field_valid(str_past_row) && !nve_field_valid(u64_past_row));
  CHECK(!nve_field_valid(bad_kind) && !nve_field_valid(bad_opt) && !nve_field_valid(bad_wide) && !nve_field_valid(bad_tag_xor));
  CHECK(nve_field_valid(label) && nve_field_valid(opacity) && nve_field_valid(big_u64) && nve_field_valid(grid_col));
  CHECK(nve_field_i32(UINT32_MAX, zero_bits) == 0);

  /* Optional encodings (all 6) + explicit bit + EventSet word + 64-bit read. */
  const uint32_t u64_row[3] = {0x00000005u, UINT32_C(0x89ABCDEF), UINT32_C(0x01234567)};
  CHECK(nve_field_u64(u64_row, 3, big_u64) == UINT64_C(0x0123456789ABCDEF));
  CHECK(nve_field_u64(u64_row, 2, big_u64) == 0u);
  CHECK(nve_row_events(u64_row, 3, 0) == 5u && nve_row_events(u64_row, 3, NVE_NO_EVENTS_WORD) == 0u);
  const NveFieldSpec inh_set = {1, 0, 32, NVE_FIELD_U32, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, 2, 1};
  const NveFieldSpec inh_unset = {1, 0, 32, NVE_FIELD_U32, NVE_OPT_REQUIRED, NVE_NO_VAL_WORD, 1, 1};
  CHECK(nve_field_explicit(u64_row, 3, inh_set) == 1 && nve_field_explicit(u64_row, 3, inh_unset) == 0);
  CHECK(nve_field_explicit(u64_row, 3, opacity) == 1);

  /* ZeroNiche, Bit0Present (unsigned + signed), PlusOne, TagXor (Length + GridLine), WideBit. */
  const NveFieldSpec opt_zn = {0, 0, 16, NVE_FIELD_U32, NVE_OPT_ZERO_NICHE, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec opt_b0_i = {0, 1, 17, NVE_FIELD_I32, NVE_OPT_BIT0_PRESENT, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec opt_p1 = {0, 0, 6, NVE_FIELD_ENUM, NVE_OPT_PLUS_ONE, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec opt_len = {0, 0, 32, NVE_FIELD_LENGTH, NVE_OPT_TAG_XOR, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec opt_gl = {0, 0, 16, NVE_FIELD_GRID_LINE, NVE_OPT_TAG_XOR, NVE_NO_VAL_WORD, NVE_NO_EXPLICIT_BIT, 0};
  const NveFieldSpec opt_wb = {0, 3, 1, NVE_FIELD_F32, NVE_OPT_WIDE_BIT, 1, NVE_NO_EXPLICIT_BIT, 0};
  uint32_t out_u = 0u, wb = 0u, vw = 0u;
  int32_t out_i = 0;
  const uint32_t zero_row[2] = {0u, 0u};
  CHECK(!nve_field_opt_bits(zero_row, 2, opt_zn, &out_u) && !nve_field_opt_i32(zero_row, 2, opt_b0_i, &out_i));
  CHECK(!nve_field_opt_bits(zero_row, 2, opt_p1, &out_u) && !nve_field_opt_bits(zero_row, 2, opt_len, &out_u));
  CHECK(!nve_field_opt_bits(zero_row, 2, opt_gl, &out_u) && !nve_field_opt_bits(zero_row, 2, opt_wb, &out_u));
  CHECK(nve_field_opt_encode(opt_b0_i, 1, (uint32_t)(-5), &wb, &vw));
  const uint32_t b0_row[1] = {wb << 1u};
  CHECK(nve_field_opt_i32(b0_row, 1, opt_b0_i, &out_i) && out_i == -5);
  CHECK(nve_field_opt_encode(opt_p1, 1, 0u, &wb, &vw) && wb == 1u);
  const uint32_t p1_row[1] = {wb};
  CHECK(nve_field_opt_bits(p1_row, 1, opt_p1, &out_u) && out_u == 0u);
  CHECK(nve_field_opt_encode(opt_len, 1, NVE_LENGTH_AUTO, &wb, &vw));
  const uint32_t len_row[1] = {wb};
  CHECK(nve_field_opt_bits(len_row, 1, opt_len, &out_u) && nve_length_unit(out_u) == NVE_LENGTH_AUTO);
  const NveGridLine gl_in = {NVE_GRID_LINE_LINE, -7, 0u};
  CHECK(nve_field_opt_encode(opt_gl, 1, nve_grid_line_encode(gl_in), &wb, &vw));
  const uint32_t gl_row[1] = {wb};
  CHECK(nve_field_opt_bits(gl_row, 1, opt_gl, &out_u));
  const NveGridLine gl_out = nve_grid_line_decode(out_u);
  CHECK(gl_out.kind == NVE_GRID_LINE_LINE && gl_out.line == -7);
  /* GridLine saturation & Span round-trip. */
  const NveGridLine gl_hi = {NVE_GRID_LINE_LINE, 30000, 0u};
  const NveGridLine gl_lo = {NVE_GRID_LINE_LINE, -30000, 0u};
  const NveGridLine gl_sp = {NVE_GRID_LINE_SPAN, 0, 20000u};
  CHECK(nve_grid_line_decode(nve_grid_line_encode(gl_hi)).line == NVE_GRID_LINE_MAX_LINE);
  CHECK(nve_grid_line_decode(nve_grid_line_encode(gl_lo)).line == NVE_GRID_LINE_MIN_LINE);
  CHECK(nve_grid_line_decode(nve_grid_line_encode(gl_sp)).span == NVE_GRID_LINE_MAX_SPAN);
  CHECK(nve_grid_line_decode(0xC123u).kind == NVE_GRID_LINE_AUTO); /* tag 11 normalizes to Auto */
  CHECK(nve_field_opt_encode(opt_wb, 1, UINT32_C(0x3F800000), &wb, &vw) && wb == 1u && vw == UINT32_C(0x3F800000));
  const uint32_t wb_row[2] = {wb << 3u, vw};
  CHECK(nve_field_opt_bits(wb_row, 2, opt_wb, &out_u) && out_u == UINT32_C(0x3F800000));

  /* Length: 1.0f points, 50 % percent, auto, and the unused tag `11`. */
  CHECK(nve_length_unit(UINT32_C(0x3F800000)) == NVE_LENGTH_POINTS);
  CHECK(nve_length_value_bits(UINT32_C(0x3F800000)) == UINT32_C(0x3F800000));
  CHECK(nve_length_unit(UINT32_C(0x42480001)) == NVE_LENGTH_PERCENT);
  CHECK(nve_length_value_bits(UINT32_C(0x42480001)) == UINT32_C(0x42480000));
  CHECK(nve_length_unit(2u) == NVE_LENGTH_AUTO);
  CHECK(nve_length_value_bits(2u) == 0u);
  CHECK(nve_length_unit(UINT32_C(0x3F800003)) == NVE_LENGTH_AUTO);
  CHECK(nve_length_value_bits(UINT32_C(0x3F800003)) == 0u);
  CHECK(nve_color_channel(UINT32_C(0x80402010), 0) == 0x10);
  CHECK(nve_color_channel(UINT32_C(0x80402010), 3) == 0x80);
  CHECK(nve_color_channel(UINT32_C(0x80402010), 4) == 0); /* no shift by 32 (UBSan) */
  CHECK(nve_color_channel(UINT32_C(0x80402010), 99) == 0);
  CHECK(nve_color_channel(UINT32_C(0x80402010), UINT32_MAX) == 0);
}

static void test_utf8(void) {
  const uint8_t ok_ascii[] = "plain";
  const uint8_t ok_multi[] = {0xC3, 0xA9, 0xE2, 0x80, 0x94, 0xF0, 0x9F, 0x9A, 0x80, 0xF4, 0x8F, 0xBF, 0xBF};
  const uint8_t overlong2[] = {0xC0, 0xAF};
  const uint8_t overlong3[] = {0xE0, 0x80, 0xAF};
  const uint8_t overlong4[] = {0xF0, 0x80, 0x80, 0xAF};
  const uint8_t surrogate[] = {0xED, 0xA0, 0x80};
  const uint8_t too_big[] = {0xF4, 0x90, 0x80, 0x80};
  const uint8_t truncated[] = {0xE2, 0x80};
  const uint8_t bad_cont[] = {0xE2, 0x28, 0xA1};
  const uint8_t stray_cont[] = {0x80};
  const uint8_t f5[] = {0xF5, 0x80, 0x80, 0x80};
  CHECK(nve_utf8_valid(ok_ascii, 5));
  CHECK(nve_utf8_valid(ok_multi, sizeof ok_multi));
  CHECK(nve_utf8_valid(NULL, 0));
  CHECK(!nve_utf8_valid(NULL, 5)); /* never dereferences NULL */
  CHECK(!nve_utf8_valid(overlong2, sizeof overlong2));
  CHECK(!nve_utf8_valid(overlong3, sizeof overlong3));
  CHECK(!nve_utf8_valid(overlong4, sizeof overlong4));
  CHECK(!nve_utf8_valid(surrogate, sizeof surrogate));
  CHECK(!nve_utf8_valid(too_big, sizeof too_big));
  CHECK(!nve_utf8_valid(truncated, sizeof truncated));
  CHECK(!nve_utf8_valid(bad_cont, sizeof bad_cont));
  CHECK(!nve_utf8_valid(stray_cont, sizeof stray_cont));
  CHECK(!nve_utf8_valid(f5, sizeof f5));
}

int main(void) {
  test_xxh64_vectors();
  test_key64();
  test_le_loaders();
  test_bits();
  test_table_word();
  test_utf8();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_c11_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_c11_test: ok (%u vectors)\n", (unsigned)NVE_HASH_VECTOR_COUNT);
  return 0;
}
