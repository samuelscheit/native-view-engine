#pragma once

#include <cstddef>
#include <cstdint>

struct NveJniTestLog {
  uint32_t v1_create_count;
  uint32_t v1_surface_id;
  uint32_t v1_destroy_count;
  uint32_t v1_snapshot_count;
  uint64_t v1_last_frame_id;

  uint32_t v2_create_count;
  uint32_t v2_surface_id;
  uint32_t v2_destroy_count;
  uint32_t v2_restore_count;
  uint64_t v2_last_frame_id;
  char v2_restored_state[64];
  size_t v2_restored_len;
};

extern "C" {
NveJniTestLog* nve_jni_test_log(void);
void nve_jni_test_log_reset(void);
}
