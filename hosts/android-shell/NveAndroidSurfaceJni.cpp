#include "NveAndroidDirectChannel.hpp"
#include "NveAndroidSurfaceInternal.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <string>
#include <string_view>

namespace {

using namespace nve::android_internal;

void release_all_slab_buffers(AndroidSurfaceState* state, JNIEnv* env) noexcept {
  if (!state) return;
  for (auto& slot : state->slab_buffers) {
    slot.release(env);
  }
  state->slab_epoch_mask = 0u;
  state->meta_records.clear();
  state->meta_records.shrink_to_fit();
  state->meta_blob.clear();
  state->meta_blob.shrink_to_fit();
  state->meta_record_count = 0u;
  state->meta_blob_used_bytes = 0u;
  state->metadata_cache_frame = 0u;
  state->metadata_cache_valid = false;
  state->metadata_cache.clear();
  state->image_resize_modes.clear();
}

jobject return_cached_slab(AndroidSurfaceState* state, JNIEnv* env, SlabSlot slot) {
  if (env == nullptr) {
    return nullptr;
  }
  if (!state) {
    return env->NewDirectByteBuffer(&g_empty_slab_sentinel, 0);
  }
  CachedDirectBuffer& cached = state->slab_buffers[slot];
  if (cached.global_ref == nullptr) {
    (void)cached.sync(env, nullptr, 0u);
  }
  return cached.global_ref != nullptr ? env->NewLocalRef(cached.global_ref) : nullptr;
}

std::u16string utf8_to_utf16(std::string_view input) {
  std::u16string out;
  out.reserve(input.size());
  size_t i = 0;
  const size_t n = input.size();
  while (i < n) {
    const unsigned char c = static_cast<unsigned char>(input[i]);
    if (c < 0x80u) {
      out.push_back(static_cast<char16_t>(c));
      ++i;
      continue;
    }
    uint32_t cp = 0xFFFDu;
    size_t adv = 1;
    if ((c & 0xE0u) == 0xC0u && i + 1 < n) {
      const unsigned char c1 = static_cast<unsigned char>(input[i + 1]);
      if ((c1 & 0xC0u) == 0x80u) {
        cp = ((static_cast<uint32_t>(c & 0x1Fu) << 6) | static_cast<uint32_t>(c1 & 0x3Fu));
        adv = 2;
      }
    } else if ((c & 0xF0u) == 0xE0u && i + 2 < n) {
      const unsigned char c1 = static_cast<unsigned char>(input[i + 1]);
      const unsigned char c2 = static_cast<unsigned char>(input[i + 2]);
      if ((c1 & 0xC0u) == 0x80u && (c2 & 0xC0u) == 0x80u) {
        cp = ((static_cast<uint32_t>(c & 0x0Fu) << 12) |
              (static_cast<uint32_t>(c1 & 0x3Fu) << 6) |
              static_cast<uint32_t>(c2 & 0x3Fu));
        adv = 3;
      }
    } else if ((c & 0xF8u) == 0xF0u && i + 3 < n) {
      const unsigned char c1 = static_cast<unsigned char>(input[i + 1]);
      const unsigned char c2 = static_cast<unsigned char>(input[i + 2]);
      const unsigned char c3 = static_cast<unsigned char>(input[i + 3]);
      if ((c1 & 0xC0u) == 0x80u && (c2 & 0xC0u) == 0x80u && (c3 & 0xC0u) == 0x80u) {
        cp = ((static_cast<uint32_t>(c & 0x07u) << 18) |
              (static_cast<uint32_t>(c1 & 0x3Fu) << 12) |
              (static_cast<uint32_t>(c2 & 0x3Fu) << 6) |
              static_cast<uint32_t>(c3 & 0x3Fu));
        adv = 4;
      }
    }
    if (cp <= 0xFFFFu) {
      out.push_back(static_cast<char16_t>(cp));
    } else if (cp <= 0x10FFFFu) {
      const uint32_t u = cp - 0x10000u;
      out.push_back(static_cast<char16_t>(0xD800u + (u >> 10)));
      out.push_back(static_cast<char16_t>(0xDC00u + (u & 0x3FFu)));
    } else {
      out.push_back(static_cast<char16_t>(0xFFFDu));
    }
    i += adv;
  }
  return out;
}

float sanitize_positive(float value, float fallback) noexcept {
  return (std::isfinite(value) && value > 0.0f) ? value : fallback;
}

float sanitize_finite(float value) noexcept {
  return std::isfinite(value) ? value : 0.0f;
}

} // namespace

