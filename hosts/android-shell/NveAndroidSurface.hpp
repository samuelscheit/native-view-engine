// Per-surface JNI state of `dev.nve.host.NativeRuntime` over `libnve_runtime` (ABI v2).
#pragma once

#include <jni.h>

#include <nve/generated/nve_abi.generated.h>
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
inline constexpr size_t kHeaderSceneEpoch = 8u;
inline constexpr size_t kHeaderDirtyResourceCount = 9u;
inline constexpr size_t kHeaderTables = 10u;
inline constexpr size_t kHeaderTableStride = 5u;
inline constexpr size_t kHeaderLen = kHeaderTables + kMaxTables * kHeaderTableStride;
inline constexpr size_t kSlabTables = 7u;
inline constexpr size_t kSlabTableStride = 3u;
inline constexpr size_t kSlabsLen = kSlabTables + kMaxTables * kSlabTableStride;
enum SlabSlot : size_t { kSlabRows = 0, kSlabRects, kSlabText, kSlabChildren, kSlabDirty, kSlabRemoved, kSlabDirtyResources };

#if defined(NVE_JAVA_RUNTIME_SCHEMA)
static_assert(NVE_JAVA_RUNTIME_SCHEMA == NVE_RUNTIME_SCHEMA_HASH,
              "NveRuntimeAbi.RUNTIME_SCHEMA_HASH is stale: rerun `nve codegen abi`");
static_assert(NVE_JAVA_MAX_TABLES == kMaxTables && NVE_JAVA_H_TABLES == kHeaderTables &&
                  NVE_JAVA_H_SCENE_EPOCH == kHeaderSceneEpoch &&
                  NVE_JAVA_H_DIRTY_RESOURCE_COUNT == kHeaderDirtyResourceCount &&
                  NVE_JAVA_H_TABLE_STRIDE == kHeaderTableStride && NVE_JAVA_S_TABLES == kSlabTables &&
                  NVE_JAVA_S_DIRTY_RESOURCES == kSlabDirtyResources && NVE_JAVA_S_TABLE_STRIDE == kSlabTableStride,
              "RuntimeSceneReader packed frame layout drifted from the JNI packer");
static_assert(NVE_JAVA_STATUS_BUSY == NVE_RUNTIME_BUSY && NVE_JAVA_STATUS_ERROR == NVE_RUNTIME_ERROR &&
                  NVE_JAVA_STATUS_NOT_FOUND == NVE_RUNTIME_NOT_FOUND,
              "NveRuntimeAbi.STATUS_* is stale: rerun `nve codegen abi`");
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

/// Drops every cached slab `DirectByteBuffer` (app swap, pairing change, destroy).
void release_buffers(AndroidSurface& s, JNIEnv* env) noexcept;

/// Records `status` (with the runtime's message) as the surface's last error and returns it
/// (`NVE_RUNTIME_OK` clears the last error). Every JNI entry point returns a status, never a value.
jint status_of(AndroidSurface& s, NveRuntimeStatus status);

/// Writes `value` to `out[0]` when `out` is a non-empty array.
void write_out(JNIEnv* env, jintArray out, jint value);

/// UTF-8 copy of a Java string (`""` for null; lone surrogates become U+FFFD).
std::string utf8_of(JNIEnv* env, jstring str);

/// Java string of UTF-8 bytes (malformed sequences become U+FFFD; never modified UTF-8).
jstring java_string(JNIEnv* env, const uint8_t* utf8, size_t len);

}  // namespace nve::android
