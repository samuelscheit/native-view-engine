#include <jni.h>

#include "nve_host_runtime.hpp"
#include "nve_scene_builder.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <string>
#include <string_view>

namespace {

struct AndroidSurfaceState {
  nve::NativeHostSession session;
  nve::SceneTableBuilder scene_builder;
  NveSurfaceConfig config{};
  uint64_t scene_frame_seq{0};
  std::string last_error;
  JavaVM* jvm{nullptr};
  jclass activity_class{nullptr};
  jmethodID measure_text_method{nullptr};
};

// Non-null sentinel passed to `NewDirectByteBuffer` with capacity 0 when a slab is empty,
// avoiding NULL address edge cases across JVMs / ART.
uint8_t g_empty_slab_sentinel = 0;

jobject make_direct_buffer(JNIEnv* env, const void* ptr, size_t byte_len) {
  if (env == nullptr) {
    return nullptr;
  }
  if (ptr == nullptr || byte_len == 0u) {
    return env->NewDirectByteBuffer(&g_empty_slab_sentinel, 0);
  }
  return env->NewDirectByteBuffer(const_cast<void*>(ptr), static_cast<jlong>(byte_len));
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

extern "C" JNIEXPORT jlong JNICALL
Java_dev_nve_android_NveMainActivity_nativeCreateSurface(
    JNIEnv* env,
    jclass clazz,
    jint surface_id,
    jfloat width,
    jfloat height,
    jfloat scale_factor) {
  auto* state = new AndroidSurfaceState();
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
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) {
    return;
  }
  state->scene_builder.clear();
  state->session.reset();
  if (state->activity_class != nullptr && env != nullptr) {
    env->DeleteGlobalRef(state->activity_class);
    state->activity_class = nullptr;
  }
  delete state;
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NveMainActivity_nativeLoadDylib(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jstring so_path) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state || !so_path) {
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
  if (!state || !state->session.is_valid() || node_id <= 0) {
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
  if (!state || !state->session.is_valid() || node_id <= 0) {
    return 0;
  }

  std::string utf8;
  if (text != nullptr) {
    const char* raw = env->GetStringUTFChars(text, nullptr);
    if (raw != nullptr) {
      utf8.assign(raw);
      env->ReleaseStringUTFChars(text, raw);
    }
  }

  auto* bridge = state->session.bridge();
  if (!bridge) {
    return 0;
  }
  nve::fabric_host_enqueue_text_change(
      bridge->fabric_host(),
      static_cast<uint32_t>(node_id),
      nve::current_timestamp_us(),
      utf8);
  const NveBufferSlice inbound = nve_fabric_host_drain_events(bridge->fabric_host());
  const uint32_t dirty = bridge->dispatch_native_events(inbound, /*auto_render=*/true);
  (void)state->session.drain_events();
  return static_cast<jint>(dirty);
}

/// Renders/snapshots the active surface directly into `SceneTableBuilder`'s zero-copy 64-byte
/// `NveRenderNode` slabs and writes `[schema_hash, frame_id, node_count, dirty_count,
/// removed_count, text_slab_len, children_slab_len]` into `out_meta`.
extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeRenderFrame(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jlongArray out_meta) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state || !state->session.is_valid()) {
    return -1;
  }
  if (state->session.bridge()->frame_count() == 0) {
    (void)state->session.render(nullptr);
  }
  const auto snapshots = state->session.snapshot_nodes();
  state->scene_builder.sync(++state->scene_frame_seq, snapshots);
  const NveSceneView view = state->scene_builder.view();
  if (!nve::SceneTableBuilder::verify_view_schema(view)) {
    state->last_error = "NveSceneView schema hash verification failed";
    return -2;
  }
  if (env != nullptr && out_meta != nullptr && env->GetArrayLength(out_meta) >= 7) {
    const jlong meta[7] = {
        static_cast<jlong>(view.schema_hash),
        static_cast<jlong>(view.frame_id),
        static_cast<jlong>(view.node_count),
        static_cast<jlong>(view.dirty_count),
        static_cast<jlong>(view.removed_count),
        static_cast<jlong>(view.text_slab_len),
        static_cast<jlong>(view.children_slab_len),
    };
    env->SetLongArrayRegion(out_meta, 0, 7, meta);
  }
  return static_cast<jint>(view.node_count);
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetNodesBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) {
    return make_direct_buffer(env, nullptr, 0);
  }
  const NveSceneView view = state->scene_builder.view();
  return make_direct_buffer(
      env, view.nodes, static_cast<size_t>(view.node_count) * sizeof(NveRenderNode));
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetTextSlabBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) {
    return make_direct_buffer(env, nullptr, 0);
  }
  const NveSceneView view = state->scene_builder.view();
  return make_direct_buffer(env, view.text_slab, static_cast<size_t>(view.text_slab_len));
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetChildrenSlabBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) {
    return make_direct_buffer(env, nullptr, 0);
  }
  const NveSceneView view = state->scene_builder.view();
  return make_direct_buffer(
      env, view.children_slab, static_cast<size_t>(view.children_slab_len) * sizeof(uint32_t));
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetDirtyIndicesBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) {
    return make_direct_buffer(env, nullptr, 0);
  }
  const NveSceneView view = state->scene_builder.view();
  return make_direct_buffer(
      env, view.dirty_indices, static_cast<size_t>(view.dirty_count) * sizeof(uint32_t));
}

extern "C" JNIEXPORT jobject JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetRemovedIdsBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state) {
    return make_direct_buffer(env, nullptr, 0);
  }
  const NveSceneView view = state->scene_builder.view();
  return make_direct_buffer(
      env, view.removed_ids, static_cast<size_t>(view.removed_count) * sizeof(uint32_t));
}
