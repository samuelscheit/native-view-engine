#include <nve/generated/nve_abi.generated.h>
#include "nve_value.hpp"

#include <jni.h>

#include <cstdint>
#include <cstring>
#include <iostream>
#include <limits>
#include <string>
#include <string_view>
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
    JNIEnv* env,
    jclass clazz,
    jint surface_id,
    jfloat width,
    jfloat height,
    jfloat scale_factor);

JNIEXPORT void JNICALL Java_dev_nve_android_NveMainActivity_nativeDestroySurface(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jboolean JNICALL Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jstring so_path);

JNIEXPORT jint JNICALL Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jlongArray out_meta);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetEventScratchBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_android_NveMainActivity_nativeGetPropScratchBuffer(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr);

JNIEXPORT jboolean JNICALL Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jint byte_len);

JNIEXPORT jboolean JNICALL Java_dev_nve_android_NveMainActivity_nativeSetPropsDirect(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jint byte_len);

JNIEXPORT jboolean JNICALL Java_dev_nve_android_NveMainActivity_nativeSubmitEvents(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jbyteArray event_ring_bytes);

JNIEXPORT jboolean JNICALL Java_dev_nve_android_NveMainActivity_nativeSetPropsBinary(
    JNIEnv* env,
    jclass clazz,
    jlong handle_ptr,
    jbyteArray props_bytes);

JNIEXPORT jobject JNICALL Java_dev_nve_compose_NveJniBridge_nativeGetEventScratchBuffer(
    JNIEnv* env,
    jobject thiz,
    jlong handle_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_compose_NveJniBridge_nativeGetPropScratchBuffer(
    JNIEnv* env,
    jobject thiz,
    jlong handle_ptr);

JNIEXPORT jboolean JNICALL Java_dev_nve_compose_NveJniBridge_nativeSubmitEventsDirect(
    JNIEnv* env,
    jobject thiz,
    jlong handle_ptr,
    jint byte_len);

JNIEXPORT jboolean JNICALL Java_dev_nve_compose_NveJniBridge_nativeSetPropsDirect(
    JNIEnv* env,
    jobject thiz,
    jlong handle_ptr,
    jint byte_len);

JNIEXPORT jboolean JNICALL Java_dev_nve_compose_NveJniBridge_nativeSubmitEvents(
    JNIEnv* env,
    jobject thiz,
    jlong handle_ptr,
    jbyteArray event_ring_bytes);

JNIEXPORT jboolean JNICALL Java_dev_nve_compose_NveJniBridge_nativeSetPropsBinary(
    JNIEnv* env,
    jobject thiz,
    jlong handle_ptr,
    jbyteArray props_bytes);
}

