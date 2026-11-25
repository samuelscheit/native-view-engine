#pragma once

#include "NveAndroidSurfaceInternal.hpp"

#include <jni.h>

#include <cstddef>
#include <cstdint>
#include <vector>

namespace nve::android_internal {

/// Default size of each per-surface reusable native scratch `DirectByteBuffer` (64 KiB).
/// Covers >99.9% of 60–120 Hz touch/scroll/key event rings and binary prop updates with zero
/// per-frame JVM `ByteArray` allocations or JNI `GetByteArrayElements` copies.
inline constexpr size_t kDefaultDirectScratchBytes = 64u * 1024u;

/// Derived surface session holding the reusable `DirectByteBuffer` scratch channels alongside
/// `AndroidSurfaceState`. `AndroidSurfaceState` is the sole base subobject at offset 0, so
/// `jlong` handles remain valid `AndroidSurfaceState*` pointers with zero extra indirection.
struct AndroidSurfaceSession final : public AndroidSurfaceState {
  std::vector<uint8_t> event_scratch;
  CachedDirectBuffer event_scratch_buf;
  std::vector<uint8_t> prop_scratch;
  CachedDirectBuffer prop_scratch_buf;
};

inline AndroidSurfaceSession* as_surface_session(jlong handle_ptr) noexcept {
  return static_cast<AndroidSurfaceSession*>(reinterpret_cast<AndroidSurfaceState*>(handle_ptr));
}

/// Releases cached `GlobalRef` direct buffers for the event & prop scratch channels and frees
/// their backing storage. Safe to call with `env == nullptr` or `session == nullptr`.
void release_direct_scratch_buffers(AndroidSurfaceSession* session, JNIEnv* env) noexcept;

/// Lazily allocates and returns the session's reusable 64 KiB native `DirectByteBuffer` for
/// binary `EventRing` submission.
jobject get_event_scratch_buffer(JNIEnv* env, jlong handle_ptr) noexcept;

/// Lazily allocates and returns the session's reusable 64 KiB native `DirectByteBuffer` for
/// binary `NveBinValue` prop updates.
jobject get_prop_scratch_buffer(JNIEnv* env, jlong handle_ptr) noexcept;

/// Zero-copy `EventRing` submission reading `[0, byte_len)` directly from `event_scratch`.
jboolean submit_events_direct(jlong handle_ptr, jint byte_len) noexcept;

/// Fallback `EventRing` submission accepting a JVM `ByteArray` (e.g. oversized batches > 64 KiB).
jboolean submit_events_array(JNIEnv* env, jlong handle_ptr, jbyteArray event_ring_bytes) noexcept;

/// Zero-copy binary props submission reading `[0, byte_len)` directly from `prop_scratch`.
jboolean set_props_direct(jlong handle_ptr, jint byte_len) noexcept;

/// Fallback binary props submission accepting a JVM `ByteArray` (e.g. oversized props > 64 KiB).
jboolean set_props_array(JNIEnv* env, jlong handle_ptr, jbyteArray props_bytes) noexcept;

} // namespace nve::android_internal
