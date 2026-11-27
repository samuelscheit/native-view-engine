// `dev.nve.host.NativeRuntime` lifecycle, app attach (hot reload), frames, viewport, tick,
// hit testing, scrolling, and pairing over `nve::host::RuntimeSession` (`libnve_runtime`).
#include "NveAndroidSurface.hpp"

#include <dlfcn.h>

#include <cmath>
#include <span>
#include <utility>

using nve::android::AndroidSurface;
using nve::android::changed_or_fail;
using nve::android::fail;
using nve::android::release_buffers;
using nve::android::release_scene;
using nve::android::surface_of;
using nve::android::utf8_of;

namespace nve::android {

void release_scene(AndroidSurface& s) noexcept { s.scene.reset(); }

void release_buffers(AndroidSurface& s, JNIEnv* env) noexcept {
  release_scene(s);
  for (CachedDirectBuffer& b : s.slabs) b.release(env);
  for (CachedDirectBuffer& b : s.resources) b.release(env);
  s.resources.clear();
}

jint fail(AndroidSurface& s, NveRuntimeStatus status) {
  std::string msg = s.session.last_error();
  s.last_error = msg.empty() ? "nve_runtime status " + std::to_string(static_cast<int>(status)) : std::move(msg);
  return static_cast<jint>(status);
}

jint changed_or_fail(AndroidSurface& s, NveRuntimeStatus status, bool changed) {
  if (status != NVE_RUNTIME_OK) return -fail(s, status);
  return changed ? 1 : 0;
}

std::string utf8_of(JNIEnv* env, jstring str) {
  std::string out;
  if (env == nullptr || str == nullptr) return out;
  const jsize n = env->GetStringLength(str);
  const jchar* chars = env->GetStringCritical(str, nullptr);
  if (chars == nullptr) return out;
  out.reserve(static_cast<size_t>(n));
  for (jsize i = 0; i < n; ++i) {
    uint32_t cp = chars[i];
    if (cp >= 0xD800u && cp <= 0xDBFFu && i + 1 < n && chars[i + 1] >= 0xDC00u && chars[i + 1] <= 0xDFFFu) {
      cp = 0x10000u + ((cp - 0xD800u) << 10u) + (chars[i + 1] - 0xDC00u);
      ++i;
    } else if (cp >= 0xD800u && cp <= 0xDFFFu) {
      cp = 0xFFFDu;  // lone surrogate: never emit invalid UTF-8
    }
    if (cp < 0x80u) {
      out.push_back(static_cast<char>(cp));
    } else if (cp < 0x800u) {
      out.push_back(static_cast<char>(0xC0u | (cp >> 6u)));
      out.push_back(static_cast<char>(0x80u | (cp & 0x3Fu)));
    } else if (cp < 0x10000u) {
      out.push_back(static_cast<char>(0xE0u | (cp >> 12u)));
      out.push_back(static_cast<char>(0x80u | ((cp >> 6u) & 0x3Fu)));
      out.push_back(static_cast<char>(0x80u | (cp & 0x3Fu)));
    } else {
      out.push_back(static_cast<char>(0xF0u | (cp >> 18u)));
      out.push_back(static_cast<char>(0x80u | ((cp >> 12u) & 0x3Fu)));
      out.push_back(static_cast<char>(0x80u | ((cp >> 6u) & 0x3Fu)));
      out.push_back(static_cast<char>(0x80u | (cp & 0x3Fu)));
    }
  }
  env->ReleaseStringCritical(str, chars);
  return out;
}

}  // namespace nve::android

namespace {

float positive_or(float v, float fallback) noexcept { return std::isfinite(v) && v > 0.0f ? v : fallback; }

void close_library(void*& lib) noexcept {
  if (lib != nullptr) {
    dlclose(lib);
    lib = nullptr;
  }
}

}  // namespace

extern "C" {

JNIEXPORT jlong JNICALL Java_dev_nve_host_NativeRuntime_create(JNIEnv* env, jclass, jint surface_id, jfloat width,
                                                              jfloat height, jfloat scale) {
  const NveRuntimeApi* api = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
  const uint32_t id = surface_id > 0 ? static_cast<uint32_t>(surface_id) : 1u;
  std::optional<nve::host::RuntimeSession> session = nve::host::RuntimeSession::create(
      api, nve::host::RuntimeSession::default_config(id, positive_or(width, 412.0f), positive_or(height, 915.0f),
                                                     positive_or(scale, 1.0f)));
  if (!session) return 0;
  auto* s = new AndroidSurface();
  s->session = std::move(*session);
  s->surface_id = id;
  if (env != nullptr) (void)env->GetJavaVM(&s->jvm);
  return reinterpret_cast<jlong>(s);
}

JNIEXPORT void JNICALL Java_dev_nve_host_NativeRuntime_destroy(JNIEnv* env, jclass, jlong handle) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return;
  release_buffers(*s, env);
  // Drain in-flight wake / measure calls before their Java targets go away.
  (void)s->session.clear_wake_callback();
  s->session.reset();
  close_library(s->app_library);
  s->wake.release(env);
  s->measurer.release(env);
  if (s->measure_out != nullptr && env != nullptr) env->DeleteGlobalRef(s->measure_out);
  delete s;
}