namespace nve::android_jni {

struct SurfaceHandleAccess {
  nve::NativeHostSession* session{nullptr};
  NveSurfaceConfig* config{nullptr};
  std::string* last_error{nullptr};
  JavaVM* jvm{nullptr};
};

SurfaceHandleAccess get_surface_access(jlong handle_ptr) noexcept {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) return {};
  return SurfaceHandleAccess{
      .session = &state->session,
      .config = &state->config,
      .last_error = &state->last_error,
      .jvm = state->jvm,
  };
}

void invalidate_surface_slabs(jlong handle_ptr, JNIEnv* env) noexcept {
  release_all_slab_buffers(reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env);
}

void set_surface_pair_wake(
    jlong handle_ptr,
    JNIEnv* env,
    jobject wake_global_ref,
    jmethodID wake_method) noexcept {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) return;
  if (state->pair_wake_target != nullptr && env != nullptr) {
    env->DeleteGlobalRef(state->pair_wake_target);
  }
  state->pair_wake_target = wake_global_ref;
  state->pair_wake_method = wake_method;
}

void release_surface_pair_wake(jlong handle_ptr, JNIEnv* env) noexcept {
  set_surface_pair_wake(handle_ptr, env, nullptr, nullptr);
}

} // namespace nve::android_jni

extern "C" JNIEXPORT jlong JNICALL
Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
    JNIEnv* env,
    jclass clazz,
    jint surface_id,
    jfloat width,
    jfloat height,
    jfloat scale_factor) {
  auto* session = new AndroidSurfaceSession();
  AndroidSurfaceState* state = session;
  state->config = NveSurfaceConfig{
      .surface_id = static_cast<uint32_t>(surface_id > 0 ? surface_id : 1),
      .width = sanitize_positive(width, 412.0f),
      .height = sanitize_positive(height, 915.0f),
      .scale_factor = sanitize_positive(scale_factor, 1.0f),
  };

  if (env != nullptr && clazz != nullptr && env->GetJavaVM(&state->jvm) == JNI_OK &&
      state->jvm != nullptr) {
    jmethodID mid = env->GetStaticMethodID(clazz, "measureTextDp", "(Ljava/lang/String;FF)[F");
    if (env->ExceptionCheck()) {
      env->ExceptionClear();
      mid = nullptr;
    }
    if (mid != nullptr) {
      state->activity_class = static_cast<jclass>(env->NewGlobalRef(clazz));
      state->measure_text_method = mid;
      JavaVM* jvm = state->jvm;
      jclass cls = state->activity_class;
      state->session.set_text_measurer(
          [jvm, cls, mid](std::string_view text, float font_size, float max_width)
              -> NveIntrinsicSize {
            if (text.empty()) {
              return NveIntrinsicSize{0.0f, 0.0f};
            }
            JNIEnv* cb_env = nullptr;
            if (jvm->GetEnv(reinterpret_cast<void**>(&cb_env), JNI_VERSION_1_6) != JNI_OK ||
                cb_env == nullptr) {
              const float fs = sanitize_positive(font_size, 16.0f);
              return NveIntrinsicSize{
                  static_cast<float>(text.size()) * fs * 0.6f + 4.0f, fs * 1.3f};
            }
            const std::u16string u16 = utf8_to_utf16(text);
            jstring jtext = cb_env->NewString(
                reinterpret_cast<const jchar*>(u16.data()), static_cast<jsize>(u16.size()));
            if (!jtext) {
              if (cb_env->ExceptionCheck()) cb_env->ExceptionClear();
              return NveIntrinsicSize{0.0f, 0.0f};
            }
            jfloatArray arr = static_cast<jfloatArray>(cb_env->CallStaticObjectMethod(
                cls, mid, jtext, static_cast<jfloat>(font_size), static_cast<jfloat>(max_width)));
            cb_env->DeleteLocalRef(jtext);
            if (cb_env->ExceptionCheck() || arr == nullptr) {
              if (cb_env->ExceptionCheck()) cb_env->ExceptionClear();
              if (arr != nullptr) cb_env->DeleteLocalRef(arr);
              return NveIntrinsicSize{0.0f, 0.0f};
            }
            jfloat dims[2] = {0.0f, 0.0f};
            if (cb_env->GetArrayLength(arr) >= 2) {
              cb_env->GetFloatArrayRegion(arr, 0, 2, dims);
            }
            cb_env->DeleteLocalRef(arr);
            return NveIntrinsicSize{
                std::max(0.0f, sanitize_finite(dims[0])),
                std::max(0.0f, sanitize_finite(dims[1])),
            };
          });
    }
  }

  return reinterpret_cast<jlong>(state);
}

