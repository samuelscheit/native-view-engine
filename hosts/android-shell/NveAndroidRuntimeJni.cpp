// `dev.nve.host.NativeRuntime` lifecycle, app attach (hot reload), frames, viewport, tick,
// hit testing, scrolling, and pairing over `nve::host::RuntimeSession` (`libnve_runtime`).
#include "NveAndroidSurface.hpp"

#include <cmath>
#include <limits>
#include <span>
#include <utility>

using nve::android::AndroidSurface;
using nve::android::clear_exception;
using nve::android::direct_buffer_span;
using nve::android::release_buffers;
using nve::android::release_scene;
using nve::android::status_of;
using nve::android::owned_surface;
using nve::android::surface_of;
using nve::android::utf8_of;
using nve::android::write_out;

namespace nve::android {

void release_scene(AndroidSurface& s) noexcept { s.scene.reset(); }

void release_buffers(AndroidSurface& s, JNIEnv* env) noexcept {
  release_scene(s);
  for (CachedDirectBuffer& b : s.slabs) b.release(env);
}

jint status_of(AndroidSurface& s, NveRuntimeStatus status) {
  if (status == NVE_RUNTIME_OK) {
    s.last_error.clear();
    return NVE_RUNTIME_OK;
  }
  std::string msg = s.session.last_error();
  s.last_error = msg.empty() ? "nve_runtime status " + std::to_string(static_cast<int>(status)) : std::move(msg);
  return static_cast<jint>(status);
}

void write_out(JNIEnv* env, jintArray out, jint value) {
  if (out != nullptr && env->GetArrayLength(out) >= 1) env->SetIntArrayRegion(out, 0, 1, &value);
}

jstring java_string(JNIEnv* env, const uint8_t* p, size_t n) {
  std::u16string out;
  out.reserve(n);
  for (size_t i = 0u; i < n;) {
    const uint8_t b = p[i];
    const size_t need = b < 0x80u ? 0u : (b >> 5u) == 0x6u ? 1u : (b >> 4u) == 0xEu ? 2u : (b >> 3u) == 0x1Eu ? 3u : 4u;
    bool ok = need < 4u && n - i > need;
    uint32_t cp = need == 0u ? b : (b & (0x3Fu >> need));
    for (size_t k = 1u; ok && k <= need; ++k) {
      ok = (p[i + k] & 0xC0u) == 0x80u;
      cp = (cp << 6u) | (p[i + k] & 0x3Fu);
    }
    // Overlong forms, surrogates, and values past U+10FFFF are malformed too.
    static constexpr uint32_t kMin[4] = {0u, 0x80u, 0x800u, 0x10000u};
    if (ok && need > 0u) ok = cp >= kMin[need] && cp <= 0x10FFFFu && (cp < 0xD800u || cp > 0xDFFFu);
    if (!ok) {
      out.push_back(u'\uFFFD');
      i += 1u;
      continue;
    }
    if (cp >= 0x10000u) {
      out.push_back(static_cast<char16_t>(0xD800u + ((cp - 0x10000u) >> 10u)));
      out.push_back(static_cast<char16_t>(0xDC00u + ((cp - 0x10000u) & 0x3FFu)));
    } else {
      out.push_back(static_cast<char16_t>(cp));
    }
    i += need + 1u;
  }
  jstring str = env->NewString(reinterpret_cast<const jchar*>(out.data()), static_cast<jsize>(out.size()));
  if (str == nullptr) clear_exception(env);
  return str;
}

std::string utf8_of(JNIEnv* env, jstring str) {
  std::string out;
  if (env == nullptr || str == nullptr) return out;
  const jsize n = env->GetStringLength(str);
  // A plain copy, not `GetStringCritical`: no GC pinning, and allocation below is legal.
  std::u16string units(static_cast<size_t>(n), u'\0');
  if (n > 0) env->GetStringRegion(str, 0, n, reinterpret_cast<jchar*>(units.data()));
  if (clear_exception(env)) return out;
  const char16_t* chars = units.data();
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
  return out;
}

}  // namespace nve::android

