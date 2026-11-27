// A functional fake of the JNI surface the generated glue and `nve/host/jni.hpp` use (direct
// buffers, UTF-16 strings, byte arrays, exceptions), recording what Java would see, for tests
// that compile and *run* the glue without a JVM.
#pragma once

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>

#define JNIEXPORT
#define JNICALL
#define JNI_FALSE 0
#define JNI_TRUE 1

typedef uint8_t jboolean;
typedef int8_t jbyte;
typedef uint16_t jchar;
typedef int32_t jint;
typedef int64_t jlong;
typedef float jfloat;
typedef double jdouble;
typedef int32_t jsize;

struct _jobject {
  virtual ~_jobject() = default;
};
typedef _jobject* jobject;
struct _jclass : _jobject {};
typedef _jclass* jclass;
struct _jstring : _jobject {
  std::u16string units;
};
typedef _jstring* jstring;
struct _jbyteArray : _jobject {
  std::vector<jbyte> bytes;
};
typedef _jbyteArray* jbyteArray;

/// A `java.nio.ByteBuffer`: direct with `address`, or a heap buffer (`address == nullptr`,
/// capacity `-1`, as JNI reports it).
struct FakeByteBuffer : _jobject {
  void* address = nullptr;
  jlong capacity = -1;
};

struct JNIEnv {
  std::string pending;  // the pending exception's message ("" = none)
  int thrown = 0;
  _jclass exception_class;
  std::vector<_jobject*> locals;

  JNIEnv() = default;
  JNIEnv(const JNIEnv&) = delete;
  JNIEnv& operator=(const JNIEnv&) = delete;
  ~JNIEnv() {
    for (_jobject* o : locals) delete o;
  }

  jboolean ExceptionCheck() { return pending.empty() ? JNI_FALSE : JNI_TRUE; }
  void ExceptionClear() { pending.clear(); }
  jclass FindClass(const char*) { return &exception_class; }
  jint ThrowNew(jclass, const char* message) {
    pending = message;
    ++thrown;
    return 0;
  }
  void DeleteLocalRef(jobject) {}
  void* GetDirectBufferAddress(jobject buffer) { return static_cast<FakeByteBuffer*>(buffer)->address; }
  jlong GetDirectBufferCapacity(jobject buffer) { return static_cast<FakeByteBuffer*>(buffer)->capacity; }
  jbyteArray NewByteArray(jsize len) {
    auto* array = new _jbyteArray();
    array->bytes.resize(static_cast<size_t>(len));
    locals.push_back(array);
    return array;
  }
  void SetByteArrayRegion(jbyteArray array, jsize start, jsize len, const jbyte* src) {
    std::memcpy(array->bytes.data() + start, src, static_cast<size_t>(len));
  }
  jsize GetStringLength(jstring s) { return static_cast<jsize>(s->units.size()); }
  void GetStringRegion(jstring s, jsize start, jsize len, jchar* dst) {
    std::memcpy(dst, s->units.data() + start, static_cast<size_t>(len) * sizeof(jchar));
  }
  jstring NewString(const jchar* units, jsize len) {
    auto* s = new _jstring();
    s->units.assign(reinterpret_cast<const char16_t*>(units), static_cast<size_t>(len));
    locals.push_back(s);
    return s;
  }
};
