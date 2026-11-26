#pragma once

#include "nve_scene_abi.generated.h"

#include <jni.h>

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

extern "C" {
JNIEXPORT jlong JNICALL Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
    JNIEnv* env, jclass clazz, jint surface_id, jfloat width, jfloat height, jfloat scale_factor);
JNIEXPORT void JNICALL Java_dev_nve_android_NveMainActivity_nativeDestroySurface(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jboolean JNICALL Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
    JNIEnv* env, jclass clazz, jlong handle_ptr, jstring so_path);
JNIEXPORT jstring JNICALL Java_dev_nve_android_NveMainActivity_nativeGetLastError(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeDispatchTextChange(
    JNIEnv* env, jclass clazz, jlong handle_ptr, jint node_id, jstring text);
JNIEXPORT jboolean JNICALL Java_dev_nve_android_NvePairController_nativeConnectPaired(
    JNIEnv* env, jclass clazz, jlong handle_ptr, jstring endpoint, jstring token,
    jstring transport, jobject wake_target);
JNIEXPORT jint JNICALL Java_dev_nve_android_NvePairController_nativePollPairedFrames(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT void JNICALL Java_dev_nve_android_NvePairController_nativeDisconnectPaired(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jboolean JNICALL Java_dev_nve_android_NvePairController_nativeIsPaired(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeDispatchPress(
    JNIEnv* env, jclass clazz, jlong handle_ptr, jint node_id, jfloat x, jfloat y);
JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeDispatchCustomEvent(
    JNIEnv* env, jclass clazz, jlong handle_ptr, jint node_id, jstring name, jstring payload);
JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeDispatchScroll(
    JNIEnv* env, jclass clazz, jlong handle_ptr, jint node_id, jfloat scroll_x, jfloat scroll_y);
JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
    JNIEnv* env, jclass clazz, jlong handle_ptr, jlongArray out_meta);
JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeGetImageResizeMode(
    JNIEnv* env, jclass clazz, jlong handle_ptr, jint node_id);
JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetChildrenSlabBuffer(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetRemovedIdsBuffer(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetMetaRecordsBuffer(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetMetaBlobBuffer(
    JNIEnv* env, jclass clazz, jlong handle_ptr);
}

namespace nve::android_test {

struct FakeJString {
  std::string value;
};

struct FakeLongArray {
  std::vector<jlong> data;
};

struct FakeDirectBuffer {
  void* address{nullptr};
  jlong capacity{0};
  int ref_count{1};
};

struct ParsedMetaRecord {
  uint32_t node_id{0};
  uint32_t flags{0};
  std::string label;
  std::string role;
  std::string hint;
  std::string state_desc;
  std::string placeholder;
  std::string test_id;
  std::vector<uint8_t> draw_list;
};

inline uint32_t read_u32_le(const uint8_t* p) {
  return static_cast<uint32_t>(p[0]) |
         (static_cast<uint32_t>(p[1]) << 8) |
         (static_cast<uint32_t>(p[2]) << 16) |
         (static_cast<uint32_t>(p[3]) << 24);
}

inline std::unordered_map<uint32_t, ParsedMetaRecord> parse_meta_slabs(
    const FakeDirectBuffer* records_buf,
    const FakeDirectBuffer* blob_buf,
    uint32_t record_count,
    uint32_t blob_used_bytes) {
  NVE_CHECK(records_buf != nullptr && records_buf->address != nullptr);
  NVE_CHECK(blob_buf != nullptr && blob_buf->address != nullptr);
  NVE_CHECK(static_cast<jlong>(record_count * sizeof(NveNodeMeta)) <= records_buf->capacity);
  NVE_CHECK(static_cast<jlong>(blob_used_bytes) <= blob_buf->capacity);

  const auto* recs = reinterpret_cast<const NveNodeMeta*>(records_buf->address);
  const auto* blob = reinterpret_cast<const uint8_t*>(blob_buf->address);
  std::unordered_map<uint32_t, ParsedMetaRecord> out;

  auto read_str = [&](uint32_t offset, uint32_t len) -> std::string {
    if (len == 0u) return {};
    NVE_CHECK(static_cast<uint64_t>(offset) + static_cast<uint64_t>(len) <= blob_used_bytes);
    return std::string(reinterpret_cast<const char*>(blob + offset), len);
  };

  for (uint32_t i = 0; i < record_count; ++i) {
    const NveNodeMeta& r = recs[i];
    ParsedMetaRecord rec{};
    rec.node_id = r.node_id;
    rec.flags = r.flags;
    rec.label = read_str(r.label_offset, r.label_len);
    rec.role = read_str(r.role_offset, r.role_len);
    rec.hint = read_str(r.hint_offset, r.hint_len);
    rec.state_desc = read_str(r.state_desc_offset, r.state_desc_len);
    rec.placeholder = read_str(r.placeholder_offset, r.placeholder_len);
    rec.test_id = read_str(r.test_id_offset, r.test_id_len);
    if (r.draw_list_len > 0u) {
      NVE_CHECK(
          static_cast<uint64_t>(r.draw_list_offset) + static_cast<uint64_t>(r.draw_list_len) <=
          blob_used_bytes);
      rec.draw_list.assign(blob + r.draw_list_offset, blob + r.draw_list_offset + r.draw_list_len);
    }
    out[rec.node_id] = std::move(rec);
  }
  NVE_CHECK(out.size() == record_count);
  return out;
}

inline int g_active_utf_borrows = 0;
inline int g_direct_buffer_allocations = 0;
inline std::unordered_set<FakeDirectBuffer*> g_live_global_refs;
inline std::vector<FakeDirectBuffer*> g_all_direct_buffers;

inline const char* JNICALL fake_get_string_utf_chars(JNIEnv*, jstring str, jboolean* is_copy) {
  if (is_copy) *is_copy = JNI_FALSE;
  g_active_utf_borrows += 1;
  return reinterpret_cast<const FakeJString*>(str)->value.c_str();
}

inline void JNICALL fake_release_string_utf_chars(JNIEnv*, jstring, const char*) {
  g_active_utf_borrows -= 1;
}

inline jsize JNICALL fake_get_array_length(JNIEnv*, jarray arr) {
  return static_cast<jsize>(reinterpret_cast<const FakeLongArray*>(arr)->data.size());
}

inline void JNICALL fake_set_long_array_region(
    JNIEnv*, jlongArray arr, jsize start, jsize len, const jlong* buf) {
  auto* target = reinterpret_cast<FakeLongArray*>(arr);
  for (jsize i = 0; i < len; ++i) {
    target->data[static_cast<size_t>(start + i)] = buf[i];
  }
}

inline jobject JNICALL fake_new_direct_byte_buffer(JNIEnv*, void* address, jlong capacity) {
  auto* buf = new FakeDirectBuffer{address, capacity, 1};
  g_direct_buffer_allocations += 1;
  g_all_direct_buffers.push_back(buf);
  return reinterpret_cast<jobject>(buf);
}

inline jobject JNICALL fake_new_global_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    buf->ref_count += 1;
    g_live_global_refs.insert(buf);
  }
  return obj;
}

inline void JNICALL fake_delete_global_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    NVE_CHECK(g_live_global_refs.erase(buf) == 1u);
    buf->ref_count -= 1;
  }
}

inline jobject JNICALL fake_new_local_ref(JNIEnv*, jobject obj) {
  return obj;
}

inline void JNICALL fake_delete_local_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    buf->ref_count -= 1;
  }
}

