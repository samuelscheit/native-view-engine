#include "jni_test_recorder.h"

#include <jni.h>

#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

extern "C" {
JNIEXPORT jlong JNICALL Java_dev_nve_host_NveNativeBridge_nativeCreateHost(
    JNIEnv* env,
    jclass clazz,
    jint surface_id,
    jfloat width,
    jfloat height,
    jfloat scale_factor);

JNIEXPORT jboolean JNICALL Java_dev_nve_host_NveNativeBridge_nativeHotSwapApp(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jstring so_path);
}

namespace {

struct FakeJString {
  std::string value;
};

int g_active_utf_borrows = 0;

const char* JNICALL fake_get_string_utf_chars(JNIEnv*, jstring str, jboolean* is_copy) {
  if (is_copy) *is_copy = JNI_FALSE;
  g_active_utf_borrows += 1;
  return reinterpret_cast<const FakeJString*>(str)->value.c_str();
}

void JNICALL fake_release_string_utf_chars(JNIEnv*, jstring, const char*) {
  g_active_utf_borrows -= 1;
}

jstring as_jstring(FakeJString& s) {
  return reinterpret_cast<jstring>(&s);
}

} // namespace

int main(int argc, char** argv) {
  NVE_CHECK(argc == 5);
  const std::string v1_path = argv[1];
  const std::string v2_path = argv[2];
  const std::string bad_iface_path = argv[3];
  const std::string no_symbol_path = argv[4];

  JNINativeInterface_ table{};
  table.GetStringUTFChars = &fake_get_string_utf_chars;
  table.ReleaseStringUTFChars = &fake_release_string_utf_chars;
  JNIEnv_ fake_env{};
  fake_env.functions = &table;

  nve_jni_test_log_reset();

  // 1. Null host handle fails cleanly.
  FakeJString missing_str{"/nonexistent/libnve_missing.so"};
  NVE_CHECK(
      Java_dev_nve_host_NveNativeBridge_nativeHotSwapApp(
          &fake_env, nullptr, 0, as_jstring(missing_str)) == JNI_FALSE);
  NVE_CHECK(g_active_utf_borrows == 0);

  // 2. Create host succeeds and returns non-zero pointer handle.
  const jlong host = Java_dev_nve_host_NveNativeBridge_nativeCreateHost(
      &fake_env, nullptr, 77, 390.0f, 844.0f, 3.0f);
  NVE_CHECK(host != 0);

  // 3. Invalid dylib path returns JNI_FALSE and releases JNI UTF chars.
  NVE_CHECK(
      Java_dev_nve_host_NveNativeBridge_nativeHotSwapApp(
          &fake_env, nullptr, host, as_jstring(missing_str)) == JNI_FALSE);
  NVE_CHECK(g_active_utf_borrows == 0);

  // 4. Shared library missing `nve_get_interface` returns JNI_FALSE.
  FakeJString no_sym_str{no_symbol_path};
  NVE_CHECK(
      Java_dev_nve_host_NveNativeBridge_nativeHotSwapApp(
          &fake_env, nullptr, host, as_jstring(no_sym_str)) == JNI_FALSE);
  NVE_CHECK(g_active_utf_borrows == 0);

  // 5. Shared library returning NULL for NVE_IFACE_SURFACE returns JNI_FALSE.
  FakeJString bad_iface_str{bad_iface_path};
  NVE_CHECK(
      Java_dev_nve_host_NveNativeBridge_nativeHotSwapApp(
          &fake_env, nullptr, host, as_jstring(bad_iface_str)) == JNI_FALSE);
  NVE_CHECK(g_active_utf_borrows == 0);

  // 6. Load generation v1 -> creates surface 77 and renders frame 1.
  FakeJString v1_str{v1_path};
  NVE_CHECK(
      Java_dev_nve_host_NveNativeBridge_nativeHotSwapApp(
          &fake_env, nullptr, host, as_jstring(v1_str)) == JNI_TRUE);
  NVE_CHECK(g_active_utf_borrows == 0);

  const auto* log = nve_jni_test_log();
  NVE_CHECK(log->v1_create_count == 1);
  NVE_CHECK(log->v1_surface_id == 77);
  NVE_CHECK(log->v1_last_frame_id == 1);
  NVE_CHECK(log->v1_destroy_count == 0);
  NVE_CHECK(log->v1_snapshot_count == 0);

  // 7. Hot-swap v1 -> v2 snapshots v1 state, destroys v1, creates v2, restores state, renders frame 2.
  FakeJString v2_str{v2_path};
  NVE_CHECK(
      Java_dev_nve_host_NveNativeBridge_nativeHotSwapApp(
          &fake_env, nullptr, host, as_jstring(v2_str)) == JNI_TRUE);
  NVE_CHECK(g_active_utf_borrows == 0);

  NVE_CHECK(log->v1_snapshot_count == 1);
  NVE_CHECK(log->v1_destroy_count == 1);
  NVE_CHECK(log->v2_create_count == 1);
  NVE_CHECK(log->v2_surface_id == 77);
  NVE_CHECK(log->v2_restore_count == 1);
  NVE_CHECK(std::string(log->v2_restored_state, log->v2_restored_len) == "v1-counter=42");
  NVE_CHECK(log->v2_last_frame_id == 2);

  std::cout << "nve_jni_bridge_test passed\n";
  return 0;
}