namespace {

float positive_or(float v, float fallback) noexcept { return std::isfinite(v) && v > 0.0f ? v : fallback; }

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

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_destroy(JNIEnv* env, jclass, jlong handle) {
  // Off the owner thread freeing the surface (and the app it runs) would be a use-after-free:
  // refused and leaked instead.
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  // Drain in-flight wake / intrinsic-size / host / measure calls before their Java targets go
  // away; the runtime drops the app and unloads its module (`load_app_module`) in `reset`.
  (void)s->session.clear_wake_callback();
  (void)s->session.clear_intrinsic_size_invalidated_callback();
  (void)s->session.clear_host_callbacks();
  s->session.reset();
  s->wake.release(env);
  s->intrinsic_size_invalidated.release(env);
  s->host_bridge.release(env);
  s->measurer.release(env);  // the runtime (and its measurer hook) is gone: no call can race this
  if (s->measure_out != nullptr && env != nullptr) env->DeleteGlobalRef(s->measure_out);
  delete s;
  return NVE_RUNTIME_OK;
}

JNIEXPORT jstring JNICALL Java_dev_nve_host_NativeRuntime_lastError(JNIEnv* env, jclass, jlong handle) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return nve::android::java_string(env, nullptr, 0);
  if (const char* refused = s->refused.exchange(nullptr, std::memory_order_relaxed)) s->last_error = refused;
  const std::string& msg = s->last_error;
  return nve::android::java_string(env, reinterpret_cast<const uint8_t*>(msg.data()), msg.size());
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_attachApp(JNIEnv* env, jclass, jlong handle, jstring so_path,
                                                                jboolean carry_state) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  // The runtime owns the module (`load_app_module`): it unloads an old module only after its app
  // is destroyed, so no host-side `dlclose` can ever race a live app.
  return status_of(*s, s->session.load_app_module(utf8_of(env, so_path), carry_state == JNI_TRUE));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_detachApp(JNIEnv* env, jclass, jlong handle) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  return status_of(*s, s->session.detach_app());
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_submitFrame(JNIEnv* env, jclass, jlong handle, jobject buf,
                                                                  jint len) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr || buf == nullptr || len < 0) return NVE_RUNTIME_INVALID_ARG;
  const auto* data = static_cast<const uint8_t*>(env->GetDirectBufferAddress(buf));
  if (data == nullptr || len > env->GetDirectBufferCapacity(buf)) {
    s->last_error = "submitFrame needs a direct buffer holding `length` bytes";
    return NVE_RUNTIME_INVALID_ARG;
  }
  release_scene(*s);
  const NveRuntimeStatus st = s->session.submit_nve2(std::span<const uint8_t>(data, static_cast<size_t>(len)));
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_setViewport(JNIEnv*, jclass, jlong handle, jfloat width,
                                                                  jfloat height, jfloat scale) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  const NveRuntimeStatus st = s->session.set_viewport(width, height, scale);
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_tick(JNIEnv* env, jclass, jlong handle, jfloat dt,
                                                           jintArray out_changed) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  bool changed = false;
  const NveRuntimeStatus st = s->session.tick(std::isfinite(dt) && dt > 0.0f ? dt : 0.0f, &changed);
  write_out(env, out_changed, changed ? 1 : 0);
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_hitTest(JNIEnv* env, jclass, jlong handle, jfloat x, jfloat y,
                                                              jintArray out_node) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  uint32_t node = 0u;
  NveRuntimeStatus st = std::isfinite(x) && std::isfinite(y) ? s->session.hit_test(x, y, &node) : NVE_RUNTIME_NOT_FOUND;
  if (st == NVE_RUNTIME_NOT_FOUND) {
    node = 0u;  // a miss is not an error
    st = NVE_RUNTIME_OK;
  }
  write_out(env, out_node, static_cast<jint>(node));
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_scrollBy(JNIEnv* env, jclass, jlong handle, jint node_id,
                                                               jfloat dx, jfloat dy, jfloatArray out_xy) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  float xy[2] = {0.0f, 0.0f};
  if (!std::isfinite(dx) || !std::isfinite(dy)) {
    s->last_error = "scrollBy needs finite deltas";
    return NVE_RUNTIME_INVALID_ARG;
  }
  const NveRuntimeStatus st = s->session.scroll_by(static_cast<uint32_t>(node_id), dx, dy, &xy[0], &xy[1]);
  if (st == NVE_RUNTIME_OK && out_xy != nullptr && env->GetArrayLength(out_xy) >= 2) {
    env->SetFloatArrayRegion(out_xy, 0, 2, xy);
  }
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_pairConnect(JNIEnv* env, jclass, jlong handle, jstring uri,
                                                                  jstring token) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  const NveRuntimeStatus st = s->session.pair_connect(utf8_of(env, uri), utf8_of(env, token));
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_pairDisconnect(JNIEnv* env, jclass, jlong handle) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  const NveRuntimeStatus st = s->session.pair_disconnect();
  return status_of(*s, st);
}

JNIEXPORT jboolean JNICALL Java_dev_nve_host_NativeRuntime_pairIsConnected(JNIEnv*, jclass, jlong handle) {
  // Any thread (`RuntimeSurface.isPaired`): a read-only query the runtime serves thread-safely.
  const AndroidSurface* s = surface_of(handle);
  return s != nullptr && s->session.pair_is_connected() ? JNI_TRUE : JNI_FALSE;
}

