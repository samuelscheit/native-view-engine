// `NVEV` v2 `EventRing` (written by `dev.nve.host.EventRingWriter`) → aligned `NveEventBatch` →
// `NveRuntimeApi::dispatch_events`, plus the host text measurer and wake listener callbacks into
// Java.
#include "NveAndroidSurface.hpp"

#include <cmath>
#include <cstring>
#include <span>

using nve::android::AndroidSurface;
using nve::android::release_scene;
using nve::android::status_of;
using nve::android::owned_surface;
using nve::android::write_out;

namespace {

/// Validates an `NVEV` v2 wire frame (14-byte header + `count * 64 B NveEvent` + `bytes_len` byte
/// arena), copies the records into the 64-byte-aligned reusable scratch `out` (patching
/// `surface == 0` to `default_surface_id`), and borrows the arena slice into `*out_arena`.
bool decode_nvev(const uint8_t* data, size_t len, uint32_t default_surface_id,
                 std::vector<NveEvent, nve::host::AlignedAllocator<NveEvent, NVE_EVENT_ALIGN>>& out,
                 std::span<const uint8_t>* out_arena) {
  if (len < nve::host::kEventWireHeaderSize) return false;
  if (nve_load_u32_le(data + 0u) != nve::host::kEventWireMagic) return false;
  if (nve_load_u16_le(data + 4u) != nve::host::kEventWireVersion) return false;
  const uint32_t count = nve_load_u32_le(data + 6u);
  const uint32_t blen = nve_load_u32_le(data + 10u);
  const uint64_t ev_bytes = uint64_t{count} * NVE_EVENT_SIZE;
  const uint64_t expected = nve::host::kEventWireHeaderSize + ev_bytes + uint64_t{blen};
  if (expected != len) return false;
  const std::span<const uint8_t> arena{
      data + nve::host::kEventWireHeaderSize + static_cast<size_t>(ev_bytes), blen};
  out.resize(count);
  for (uint32_t i = 0u; i < count; ++i) {
    std::memcpy(&out[i], data + nve::host::kEventWireHeaderSize + size_t{i} * NVE_EVENT_SIZE, NVE_EVENT_SIZE);
    if (!nve::host::validate_event_record(out[i], arena)) {
      out.clear();
      return false;
    }
    out[i].surface = nve::host::canonical_event_handle(out[i].surface);
    out[i].node = nve::host::canonical_event_handle(out[i].node);
    if (out[i].surface == 0u) out[i].surface = nve::host::canonical_event_handle(default_surface_id);
  }
  *out_arena = arena;
  return true;
}

/// Detaches a runtime thread this library attached, when that thread exits.
struct ThreadAttachment {
  JavaVM* jvm = nullptr;
  ~ThreadAttachment() {
    if (jvm != nullptr) (void)jvm->DetachCurrentThread();
  }
};

/// JNI env of the current thread. A runtime thread (wake, measurement) is attached ONCE, as a
/// daemon (it never keeps the JVM alive), and detached when it exits — not attached / detached
/// per callback at frame rate.
JNIEnv* thread_env(JavaVM* jvm) {
  JNIEnv* env = nullptr;
  const jint rc = jvm->GetEnv(reinterpret_cast<void**>(&env), JNI_VERSION_1_6);
  if (rc == JNI_OK) return env;
  if (rc != JNI_EDETACHED) return nullptr;
  thread_local ThreadAttachment attachment;
#if defined(__ANDROID__)
  if (jvm->AttachCurrentThreadAsDaemon(&env, nullptr) != JNI_OK) return nullptr;
#else
  if (jvm->AttachCurrentThreadAsDaemon(reinterpret_cast<void**>(&env), nullptr) != JNI_OK) return nullptr;
#endif
  attachment.jvm = jvm;
  return env;
}

/// Calls `callback(env)` on the current thread. A Java exception the callback raises is reported
/// (`ExceptionDescribe` logs and clears it) and never propagates into the runtime.
template <class Fn>
void with_env(JavaVM* jvm, Fn&& callback) {
  JNIEnv* env = thread_env(jvm);
  if (env == nullptr) return;
  callback(env);
  if (env->ExceptionCheck()) env->ExceptionDescribe();
}

void on_wake(void* ctx) {
  auto* s = static_cast<AndroidSurface*>(ctx);
  if (s->jvm == nullptr || s->wake.target == nullptr) return;
  with_env(s->jvm, [&](JNIEnv* env) { env->CallVoidMethod(s->wake.target, s->wake.method); });
}

/// `NveRuntimeMeasureTextFn` over `NativeRuntime.TextMeasurer`, which takes the size and weight
/// Compose paints with (`nowrap` / `pre` already arrive as an INFINITY `max_width`). Leaves the
/// runtime's zeroed out slots untouched when Java fails (the runtime maps non-finite / negative
/// sizes to 0 itself).
void on_measure(void* ctx, NveStrSlice text, const NveRuntimeTextStyle* style, float max_width, float* out_w,
                float* out_h) {
  const float font_size = style->font_size;
  const uint32_t font_weight = style->font_weight;
  auto* s = static_cast<AndroidSurface*>(ctx);
  if (s->jvm == nullptr || s->measurer.target == nullptr || s->measure_out == nullptr) return;
  with_env(s->jvm, [&](JNIEnv* env) {
    jstring jtext = nve::android::java_string(env, text.ptr, text.len);
    if (jtext == nullptr) return;
    const float nan_out[2] = {NAN, NAN};
    env->SetFloatArrayRegion(s->measure_out, 0, 2, nan_out);  // a measurer that writes nothing -> 0 x 0
    env->CallVoidMethod(s->measurer.target, s->measurer.method, jtext, font_size, static_cast<jint>(font_weight),
                        max_width, s->measure_out);
    env->DeleteLocalRef(jtext);
    if (env->ExceptionCheck()) return;  // reported and cleared by `with_env`
    float wh[2] = {NAN, NAN};
    env->GetFloatArrayRegion(s->measure_out, 0, 2, wh);
    *out_w = wh[0];
    *out_h = wh[1];
  });
}

/// Replaces `slot` with `target`'s `name` + `sig` method; `false` (slot cleared) when it has none.
bool bind_callback(JNIEnv* env, nve::android::JavaCallback& slot, jobject target, const char* name,
                   const char* sig) {
  slot.release(env);
  if (target == nullptr) return true;
  jclass cls = env->GetObjectClass(target);
  jmethodID mid = cls != nullptr ? env->GetMethodID(cls, name, sig) : nullptr;
  if (cls != nullptr) env->DeleteLocalRef(cls);
  if (mid == nullptr) {
    nve::android::clear_exception(env);
    return false;
  }
  slot.target = env->NewGlobalRef(target);
  if (slot.target == nullptr) {
    nve::android::clear_exception(env);
    return false;
  }
  slot.method = mid;
  return true;
}

}  // namespace

