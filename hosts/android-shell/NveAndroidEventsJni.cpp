// `EventRing` (`nve_abi::encode_events`, written by `dev.nve.host.EventRingWriter`) → borrowed
// `NveDirectEvent` batch → `NveRuntimeApi::dispatch_events`, plus the host text measurer and wake
// listener callbacks into Java.
#include "NveAndroidSurface.hpp"

#include <cmath>
#include <cstring>

using nve::android::AndroidSurface;
using nve::android::changed_or_fail;
using nve::android::fail;
using nve::android::release_scene;
using nve::android::surface_of;

namespace {

static_assert(NVE_EVENT_PAYLOAD_EMPTY == 0 && NVE_EVENT_PAYLOAD_POINT == 1 && NVE_EVENT_PAYLOAD_SCROLL == 2 &&
                  NVE_EVENT_PAYLOAD_TEXT == 3 && NVE_EVENT_PAYLOAD_LAYOUT == 4 && NVE_EVENT_PAYLOAD_CUSTOM == 5 &&
                  NVE_EVENT_PAYLOAD_KEY == 6 && NVE_EVENT_PAYLOAD_POINTER == 7 && NVE_EVENT_PAYLOAD_WHEEL == 8,
              "EventRingWriter payload tags drifted from NveEventPayloadTag");

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
};

/// Decodes the whole ring into `out` (borrowing strings from `ring`); `false` on any malformed byte.
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
        !c.read(ev.payload_tag)) {
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
        ok = c.prefixed<uint32_t>(ev.text);
        break;
      case NVE_EVENT_PAYLOAD_CUSTOM: {
        NveStrSlice data{};
        ok = c.prefixed<uint16_t>(ev.name) && c.prefixed<uint32_t>(data);
        if (ok && data.len > 0u) {
          ev.value.tag = NVE_VAL_BYTES;
          ev.value.data.bytes_val = NveBufferSlice{data.ptr, data.len};
        }
        break;
      }
      case NVE_EVENT_PAYLOAD_KEY: {
        uint8_t composing = 0u;
        ok = c.prefixed<uint16_t>(ev.text) && c.prefixed<uint16_t>(ev.name) && c.read(ev.modifiers) &&
             c.read(ev.key_code) && c.read(composing);
        ev.event_flags = composing != 0u ? NVE_EVENT_FLAG_COMPOSING : 0u;
        break;
      }
      case NVE_EVENT_PAYLOAD_POINTER:
        ok = c.read(ev.x) && c.read(ev.y) && c.read(ev.event_flags) && c.read(ev.modifiers);
        break;
      case NVE_EVENT_PAYLOAD_WHEEL:
        ok = c.read(ev.x) && c.read(ev.y) && c.read(ev.modifiers);
        break;
      default:
        return false;
    }
    if (!ok) return false;
    out.push_back(ev);
  }
  return c.at == len;
}

/// Calls `callback(env)` with a JNI env for the current thread, attaching it for the call if needed.
template <class Fn>
void with_env(JavaVM* jvm, Fn&& callback) {
  JNIEnv* env = nullptr;
  const jint rc = jvm->GetEnv(reinterpret_cast<void**>(&env), JNI_VERSION_1_6);
  bool attached = false;
  if (rc == JNI_EDETACHED) {
#if defined(__ANDROID__)
    if (jvm->AttachCurrentThread(&env, nullptr) != JNI_OK) return;
#else
    if (jvm->AttachCurrentThread(reinterpret_cast<void**>(&env), nullptr) != JNI_OK) return;
#endif
    attached = true;
  } else if (rc != JNI_OK) {
    return;
  }
  callback(env);
  if (env->ExceptionCheck()) env->ExceptionClear();
  if (attached) (void)jvm->DetachCurrentThread();
}

void on_wake(void* ctx) {
  auto* s = static_cast<AndroidSurface*>(ctx);
  if (s->jvm == nullptr || s->wake.target == nullptr) return;
  with_env(s->jvm, [&](JNIEnv* env) { env->CallVoidMethod(s->wake.target, s->wake.method); });
}

/// UTF-8 → UTF-16 (`U+FFFD` for malformed input; the runtime only hands out valid UTF-8).
std::u16string utf16_of(const uint8_t* p, size_t n) {
  std::u16string out;
  out.reserve(n);
  for (size_t i = 0u; i < n;) {
    const uint8_t b = p[i];
    const size_t need = b < 0x80u ? 0u : (b >> 5u) == 0x6u ? 1u : (b >> 4u) == 0xEu ? 2u : (b >> 3u) == 0x1Eu ? 3u : 4u;
    if (need == 4u || n - i <= need) {
      out.push_back(b < 0x80u ? static_cast<char16_t>(b) : u'\uFFFD');
      i += 1u;
      continue;
    }
    uint32_t cp = need == 0u ? b : (b & (0x3Fu >> need));
    for (size_t k = 1u; k <= need; ++k) cp = (cp << 6u) | (p[i + k] & 0x3Fu);
    if (cp >= 0x10000u) {
      out.push_back(static_cast<char16_t>(0xD800u + ((cp - 0x10000u) >> 10u)));
      out.push_back(static_cast<char16_t>(0xDC00u + ((cp - 0x10000u) & 0x3FFu)));
    } else {
      out.push_back(static_cast<char16_t>(cp));
    }
    i += need + 1u;
  }
  return out;
}

