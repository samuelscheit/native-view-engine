// NVE host runtime: JNI helpers of the generated Android hybrid glue (`nve codegen hybrid`). Only
// JNI code includes this header (it needs `<jni.h>`); every helper is CheckJNI-clean.
#pragma once

#include <jni.h>

#include <cstddef>
#include <cstdint>
#include <limits>
#include <optional>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>

#include "utf16.hpp"

namespace nve::host::jni {

/// The bytes of a direct `ByteBuffer`, borrowed for the call. Throws for a null or non-direct
/// buffer (only a valid empty buffer is empty).
inline std::span<const uint8_t> direct_buffer_span(JNIEnv* env, jobject buffer) {
  if (buffer == nullptr) throw std::invalid_argument("null ByteBuffer");
  void* ptr = env->GetDirectBufferAddress(buffer);
  const jlong capacity = env->GetDirectBufferCapacity(buffer);
  if (capacity < 0 || (capacity > 0 && ptr == nullptr)) throw std::invalid_argument("ByteBuffer is not direct");
  if (static_cast<unsigned long long>(capacity) > std::numeric_limits<size_t>::max()) {
    throw std::length_error("ByteBuffer exceeds the address space");
  }
  return {static_cast<const uint8_t*>(ptr), static_cast<size_t>(capacity)};
}

/// A new `byte[]` holding `data`, or `nullptr` with a pending Java exception.
inline jbyteArray to_jbyte_array(JNIEnv* env, std::span<const uint8_t> data) {
  if (data.size() > static_cast<size_t>(std::numeric_limits<jsize>::max())) {
    throw std::length_error("result exceeds a Java array");
  }
  const auto len = static_cast<jsize>(data.size());
  jbyteArray array = env->NewByteArray(len);
  if (array == nullptr || env->ExceptionCheck()) return nullptr;
  if (len > 0) env->SetByteArrayRegion(array, 0, len, reinterpret_cast<const jbyte*>(data.data()));
  return array;
}

/// A Java string as UTF-8, read as UTF-16 code units (never Modified UTF-8, so NUL and
/// supplementary characters survive). Throws for null and for a lone surrogate.
inline std::string utf8_of(JNIEnv* env, jstring s) {
  if (s == nullptr) throw std::invalid_argument("null String");
  const jsize len = env->GetStringLength(s);
  if (len <= 0) return {};
  std::u16string units(static_cast<size_t>(len), u'\0');
  env->GetStringRegion(s, 0, len, reinterpret_cast<jchar*>(units.data()));
  if (env->ExceptionCheck()) throw std::runtime_error("GetStringRegion failed");
  std::optional<std::string> utf8 = utf16_to_utf8(units);
  if (!utf8) throw std::invalid_argument("String holds a lone surrogate");
  return std::move(*utf8);
}

/// A new Java string of UTF-8 `s` (through UTF-16, never Modified UTF-8), or `nullptr` with a
/// pending Java exception. Throws when `s` is not UTF-8.
inline jstring java_string(JNIEnv* env, std::string_view s) {
  const std::optional<std::u16string> units = utf8_to_utf16(s);
  if (!units) throw std::runtime_error("result is not valid UTF-8");
  if (units->size() > static_cast<size_t>(std::numeric_limits<jsize>::max())) {
    throw std::length_error("result exceeds a Java string");
  }
  return env->NewString(reinterpret_cast<const jchar*>(units->data()), static_cast<jsize>(units->size()));
}

/// Raises `IllegalStateException(msg)` unless a Java exception is already pending (it wins).
/// Constructs the exception with a UTF-16 `jstring` (`NewString` + `<init>(String)`) so
/// supplementary UTF-8 characters and NUL never reach `ThrowNew`'s Modified UTF-8 parser; falls
/// back to ASCII `ThrowNew` if constructing the UTF-16 message or exception fails.
inline void throw_java_exception(JNIEnv* env, const char* msg) noexcept {
  if (env->ExceptionCheck()) return;
  jclass cls = env->FindClass("java/lang/IllegalStateException");
  if (cls == nullptr) return;  // `FindClass` raised (e.g. `OutOfMemoryError`)
  const char* text = msg != nullptr ? msg : "NVE hybrid call failed";
  bool thrown = false;
  try {
    jstring jmsg = java_string(env, text);
    if (jmsg != nullptr && !env->ExceptionCheck()) {
      jmethodID ctor = env->GetMethodID(cls, "<init>", "(Ljava/lang/String;)V");
      if (ctor != nullptr && !env->ExceptionCheck()) {
        jobject exc = env->NewObject(cls, ctor, jmsg);
        if (exc != nullptr && !env->ExceptionCheck()) {
          thrown = env->Throw(static_cast<jthrowable>(exc)) == 0;
        }
        if (exc != nullptr) env->DeleteLocalRef(exc);
      }
    }
    if (jmsg != nullptr) env->DeleteLocalRef(jmsg);
  } catch (...) {
    // Non-UTF-8 / oversized / OOM message: fall back to an ASCII `ThrowNew` below.
  }
  if (!thrown && !env->ExceptionCheck()) {
    env->ThrowNew(cls, "NVE hybrid call failed");
  }
  env->DeleteLocalRef(cls);
}

}  // namespace nve::host::jni
