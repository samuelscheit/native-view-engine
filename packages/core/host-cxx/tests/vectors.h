/* Golden XXH64 vectors, computed by the canonical Rust `nve-hash` crate (shared by C and C++). */
#ifndef NVE_HOST_TEST_VECTORS_H
#define NVE_HOST_TEST_VECTORS_H

#include <stdint.h>

#include "check.h"
#include "nve/host/xxh64.h"

typedef enum VectorInput { INPUT_TEXT = 0, INPUT_SEQ = 1, INPUT_CYAN = 2, INPUT_BYTES = 3 } VectorInput;

typedef struct HashVector {
  const char* label;
  VectorInput input;
  const char* text; /* INPUT_TEXT (UTF-8) / INPUT_BYTES (raw) */
  size_t len;
  uint64_t seed;
  uint64_t expected;
} HashVector;

#define PRIME32_1 UINT64_C(0x9E3779B1)
#define WIRE_SEED UINT64_C(0x4E56455749524532)

static const HashVector kVectors[] = {
    {"empty", INPUT_TEXT, "", 0, 0, UINT64_C(0xEF46DB3751D8E999)},
    {"a", INPUT_TEXT, "a", 1, 0, UINT64_C(0xD24EC4F1A98C6E5B)},
    {"abc", INPUT_TEXT, "abc", 3, 0, UINT64_C(0x44BC2CF5AD770999)},
    {"nve.visual", INPUT_TEXT, "nve.visual", 10, 0, UINT64_C(0x33E73115985A4FD6)},
    {"high_bits", INPUT_BYTES, "\x00\x01\x7F\x80\xFF", 5, 0, UINT64_C(0x8BABCAB499A141EA)},
    {"utf8", INPUT_TEXT, "h\xC3\xA9llo w\xC3\xB6rld \xE2\x80\x94 \xE6\x97\xA5\xE6\x9C\xAC\xE8\xAA\x9E \xF0\x9F\x9A\x80", 32, 0,
     UINT64_C(0x72AC442FF213F592)},
    {"seq101", INPUT_SEQ, NULL, 101, 0, UINT64_C(0xFBFDF3FA1A53BC7F)},
    {"seq101_wire", INPUT_SEQ, NULL, 101, WIRE_SEED, UINT64_C(0x277E092A8C3DB1BC)},
    {"seq37_prime1", INPUT_SEQ, NULL, 37, NVE_XXH64_PRIME1, UINT64_C(0x33DDB9A885FC58A4)},
    {"seq1024", INPUT_SEQ, NULL, 1024, 0, UINT64_C(0x149AA44972CDAE00)},
    {"cyan1", INPUT_CYAN, NULL, 1, 0, UINT64_C(0xE934A84ADB052768)},
    {"cyan4", INPUT_CYAN, NULL, 4, 0, UINT64_C(0xE244DDBCF81E29E4)},
    {"cyan14", INPUT_CYAN, NULL, 14, 0, UINT64_C(0xB89B3598E0BD0A0A)},
    {"cyan222", INPUT_CYAN, NULL, 222, 0, UINT64_C(0x06CC5BD930BCEC3A)},
    {"cyan1024", INPUT_CYAN, NULL, 1024, 0, UINT64_C(0x2E307F2FE063991A)},
    {"cyan1_p32", INPUT_CYAN, NULL, 1, PRIME32_1, UINT64_C(0x5014607643A9B4C3)},
    {"cyan222_p32", INPUT_CYAN, NULL, 222, PRIME32_1, UINT64_C(0x0B105469DF89AF66)},
    {"cyan32_p1", INPUT_CYAN, NULL, 32, NVE_XXH64_PRIME1, UINT64_C(0x6D0242B0B7E278ED)},
    {"cyan1024_p1", INPUT_CYAN, NULL, 1024, NVE_XXH64_PRIME1, UINT64_C(0xF464C3F69A39421C)},
};

#define VECTOR_COUNT (sizeof kVectors / sizeof kVectors[0])
#define MAX_VECTOR_LEN 1024u

/* Materializes vector input into `out` (capacity `MAX_VECTOR_LEN`). */
static void vector_bytes(const HashVector* v, uint8_t* out) {
  uint64_t gen = PRIME32_1;
  for (size_t i = 0; i < v->len; ++i) {
    switch (v->input) {
      case INPUT_TEXT:
      case INPUT_BYTES:
        out[i] = (uint8_t)v->text[i];
        break;
      case INPUT_SEQ:
        out[i] = (uint8_t)((i * 31u + 7u) & 0xFFu);
        break;
      case INPUT_CYAN:
        out[i] = (uint8_t)(gen >> 56);
        gen *= NVE_XXH64_PRIME1;
        break;
    }
  }
}

#endif /* NVE_HOST_TEST_VECTORS_H */
