#include "nve_nitro_jni.hpp"

#include <array>
#include <atomic>
#include <deque>
#include <limits>
#include <memory>
#include <mutex>
#include <new>
#include <string>
#include <unordered_map>

namespace {

using namespace nve::nitro::jni;

constexpr size_t kFastMethodTableSize = 256u;

/// Lock-free hot-path lookup table for method IDs `0..255`, backed by a mutex-guarded string
/// arena (pointers remain valid for process lifetime) and an overflow map for larger/negative IDs.
struct MethodIdRegistry {
  std::array<std::atomic<const char*>, kFastMethodTableSize> fast_slots{};
  std::mutex mu;
  std::deque<std::string> string_arena;
  std::unordered_map<int32_t, const char*> overflow_slots;

  MethodIdRegistry() {
    for (auto& slot : fast_slots) {
      slot.store(nullptr, std::memory_order_relaxed);
    }
    register_id(kMethodEchoBytes, "echoBytes");
    register_id(kMethodPing, "ping");
    register_id(kMethodIncrement, "increment");
    register_id(kMethodTag, "tag");
    register_id(kMethodFail, "fail");
  }

  bool register_id(int32_t method_id, const char* method_name) {
    if (method_name == nullptr || method_name[0] == '\0') {
      return false;
    }
    std::lock_guard<std::mutex> lock(mu);
    string_arena.emplace_back(method_name);
    const char* stable_ptr = string_arena.back().c_str();
    if (method_id >= 0 && static_cast<size_t>(method_id) < kFastMethodTableSize) {
      fast_slots[static_cast<size_t>(method_id)].store(stable_ptr, std::memory_order_release);
    } else {
      overflow_slots[method_id] = stable_ptr;
    }
    return true;
  }

  const char* resolve(int32_t method_id) noexcept {
    if (method_id >= 0 && static_cast<size_t>(method_id) < kFastMethodTableSize) {
      return fast_slots[static_cast<size_t>(method_id)].load(std::memory_order_acquire);
    }
    std::lock_guard<std::mutex> lock(mu);
    const auto it = overflow_slots.find(method_id);
    return it != overflow_slots.end() ? it->second : nullptr;
  }
};

MethodIdRegistry& method_registry() {
  static MethodIdRegistry instance;
  return instance;
}

/// Owns raw backing storage for buffers allocated via `nativeAllocDirectBuffer` until explicitly
/// released by `nativeFreeDirectBuffer` (or process teardown), guaranteeing zero leaks.
struct DirectBufferArena {
  std::mutex mu;
  std::unordered_map<void*, std::unique_ptr<uint8_t[]>> allocations;

  void* allocate(size_t bytes) {
    auto storage = std::unique_ptr<uint8_t[]>(new (std::nothrow) uint8_t[bytes]{});
    if (!storage) {
      return nullptr;
    }
    void* raw = storage.get();
    std::lock_guard<std::mutex> lock(mu);
    allocations.emplace(raw, std::move(storage));
    return raw;
  }

  void release(void* ptr) noexcept {
    if (ptr == nullptr) {
      return;
    }
    std::lock_guard<std::mutex> lock(mu);
    allocations.erase(ptr);
  }

  size_t live_count() {
    std::lock_guard<std::mutex> lock(mu);
    return allocations.size();
  }
};

DirectBufferArena& direct_buffer_arena() {
  static DirectBufferArena instance;
  return instance;
}

} // namespace