inline void* JNICALL fake_get_direct_buffer_address(JNIEnv*, jobject obj) {
  return reinterpret_cast<FakeDirectBuffer*>(obj)->address;
}

inline jlong JNICALL fake_get_direct_buffer_capacity(JNIEnv*, jobject obj) {
  return reinterpret_cast<FakeDirectBuffer*>(obj)->capacity;
}

inline JNIEnv_ make_fake_env(JNINativeInterface_& table) {
  table = JNINativeInterface_{};
  table.GetStringUTFChars = &fake_get_string_utf_chars;
  table.ReleaseStringUTFChars = &fake_release_string_utf_chars;
  table.GetArrayLength = &fake_get_array_length;
  table.SetLongArrayRegion = &fake_set_long_array_region;
  table.NewDirectByteBuffer = &fake_new_direct_byte_buffer;
  table.NewGlobalRef = &fake_new_global_ref;
  table.DeleteGlobalRef = &fake_delete_global_ref;
  table.NewLocalRef = &fake_new_local_ref;
  table.DeleteLocalRef = &fake_delete_local_ref;
  table.GetDirectBufferAddress = &fake_get_direct_buffer_address;
  table.GetDirectBufferCapacity = &fake_get_direct_buffer_capacity;
  JNIEnv_ env{};
  env.functions = &table;
  return env;
}

inline void cleanup_fake_buffers() {
  for (auto* b : g_all_direct_buffers) {
    delete b;
  }
  g_all_direct_buffers.clear();
}

} // namespace nve::android_test