extern "C" JNIEXPORT void JNICALL
Java_dev_nve_android_NveMainActivity_nativeDestroySurface(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* session = as_surface_session(handle_ptr);
  if (!session) {
    return;
  }
  AndroidSurfaceState* state = session;
  state->session.disconnect_paired();
  nve::android_jni::release_surface_pair_wake(handle_ptr, env);
  release_direct_scratch_buffers(session, env);
  release_all_slab_buffers(state, env);
  state->session.reset();
  if (state->activity_class != nullptr && env != nullptr) {
    env->DeleteGlobalRef(state->activity_class);
    state->activity_class = nullptr;
  }
  delete session;
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jstring so_path) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state || !so_path || !env) {
    return JNI_FALSE;
  }

  const char* raw_path = env->GetStringUTFChars(so_path, nullptr);
  if (!raw_path) {
    state->last_error = "Failed to read dylib path from Java string";
    return JNI_FALSE;
  }
  std::string path(raw_path);
  env->ReleaseStringUTFChars(so_path, raw_path);

  int err_code = 0;
  std::string err_msg;
  const bool ok = state->session.is_valid()
                      ? state->session.reload_dylib(path, &err_code, &err_msg)
                      : state->session.load_dylib(path, state->config, &err_code, &err_msg);
  if (!ok) {
    state->last_error =
        err_msg.empty() ? ("Failed to load dylib (code " + std::to_string(err_code) + ")")
                        : std::move(err_msg);
    return JNI_FALSE;
  }

  // Invalidate cached slab DirectByteBuffers after successful load/reload so Java never observes
  // a recycled heap address (ABA hazard) as an unchanged slab.
  release_all_slab_buffers(state, env);
  state->last_error.clear();
  return JNI_TRUE;
}

extern "C" JNIEXPORT jstring JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetLastError(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  return env->NewStringUTF(state ? state->last_error.c_str() : "");
}