int32_t on_measure(void* ctx, NveStrSlice text, float font_size, uint32_t font_weight, float max_width,
                   float* out_w, float* out_h) {
  auto* s = static_cast<AndroidSurface*>(ctx);
  int32_t status = 1;  // fall back to the heuristic unless Java measures
  if (s->jvm == nullptr || s->measurer.target == nullptr) return status;
  with_env(s->jvm, [&](JNIEnv* env) {
    const std::u16string u16 = utf16_of(text.ptr, text.len);
    jstring jtext = env->NewString(reinterpret_cast<const jchar*>(u16.data()), static_cast<jsize>(u16.size()));
    if (jtext == nullptr) return;
    const jboolean ok = env->CallBooleanMethod(s->measurer.target, s->measurer.method, jtext, font_size,
                                               static_cast<jint>(font_weight), max_width, s->measure_out);
    env->DeleteLocalRef(jtext);
    if (env->ExceptionCheck() || ok != JNI_TRUE) return;
    float wh[2] = {NAN, NAN};
    env->GetFloatArrayRegion(s->measure_out, 0, 2, wh);
    *out_w = wh[0];
    *out_h = wh[1];
    status = 0;
  });
  return status;
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
    if (env->ExceptionCheck()) env->ExceptionClear();
    return false;
  }
  slot.target = env->NewGlobalRef(target);
  slot.method = mid;
  return slot.target != nullptr;
}

}  // namespace

extern "C" {

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_dispatchEvents(JNIEnv* env, jclass, jlong handle,
                                                                     jobject ring, jint len) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return -NVE_RUNTIME_INVALID_ARG;
  const auto* data = ring != nullptr ? static_cast<const uint8_t*>(env->GetDirectBufferAddress(ring)) : nullptr;
  if (data == nullptr || len < 4 || len > env->GetDirectBufferCapacity(ring) ||
      !decode_ring(data, static_cast<size_t>(len), s->events)) {
    s->last_error = "malformed EventRing (direct buffer, exact length, known payload tags required)";
    return -NVE_RUNTIME_INVALID_ARG;
  }
  for (NveDirectEvent& ev : s->events) {
    if (ev.surface_id == 0u) ev.surface_id = s->surface_id;
  }
  release_scene(*s);
  bool changed = false;
  const NveRuntimeStatus st = s->session.dispatch_events(
      NveEventBatch{s->events.data(), static_cast<uint32_t>(s->events.size()), 0u}, &changed);
  return changed_or_fail(*s, st, changed);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setTextMeasurer(JNIEnv* env, jclass, jlong handle,
                                                                      jobject measurer) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  // Detach the runtime from the old target before dropping its global ref.
  NveRuntimeStatus st = s->session.set_text_measurer(nullptr, nullptr);
  if (st != NVE_RUNTIME_OK) return fail(*s, st);
  if (!bind_callback(env, s->measurer, measurer, "measure", "(Ljava/lang/String;FIF[F)Z")) {
    s->last_error = "TextMeasurer has no measure(String, float, int, float, float[]) method";
    return NVE_RUNTIME_INVALID_ARG;
  }
  if (measurer == nullptr) return NVE_RUNTIME_OK;
  if (s->measure_out == nullptr) {
    jfloatArray local = env->NewFloatArray(2);
    s->measure_out = local != nullptr ? static_cast<jfloatArray>(env->NewGlobalRef(local)) : nullptr;
    if (local != nullptr) env->DeleteLocalRef(local);
    if (s->measure_out == nullptr) return NVE_RUNTIME_ERROR;
  }
  st = s->session.set_text_measurer(&on_measure, s);
  return st == NVE_RUNTIME_OK ? NVE_RUNTIME_OK : fail(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setWakeListener(JNIEnv* env, jclass, jlong handle,
                                                                      jobject listener) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  // Clearing drains in-flight wakes, so the old global ref is never used after release.
  NveRuntimeStatus st = s->session.clear_wake_callback();
  if (st != NVE_RUNTIME_OK) return fail(*s, st);
  if (!bind_callback(env, s->wake, listener, "run", "()V")) {
    s->last_error = "wake listener has no run() method";
    return NVE_RUNTIME_INVALID_ARG;
  }
  if (listener == nullptr) return NVE_RUNTIME_OK;
  st = s->session.set_wake_callback(&on_wake, s);
  return st == NVE_RUNTIME_OK ? NVE_RUNTIME_OK : fail(*s, st);
}

}  // extern "C"
