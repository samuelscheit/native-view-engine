/* Golden XXH64 vectors for the C and C++ suites. The vectors themselves are generated from the
 * canonical Rust `nve_hash::xxh64` by `nve codegen abi` (`hash_vectors.generated.h`). */
#ifndef NVE_HOST_TEST_VECTORS_H
#define NVE_HOST_TEST_VECTORS_H

#include <stddef.h>
#include <stdint.h>

#include "check.h"
#include "hash_vectors.generated.h"
#include "nve/host/xxh64.h"

#define PRIME32_1 UINT64_C(0x9E3779B1)
#define WIRE_SEED UINT64_C(0x4E56455749524532)

static uint8_t hex_nibble(char c) {
  return (uint8_t)(c <= '9' ? c - '0' : c - 'a' + 10);
}

/* Decodes `v->hex` into `out` (capacity `NVE_HASH_VECTOR_MAX_LEN`); returns the byte count. */
static size_t vector_bytes(const NveHashVector* v, uint8_t* out) {
  size_t n = 0;
  for (const char* p = v->hex; p[0] != '\0' && p[1] != '\0'; p += 2) {
    out[n++] = (uint8_t)((unsigned)hex_nibble(p[0]) << 4 | hex_nibble(p[1]));
  }
  return n;
}

#endif /* NVE_HOST_TEST_VECTORS_H */