extern "C" {

bool nve_nitro_jni_register_method_id(int32_t method_id, const char* method_name) {
  return method_registry().register_id(method_id, method_name);
}

const char* nve_nitro_jni_resolve_method_id(int32_t method_id) {
  return method_registry().resolve(method_id);
}

size_t nve_nitro_jni_live_direct_buffer_count() {
  return direct_buffer_arena().live_count();
}

JNIEXPORT jlong JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeCreateHybridObject(
    JNIEnv* env,
    jclass /*clazz*/,
    jstring name) {
  if (env == nullptr || name == nullptr) {
    return 0;
  }
  const char* utf = env->GetStringUTFChars(name, nullptr);
  if (utf == nullptr) {
    return 0;
  }
  NveNitroHandle handle = nve_nitro_create(utf);
  env->ReleaseStringUTFChars(name, utf);
  return reinterpret_cast<jlong>(handle);
}

JNIEXPORT void JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeDisposeHybridObject(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong hybrid_ptr) {
  if (hybrid_ptr == 0) {
    return;
  }
  auto handle = reinterpret_cast<NveNitroHandle>(hybrid_ptr);
  nve_nitro_dispose(handle);
  nve_nitro_release(handle);
}

JNIEXPORT jlong JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeGetMemorySize(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong hybrid_ptr) {
  if (hybrid_ptr == 0) {
    return 0;
  }
  auto handle = reinterpret_cast<NveNitroHandle>(hybrid_ptr);
  return static_cast<jlong>(nve_nitro_memory_size(handle));
}

JNIEXPORT jobject JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeAllocDirectBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jint bytes) {
  if (env == nullptr || bytes <= 0) {
    return nullptr;
  }
  const size_t size = static_cast<size_t>(bytes);
  if (size > kMaxDirectBufferBytes) {
    return nullptr;
  }
  void* ptr = direct_buffer_arena().allocate(size);
  if (ptr == nullptr) {
    return nullptr;
  }
  jobject direct_buf = env->NewDirectByteBuffer(ptr, static_cast<jlong>(size));
  if (direct_buf == nullptr) {
    direct_buffer_arena().release(ptr);
    return nullptr;
  }
  return direct_buf;
}

JNIEXPORT void JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeFreeDirectBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jobject direct_buffer) {
  if (env == nullptr || direct_buffer == nullptr) {
    return;
  }
  void* ptr = env->GetDirectBufferAddress(direct_buffer);
  direct_buffer_arena().release(ptr);
}

JNIEXPORT jint JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong hybrid_ptr,
    jint method_id,
    jobject direct_in,
    jint in_len,
    jobject direct_out) {
  if (hybrid_ptr == 0) {
    return kErrInvalidHandle;
  }
  if (in_len < 0) {
    return kErrInvalidBuffer;
  }

  const char* method_name = method_registry().resolve(static_cast<int32_t>(method_id));
  if (method_name == nullptr) {
    return kErrNotFound;
  }

  const uint8_t* in_ptr = nullptr;
  const size_t arg_len = static_cast<size_t>(in_len);
  if (arg_len > 0u) {
    if (env == nullptr || direct_in == nullptr) {
      return kErrInvalidBuffer;
    }
    void* raw_in = env->GetDirectBufferAddress(direct_in);
    const jlong in_cap = env->GetDirectBufferCapacity(direct_in);
    if (raw_in == nullptr || in_cap < static_cast<jlong>(in_len)) {
      return kErrInvalidBuffer;
    }
    in_ptr = static_cast<const uint8_t*>(raw_in);
  }

  uint8_t* out_ptr = nullptr;
  size_t out_cap = 0u;
  if (direct_out != nullptr) {
    if (env == nullptr) {
      return kErrInvalidBuffer;
    }
    void* raw_out = env->GetDirectBufferAddress(direct_out);
    const jlong out_cap_j = env->GetDirectBufferCapacity(direct_out);
    if (raw_out == nullptr || out_cap_j < 0) {
      return kErrInvalidBuffer;
    }
    out_ptr = static_cast<uint8_t*>(raw_out);
    out_cap = static_cast<size_t>(out_cap_j);
  }

  size_t out_len = 0u;
  const int32_t status = nve_nitro_invoke_bytes(
      reinterpret_cast<NveNitroHandle>(hybrid_ptr),
      method_name,
      in_ptr,
      arg_len,
      out_ptr,
      out_cap,
      &out_len);

  if (status != NVE_NITRO_OK) {
    return -static_cast<jint>(status);
  }
  if (out_len > static_cast<size_t>(std::numeric_limits<jint>::max())) {
    return kErrBufferTooSmall;
  }
  return static_cast<jint>(out_len);
}

} // extern "C"
