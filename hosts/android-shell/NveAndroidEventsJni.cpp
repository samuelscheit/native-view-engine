// `NVEV` v2 `EventRing` (written by `dev.nve.host.EventRingWriter`) -> aligned `NveEventBatch` ->
// `NveRuntimeApi::dispatch_events`, outbound `drain_emitted_events` -> `NVEV` v2, binary
// `NveBinValue` map props -> `set_props`, plus the host text measurer, wake, intrinsic-size
// invalidation, and `NVE_IFACE_HOST = 5` callback bridges into Java.
#include "NveAndroidSurface.hpp"

#include <nve/host/bin_codec.hpp>
#include <nve/host/value_tree.hpp>

#include <cmath>
#include <cstring>
#include <limits>
#include <span>

using nve::android::AndroidSurface;
using nve::android::clear_exception;
using nve::android::direct_buffer_span;
using nve::android::owned_surface;
using nve::android::release_scene;
using nve::android::status_of;
using nve::android::surface_of;
using nve::android::write_out;

namespace nve::android {

void JavaHostBridge::release(JNIEnv* env) noexcept {
  if (env != nullptr) {
    if (target != nullptr) env->DeleteGlobalRef(target);
    if (byte_array_cls != nullptr) env->DeleteGlobalRef(byte_array_cls);
  }
  target = nullptr;
  invoke_method = nullptr;
  publish_method = nullptr;
  byte_array_cls = nullptr;
}

}  // namespace nve::android

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

/// Calls `callback(env)` on the current thread inside a bounded local reference frame (defense in
/// depth for daemon threads that never return to Java). Any Java exception is logged and cleared.
template <class Fn>
void with_env(JavaVM* jvm, Fn&& callback) {
  JNIEnv* env = thread_env(jvm);
  if (env == nullptr) return;
  if (env->PushLocalFrame(16) != JNI_OK) {
    clear_exception(env);
    return;
  }
  callback(env);
  if (env->ExceptionCheck()) {
    env->ExceptionDescribe();
    env->ExceptionClear();
  }
  env->PopLocalFrame(nullptr);
}

/// Copies a Java `byte[]` into a small stack buffer (or heap vector above 512 B) and passes its
/// span to `fn`, avoiding off-heap `DirectByteBuffer` allocations on cross-thread host ingress.
template <class Fn>
jint with_byte_array_span(JNIEnv* env, jbyteArray arr, Fn&& fn) {
  if (arr == nullptr) return static_cast<jint>(fn(std::span<const uint8_t>{}));
  const jsize len = env->GetArrayLength(arr);
  if (len <= 0) return static_cast<jint>(fn(std::span<const uint8_t>{}));
  constexpr jsize kStackCap = 512;
  if (len <= kStackCap) {
    uint8_t stack[kStackCap];
    env->GetByteArrayRegion(arr, 0, len, reinterpret_cast<jbyte*>(stack));
    if (clear_exception(env)) return NVE_RUNTIME_INVALID_ARG;
    return static_cast<jint>(fn(std::span<const uint8_t>(stack, static_cast<size_t>(len))));
  }
  std::vector<uint8_t> heap(static_cast<size_t>(len));
  env->GetByteArrayRegion(arr, 0, len, reinterpret_cast<jbyte*>(heap.data()));
  if (clear_exception(env)) return NVE_RUNTIME_INVALID_ARG;
  return static_cast<jint>(fn(std::span<const uint8_t>(heap.data(), heap.size())));
}

void on_wake(void* ctx) {
  auto* s = static_cast<AndroidSurface*>(ctx);
  if (s->jvm == nullptr || s->wake.target == nullptr) return;
  with_env(s->jvm, [&](JNIEnv* env) { env->CallVoidMethod(s->wake.target, s->wake.method); });
}

void on_intrinsic_size_invalidated(void* ctx) {
  auto* s = static_cast<AndroidSurface*>(ctx);
  if (s->jvm == nullptr || s->intrinsic_size_invalidated.target == nullptr) return;
  with_env(s->jvm, [&](JNIEnv* env) {
    env->CallVoidMethod(s->intrinsic_size_invalidated.target, s->intrinsic_size_invalidated.method);
  });
}