namespace {

enum class ArrayKind : uint8_t {
  Longs = 1,
  Bytes = 2,
};

struct FakeArrayHeader {
  ArrayKind kind;
};

struct FakeLongArray {
  FakeArrayHeader hdr{ArrayKind::Longs};
  std::vector<jlong> data;
};

struct FakeByteArray {
  FakeArrayHeader hdr{ArrayKind::Bytes};
  std::vector<jbyte> data;
};

struct FakeJString {
  std::string value;
};

struct FakeDirectBuffer {
  void* address{nullptr};
  jlong capacity{0};
  int ref_count{1};
};

int g_active_utf_borrows = 0;
int g_active_byte_array_borrows = 0;
int g_total_byte_array_borrows = 0;
int g_direct_buffer_allocations = 0;
int g_exception_checks = 0;
std::unordered_set<FakeDirectBuffer*> g_live_global_refs;
std::vector<FakeDirectBuffer*> g_all_direct_buffers;

const char* JNICALL fake_get_string_utf_chars(JNIEnv*, jstring str, jboolean* is_copy) {
  if (is_copy) *is_copy = JNI_FALSE;
  g_active_utf_borrows += 1;
  return reinterpret_cast<const FakeJString*>(str)->value.c_str();
}

void JNICALL fake_release_string_utf_chars(JNIEnv*, jstring, const char*) {
  g_active_utf_borrows -= 1;
}

jsize JNICALL fake_get_array_length(JNIEnv*, jarray arr) {
  const auto* hdr = reinterpret_cast<const FakeArrayHeader*>(arr);
  if (hdr->kind == ArrayKind::Longs) {
    return static_cast<jsize>(reinterpret_cast<const FakeLongArray*>(arr)->data.size());
  }
  return static_cast<jsize>(reinterpret_cast<const FakeByteArray*>(arr)->data.size());
}

void JNICALL fake_set_long_array_region(
    JNIEnv*,
    jlongArray arr,
    jsize start,
    jsize len,
    const jlong* buf) {
  auto* target = reinterpret_cast<FakeLongArray*>(arr);
  for (jsize i = 0; i < len; ++i) {
    target->data[static_cast<size_t>(start + i)] = buf[i];
  }
}

jbyte* JNICALL fake_get_byte_array_elements(JNIEnv*, jbyteArray arr, jboolean* is_copy) {
  if (is_copy) *is_copy = JNI_FALSE;
  g_active_byte_array_borrows += 1;
  g_total_byte_array_borrows += 1;
  return reinterpret_cast<FakeByteArray*>(arr)->data.data();
}

void JNICALL fake_release_byte_array_elements(JNIEnv*, jbyteArray, jbyte*, jint mode) {
  NVE_CHECK(mode == JNI_ABORT);
  g_active_byte_array_borrows -= 1;
}

jobject JNICALL fake_new_direct_byte_buffer(JNIEnv*, void* address, jlong capacity) {
  auto* buf = new FakeDirectBuffer{address, capacity, 1};
  g_direct_buffer_allocations += 1;
  g_all_direct_buffers.push_back(buf);
  return reinterpret_cast<jobject>(buf);
}

jobject JNICALL fake_new_global_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    buf->ref_count += 1;
    g_live_global_refs.insert(buf);
  }
  return obj;
}

void JNICALL fake_delete_global_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    NVE_CHECK(g_live_global_refs.erase(buf) == 1u);
    buf->ref_count -= 1;
  }
}

jobject JNICALL fake_new_local_ref(JNIEnv*, jobject obj) {
  return obj;
}

void JNICALL fake_delete_local_ref(JNIEnv*, jobject obj) {
  auto* buf = reinterpret_cast<FakeDirectBuffer*>(obj);
  if (buf != nullptr) {
    buf->ref_count -= 1;
  }
}

jboolean JNICALL fake_exception_check(JNIEnv*) {
  g_exception_checks += 1;
  return JNI_FALSE;
}

JNIEnv_ make_fake_env(JNINativeInterface_& table) {
  table = JNINativeInterface_{};
  table.GetStringUTFChars = &fake_get_string_utf_chars;
  table.ReleaseStringUTFChars = &fake_release_string_utf_chars;
  table.GetArrayLength = &fake_get_array_length;
  table.SetLongArrayRegion = &fake_set_long_array_region;
  table.GetByteArrayElements = &fake_get_byte_array_elements;
  table.ReleaseByteArrayElements = &fake_release_byte_array_elements;
  table.NewDirectByteBuffer = &fake_new_direct_byte_buffer;
  table.NewGlobalRef = &fake_new_global_ref;
  table.DeleteGlobalRef = &fake_delete_global_ref;
  table.NewLocalRef = &fake_new_local_ref;
  table.DeleteLocalRef = &fake_delete_local_ref;
  table.ExceptionCheck = &fake_exception_check;
  JNIEnv_ env{};
  env.functions = &table;
  return env;
}

void cleanup_fake_buffers() {
  for (auto* b : g_all_direct_buffers) {
    delete b;
  }
  g_all_direct_buffers.clear();
}

template <typename T>
void push_le(std::vector<uint8_t>& out, T val) {
  const auto* p = reinterpret_cast<const uint8_t*>(&val);
  out.insert(out.end(), p, p + sizeof(T));
}

std::vector<uint8_t> encode_press_event_ring(uint32_t surface_id, uint32_t node_id, float x, float y) {
  std::vector<uint8_t> out;
  out.reserve(30u);
  push_le<uint32_t>(out, 1u); // count = 1
  push_le<uint32_t>(out, surface_id);
  push_le<uint32_t>(out, node_id);
  push_le<uint64_t>(out, 12345ULL);
  push_le<uint8_t>(out, 1u); // EventKind::Press
  push_le<uint8_t>(out, 1u); // Point payload
  push_le<float>(out, x);
  push_le<float>(out, y);
  return out;
}

