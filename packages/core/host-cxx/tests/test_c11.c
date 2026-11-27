/* C11 suite: <nve/host/api.h>, <nve/host/xxh64.h>, <nve/host/utf8.h>, <nve/host/table_word.h>. */
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
NVE_STATIC_ASSERT(sizeof(NveFieldSpec) == 4, "NveFieldSpec packs into 4 bytes");

static void test_xxh64_vectors(void) {
  static uint8_t buf[MAX_VECTOR_LEN];
  static const size_t steps[] = {1, 3, 7, 13, 24, 31, 32, 33, 64, 100};
  for (size_t v = 0; v < VECTOR_COUNT; ++v) {
    const HashVector* hv = &kVectors[v];
    vector_bytes(hv, buf);
    CHECK(nve_xxh64(buf, hv->len, hv->seed) == hv->expected);
    for (size_t s = 0; s < sizeof steps / sizeof steps[0]; ++s) {
      NveXxh64State st = nve_xxh64_init(hv->seed);
      for (size_t at = 0; at < hv->len; at += steps[s]) {
        const size_t n = hv->len - at < steps[s] ? hv->len - at : steps[s];
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
  const NveFieldSpec opacity = {2, 8, 8, NVE_FIELD_U32};
  const NveFieldSpec delta = {0, 4, 4, NVE_FIELD_I32};
  const NveFieldSpec label = {3, 0, 32, NVE_FIELD_STR};
  CHECK(nve_field_bits(UINT32_C(0x0000AB00), opacity) == 0xABu);
  CHECK(nve_field_i32(UINT32_C(0x000000F0), delta) == -1);
  CHECK(nve_field_fits(opacity, 3));
  CHECK(!nve_field_fits(opacity, 2));
  CHECK(nve_field_fits(label, 5));
  CHECK(!nve_field_fits(label, 4)); /* second string word missing */
  const NveFieldSpec too_wide = {0, 24, 16, NVE_FIELD_U32};
  CHECK(!nve_field_fits(too_wide, 1));
  const NveFieldSpec zero_bits = {0, 0, 0, NVE_FIELD_U32};
  CHECK(!nve_field_fits(zero_bits, 1));
  const NveFieldSpec past_row = {32, 0, 32, NVE_FIELD_U32};
  const NveFieldSpec str_past_row = {31, 0, 32, NVE_FIELD_STR};
  const NveFieldSpec bad_kind = {0, 0, 32, 10};
  CHECK(!nve_field_valid(past_row) && !nve_field_valid(str_past_row) && !nve_field_valid(bad_kind));
  CHECK(nve_field_valid(label) && nve_field_valid(opacity));
  CHECK(nve_field_i32(UINT32_MAX, zero_bits) == 0);
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
  printf("nve_host_c11_test: ok (%u vectors)\n", (unsigned)VECTOR_COUNT);
  return 0;
}