JNIEXPORT jstring JNICALL Java_dev_nve_host_NativeRuntime_lastError(JNIEnv* env, jclass, jlong handle) {
  const AndroidSurface* s = surface_of(handle);
  return env->NewStringUTF(s != nullptr ? s->last_error.c_str() : "");
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_attachApp(JNIEnv* env, jclass, jlong handle, jstring so_path) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  const std::string path = utf8_of(env, so_path);
  void* lib = path.empty() ? nullptr : dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
  if (lib == nullptr) {
    const char* why = dlerror();
    s->last_error = "dlopen(" + path + ") failed: " + (why != nullptr ? why : "empty path");
    return NVE_RUNTIME_NOT_FOUND;
  }
  auto get_iface = reinterpret_cast<NveGetInterfaceFn>(dlsym(lib, "nve_get_interface"));
  if (get_iface == nullptr) {
    dlclose(lib);
    s->last_error = path + " exports no nve_get_interface";
    return NVE_RUNTIME_NOT_FOUND;
  }
  const NveRuntimeStatus st = s->session.attach_app_interface(get_iface);
  if (st != NVE_RUNTIME_OK) {
    dlclose(lib);  // the previous app (if any) stays attached
    return fail(*s, st);
  }
  // The runtime dropped the previous app instance inside `attach`: its code is unreferenced now.
  close_library(s->app_library);
  s->app_library = lib;
  s->last_error.clear();
  return NVE_RUNTIME_OK;
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_detachApp(JNIEnv* env, jclass, jlong handle) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  const NveRuntimeStatus st = s->session.detach_app();
  if (st != NVE_RUNTIME_OK) return fail(*s, st);
  close_library(s->app_library);
  return NVE_RUNTIME_OK;
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_submitFrame(JNIEnv* env, jclass, jlong handle, jobject buf,
                                                                  jint len) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr || buf == nullptr || len < 0) return NVE_RUNTIME_INVALID_ARG;
  const auto* data = static_cast<const uint8_t*>(env->GetDirectBufferAddress(buf));
  if (data == nullptr || len > env->GetDirectBufferCapacity(buf)) {
    s->last_error = "submitFrame needs a direct buffer holding `length` bytes";
    return NVE_RUNTIME_INVALID_ARG;
  }
  release_scene(*s);
  const NveRuntimeStatus st = s->session.submit_nve2(std::span<const uint8_t>(data, static_cast<size_t>(len)));
  return st == NVE_RUNTIME_OK ? NVE_RUNTIME_OK : fail(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setViewport(JNIEnv*, jclass, jlong handle, jfloat width,
                                                                  jfloat height, jfloat scale) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  const NveRuntimeStatus st = s->session.set_viewport(width, height, scale);
  return st == NVE_RUNTIME_OK ? NVE_RUNTIME_OK : fail(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_tick(JNIEnv*, jclass, jlong handle, jfloat dt) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return -NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  bool changed = false;
  const NveRuntimeStatus st = s->session.tick(dt, &changed);
  return changed_or_fail(*s, st, changed);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_hitTest(JNIEnv*, jclass, jlong handle, jfloat x, jfloat y) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return -NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  uint32_t node = 0u;
  const NveRuntimeStatus st = s->session.hit_test(x, y, &node);
  if (st == NVE_RUNTIME_NOT_FOUND) return 0;
  return st == NVE_RUNTIME_OK ? static_cast<jint>(node) : -fail(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_scrollBy(JNIEnv* env, jclass, jlong handle, jint node_id,
                                                               jfloat dx, jfloat dy, jfloatArray out_xy) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  float xy[2] = {0.0f, 0.0f};
  const NveRuntimeStatus st = s->session.scroll_by(static_cast<uint32_t>(node_id), dx, dy, &xy[0], &xy[1]);
  if (st != NVE_RUNTIME_OK) return fail(*s, st);
  if (out_xy != nullptr && env->GetArrayLength(out_xy) >= 2) env->SetFloatArrayRegion(out_xy, 0, 2, xy);
  return NVE_RUNTIME_OK;
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_pairConnect(JNIEnv* env, jclass, jlong handle, jstring uri,
                                                                  jstring token) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  const NveRuntimeStatus st = s->session.pair_connect(utf8_of(env, uri), utf8_of(env, token));
  return st == NVE_RUNTIME_OK ? NVE_RUNTIME_OK : fail(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_pairDisconnect(JNIEnv* env, jclass, jlong handle) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  const NveRuntimeStatus st = s->session.pair_disconnect();
  return st == NVE_RUNTIME_OK ? NVE_RUNTIME_OK : fail(*s, st);
}

JNIEXPORT jboolean JNICALL Java_dev_nve_host_NativeRuntime_pairIsConnected(JNIEnv*, jclass, jlong handle) {
  const AndroidSurface* s = surface_of(handle);
  return s != nullptr && s->session.pair_is_connected() ? JNI_TRUE : JNI_FALSE;
}

}  // extern "C"