extern "C" {

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_dispatchEvents(JNIEnv* env, jclass, jlong handle,
                                                                     jobject ring, jint len, jintArray out_changed) {
  AndroidSurface* s = owned_surface(handle);
  write_out(env, out_changed, 0);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  const auto* data = ring != nullptr ? static_cast<const uint8_t*>(env->GetDirectBufferAddress(ring)) : nullptr;
  std::span<const uint8_t> arena;
  if (data == nullptr || len < static_cast<jint>(nve::host::kEventWireHeaderSize) ||
      static_cast<jlong>(len) > env->GetDirectBufferCapacity(ring) ||
      !decode_nvev(data, static_cast<size_t>(len), s->surface_id, s->events, &arena)) {
    s->last_error = "malformed NVEV v2 EventRing (direct buffer, exact length, valid framing & UTF-8 required)";
    return NVE_RUNTIME_INVALID_ARG;
  }
  release_scene(*s);
  bool changed = false;
  const NveEventBatch batch{
      s->events.empty() ? nullptr : s->events.data(),
      arena.empty() ? nullptr : arena.data(),
      static_cast<uint32_t>(s->events.size()),
      static_cast<uint32_t>(arena.size()),
  };
  const NveRuntimeStatus st = s->session.dispatch_events(batch, &changed);
  write_out(env, out_changed, changed ? 1 : 0);
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setTextMeasurer(JNIEnv* env, jclass, jlong handle,
                                                                      jobject measurer) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  // Detach the runtime from the old target before dropping its global ref.
  NveRuntimeStatus st = s->session.clear_text_measurer();
  if (st != NVE_RUNTIME_OK) return status_of(*s, st);
  if (!bind_callback(env, s->measurer, measurer, "measure", "(Ljava/lang/String;FIF[F)V")) {
    s->last_error = "TextMeasurer has no measure(String, float, int, float, float[]) method";
    return NVE_RUNTIME_INVALID_ARG;
  }
  if (measurer == nullptr) return status_of(*s, NVE_RUNTIME_OK);
  if (s->measure_out == nullptr) {
    jfloatArray local = env->NewFloatArray(2);
    s->measure_out = local != nullptr ? static_cast<jfloatArray>(env->NewGlobalRef(local)) : nullptr;
    if (local != nullptr) env->DeleteLocalRef(local);
    if (s->measure_out == nullptr) {
      nve::android::clear_exception(env);
      s->measurer.release(env);
      s->last_error = "out of memory allocating the measurer out array";
      return NVE_RUNTIME_ERROR;
    }
  }
  return status_of(*s, s->session.set_text_measurer(&on_measure, s));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setWakeListener(JNIEnv* env, jclass, jlong handle,
                                                                      jobject listener) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  // Clearing drains in-flight wakes, so the old global ref is never used after release.
  NveRuntimeStatus st = s->session.clear_wake_callback();
  if (st != NVE_RUNTIME_OK) return status_of(*s, st);
  if (!bind_callback(env, s->wake, listener, "run", "()V")) {
    s->last_error = "wake listener has no run() method";
    return NVE_RUNTIME_INVALID_ARG;
  }
  if (listener == nullptr) return status_of(*s, NVE_RUNTIME_OK);
  return status_of(*s, s->session.set_wake_callback(&on_wake, s));
}

}  // extern "C"
