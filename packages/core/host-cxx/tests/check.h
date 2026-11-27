/* Minimal assertion harness shared by the C and C++ suites (failures are counted, not fatal). */
#ifndef NVE_HOST_TEST_CHECK_H
#define NVE_HOST_TEST_CHECK_H

#include <stdio.h>

static int g_failures = 0;

#define CHECK(cond)                                                    \
  do {                                                                 \
    if (!(cond)) {                                                     \
      fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
      ++g_failures;                                                    \
    }                                                                  \
  } while (0)

#endif /* NVE_HOST_TEST_CHECK_H */
