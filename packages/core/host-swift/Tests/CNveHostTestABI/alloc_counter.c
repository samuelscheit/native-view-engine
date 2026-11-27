/* Test-only allocation counter: libmalloc calls `malloc_logger` (the hook Instruments and
 * MallocStackLogging use) for every malloc-family call, so temporary allocations that are freed
 * again — invisible to live-block statistics — are counted too. Only the calling thread counts. */
#include "nve_host_test_abi.h"

#if defined(__APPLE__)
#include <pthread.h>
#include <stdatomic.h>

typedef void(NveMallocLogger)(uint32_t type, uintptr_t arg1, uintptr_t arg2, uintptr_t arg3, uintptr_t result,
                              uint32_t frames_to_skip);
extern NveMallocLogger* malloc_logger; /* exported by libsystem_malloc */

enum { NVE_MALLOC_LOG_TYPE_ALLOCATE = 2 };

static NveMallocLogger* g_previous;
static pthread_t g_thread;
static _Atomic int64_t g_count;

static void nve_count_allocation(uint32_t type, uintptr_t arg1, uintptr_t arg2, uintptr_t arg3, uintptr_t result,
                                 uint32_t frames_to_skip) {
  if ((type & NVE_MALLOC_LOG_TYPE_ALLOCATE) != 0 && pthread_equal(pthread_self(), g_thread)) {
    atomic_fetch_add(&g_count, 1);
  }
  if (g_previous != NULL) g_previous(type, arg1, arg2, arg3, result, frames_to_skip + 1);
}

int nve_test_alloc_count_begin(void) {
  g_thread = pthread_self();
  atomic_store(&g_count, 0);
  g_previous = malloc_logger;
  malloc_logger = nve_count_allocation;
  return 1;
}

int64_t nve_test_alloc_count_end(void) {
  malloc_logger = g_previous;
  return atomic_load(&g_count);
}
#else
int nve_test_alloc_count_begin(void) { return 0; }
int64_t nve_test_alloc_count_end(void) { return -1; }
#endif