extern "C" JNIEXPORT void JNICALL
Java_dev_nve_android_NveMainActivity_nativeResize(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jfloat width,
    jfloat height,
    jfloat scale_factor) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) {
    return;
  }
  if (!std::isfinite(width) || !std::isfinite(height) || width <= 0.0f || height <= 0.0f) {
    return;
  }
  const float scale = sanitize_positive(scale_factor, state->config.scale_factor);
  state->config.width = width;
  state->config.height = height;
  state->config.scale_factor = scale;
  state->session.set_surface_size(width, height, scale);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeDispatchPress(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint node_id,
    jfloat x,
    jfloat y) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!is_session_active(state) || node_id <= 0) {
    return 0;
  }
  const uint32_t dirty = state->session.dispatch_press(
      static_cast<uint32_t>(node_id),
      sanitize_finite(x),
      sanitize_finite(y),
      /*auto_render=*/true);
  (void)state->session.drain_events();
  return static_cast<jint>(dirty);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeDispatchTextChange(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint node_id,
    jstring text) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!is_session_active(state) || node_id <= 0) {
    return 0;
  }

  std::string utf8;
  if (text != nullptr && env != nullptr) {
    const char* raw = env->GetStringUTFChars(text, nullptr);
    if (raw != nullptr) {
      utf8.assign(raw);
      env->ReleaseStringUTFChars(text, raw);
    }
  }

  const NveEvent ev = nve::make_text_event(
      state->config.surface_id,
      static_cast<uint32_t>(node_id),
      nve::current_timestamp_us(),
      NVE_TEXT_INPUT_EVENT_CHANGE,
      static_cast<uint32_t>(utf8.size()));
  const uint32_t dirty = state->session.dispatch_events(
      nve::make_single_event_batch(
          ev,
          utf8.empty() ? nullptr : reinterpret_cast<const uint8_t*>(utf8.data()),
          static_cast<uint32_t>(utf8.size())),
      /*auto_render=*/true);
  (void)state->session.drain_events();
  return static_cast<jint>(dirty);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeDispatchCustomEvent(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint node_id,
    jstring name,
    jstring payload) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!is_session_active(state) || node_id <= 0 || name == nullptr || env == nullptr) {
    return 0;
  }

  std::string event_name;
  const char* raw_name = env->GetStringUTFChars(name, nullptr);
  if (raw_name != nullptr) {
    event_name.assign(raw_name);
    env->ReleaseStringUTFChars(name, raw_name);
  }
  if (event_name.empty()) {
    return 0;
  }

  std::string utf8_payload;
  if (payload != nullptr) {
    const char* raw_payload = env->GetStringUTFChars(payload, nullptr);
    if (raw_payload != nullptr) {
      utf8_payload.assign(raw_payload);
      env->ReleaseStringUTFChars(payload, raw_payload);
    }
  }

  nve::EventBatchBuilder builder;
  (void)builder.push_custom(
      state->config.surface_id,
      static_cast<uint32_t>(node_id),
      nve::current_timestamp_us(),
      event_name,
      utf8_payload.empty() ? nullptr : reinterpret_cast<const uint8_t*>(utf8_payload.data()),
      utf8_payload.size());
  const uint32_t dirty =
      state->session.dispatch_events(builder.as_batch(), /*auto_render=*/true);
  (void)state->session.drain_events();
  return static_cast<jint>(dirty);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeDispatchScroll(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint node_id,
    jfloat scroll_x,
    jfloat scroll_y) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!is_session_active(state) || node_id <= 0) {
    return 0;
  }
  const NveEvent ev = nve::make_scroll_event(
      state->config.surface_id,
      static_cast<uint32_t>(node_id),
      nve::current_timestamp_us(),
      sanitize_finite(scroll_x),
      sanitize_finite(scroll_y),
      0.0f,
      0.0f);
  const uint32_t dirty = state->session.dispatch_events(
      nve::make_single_event_batch(ev), /*auto_render=*/true);
  (void)state->session.drain_events();
  return static_cast<jint>(dirty);
}