int32_t on_host_invoke(void* host_ctx, uint32_t service_id, uint32_t method_id, uint64_t request_id,
                       NveBufferSlice request, uint8_t* out_buf, size_t out_cap, size_t* out_len) {
  if (out_len != nullptr) *out_len = 0u;
  auto* s = static_cast<AndroidSurface*>(host_ctx);
  if (s == nullptr || s->jvm == nullptr || s->host_bridge.target == nullptr) return NVE_HOST_UNSUPPORTED;
  if ((request.len > 0u && request.ptr == nullptr) ||
      request.len > static_cast<size_t>(std::numeric_limits<jsize>::max())) {
    return NVE_HOST_INVALID_ARG;
  }
  int32_t rc = NVE_HOST_INTERNAL_ERROR;
  with_env(s->jvm, [&](JNIEnv* env) {
    jbyteArray jreq = env->NewByteArray(static_cast<jsize>(request.len));
    if (jreq == nullptr || env->ExceptionCheck()) return;
    if (request.len > 0u) {
      env->SetByteArrayRegion(jreq, 0, static_cast<jsize>(request.len),
                              reinterpret_cast<const jbyte*>(request.ptr));
      if (env->ExceptionCheck()) {
        env->DeleteLocalRef(jreq);
        return;
      }
    }
    jobjectArray out_holder = env->NewObjectArray(1, s->host_bridge.byte_array_cls, nullptr);
    if (out_holder == nullptr || env->ExceptionCheck()) {
      env->DeleteLocalRef(jreq);
      return;
    }
    const jint jrc = env->CallIntMethod(s->host_bridge.target, s->host_bridge.invoke_method,
                                        static_cast<jint>(service_id), static_cast<jint>(method_id),
                                        static_cast<jlong>(request_id), jreq, out_holder);
    env->DeleteLocalRef(jreq);
    if (env->ExceptionCheck()) {
      env->DeleteLocalRef(out_holder);
      rc = NVE_HOST_INTERNAL_ERROR;
      return;
    }
    if (jrc != NVE_HOST_OK) {
      env->DeleteLocalRef(out_holder);
      rc = static_cast<int32_t>(jrc);
      return;
    }
    auto reply = static_cast<jbyteArray>(env->GetObjectArrayElement(out_holder, 0));
    env->DeleteLocalRef(out_holder);
    if (env->ExceptionCheck()) {
      rc = NVE_HOST_INTERNAL_ERROR;
      return;
    }
    if (reply == nullptr) {
      rc = NVE_HOST_OK;
      return;
    }
    const jsize rlen = env->GetArrayLength(reply);
    const size_t reply_len = rlen > 0 ? static_cast<size_t>(rlen) : 0u;
    if (out_len != nullptr) *out_len = reply_len;
    if (reply_len > out_cap) {
      env->DeleteLocalRef(reply);
      rc = NVE_HOST_BUFFER_TOO_SMALL;
      return;
    }
    if (reply_len > 0u) {
      if (out_buf == nullptr) {
        env->DeleteLocalRef(reply);
        rc = NVE_HOST_INVALID_ARG;
        return;
      }
      env->GetByteArrayRegion(reply, 0, rlen, reinterpret_cast<jbyte*>(out_buf));
    }
    env->DeleteLocalRef(reply);
    rc = env->ExceptionCheck() ? NVE_HOST_INTERNAL_ERROR : NVE_HOST_OK;
  });
  return rc;
}

