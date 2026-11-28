// `EventRing` (`nve_abi::encode_events`, written by `dev.nve.host.EventRingWriter`) → borrowed
// `NveDirectEvent` batch → `NveRuntimeApi::dispatch_events`, plus the host text measurer and wake
// listener callbacks into Java.
#include "NveAndroidSurface.hpp"

#include <cmath>
#include <cstring>

using nve::android::AndroidSurface;
using nve::android::release_scene;
using nve::android::status_of;
using nve::android::owned_surface;
using nve::android::write_out;

namespace {

static_assert(NVE_EVENT_PAYLOAD_EMPTY == 0 && NVE_EVENT_PAYLOAD_POINT == 1 && NVE_EVENT_PAYLOAD_SCROLL == 2 &&
                  NVE_EVENT_PAYLOAD_TEXT == 3 && NVE_EVENT_PAYLOAD_LAYOUT == 4 && NVE_EVENT_PAYLOAD_CUSTOM == 5 &&
                  NVE_EVENT_PAYLOAD_KEY == 6 && NVE_EVENT_PAYLOAD_POINTER == 7 && NVE_EVENT_PAYLOAD_WHEEL == 8 &&
                  NVE_EVENT_PAYLOAD_TABLE_EVENT == 9,
              "EventRingWriter payload tags drifted from NveEventPayloadTag");

static_assert(NVE_EVENT_PRESS == 1 && NVE_EVENT_TABLE_EVENT == 21, "NveEventKind range changed: update decode_ring");

constexpr size_t kEventHeaderBytes = 18u;

/// Bounds-checked little-endian cursor over one ring.
struct Cursor {
  const uint8_t* p;
  size_t len;
  size_t at = 0u;

  template <class T>
  bool read(T& out) noexcept {
    if (len - at < sizeof(T)) return false;
    std::memcpy(&out, p + at, sizeof(T));
    at += sizeof(T);
    return true;
  }

  bool slice(size_t n, NveStrSlice& out) noexcept {
    if (len - at < n) return false;
    out = NveStrSlice{n == 0u ? nullptr : p + at, n};
    at += n;
    return true;
  }

  template <class Len>
  bool prefixed(NveStrSlice& out) noexcept {
    Len n = 0;
    return read(n) && slice(n, out);
  }

  /// A length-prefixed string that must be well-formed UTF-8 (the runtime borrows it as `&str`).
  template <class Len>
  bool utf8(NveStrSlice& out) noexcept {
    return prefixed<Len>(out) && (out.len == 0u || nve_utf8_valid(out.ptr, out.len) != 0);
  }
};

/// Decodes the whole ring (`nve_abi::encode_events`) into `out`, borrowing strings / bytes from
/// `ring`, with `nve_abi::direct_event_to_native`'s field mapping; `false` on any malformed byte,
/// unknown event kind, or string that is not UTF-8 (text, custom name, key, code).
bool decode_ring(const uint8_t* ring, size_t len, std::vector<NveDirectEvent>& out) {
  Cursor c{ring, len};
  uint32_t count = 0u;
  if (!c.read(count) || count > (len - c.at) / kEventHeaderBytes) return false;
  out.clear();
  out.reserve(count);
  for (uint32_t i = 0u; i < count; ++i) {
    NveDirectEvent ev{};
    ev.value.tag = NVE_VAL_NULL;
    if (!c.read(ev.surface_id) || !c.read(ev.node_id) || !c.read(ev.timestamp_us) || !c.read(ev.kind) ||
        !c.read(ev.payload_tag) || ev.kind < NVE_EVENT_PRESS || ev.kind > NVE_EVENT_TABLE_EVENT) {
      return false;
    }
    bool ok = true;
    switch (ev.payload_tag) {
      case NVE_EVENT_PAYLOAD_EMPTY:
        break;
      case NVE_EVENT_PAYLOAD_POINT:
        ok = c.read(ev.x) && c.read(ev.y);
        break;
      case NVE_EVENT_PAYLOAD_SCROLL:
      case NVE_EVENT_PAYLOAD_LAYOUT:
        ok = c.read(ev.x) && c.read(ev.y) && c.read(ev.width) && c.read(ev.height);
        break;
      case NVE_EVENT_PAYLOAD_TEXT:
        ok = c.utf8<uint32_t>(ev.text);
        break;
      case NVE_EVENT_PAYLOAD_CUSTOM: {
        NveStrSlice data{};
        ok = c.utf8<uint16_t>(ev.name) && c.prefixed<uint32_t>(data);
        if (ok && data.len > 0u) {
          ev.value.tag = NVE_VAL_BYTES;
          ev.value.data.bytes_val = NveBufferSlice{data.ptr, data.len};
        }
        break;
      }
      case NVE_EVENT_PAYLOAD_KEY: {
        // modifiers, is_composing, key_code, key (u16 + utf8) -> text, code (u16 + utf8) -> name
        uint8_t composing = 0u;
        ok = c.read(ev.modifiers) && c.read(composing) && c.read(ev.key_code) && c.utf8<uint16_t>(ev.text) &&
             c.utf8<uint16_t>(ev.name);
        ev.event_flags = composing != 0u ? NVE_EVENT_FLAG_COMPOSING : 0u;
        break;
      }
      case NVE_EVENT_PAYLOAD_POINTER:
        ok = c.read(ev.x) && c.read(ev.y) && c.read(ev.modifiers) && c.read(ev.event_flags);
        break;
      case NVE_EVENT_PAYLOAD_WHEEL:
        ok = c.read(ev.x) && c.read(ev.y) && c.read(ev.modifiers);
        break;
      case NVE_EVENT_PAYLOAD_TABLE_EVENT: {
        // table_key (u64) -> value i64, event_id -> key_code, payload (u32 + bytes) -> text
        uint64_t table_key = 0u;
        ok = c.read(table_key) && c.read(ev.key_code) && c.prefixed<uint32_t>(ev.text);
        ev.value.tag = NVE_VAL_I64;
        ev.value.data.i64_val = static_cast<int64_t>(table_key);
        break;
      }
      default:
        return false;
    }
    if (!ok) return false;
    out.push_back(ev);
  }
  return c.at == len;
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
  if (data == nullptr || len < 4 || len > env->GetDirectBufferCapacity(ring) ||
      !decode_ring(data, static_cast<size_t>(len), s->events)) {
    s->last_error = "malformed EventRing (direct buffer, exact length, known kinds / payload tags, UTF-8 strings required)";
    return NVE_RUNTIME_INVALID_ARG;
  }
  for (NveDirectEvent& ev : s->events) {
    if (ev.surface_id == 0u) ev.surface_id = s->surface_id;
  }
  release_scene(*s);
  bool changed = false;
  const NveRuntimeStatus st = s->session.dispatch_events(
      NveEventBatch{s->events.data(), static_cast<uint32_t>(s->events.size()), 0u}, &changed);
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
