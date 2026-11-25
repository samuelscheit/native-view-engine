#include "NveAndroidDirectChannel.hpp"

#include <string_view>

namespace nve::android_internal {

namespace {

/// Minimum valid `EventRing` wire frame is the 4-byte little-endian `event_count` header.
constexpr size_t kMinEventRingBytes = 4u;

jobject ensure_scratch_buffer(
    JNIEnv* env,
    std::vector<uint8_t>& storage,
    CachedDirectBuffer& cached) noexcept {
  if (env == nullptr) {
    return nullptr;
  }
  if (storage.size() < kDefaultDirectScratchBytes) {
    storage.assign(kDefaultDirectScratchBytes, 0u);
  }
  (void)cached.sync(env, storage.data(), storage.size());
  return cached.global_ref != nullptr ? env->NewLocalRef(cached.global_ref) : nullptr;
}

jboolean dispatch_event_ring_slice(
    AndroidSurfaceState* state,
    const uint8_t* ptr,
    size_t len) noexcept {
  if (!is_session_active(state) || ptr == nullptr || len < kMinEventRingBytes) {
    return JNI_FALSE;
  }
  const NveBufferSlice slice{ptr, len};
  auto* bridge = state->session.bridge();
  if (bridge != nullptr) {
    (void)bridge->dispatch_native_events(slice, /*auto_render=*/true);
    (void)state->session.drain_events();
    return JNI_TRUE;
  }
  if (state->session.is_paired()) {
    return state->session.send_paired_event_ring(slice) ? JNI_TRUE : JNI_FALSE;
  }
  return JNI_FALSE;
}

jboolean apply_props_slice(
    AndroidSurfaceState* state,
    const uint8_t* ptr,
    size_t len) noexcept {
  if (state == nullptr || !state->session.is_valid() || ptr == nullptr || len == 0u) {
    return JNI_FALSE;
  }
  const std::string_view raw(reinterpret_cast<const char*>(ptr), len);
  const bool applied = state->session.update_props_json(raw, /*auto_render=*/true);
  if (applied) {
    (void)state->session.drain_events();
  }
  return applied ? JNI_TRUE : JNI_FALSE;
}

} // namespace

void release_direct_scratch_buffers(AndroidSurfaceSession* session, JNIEnv* env) noexcept {
  if (!session) {
    return;
  }
  session->event_scratch_buf.release(env);
  session->event_scratch.clear();
  session->event_scratch.shrink_to_fit();

  session->prop_scratch_buf.release(env);
  session->prop_scratch.clear();
  session->prop_scratch.shrink_to_fit();
}

jobject get_event_scratch_buffer(JNIEnv* env, jlong handle_ptr) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (!session || env == nullptr) {
    return nullptr;
  }
  return ensure_scratch_buffer(env, session->event_scratch, session->event_scratch_buf);
}

jobject get_prop_scratch_buffer(JNIEnv* env, jlong handle_ptr) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (!session || env == nullptr) {
    return nullptr;
  }
  return ensure_scratch_buffer(env, session->prop_scratch, session->prop_scratch_buf);
}

jboolean submit_events_direct(jlong handle_ptr, jint byte_len) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (!session || byte_len < static_cast<jint>(kMinEventRingBytes)) {
    return JNI_FALSE;
  }
  const size_t len = static_cast<size_t>(byte_len);
  if (len > session->event_scratch.size()) {
    return JNI_FALSE;
  }
  return dispatch_event_ring_slice(session, session->event_scratch.data(), len);
}