std::vector<uint8_t> encode_custom_event_ring(
    uint32_t surface_id,
    uint32_t node_id,
    std::string_view name,
    std::string_view payload) {
  std::vector<uint8_t> out;
  out.reserve(28u + name.size() + payload.size());
  push_le<uint32_t>(out, 1u); // count = 1
  push_le<uint32_t>(out, surface_id);
  push_le<uint32_t>(out, node_id);
  push_le<uint64_t>(out, 99999ULL);
  push_le<uint8_t>(out, 8u); // EventKind::Custom
  push_le<uint8_t>(out, 5u); // Custom payload
  push_le<uint16_t>(out, static_cast<uint16_t>(name.size()));
  out.insert(out.end(), name.begin(), name.end());
  push_le<uint32_t>(out, static_cast<uint32_t>(payload.size()));
  out.insert(out.end(), payload.begin(), payload.end());
  return out;
}

std::string read_node2_text(JNIEnv* env, jlong handle) {
  FakeLongArray meta{};
  meta.data.assign(8u, 0);
  const jint count = Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
      env, nullptr, handle, reinterpret_cast<jlongArray>(&meta));
  NVE_CHECK(count > 0);
  auto* nodes_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(env, nullptr, handle));
  auto* text_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(env, nullptr, handle));
  const auto* nodes = reinterpret_cast<const NveRenderNode*>(nodes_buf->address);
  const auto* text = reinterpret_cast<const char*>(text_buf->address);
  for (jint i = 0; i < count; ++i) {
    if (nodes[i].node_id == 2u) {
      return std::string(text + nodes[i].text_offset, nodes[i].text_len);
    }
  }
  return {};
}

