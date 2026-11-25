#include "nve_nitro_jni.hpp"

#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::fprintf(stderr, "NVE_CHECK failed at %s:%d: %s\n", __FILE__, __LINE__, #expr);       \
      std::abort();                                                                             \
    }                                                                                           \
  } while (false)

namespace {

using namespace nve::nitro;
using namespace nve::nitro::jni;

struct FakeJString {
  std::string value;
};

struct FakeDirectBuffer {
  void* address{nullptr};
  jlong capacity{0};
};

int g_active_utf_borrows = 0;
int g_exception_checks = 0;
std::vector<std::unique_ptr<FakeDirectBuffer>> g_fake_buffers;

const char* JNICALL fake_get_string_utf_chars(JNIEnv*, jstring str, jboolean* is_copy) {
  if (is_copy) *is_copy = JNI_FALSE;
  g_active_utf_borrows += 1;
  return reinterpret_cast<const FakeJString*>(str)->value.c_str();
}

void JNICALL fake_release_string_utf_chars(JNIEnv*, jstring, const char*) {
  g_active_utf_borrows -= 1;
}

jobject JNICALL fake_new_direct_byte_buffer(JNIEnv*, void* address, jlong capacity) {
  auto box = std::make_unique<FakeDirectBuffer>(FakeDirectBuffer{address, capacity});
  auto* raw = box.get();
  g_fake_buffers.push_back(std::move(box));
  return reinterpret_cast<jobject>(raw);
}

void* JNICALL fake_get_direct_buffer_address(JNIEnv*, jobject obj) {
  return reinterpret_cast<FakeDirectBuffer*>(obj)->address;
}

jlong JNICALL fake_get_direct_buffer_capacity(JNIEnv*, jobject obj) {
  return reinterpret_cast<FakeDirectBuffer*>(obj)->capacity;
}

jboolean JNICALL fake_exception_check(JNIEnv*) {
  g_exception_checks += 1;
  return JNI_FALSE;
}

JNIEnv_ make_fake_env(JNINativeInterface_& table) {
  table = JNINativeInterface_{};
  table.GetStringUTFChars = &fake_get_string_utf_chars;
  table.ReleaseStringUTFChars = &fake_release_string_utf_chars;
  table.NewDirectByteBuffer = &fake_new_direct_byte_buffer;
  table.GetDirectBufferAddress = &fake_get_direct_buffer_address;
  table.GetDirectBufferCapacity = &fake_get_direct_buffer_capacity;
  table.ExceptionCheck = &fake_exception_check;
  JNIEnv_ env{};
  env.functions = &table;
  return env;
}

class BenchHybrid final : public HybridObject {
 public:
  explicit BenchHybrid(int64_t initial = 0) : HybridObject("BenchHybrid"), value_(initial) {}

  size_t memory_size() const noexcept override { return sizeof(*this) + scratch_.size(); }

  void dispose() override {
    if (!is_disposed()) {
      scratch_.clear();
      HybridObject::dispose();
    }
  }

  NitroVariant invoke_method(std::string_view method, std::span<const NitroVariant> args) override {
    if (is_disposed()) return NitroVariant::from_error("DISPOSED");
    if (method == "echoBytes") {
      if (args.empty()) return NitroVariant::from_bytes(ArrayBuffer::allocate(0u));
      if (auto ab = args[0].as_array_buffer()) {
        return NitroVariant::from_bytes(ArrayBuffer::copy(ab->span()));
      }
      return NitroVariant::from_error("EXPECTED_BYTES");
    }
    if (method == "ping") {
      return NitroVariant::from_string("pong");
    }
    if (method == "increment") {
      int64_t delta = 1;
      if (!args.empty()) {
        if (auto ab = args[0].as_array_buffer(); ab && ab->size() == 1u) {
          delta = static_cast<int64_t>(ab->data()[0]);
        }
      }
      value_ += delta;
      return NitroVariant::from_i64(value_);
    }
    if (method == "xorBytes") {
      if (args.empty()) return NitroVariant::from_bytes(ArrayBuffer::allocate(0u));
      auto ab = args[0].as_array_buffer();
      if (!ab) return NitroVariant::from_error("EXPECTED_BYTES");
      std::vector<uint8_t> out(ab->size());
      for (size_t i = 0; i < ab->size(); ++i) {
        out[i] = static_cast<uint8_t>(ab->data()[i] ^ 0xA5u);
      }
      return NitroVariant::from_bytes(ArrayBuffer::from_vector(std::move(out)));
    }
    if (method == "tag") {
      return NitroVariant::from_string("bench:" + std::to_string(value_));
    }
    if (method == "fail") {
      return NitroVariant::from_error("EXPLICIT_FAILURE");
    }
    return HybridObject::invoke_method(method, args);
  }

