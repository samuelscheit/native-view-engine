#include "jni_test_recorder.h"

#include <cstring>

namespace {
NveJniTestLog g_log{};
}

extern "C" NveJniTestLog* nve_jni_test_log(void) {
  return &g_log;
}

extern "C" void nve_jni_test_log_reset(void) {
  std::memset(&g_log, 0, sizeof(g_log));
}
