#pragma once

#include "nve_nitro_core.hpp"

#include <jni.h>

#include <cstddef>
#include <cstdint>

namespace nve::nitro::jni {

/// Negative status codes returned directly from `nativeInvokeBinary` (non-negative values are
/// exact output byte counts written into `directOut`, avoiding any wrapper object allocation).
inline constexpr jint kErrInvalidHandle = -static_cast<jint>(NVE_NITRO_ERR_INVALID_HANDLE);       // -1
inline constexpr jint kErrNotFound = -static_cast<jint>(NVE_NITRO_ERR_NOT_FOUND);                 // -2
inline constexpr jint kErrDisposed = -static_cast<jint>(NVE_NITRO_ERR_DISPOSED);                 // -3
inline constexpr jint kErrBufferTooSmall = -static_cast<jint>(NVE_NITRO_ERR_BUFFER_TOO_SMALL);   // -4
inline constexpr jint kErrInvocationFailed = -static_cast<jint>(NVE_NITRO_ERR_INVOCATION_FAILED); // -5
inline constexpr jint kErrInvalidBuffer = -6;

/// Upper bound on single `nativeAllocDirectBuffer` allocation (64 MiB).
inline constexpr size_t kMaxDirectBufferBytes = 64u * 1024u * 1024u;

/// Built-in method IDs pre-registered for zero-allocation integer dispatch.
inline constexpr jint kMethodEchoBytes = 1;
inline constexpr jint kMethodPing = 2;
inline constexpr jint kMethodIncrement = 3;
inline constexpr jint kMethodTag = 4;
inline constexpr jint kMethodFail = 5;

} // namespace nve::nitro::jni

extern "C" {

/// Registers a `method_id -> method_name` mapping for O(1) lock-free dispatch in `nativeInvokeBinary`.
/// Returns `true` on success, `false` if `method_name` is null or empty.
bool nve_nitro_jni_register_method_id(int32_t method_id, const char* method_name);

/// Resolves a registered `method_id` to its null-terminated name, or `nullptr` if unregistered.
const char* nve_nitro_jni_resolve_method_id(int32_t method_id);

/// Returns the number of live native buffers allocated via `nativeAllocDirectBuffer`.
size_t nve_nitro_jni_live_direct_buffer_count();

// =============================================================================
// Pure <jni.h> Zero-fbjni Exports for `dev.nve.nitro.NveNitroRuntime`
// Raw `jlong hybridPtr` passed directly as first arg (no `mHybridData` traversal).
// =============================================================================

JNIEXPORT jlong JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeCreateHybridObject(
    JNIEnv* env,
    jclass clazz,
    jstring name);

JNIEXPORT void JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeDisposeHybridObject(
    JNIEnv* env,
    jclass clazz,
    jlong hybrid_ptr);

JNIEXPORT jlong JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeGetMemorySize(
    JNIEnv* env,
    jclass clazz,
    jlong hybrid_ptr);

JNIEXPORT jobject JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeAllocDirectBuffer(
    JNIEnv* env,
    jclass clazz,
    jint bytes);

JNIEXPORT void JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeFreeDirectBuffer(
    JNIEnv* env,
    jclass clazz,
    jobject direct_buffer);

JNIEXPORT jint JNICALL Java_dev_nve_nitro_NveNitroRuntime_nativeInvokeBinary(
    JNIEnv* env,
    jclass clazz,
    jlong hybrid_ptr,
    jint method_id,
    jobject direct_in,
    jint in_len,
    jobject direct_out);

} // extern "C"