 private:
  int64_t value_{0};
  std::vector<uint8_t> scratch_{1u, 2u, 3u, 4u, 5u, 6u, 7u, 8u};
};

void test_lifecycle_direct_buffers_and_binary_invocation() {
  JNINativeInterface_ table{};
  JNIEnv_ env = make_fake_env(table);
  g_active_utf_borrows = 0;
  g_exception_checks = 0;
  g_fake_buffers.clear();

  std::weak_ptr<BenchHybrid> tracked;
  auto& reg = HybridObjectRegistry::global();
  reg.register_hybrid_object("BenchHybrid", [&tracked]() {
    auto inst = std::make_shared<BenchHybrid>(10);
    tracked = inst;
    return inst;
  });

  // Invalid creation edge cases.
  NVE_CHECK(Java_dev_nve_nitro_NveNitroRuntime_nativeCreateHybridObject(&env, nullptr, nullptr) == 0);
  NVE_CHECK(Java_dev_nve_nitro_NveNitroRuntime_nativeCreateHybridObject(nullptr, nullptr, nullptr) == 0);
  FakeJString missing_name{"DoesNotExist"};
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeCreateHybridObject(
          &env, nullptr, reinterpret_cast<jstring>(&missing_name)) == 0);
  NVE_CHECK(g_active_utf_borrows == 0);

  // Create HybridObject -> raw jlong pointer returned directly (zero fbjni).
  FakeJString valid_name{"BenchHybrid"};
  const jlong ptr = Java_dev_nve_nitro_NveNitroRuntime_nativeCreateHybridObject(
      &env, nullptr, reinterpret_cast<jstring>(&valid_name));
  NVE_CHECK(ptr != 0);
  NVE_CHECK(g_active_utf_borrows == 0);
  NVE_CHECK(!tracked.expired());

  // Memory size query.
  NVE_CHECK(Java_dev_nve_nitro_NveNitroRuntime_nativeGetMemorySize(&env, nullptr, 0) == 0);
  const jlong mem_size = Java_dev_nve_nitro_NveNitroRuntime_nativeGetMemorySize(&env, nullptr, ptr);
  NVE_CHECK(mem_size >= static_cast<jlong>(sizeof(BenchHybrid) + 8u));