JNIEXPORT jbyteArray JNICALL Java_dev_nve_host_NativeRuntime_snapshotState(JNIEnv* env, jclass, jlong handle,
                                                                           jintArray out_status) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) {
    write_out(env, out_status, NVE_RUNTIME_INVALID_ARG);
    return nullptr;
  }
  release_scene(*s);
  std::span<const uint8_t> state;
  const NveRuntimeStatus st = s->session.snapshot_state(&state);
  const jint code = status_of(*s, st);
  write_out(env, out_status, code);
  if (st != NVE_RUNTIME_OK) return nullptr;
  if (state.size() > static_cast<size_t>(std::numeric_limits<jsize>::max())) {
    s->last_error = "snapshotState payload exceeds JNI array limit";
    write_out(env, out_status, NVE_RUNTIME_ERROR);
    return nullptr;
  }
  jbyteArray out = env->NewByteArray(static_cast<jsize>(state.size()));
  if (out == nullptr) {
    clear_exception(env);
    s->last_error = "out of memory allocating snapshot byte array";
    write_out(env, out_status, NVE_RUNTIME_ERROR);
    return nullptr;
  }
  if (!state.empty()) {
    env->SetByteArrayRegion(out, 0, static_cast<jsize>(state.size()),
                            reinterpret_cast<const jbyte*>(state.data()));
  }
  return out;
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_restoreState(JNIEnv* env, jclass, jlong handle, jobject buf,
                                                                    jint len) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  std::span<const uint8_t> state;
  if (!direct_buffer_span(env, buf, len, &state)) {
    s->last_error = "restoreState needs a direct buffer holding `length` bytes";
    return NVE_RUNTIME_INVALID_ARG;
  }
  release_scene(*s);
  return status_of(*s, s->session.restore_state(state));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_saveStateFile(JNIEnv* env, jclass, jlong handle, jstring path,
                                                                     jint schema_version) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr || path == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  return status_of(*s, s->session.save_state_file(utf8_of(env, path), static_cast<uint32_t>(schema_version)));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_restoreStateFile(JNIEnv* env, jclass, jlong handle, jstring path,
                                                                        jint schema_version) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr || path == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  return status_of(*s, s->session.restore_state_file(utf8_of(env, path), static_cast<uint32_t>(schema_version)));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_clearStateFile(JNIEnv* env, jclass, jlong handle, jstring path) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr || path == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  return status_of(*s, s->session.clear_state_file(utf8_of(env, path)));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_saveFrame0File(JNIEnv* env, jclass, jlong handle, jstring path,
                                                                      jlong app_build_id, jlong env_hash) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr || path == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  return status_of(*s, s->session.save_frame0_file(utf8_of(env, path), static_cast<uint64_t>(app_build_id),
                                                   static_cast<uint64_t>(env_hash)));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_bootFrame0File(JNIEnv* env, jclass, jlong handle, jstring path,
                                                                      jlong app_build_id, jlong env_hash) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr || path == nullptr) return NVE_RUNTIME_INVALID_ARG;
  release_buffers(*s, env);
  return status_of(*s, s->session.boot_frame0_file(utf8_of(env, path), static_cast<uint64_t>(app_build_id),
                                                   static_cast<uint64_t>(env_hash)));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_bootFrame0Bytes(JNIEnv* env, jclass, jlong handle, jobject buf,
                                                                       jint len, jlong app_build_id, jlong env_hash) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  std::span<const uint8_t> envelope;
  if (!direct_buffer_span(env, buf, len, &envelope)) {
    s->last_error = "bootFrame0Bytes needs a direct buffer holding `length` bytes";
    return NVE_RUNTIME_INVALID_ARG;
  }
  release_buffers(*s, env);
  return status_of(*s, s->session.boot_frame0_bytes(envelope, static_cast<uint64_t>(app_build_id),
                                                    static_cast<uint64_t>(env_hash)));
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_applyHotPatch(JNIEnv* env, jclass, jlong handle, jobject buf,
                                                                     jint len, jintArray out_changed) {
  AndroidSurface* s = owned_surface(handle);
  write_out(env, out_changed, 0);
  if (s == nullptr) return NVE_RUNTIME_INVALID_ARG;
  std::span<const uint8_t> patch;
  if (!direct_buffer_span(env, buf, len, &patch)) {
    s->last_error = "applyHotPatch needs a direct buffer holding `length` bytes";
    return NVE_RUNTIME_INVALID_ARG;
  }
  release_scene(*s);
  bool changed = false;
  const NveRuntimeStatus st = s->session.apply_hot_patch(patch, &changed);
  write_out(env, out_changed, changed ? 1 : 0);
  return status_of(*s, st);
}

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_measureSurface(JNIEnv* env, jclass, jlong handle, jfloat min_w,
                                                                      jfloat min_h, jfloat max_w, jfloat max_h,
                                                                      jint flags, jfloatArray out_measure) {
  AndroidSurface* s = owned_surface(handle);
  if (s == nullptr || out_measure == nullptr || env->GetArrayLength(out_measure) < 4) return NVE_RUNTIME_INVALID_ARG;
  release_scene(*s);
  const NveSizeConstraints constraints{min_w, min_h, max_w, max_h};
  NveSurfaceMeasure measured{};
  const NveRuntimeStatus st = s->session.measure_surface(constraints, static_cast<uint32_t>(flags), &measured);
  if (st == NVE_RUNTIME_OK) {
    const float out[4] = {measured.width, measured.height, measured.first_baseline, measured.last_baseline};
    env->SetFloatArrayRegion(out_measure, 0, 4, out);
  }
  return status_of(*s, st);
}

}  // extern "C"