void test_direct_event_and_prop_channels(const std::string& mock_scene_so) {
  JNINativeInterface_ table{};
  JNIEnv_ env = make_fake_env(table);
  g_active_utf_borrows = 0;
  g_active_byte_array_borrows = 0;
  g_total_byte_array_borrows = 0;
  g_direct_buffer_allocations = 0;
  g_exception_checks = 0;

  // Guard checks on null handle.
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeGetEventScratchBuffer(&env, nullptr, 0) == nullptr);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeGetPropScratchBuffer(&env, nullptr, 0) == nullptr);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(&env, nullptr, 0, 30) == JNI_FALSE);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSetPropsDirect(&env, nullptr, 0, 8) == JNI_FALSE);

  const jlong handle = Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
      &env, nullptr, 1, 412.0f, 915.0f, 2.0f);
  NVE_CHECK(handle != 0);

  FakeJString so_str{mock_scene_so};
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
          &env, nullptr, handle, reinterpret_cast<jstring>(&so_str)) == JNI_TRUE);
  NVE_CHECK(g_active_utf_borrows == 0);

  // Submitting before acquiring scratch buffers fails safely (capacity == 0).
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(&env, nullptr, handle, 30) == JNI_FALSE);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSetPropsDirect(&env, nullptr, handle, 4) == JNI_FALSE);

  // Acquire reusable 64 KiB DirectByteBuffers once via MainActivity & verify NveJniBridge returns the same object.
  auto* ev_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetEventScratchBuffer(&env, nullptr, handle));
  NVE_CHECK(ev_buf != nullptr && ev_buf->address != nullptr);
  NVE_CHECK(ev_buf->capacity == 64 * 1024);

  auto* ev_buf_compose = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_compose_NveJniBridge_nativeGetEventScratchBuffer(&env, nullptr, handle));
  NVE_CHECK(ev_buf_compose == ev_buf);

  auto* prop_buf = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_android_NveMainActivity_nativeGetPropScratchBuffer(&env, nullptr, handle));
  NVE_CHECK(prop_buf != nullptr && prop_buf->address != nullptr);
  NVE_CHECK(prop_buf->capacity == 64 * 1024);
  NVE_CHECK(prop_buf->address != ev_buf->address);

  auto* prop_buf_compose = reinterpret_cast<FakeDirectBuffer*>(
      Java_dev_nve_compose_NveJniBridge_nativeGetPropScratchBuffer(&env, nullptr, handle));
  NVE_CHECK(prop_buf_compose == prop_buf);

  // Exactly 2 DirectByteBuffers created so far (event + prop).
  NVE_CHECK(g_direct_buffer_allocations == 2);
  NVE_CHECK(g_live_global_refs.size() == 2u);

  // Bounds checks on byte_len.
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(&env, nullptr, handle, -1) == JNI_FALSE);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(&env, nullptr, handle, 0) == JNI_FALSE);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(&env, nullptr, handle, 3) == JNI_FALSE);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(&env, nullptr, handle, 64 * 1024 + 1) ==
      JNI_FALSE);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(
          &env, nullptr, handle, std::numeric_limits<jint>::max()) == JNI_FALSE);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSetPropsDirect(&env, nullptr, handle, -5) == JNI_FALSE);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSetPropsDirect(&env, nullptr, handle, 0) == JNI_FALSE);
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeSetPropsDirect(&env, nullptr, handle, 64 * 1024 + 1) ==
      JNI_FALSE);

  // Hot-path stress: 1,000 direct event flushes with ZERO ByteArray borrows, ZERO DirectByteBuffer reallocs,
  // and ZERO ExceptionCheck() calls!
  const auto press_ring = encode_press_event_ring(1u, 3u, 12.5f, 34.5f);
  NVE_CHECK(press_ring.size() == 30u);
  const int allocs_before_loop = g_direct_buffer_allocations;
  const int exc_checks_before_loop = g_exception_checks;

  for (int i = 0; i < 1000; ++i) {
    std::memcpy(ev_buf->address, press_ring.data(), press_ring.size());
    const jboolean ok = (i % 2 == 0)
                            ? Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(
                                  &env, nullptr, handle, static_cast<jint>(press_ring.size()))
                            : Java_dev_nve_compose_NveJniBridge_nativeSubmitEventsDirect(
                                  &env, nullptr, handle, static_cast<jint>(press_ring.size()));
    NVE_CHECK(ok == JNI_TRUE);
  }
  NVE_CHECK(g_total_byte_array_borrows == 0);
  NVE_CHECK(g_active_byte_array_borrows == 0);
  NVE_CHECK(g_direct_buffer_allocations == allocs_before_loop);
  NVE_CHECK(g_exception_checks == exc_checks_before_loop);
  NVE_CHECK(read_node2_text(&env, handle) == "Count: 1000");

  // Fallback path: oversized (> 64 KiB) batch of 2,600 press events (67,604 bytes) via jbyteArray!
  std::vector<uint8_t> big_ring;
  constexpr uint32_t kBigEventCount = 2600u;
  big_ring.reserve(4u + kBigEventCount * 26u);
  push_le<uint32_t>(big_ring, kBigEventCount);
  for (uint32_t i = 0; i < kBigEventCount; ++i) {
    push_le<uint32_t>(big_ring, 1u);
    push_le<uint32_t>(big_ring, 3u);
    push_le<uint64_t>(big_ring, 50000ULL + i);
    push_le<uint8_t>(big_ring, 1u); // EventKind::Press
    push_le<uint8_t>(big_ring, 1u); // Point payload
    push_le<float>(big_ring, 1.0f);
    push_le<float>(big_ring, 2.0f);
  }
  NVE_CHECK(big_ring.size() == 4u + kBigEventCount * 26u);
  NVE_CHECK(big_ring.size() > 64u * 1024u);
  FakeByteArray big_arr{};
  big_arr.data.resize(big_ring.size());
  std::memcpy(big_arr.data.data(), big_ring.data(), big_ring.size());
  NVE_CHECK(
      Java_dev_nve_compose_NveJniBridge_nativeSubmitEvents(
          &env, nullptr, handle, reinterpret_cast<jbyteArray>(&big_arr)) == JNI_TRUE);
  NVE_CHECK(g_total_byte_array_borrows == 1);
  NVE_CHECK(g_active_byte_array_borrows == 0);
  NVE_CHECK(read_node2_text(&env, handle) == "Count: 3600");

  // Also verify direct custom event delivery through DirectByteBuffer.
  const auto custom_ring = encode_custom_event_ring(1u, 7u, "submit", "direct-ok");
  std::memcpy(ev_buf->address, custom_ring.data(), custom_ring.size());
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(
          &env, nullptr, handle, static_cast<jint>(custom_ring.size())) == JNI_TRUE);
  NVE_CHECK(g_total_byte_array_borrows == 1);
  NVE_CHECK(read_node2_text(&env, handle) == "Submit: direct-ok");

  // Guard checks on fallback jbyteArray methods.
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSubmitEvents(&env, nullptr, handle, nullptr) == JNI_FALSE);
  FakeByteArray tiny_arr{};
  tiny_arr.data = {1, 2, 3}; // < 4 bytes
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeSubmitEvents(
          &env, nullptr, handle, reinterpret_cast<jbyteArray>(&tiny_arr)) == JNI_FALSE);
  NVE_CHECK(Java_dev_nve_android_NveMainActivity_nativeSetPropsBinary(&env, nullptr, handle, nullptr) == JNI_FALSE);

  // Reject malformed non-NveBinValue prop payload cleanly.
  const uint8_t bad_props[5] = {0x7Au, 0x01u, 0x02u, 0x03u, 0x04u};
  std::memcpy(prop_buf->address, bad_props, sizeof(bad_props));
  NVE_CHECK(
      Java_dev_nve_compose_NveJniBridge_nativeSetPropsDirect(&env, nullptr, handle, 5) == JNI_FALSE);

  // Zero-copy direct prop submission using NveBinValue map (no ByteArray borrow!).
  std::vector<uint8_t> direct_props;
  {
    const NveMapEntry entry{nve::make_str_slice("count"), nve::make_i64_value(42)};
    NVE_CHECK(nve::encode_bin_value(direct_props, nve::make_map_value(&entry, 1u)));
  }
  std::memcpy(prop_buf->address, direct_props.data(), direct_props.size());
  const int borrows_before_direct_props = g_total_byte_array_borrows;
  NVE_CHECK(
      Java_dev_nve_compose_NveJniBridge_nativeSetPropsDirect(
          &env, nullptr, handle, static_cast<jint>(direct_props.size())) == JNI_TRUE);
  NVE_CHECK(g_total_byte_array_borrows == borrows_before_direct_props);
  NVE_CHECK(read_node2_text(&env, handle) == "Props: count=1 k0=count i64=42");

  // Oversized (> 64 KiB) NveBinValue map prop fallback via jbyteArray.
  std::vector<uint8_t> big_blob(68u * 1024u, 0x55u);
  std::vector<uint8_t> big_props_bin;
  {
    const NveMapEntry entry{
        nve::make_str_slice("payload"),
        nve::make_bytes_value(NveBufferSlice{big_blob.data(), big_blob.size()}),
    };
    NVE_CHECK(nve::encode_bin_value(big_props_bin, nve::make_map_value(&entry, 1u)));
  }
  NVE_CHECK(big_props_bin.size() > 64u * 1024u);
  FakeByteArray big_props{};
  big_props.data.resize(big_props_bin.size());
  std::memcpy(big_props.data.data(), big_props_bin.data(), big_props_bin.size());
  NVE_CHECK(
      Java_dev_nve_android_NveMainActivity_nativeSetPropsBinary(
          &env, nullptr, handle, reinterpret_cast<jbyteArray>(&big_props)) == JNI_TRUE);
  NVE_CHECK(g_total_byte_array_borrows == borrows_before_direct_props + 1);
  NVE_CHECK(g_active_byte_array_borrows == 0);
  NVE_CHECK(read_node2_text(&env, handle) == "Props: count=1 k0=payload blen=69632");

  // Destroying surface releases BOTH scratch GlobalRefs + all 7 scene slab GlobalRefs cleanly!
  Java_dev_nve_android_NveMainActivity_nativeDestroySurface(&env, nullptr, handle);
  NVE_CHECK(g_live_global_refs.empty());
  NVE_CHECK(g_active_utf_borrows == 0);
  NVE_CHECK(g_active_byte_array_borrows == 0);
  cleanup_fake_buffers();
}

} // namespace

int main(int argc, char** argv) {
  NVE_CHECK(argc == 2);
  test_direct_event_and_prop_channels(argv[1]);
  std::cout << "nve_android_direct_channel_test passed\n";
  return 0;
}