  // Allocate reusable DirectByteBuffers + reject invalid sizes.
  NVE_CHECK(Java_dev_nve_nitro_NveNitroRuntime_nativeAllocDirectBuffer(&env, nullptr, 0) == nullptr);
  NVE_CHECK(Java_dev_nve_nitro_NveNitroRuntime_nativeAllocDirectBuffer(&env, nullptr, -16) == nullptr);
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeAllocDirectBuffer(
          &env, nullptr, static_cast<jint>(kMaxDirectBufferBytes + 1u)) == nullptr);
  NVE_CHECK(nve_nitro_jni_live_direct_buffer_count() == 0u);

  jobject in_buf_obj = Java_dev_nve_nitro_NveNitroRuntime_nativeAllocDirectBuffer(&env, nullptr, 256);
  jobject out_buf_obj = Java_dev_nve_nitro_NveNitroRuntime_nativeAllocDirectBuffer(&env, nullptr, 256);
  NVE_CHECK(in_buf_obj != nullptr && out_buf_obj != nullptr);
  NVE_CHECK(nve_nitro_jni_live_direct_buffer_count() == 2u);

  auto* in_buf = reinterpret_cast<FakeDirectBuffer*>(in_buf_obj);
  auto* out_buf = reinterpret_cast<FakeDirectBuffer*>(out_buf_obj);
  NVE_CHECK(in_buf->address != nullptr && in_buf->capacity == 256);
  NVE_CHECK(out_buf->address != nullptr && out_buf->capacity == 256);

  // 1,000 zero-copy DirectByteBuffer roundtrips with ZERO ExceptionCheck() calls!
  auto* in_bytes = static_cast<uint8_t*>(in_buf->address);
  auto* out_bytes = static_cast<uint8_t*>(out_buf->address);
  for (int i = 0; i < 1000; ++i) {
    in_bytes[0] = static_cast<uint8_t>(i & 0xFF);
    in_bytes[1] = static_cast<uint8_t>((i >> 8) & 0xFF);
    in_bytes[2] = 0xDEu;
    in_bytes[3] = 0xADu;
    const jint written = Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
        &env, nullptr, ptr, kMethodEchoBytes, in_buf_obj, 4, out_buf_obj);
    NVE_CHECK(written == 4);
    NVE_CHECK(std::memcmp(in_bytes, out_bytes, 4u) == 0);
  }
  NVE_CHECK(g_exception_checks == 0);

  // Zero-input call (directIn == nullptr, inLen == 0) -> "pong".
  const jint pong_len = Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
      &env, nullptr, ptr, kMethodPing, nullptr, 0, out_buf_obj);
  NVE_CHECK(pong_len == 4);
  NVE_CHECK(std::memcmp(out_bytes, "pong", 4u) == 0);

  // Increment by 7 -> 17 (8-byte little-endian i64).
  in_bytes[0] = 7u;
  const jint inc_len = Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
      &env, nullptr, ptr, kMethodIncrement, in_buf_obj, 1, out_buf_obj);
  NVE_CHECK(inc_len == 8);
  int64_t decoded_val = 0;
  std::memcpy(&decoded_val, out_bytes, sizeof(decoded_val));
  NVE_CHECK(decoded_val == 17);

  // Custom fast-slot method registration (ID 42 -> "xorBytes") and overflow-slot registration (ID 1024 -> "tag").
  NVE_CHECK(!nve_nitro_jni_register_method_id(42, nullptr));
  NVE_CHECK(!nve_nitro_jni_register_method_id(42, ""));
  NVE_CHECK(nve_nitro_jni_register_method_id(42, "xorBytes"));
  NVE_CHECK(nve_nitro_jni_register_method_id(1024, "tag"));
  in_bytes[0] = 0x10u;
  in_bytes[1] = 0x20u;
  in_bytes[2] = 0x30u;
  const jint xor_len = Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
      &env, nullptr, ptr, 42, in_buf_obj, 3, out_buf_obj);
  NVE_CHECK(xor_len == 3);
  NVE_CHECK(out_bytes[0] == (0x10u ^ 0xA5u) && out_bytes[1] == (0x20u ^ 0xA5u) && out_bytes[2] == (0x30u ^ 0xA5u));

  const jint tag_len = Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
      &env, nullptr, ptr, 1024, nullptr, 0, out_buf_obj);
  NVE_CHECK(tag_len == 8); // "bench:17"
  NVE_CHECK(std::memcmp(out_bytes, "bench:17", 8u) == 0);

  // Error handling & buffer safety guards.
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(&env, nullptr, 0, kMethodPing, nullptr, 0, out_buf_obj) ==
      kErrInvalidHandle);
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(&env, nullptr, ptr, 9999, nullptr, 0, out_buf_obj) ==
      kErrNotFound);
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
          &env, nullptr, ptr, kMethodFail, nullptr, 0, out_buf_obj) == kErrInvocationFailed);
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(&env, nullptr, ptr, kMethodEchoBytes, in_buf_obj, -1, out_buf_obj) ==
      kErrInvalidBuffer);
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(&env, nullptr, ptr, kMethodEchoBytes, nullptr, 4, out_buf_obj) ==
      kErrInvalidBuffer);
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(&env, nullptr, ptr, kMethodEchoBytes, in_buf_obj, 257, out_buf_obj) ==
      kErrInvalidBuffer);

  // Non-direct ByteBuffer (GetDirectBufferAddress == nullptr) safely rejected.
  FakeDirectBuffer non_direct{nullptr, 16};
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
          &env, nullptr, ptr, kMethodEchoBytes, reinterpret_cast<jobject>(&non_direct), 4, out_buf_obj) ==
      kErrInvalidBuffer);
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
          &env, nullptr, ptr, kMethodPing, nullptr, 0, reinterpret_cast<jobject>(&non_direct)) ==
      kErrInvalidBuffer);

  // Output buffer too small -> kErrBufferTooSmall (-4).
  jobject tiny_out_obj = Java_dev_nve_nitro_NveNitroRuntime_nativeAllocDirectBuffer(&env, nullptr, 2);
  NVE_CHECK(tiny_out_obj != nullptr);
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
          &env, nullptr, ptr, kMethodPing, nullptr, 0, tiny_out_obj) == kErrBufferTooSmall);

  // Disposing the underlying HybridObject causes ERR_DISPOSED (-3) on subsequent calls.
  if (auto locked = tracked.lock()) {
    locked->dispose();
  }
  NVE_CHECK(
      Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
          &env, nullptr, ptr, kMethodPing, nullptr, 0, out_buf_obj) == kErrDisposed);

  // Releasing via nativeDisposeHybridObject destroys the HybridObject completely (zero leak).
  Java_dev_nve_nitro_NveNitroRuntime_nativeDisposeHybridObject(&env, nullptr, 0);
  Java_dev_nve_nitro_NveNitroRuntime_nativeDisposeHybridObject(&env, nullptr, ptr);
  NVE_CHECK(tracked.expired());

  // Freeing all direct buffers (plus idempotent double-free & null-free) leaves zero leaks.
  Java_dev_nve_nitro_NveNitroRuntime_nativeFreeDirectBuffer(&env, nullptr, nullptr);
  Java_dev_nve_nitro_NveNitroRuntime_nativeFreeDirectBuffer(&env, nullptr, in_buf_obj);
  Java_dev_nve_nitro_NveNitroRuntime_nativeFreeDirectBuffer(&env, nullptr, in_buf_obj);
  Java_dev_nve_nitro_NveNitroRuntime_nativeFreeDirectBuffer(&env, nullptr, out_buf_obj);
  Java_dev_nve_nitro_NveNitroRuntime_nativeFreeDirectBuffer(&env, nullptr, tiny_out_obj);
  NVE_CHECK(nve_nitro_jni_live_direct_buffer_count() == 0u);
  NVE_CHECK(g_exception_checks == 0);

  reg.unregister_hybrid_object("BenchHybrid");
  g_fake_buffers.clear();
}

} // namespace

int main() {
  test_lifecycle_direct_buffers_and_binary_invocation();
  std::puts("nve_nitro_jni_test: ALL PASSED");
  return 0;
}
