#include <jni.h>

#include "nve_host_runtime.hpp"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <string_view>
#include <vector>

namespace {

struct AndroidSurfaceState {
  nve::NativeHostSession session;
  NveSurfaceConfig config{};
  std::string last_error;
  JavaVM* jvm{nullptr};
  jclass activity_class{nullptr};
  jmethodID measure_text_method{nullptr};
};

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

void append_hex4(std::string& out, uint16_t code_unit) {
  static constexpr char kHex[] = "0123456789abcdef";
  out.push_back('\\');
  out.push_back('u');
  out.push_back(kHex[(code_unit >> 12) & 0xFu]);
  out.push_back(kHex[(code_unit >> 8) & 0xFu]);
  out.push_back(kHex[(code_unit >> 4) & 0xFu]);
  out.push_back(kHex[code_unit & 0xFu]);
}

/// Append a JSON-escaped ASCII string so JNI `NewStringUTF` always receives valid Modified UTF-8
/// even when the source UTF-8 contains supplementary plane code points (e.g. emoji > U+FFFF).
void append_json_string(std::string& out, std::string_view input) {
  out.push_back('"');
  size_t i = 0;
  const size_t n = input.size();
  while (i < n) {
    const unsigned char c = static_cast<unsigned char>(input[i]);
    if (c == '"') {
      out.append("\\\"");
      ++i;
    } else if (c == '\\') {
      out.append("\\\\");
      ++i;
    } else if (c == '\b') {
      out.append("\\b");
      ++i;
    } else if (c == '\f') {
      out.append("\\f");
      ++i;
    } else if (c == '\n') {
      out.append("\\n");
      ++i;
    } else if (c == '\r') {
      out.append("\\r");
      ++i;
    } else if (c == '\t') {
      out.append("\\t");
      ++i;
    } else if (c < 0x20u) {
      append_hex4(out, static_cast<uint16_t>(c));
      ++i;
    } else if (c < 0x80u) {
      out.push_back(static_cast<char>(c));
      ++i;
    } else {
      // Decode UTF-8 scalar value and emit \uXXXX (or UTF-16 surrogate pair for > U+FFFF).
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
        append_hex4(out, static_cast<uint16_t>(cp));
      } else if (cp <= 0x10FFFFu) {
        const uint32_t u = cp - 0x10000u;
        const uint16_t high = static_cast<uint16_t>(0xD800u + (u >> 10));
        const uint16_t low = static_cast<uint16_t>(0xDC00u + (u & 0x3FFu));
        append_hex4(out, high);
        append_hex4(out, low);
      } else {
        append_hex4(out, 0xFFFDu);
      }
      i += adv;
    }
  }
  out.push_back('"');
}

void append_float(std::string& out, float value) {
  char buf[32];
  const float safe = sanitize_finite(value);
  const int len = std::snprintf(buf, sizeof(buf), "%.3f", static_cast<double>(safe));
  if (len > 0) {
    out.append(buf, static_cast<size_t>(len));
  } else {
    out.push_back('0');
  }
}

std::string serialize_snapshots_json(const std::vector<nve::RenderedNodeSnapshot>& snapshots) {
  std::string out;
  out.reserve(snapshots.size() * 192u + 8u);
  out.push_back('[');
  for (size_t i = 0; i < snapshots.size(); ++i) {
    const auto& snap = snapshots[i];
    if (i > 0) {
      out.push_back(',');
    }
    out.append("{\"id\":");
    out.append(std::to_string(snap.node_id));
    out.append(",\"kind\":");
    out.append(std::to_string(snap.kind));
    out.append(",\"text\":");
    append_json_string(out, snap.text);
    out.append(",\"x\":");
    append_float(out, snap.layout.x);
    out.append(",\"y\":");
    append_float(out, snap.layout.y);
    out.append(",\"w\":");
    append_float(out, std::max(0.0f, sanitize_finite(snap.layout.width)));
    out.append(",\"h\":");
    append_float(out, std::max(0.0f, sanitize_finite(snap.layout.height)));
    out.append(",\"bg\":");
    out.append(std::to_string(snap.background_rgba));
    out.append(",\"fg\":");
    out.append(std::to_string(snap.text_color_rgba));
    out.append(",\"bc\":");
    out.append(std::to_string(snap.border_color_rgba));
    out.append(",\"bw\":");
    append_float(out, std::max(0.0f, sanitize_finite(snap.border_width)));
    out.append(",\"br\":");
    append_float(out, std::max(0.0f, sanitize_finite(snap.border_radius)));
    out.append(",\"op\":");
    append_float(out, std::clamp(sanitize_finite(snap.opacity), 0.0f, 1.0f));
    out.append(",\"fs\":");
    append_float(out, sanitize_positive(snap.font_size, 16.0f));
    out.append(",\"z\":");
    out.append(std::to_string(snap.z_index));
    out.append(",\"em\":");
    out.append(std::to_string(snap.event_mask));
    out.append(",\"ch\":[");
    for (size_t c = 0; c < snap.children.size(); ++c) {
      if (c > 0) {
        out.push_back(',');
      }
      out.append(std::to_string(snap.children[c]));
    }
    out.append("]}");
  }
  out.push_back(']');
  return out;
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

extern "C" JNIEXPORT jstring JNICALL
Java_dev_nve_android_NveMainActivity_nativeRenderAndSnapshotJson(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state || !state->session.is_valid()) {
    return env->NewStringUTF("[]");
  }
  if (state->session.bridge()->frame_count() == 0) {
    (void)state->session.render(nullptr);
  }
  const auto snapshots = state->session.snapshot_nodes();
  const std::string json = serialize_snapshots_json(snapshots);
  return env->NewStringUTF(json.c_str());
}
