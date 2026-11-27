// Per-surface JNI state of `dev.nve.host.NativeRuntime` over `libnve_runtime` (ABI v2).
#pragma once

#include <jni.h>

#include <nve/host/nve_abi.generated.h>
#include <nve/host/runtime_session.hpp>

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace nve::android {

// Packed scene frame shared with `dev.nve.host.RuntimeSceneReader` (`H_*` / `S_*`). CMake reads
// the Java constants and `static_assert`s them below (`NVE_JAVA_*`), so the two never drift.
inline constexpr size_t kMaxTables = 128u;
inline constexpr size_t kHeaderTables = 8u;
inline constexpr size_t kHeaderTableStride = 5u;
inline constexpr size_t kHeaderLen = kHeaderTables + kMaxTables * kHeaderTableStride;
inline constexpr size_t kSlabTables = 6u;
inline constexpr size_t kSlabTableStride = 3u;
inline constexpr size_t kSlabsLen = kSlabTables + kMaxTables * kSlabTableStride;
enum SlabSlot : size_t { kSlabRows = 0, kSlabRects, kSlabText, kSlabChildren, kSlabDirty, kSlabRemoved };

#if defined(NVE_JAVA_RUNTIME_SCHEMA)
static_assert(NVE_JAVA_RUNTIME_SCHEMA == NVE_RUNTIME_SCHEMA_HASH,
              "RuntimeSceneReader.SCHEMA drifted from NVE_RUNTIME_SCHEMA_HASH");
static_assert(NVE_JAVA_MAX_TABLES == kMaxTables && NVE_JAVA_H_TABLES == kHeaderTables &&
                  NVE_JAVA_H_TABLE_STRIDE == kHeaderTableStride && NVE_JAVA_S_TABLES == kSlabTables &&
                  NVE_JAVA_S_TABLE_STRIDE == kSlabTableStride,
              "RuntimeSceneReader packed frame layout drifted from the JNI packer");
static_assert(NVE_JAVA_STATUS_BUSY == NVE_RUNTIME_BUSY && NVE_JAVA_STATUS_ERROR == NVE_RUNTIME_ERROR,
              "NativeRuntime.STATUS_* drifted from NveRuntimeStatus");
#endif

/// `GlobalRef` `DirectByteBuffer` over native memory, recreated only when `(ptr, len)` changes.
class CachedDirectBuffer {
 public:
  /// Global ref for `[ptr, ptr + len)`; `nullptr` for an empty range or on JNI failure.
  jobject sync(JNIEnv* env, const void* ptr, size_t len);
  void release(JNIEnv* env) noexcept;

 private:
  const void* ptr_ = nullptr;
  size_t len_ = 0u;
  jobject ref_ = nullptr;
};

/// A Java callback target held as a `GlobalRef` with its resolved method.
struct JavaCallback {
  jobject target = nullptr;
  jmethodID method = nullptr;
  void release(JNIEnv* env) noexcept;
};

struct AndroidSurface {
  nve::host::RuntimeSession session;
  std::optional<nve::host::RuntimeSceneGuard> scene;
  JavaVM* jvm = nullptr;
  uint32_t surface_id = 1u;
  void* app_library = nullptr;
  std::array<CachedDirectBuffer, kSlabsLen> slabs{};
  std::vector<CachedDirectBuffer> resources;
  std::vector<NveDirectEvent> events;
  JavaCallback measurer;
  jfloatArray measure_out = nullptr;
  JavaCallback wake;
  std::string last_error;
};

[[nodiscard]] inline AndroidSurface* surface_of(jlong handle) noexcept {
  return reinterpret_cast<AndroidSurface*>(handle);
}

/// Releases the held scene (every mutating call does this first so the runtime is never `Busy`).
void release_scene(AndroidSurface& s) noexcept;

/// Drops every cached slab / resource `DirectByteBuffer` (app swap, pairing change, destroy).
void release_buffers(AndroidSurface& s, JNIEnv* env) noexcept;

/// Records `status` (with the runtime's message) as the surface's last error and returns it.
jint fail(AndroidSurface& s, NveRuntimeStatus status);

/// `1` changed / `0` unchanged on success, `-status` on failure (recorded).
jint changed_or_fail(AndroidSurface& s, NveRuntimeStatus status, bool changed);

/// UTF-8 copy of a Java string (`""` for null).
std::string utf8_of(JNIEnv* env, jstring str);

}  // namespace nve::android