jboolean submit_events_array(JNIEnv* env, jlong handle_ptr, jbyteArray event_ring_bytes) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (env == nullptr || !session || event_ring_bytes == nullptr || !is_session_active(session)) {
    return JNI_FALSE;
  }
  const jsize arr_len = env->GetArrayLength(event_ring_bytes);
  if (arr_len < static_cast<jsize>(kMinEventRingBytes)) {
    return JNI_FALSE;
  }
  jbyte* raw = env->GetByteArrayElements(event_ring_bytes, nullptr);
  if (raw == nullptr) {
    return JNI_FALSE;
  }
  const jboolean ok = dispatch_event_ring_slice(
      session, reinterpret_cast<const uint8_t*>(raw), static_cast<size_t>(arr_len));
  env->ReleaseByteArrayElements(event_ring_bytes, raw, JNI_ABORT);
  return ok;
}

jboolean set_props_direct(jlong handle_ptr, jint byte_len) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (!session || byte_len <= 0) {
    return JNI_FALSE;
  }
  const size_t len = static_cast<size_t>(byte_len);
  if (len > session->prop_scratch.size()) {
    return JNI_FALSE;
  }
  return apply_props_slice(session, session->prop_scratch.data(), len);
}

jboolean set_props_array(JNIEnv* env, jlong handle_ptr, jbyteArray props_bytes) noexcept {
  auto* session = as_surface_session(handle_ptr);
  if (env == nullptr || !session || props_bytes == nullptr || !session->session.is_valid()) {
    return JNI_FALSE;
  }
  const jsize arr_len = env->GetArrayLength(props_bytes);
  if (arr_len <= 0) {
    return JNI_FALSE;
  }
  jbyte* raw = env->GetByteArrayElements(props_bytes, nullptr);
  if (raw == nullptr) {
    return JNI_FALSE;
  }
  const jboolean ok =
      apply_props_slice(session, reinterpret_cast<const uint8_t*>(raw), static_cast<size_t>(arr_len));
  env->ReleaseByteArrayElements(props_bytes, raw, JNI_ABORT);
  return ok;
}

} // namespace nve::android_internal

// =============================================================================
// JNI Exports — dev.nve.android.NveMainActivity
// =============================================================================

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetEventScratchBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return nve::android_internal::get_event_scratch_buffer(env, handle_ptr);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetPropScratchBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return nve::android_internal::get_prop_scratch_buffer(env, handle_ptr);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeSubmitEventsDirect(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint byte_len) {
  return nve::android_internal::submit_events_direct(handle_ptr, byte_len);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeSetPropsDirect(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint byte_len) {
  return nve::android_internal::set_props_direct(handle_ptr, byte_len);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeSubmitEvents(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jbyteArray event_ring_bytes) {
  return nve::android_internal::submit_events_array(env, handle_ptr, event_ring_bytes);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeSetPropsBinary(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jbyteArray props_bytes) {
  return nve::android_internal::set_props_array(env, handle_ptr, props_bytes);
}

// =============================================================================
// JNI Exports — dev.nve.compose.NveJniBridge
// =============================================================================

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_compose_NveJniBridge_nativeGetEventScratchBuffer(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr) {
  return nve::android_internal::get_event_scratch_buffer(env, handle_ptr);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_compose_NveJniBridge_nativeGetPropScratchBuffer(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr) {
  return nve::android_internal::get_prop_scratch_buffer(env, handle_ptr);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_compose_NveJniBridge_nativeSubmitEventsDirect(
    JNIEnv* /*env*/,
    jobject /*thiz*/,
    jlong handle_ptr,
    jint byte_len) {
  return nve::android_internal::submit_events_direct(handle_ptr, byte_len);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_compose_NveJniBridge_nativeSetPropsDirect(
    JNIEnv* /*env*/,
    jobject /*thiz*/,
    jlong handle_ptr,
    jint byte_len) {
  return nve::android_internal::set_props_direct(handle_ptr, byte_len);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_compose_NveJniBridge_nativeSubmitEvents(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr,
    jbyteArray event_ring_bytes) {
  return nve::android_internal::submit_events_array(env, handle_ptr, event_ring_bytes);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_compose_NveJniBridge_nativeSetPropsBinary(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr,
    jbyteArray props_bytes) {
  return nve::android_internal::set_props_array(env, handle_ptr, props_bytes);
}