int32_t on_host_channel_publish(void* host_ctx, uint32_t channel_id, uint32_t flags, NveBufferSlice payload) {
  auto* s = static_cast<AndroidSurface*>(host_ctx);
  if (s == nullptr || s->jvm == nullptr || s->host_bridge.target == nullptr) return NVE_HOST_UNSUPPORTED;
  if ((payload.len > 0u && payload.ptr == nullptr) ||
      payload.len > static_cast<size_t>(std::numeric_limits<jsize>::max())) {
    return NVE_HOST_INVALID_ARG;
  }
  int32_t rc = NVE_HOST_INTERNAL_ERROR;
  with_env(s->jvm, [&](JNIEnv* env) {
    jbyteArray jpay = env->NewByteArray(static_cast<jsize>(payload.len));
    if (jpay == nullptr || env->ExceptionCheck()) return;
    if (payload.len > 0u) {
      env->SetByteArrayRegion(jpay, 0, static_cast<jsize>(payload.len),
                              reinterpret_cast<const jbyte*>(payload.ptr));
      if (env->ExceptionCheck()) {
        env->DeleteLocalRef(jpay);
        return;
      }
    }
    const jint jrc = env->CallIntMethod(s->host_bridge.target, s->host_bridge.publish_method,
                                        static_cast<jint>(channel_id), static_cast<jint>(flags), jpay);
    env->DeleteLocalRef(jpay);
    if (!env->ExceptionCheck()) rc = static_cast<int32_t>(jrc);
  });
  return rc;
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
    if (env->ExceptionCheck()) {
      env->DeleteLocalRef(jtext);
      return;
    }
    env->CallVoidMethod(s->measurer.target, s->measurer.method, jtext, font_size, static_cast<jint>(font_weight),
                        max_width, s->measure_out);
    env->DeleteLocalRef(jtext);
    if (env->ExceptionCheck()) return;  // reported and cleared by `with_env`
    float wh[2] = {NAN, NAN};
    env->GetFloatArrayRegion(s->measure_out, 0, 2, wh);
    if (env->ExceptionCheck()) return;
    *out_w = wh[0];
    *out_h = wh[1];
  });
}

/// Resolves `target`'s `name` + `sig` method and creates a `GlobalRef` into `out` without
/// modifying any existing slot; `false` on failure (with pending JNI exceptions cleared).
bool prepare_callback(JNIEnv* env, nve::android::JavaCallback& out, jobject target, const char* name,
                      const char* sig) {
  out = {};
  if (target == nullptr) return true;
  jclass cls = env->GetObjectClass(target);
  if (cls == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    if (cls != nullptr) env->DeleteLocalRef(cls);
    return false;
  }
  jmethodID mid = env->GetMethodID(cls, name, sig);
  env->DeleteLocalRef(cls);
  if (mid == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    return false;
  }
  jobject ref = env->NewGlobalRef(target);
  if (ref == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    if (ref != nullptr) env->DeleteGlobalRef(ref);
    return false;
  }
  out.target = ref;
  out.method = mid;
  return true;
}

