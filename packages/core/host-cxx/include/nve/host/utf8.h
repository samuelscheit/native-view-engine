/* NVE host runtime: allocation-free strict UTF-8 validation (same acceptance as Rust
 * `core::str::from_utf8`: no overlongs, no surrogates, nothing above U+10FFFF). */
#ifndef NVE_HOST_UTF8_H
#define NVE_HOST_UTF8_H

#include "api.h"

/* Returns 1 when `p[0..len)` is well-formed UTF-8 (`p` may be NULL only when `len == 0`). */
NVE_INLINE int nve_utf8_valid(const uint8_t* p, size_t len) {
  if (p == NULL) return len == 0u;
  size_t i = 0u;
  while (i < len) {
    const uint8_t b0 = p[i];
    if (b0 < 0x80u) {
      ++i;
      continue;
    }
    size_t need = 0u;
    uint8_t lo = 0x80u;
    uint8_t hi = 0xBFu;
    if (b0 >= 0xC2u && b0 <= 0xDFu) {
      need = 1u;
    } else if (b0 >= 0xE0u && b0 <= 0xEFu) {
      need = 2u;
      if (b0 == 0xE0u) lo = 0xA0u; /* overlong */
      if (b0 == 0xEDu) hi = 0x9Fu; /* surrogates */
    } else if (b0 >= 0xF0u && b0 <= 0xF4u) {
      need = 3u;
      if (b0 == 0xF0u) lo = 0x90u; /* overlong */
      if (b0 == 0xF4u) hi = 0x8Fu; /* > U+10FFFF */
    } else {
      return 0;
    }
    if (need > len - i - 1u) return 0;
    if (p[i + 1u] < lo || p[i + 1u] > hi) return 0;
    for (size_t k = 2u; k <= need; ++k) {
      if ((p[i + k] & 0xC0u) != 0x80u) return 0;
    }
    i += need + 1u;
  }
  return 1;
}

#endif /* NVE_HOST_UTF8_H */