/// Reads the live zero-copy `NveSceneView` directly from `FabricHost`, packs dirty/changed
/// accessibility/`TextInput`/`Image`/`SkiaCanvas` metadata into fixed-stride 64B `NveNodeMeta`
/// records + flat companion byte blob, updates cached JNI `GlobalRef` `DirectByteBuffer` slabs
/// ONLY when `(ptr, byte_len)` changes, and writes `[schema_hash, frame_id, node_count,
/// dirty_count, removed_count, text_slab_len, children_slab_len, meta_record_count,
/// meta_blob_used_bytes]` into `out_meta`.
extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jlongArray out_meta) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!is_session_active(state)) {
    return -1;
  }
  if (state->session.is_valid() && state->session.bridge()->frame_count() == 0) {
    (void)state->session.render(nullptr);
  } else if (state->session.is_paired()) {
    (void)state->session.poll_paired_frames(nullptr);
  }
  const NveSceneView view = state->session.scene_view();
  if (view.schema_hash != NVE_SCENE_SCHEMA_HASH) {
    state->last_error = "NveSceneView schema hash verification failed";
    return -2;
  }

  pack_dirty_metadata(state, view);

  uint32_t mask = 0u;
  if (state->slab_buffers[kSlabNodes].sync(
          env, view.nodes, static_cast<size_t>(view.node_count) * sizeof(NveRenderNode))) {
    mask |= (1u << kSlabNodes);
  }
  if (state->slab_buffers[kSlabText].sync(
          env, view.text_slab, static_cast<size_t>(view.text_slab_len))) {
    mask |= (1u << kSlabText);
  }
  if (state->slab_buffers[kSlabChildren].sync(
          env, view.children_slab, static_cast<size_t>(view.children_slab_len) * sizeof(uint32_t))) {
    mask |= (1u << kSlabChildren);
  }
  if (state->slab_buffers[kSlabDirtyIndices].sync(
          env, view.dirty_indices, static_cast<size_t>(view.dirty_count) * sizeof(uint32_t))) {
    mask |= (1u << kSlabDirtyIndices);
  }
  if (state->slab_buffers[kSlabRemovedIds].sync(
          env, view.removed_ids, static_cast<size_t>(view.removed_count) * sizeof(uint32_t))) {
    mask |= (1u << kSlabRemovedIds);
  }
  if (state->slab_buffers[kSlabMetaRecords].sync(
          env, state->meta_records.data(), state->meta_records.size() * sizeof(NveNodeMeta))) {
    mask |= (1u << kSlabMetaRecords);
  }
  if (state->slab_buffers[kSlabMetaBlob].sync(
          env, state->meta_blob.data(), state->meta_blob.size())) {
    mask |= (1u << kSlabMetaBlob);
  }
  state->slab_epoch_mask = mask;

  if (env != nullptr && out_meta != nullptr) {
    const jsize arr_len = env->GetArrayLength(out_meta);
    if (arr_len >= 7) {
      const jlong meta[9] = {
          static_cast<jlong>(view.schema_hash),
          static_cast<jlong>(view.frame_id),
          static_cast<jlong>(view.node_count),
          static_cast<jlong>(view.dirty_count),
          static_cast<jlong>(view.removed_count),
          static_cast<jlong>(view.text_slab_len),
          static_cast<jlong>(view.children_slab_len),
          static_cast<jlong>(state->meta_record_count),
          static_cast<jlong>(state->meta_blob_used_bytes),
      };
      env->SetLongArrayRegion(out_meta, 0, std::min<jsize>(arr_len, 9), meta);
    }
  }
  return static_cast<jint>(view.node_count);
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  return state ? static_cast<jint>(state->slab_epoch_mask) : 0;
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetImageResizeMode(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint node_id) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!is_session_active(state) || node_id <= 0) {
    return 0;
  }
  const NveSceneView view = state->session.scene_view();
  if (!state->metadata_cache_valid || state->metadata_cache_frame != view.frame_id) {
    pack_dirty_metadata(state, view);
  }
  const auto it = state->image_resize_modes.find(static_cast<uint32_t>(node_id));
  return it != state->image_resize_modes.end() ? static_cast<jint>(it->second) : 0;
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return return_cached_slab(reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabNodes);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return return_cached_slab(reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabText);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetChildrenSlabBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return return_cached_slab(reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabChildren);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return return_cached_slab(
      reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabDirtyIndices);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetRemovedIdsBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return return_cached_slab(
      reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabRemovedIds);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetMetaRecordsBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return return_cached_slab(
      reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabMetaRecords);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetMetaBlobBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return return_cached_slab(
      reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabMetaBlob);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_compose_NveJniBridge_nativeGetMetaRecordsBuffer(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr) {
  return return_cached_slab(
      reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabMetaRecords);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_compose_NveJniBridge_nativeGetMetaBlobBuffer(
    JNIEnv* env,
    jobject /*thiz*/,
    jlong handle_ptr) {
  return return_cached_slab(
      reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabMetaBlob);
}