/// Replaces `slot` with `target`'s `name` + `sig` method; `false` (slot cleared) when it has none.
bool bind_callback(JNIEnv* env, nve::android::JavaCallback& slot, jobject target, const char* name,
                   const char* sig) {
  slot.release(env);
  return prepare_callback(env, slot, target, name, sig);
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
  if (measurer == nullptr) {
    const NveRuntimeStatus st = s->session.clear_text_measurer();
    if (st == NVE_RUNTIME_OK) s->measurer.release(env);
    return status_of(*s, st);
  }
  // Prepare the new callback & out array before touching the runtime so replacing an active
  // measurer triggers only ONE relayout/wake (never clear + set) and leaves the old one intact on error.
  nve::android::JavaCallback next{};
  if (!prepare_callback(env, next, measurer, "measure", "(Ljava/lang/String;FIF[F)V")) {
    s->last_error = "TextMeasurer has no measure(String, float, int, float, float[]) method";
    return NVE_RUNTIME_INVALID_ARG;
  }
  if (s->measure_out == nullptr) {
    jfloatArray local = env->NewFloatArray(2);
    if (local == nullptr || env->ExceptionCheck()) {
      clear_exception(env);
      if (local != nullptr) env->DeleteLocalRef(local);
      next.release(env);
      s->last_error = "out of memory allocating the measurer out array";
      return NVE_RUNTIME_ERROR;
    }
    auto global_out = static_cast<jfloatArray>(env->NewGlobalRef(local));
    env->DeleteLocalRef(local);
    if (global_out == nullptr || env->ExceptionCheck()) {
      clear_exception(env);
      if (global_out != nullptr) env->DeleteGlobalRef(global_out);
      next.release(env);
      s->last_error = "out of memory allocating the measurer out array";
      return NVE_RUNTIME_ERROR;
    }
    s->measure_out = global_out;
  }
  nve::android::JavaCallback prev = s->measurer;
  s->measurer = next;
  const NveRuntimeStatus st = s->session.set_text_measurer(&on_measure, s);
  if (st != NVE_RUNTIME_OK) {
    s->measurer = prev;
    next.release(env);
  } else {
    prev.release(env);
  }
  return status_of(*s, st);
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
  st = s->session.set_wake_callback(&on_wake, s);
  if (st != NVE_RUNTIME_OK) s->wake.release(env);
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setProps(JNIEnv* env, jclass, jlong handle, jobject bin_map,
                                                                jint len, jintArray out_changed) {
  AndroidSurface* s = owned_surface(handle);
  write_out(env, out_changed, 0);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  std::span<const uint8_t> bytes;
  if (!direct_buffer_span(env, bin_map, len, &bytes)) {
    s->last_error = "setProps needs a direct buffer holding `length` bytes";
    return NVE_RUNTIME_INVALID_ARG;
  }
  if (bytes.empty()) {
    release_scene(*s);
    bool changed = false;
    const NveRuntimeStatus st = s->session.set_props(nullptr, 0u, &changed);
    write_out(env, out_changed, changed ? 1 : 0);
    return status_of(*s, st);
  }
  nve::host::BinReader reader(bytes);
  s->props_tree.clear();
  const NveValue root = s->props_tree.read_value(reader);
  if (!reader.ok() || !reader.at_end() || root.tag != NVE_VAL_MAP ||
      root.data.map_val.len > std::numeric_limits<uint32_t>::max()) {
    s->props_tree.clear();
    s->last_error = "setProps expects a single valid NveBinValue Map in a direct buffer";
    return NVE_RUNTIME_INVALID_ARG;
  }
  release_scene(*s);
  bool changed = false;
  const NveRuntimeStatus st =
      s->session.set_props(root.data.map_val.ptr, static_cast<uint32_t>(root.data.map_val.len), &changed);
  s->props_tree.clear();
  write_out(env, out_changed, changed ? 1 : 0);
  return status_of(*s, st);
}

JNIEXPORT jbyteArray JNICALL Java_dev_nve_host_NativeRuntime_drainEmittedEvents(JNIEnv* env, jclass, jlong handle,
                                                                                jintArray out_status) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) {
    write_out(env, out_status, NVE_RUNTIME_INVALID_ARG);
    return nullptr;
  }
  release_scene(*s);
  NveEventBatch batch{nullptr, nullptr, 0u, 0u};
  const NveRuntimeStatus st = s->session.drain_emitted_events(&batch);
  // Apps without `NVE_IFACE_COMPONENT` (or detached/Frame-0/paired surfaces) return UNSUPPORTED:
  // never clobber `last_error` or allocate an error string for a benign capability check.
  if (st == NVE_RUNTIME_UNSUPPORTED) {
    write_out(env, out_status, static_cast<jint>(st));
    return nullptr;
  }
  const jint code = status_of(*s, st);
  write_out(env, out_status, code);
  if (st != NVE_RUNTIME_OK || batch.events == nullptr || batch.event_count == 0u) return nullptr;
  if (batch.bytes_len > 0u && batch.bytes == nullptr) {
    s->last_error = "drainEmittedEvents returned null arena for non-zero bytes_len";
    write_out(env, out_status, NVE_RUNTIME_ERROR);
    return nullptr;
  }
  const uint64_t ev_bytes = uint64_t{batch.event_count} * NVE_EVENT_SIZE;
  const uint64_t total = nve::host::kEventWireHeaderSize + ev_bytes + uint64_t{batch.bytes_len};
  if (total > static_cast<uint64_t>(std::numeric_limits<jsize>::max())) {
    s->last_error = "drainEmittedEvents batch exceeds JNI array limit";
    write_out(env, out_status, NVE_RUNTIME_ERROR);
    return nullptr;
  }
  uint8_t hdr[nve::host::kEventWireHeaderSize];
  nve_store_u32_le(hdr + 0u, nve::host::kEventWireMagic);
  nve_store_u16_le(hdr + 4u, nve::host::kEventWireVersion);
  nve_store_u32_le(hdr + 6u, batch.event_count);
  nve_store_u32_le(hdr + 10u, batch.bytes_len);
  jbyteArray out = env->NewByteArray(static_cast<jsize>(total));
  if (out == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    s->last_error = "out of memory allocating emitted events byte array";
    write_out(env, out_status, NVE_RUNTIME_ERROR);
    return nullptr;
  }
  env->SetByteArrayRegion(out, 0, static_cast<jsize>(nve::host::kEventWireHeaderSize),
                          reinterpret_cast<const jbyte*>(hdr));
  if (!clear_exception(env)) {
    env->SetByteArrayRegion(out, static_cast<jsize>(nve::host::kEventWireHeaderSize), static_cast<jsize>(ev_bytes),
                            reinterpret_cast<const jbyte*>(batch.events));
  } else {
    s->last_error = "failed writing emitted events header";
    write_out(env, out_status, NVE_RUNTIME_ERROR);
    return nullptr;
  }
  if (!clear_exception(env) && batch.bytes_len > 0u) {
    env->SetByteArrayRegion(out, static_cast<jsize>(nve::host::kEventWireHeaderSize + ev_bytes),
                            static_cast<jsize>(batch.bytes_len), reinterpret_cast<const jbyte*>(batch.bytes));
  }
  if (clear_exception(env)) {
    s->last_error = "failed writing emitted events payload";
    write_out(env, out_status, NVE_RUNTIME_ERROR);
    return nullptr;
  }
  return out;
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setIntrinsicSizeInvalidatedListener(JNIEnv* env, jclass,
                                                                                           jlong handle,
                                                                                           jobject listener) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  NveRuntimeStatus st = s->session.clear_intrinsic_size_invalidated_callback();
  if (st != NVE_RUNTIME_OK) return status_of(*s, st);
  if (!bind_callback(env, s->intrinsic_size_invalidated, listener, "run", "()V")) {
    s->last_error = "intrinsic size listener has no run() method";
    return NVE_RUNTIME_INVALID_ARG;
  }
  if (listener == nullptr) return status_of(*s, NVE_RUNTIME_OK);
  st = s->session.set_intrinsic_size_invalidated_callback(&on_intrinsic_size_invalidated, s);
  if (st != NVE_RUNTIME_OK) s->intrinsic_size_invalidated.release(env);
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setHostCallbackBridge(JNIEnv* env, jclass, jlong handle,
                                                                             jobject bridge) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  NveRuntimeStatus st = s->session.clear_host_callbacks();
  if (st != NVE_RUNTIME_OK) return status_of(*s, st);
  s->host_bridge.release(env);
  if (bridge == nullptr) return status_of(*s, NVE_RUNTIME_OK);
  jclass cls = env->GetObjectClass(bridge);
  if (cls == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    if (cls != nullptr) env->DeleteLocalRef(cls);
    s->last_error = "failed resolving HostCallbackBridge class";
    return NVE_RUNTIME_INVALID_ARG;
  }
  jmethodID invoke_mid = env->GetMethodID(cls, "invoke", "(IIJ[B[[B)I");
  if (invoke_mid == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    invoke_mid = nullptr;
  }
  jmethodID publish_mid = invoke_mid != nullptr ? env->GetMethodID(cls, "publishChannel", "(II[B)I") : nullptr;
  if (publish_mid == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    publish_mid = nullptr;
  }
  env->DeleteLocalRef(cls);
  if (invoke_mid == nullptr || publish_mid == nullptr) {
    s->last_error = "HostCallbackBridge missing invoke(int,int,long,byte[],byte[][]) or publishChannel(int,int,byte[])";
    return NVE_RUNTIME_INVALID_ARG;
  }
  jclass local_byte_arr = env->FindClass("[B");
  if (local_byte_arr == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    if (local_byte_arr != nullptr) env->DeleteLocalRef(local_byte_arr);
    s->last_error = "out of memory resolving byte[] class for HostCallbackBridge";
    return NVE_RUNTIME_ERROR;
  }
  jobject global_cls = env->NewGlobalRef(local_byte_arr);
  env->DeleteLocalRef(local_byte_arr);
  if (global_cls == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    if (global_cls != nullptr) env->DeleteGlobalRef(global_cls);
    s->last_error = "out of memory binding HostCallbackBridge class ref";
    return NVE_RUNTIME_ERROR;
  }
  jobject global_target = env->NewGlobalRef(bridge);
  if (global_target == nullptr || env->ExceptionCheck()) {
    clear_exception(env);
    if (global_target != nullptr) env->DeleteGlobalRef(global_target);
    env->DeleteGlobalRef(global_cls);
    s->last_error = "out of memory binding HostCallbackBridge";
    return NVE_RUNTIME_ERROR;
  }
  s->host_bridge.target = global_target;
  s->host_bridge.invoke_method = invoke_mid;
  s->host_bridge.publish_method = publish_mid;
  s->host_bridge.byte_array_cls = static_cast<jclass>(global_cls);
  const NveHostCallbacks cbs{s, &on_host_invoke, &on_host_channel_publish};
  st = s->session.set_host_callbacks(&cbs);
  if (st != NVE_RUNTIME_OK) s->host_bridge.release(env);
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_completeHostRequest__JJILjava_nio_ByteBuffer_2I(
    JNIEnv* env, jclass, jlong handle, jlong request_id, jint host_status, jobject payload, jint len) {
  // Any thread: must not touch owner-thread fields (`scene`, `last_error`).
  const AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  std::span<const uint8_t> bytes;
  if (!direct_buffer_span(env, payload, len, &bytes)) return NVE_RUNTIME_INVALID_ARG;
  return static_cast<jint>(
      s->session.complete_host_request(static_cast<uint64_t>(request_id), static_cast<int32_t>(host_status), bytes));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_completeHostRequest__JJI_3B(JNIEnv* env, jclass, jlong handle,
                                                                                   jlong request_id, jint host_status,
                                                                                   jbyteArray payload) {
  // Any thread: must not touch owner-thread fields (`scene`, `last_error`).
  const AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  return with_byte_array_span(env, payload, [&](std::span<const uint8_t> bytes) {
    return s->session.complete_host_request(static_cast<uint64_t>(request_id), static_cast<int32_t>(host_status),
                                            bytes);
  });
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_pushHostEvent__JIILjava_nio_ByteBuffer_2I(
    JNIEnv* env, jclass, jlong handle, jint service_id, jint event_id, jobject payload, jint len) {
  // Any thread: must not touch owner-thread fields (`scene`, `last_error`).
  const AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  std::span<const uint8_t> bytes;
  if (!direct_buffer_span(env, payload, len, &bytes)) return NVE_RUNTIME_INVALID_ARG;
  return static_cast<jint>(
      s->session.push_host_event(static_cast<uint32_t>(service_id), static_cast<uint32_t>(event_id), bytes));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_pushHostEvent__JII_3B(JNIEnv* env, jclass, jlong handle,
                                                                           jint service_id, jint event_id,
                                                                           jbyteArray payload) {
  // Any thread: must not touch owner-thread fields (`scene`, `last_error`).
  const AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  return with_byte_array_span(env, payload, [&](std::span<const uint8_t> bytes) {
    return s->session.push_host_event(static_cast<uint32_t>(service_id), static_cast<uint32_t>(event_id), bytes);
  });
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_publishHostChannel__JIILjava_nio_ByteBuffer_2I(
    JNIEnv* env, jclass, jlong handle, jint channel_id, jint flags, jobject payload, jint len) {
  // Any thread: must not touch owner-thread fields (`scene`, `last_error`).
  const AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  std::span<const uint8_t> bytes;
  if (!direct_buffer_span(env, payload, len, &bytes)) return NVE_RUNTIME_INVALID_ARG;
  return static_cast<jint>(
      s->session.publish_host_channel(static_cast<uint32_t>(channel_id), static_cast<uint32_t>(flags), bytes));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_publishHostChannel__JII_3B(JNIEnv* env, jclass, jlong handle,
                                                                                jint channel_id, jint flags,
                                                                                jbyteArray payload) {
  // Any thread: must not touch owner-thread fields (`scene`, `last_error`).
  const AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  return with_byte_array_span(env, payload, [&](std::span<const uint8_t> bytes) {
    return s->session.publish_host_channel(static_cast<uint32_t>(channel_id), static_cast<uint32_t>(flags), bytes);
  });
}

}  // extern "C"
