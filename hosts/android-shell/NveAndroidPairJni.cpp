#include <jni.h>

#include "nve_host_runtime.hpp"

#include <string>

namespace nve::android_jni {

struct SurfaceHandleAccess {
  nve::NativeHostSession* session{nullptr};
  NveSurfaceConfig* config{nullptr};
  std::string* last_error{nullptr};
  JavaVM* jvm{nullptr};
};

SurfaceHandleAccess get_surface_access(jlong handle_ptr) noexcept;
void invalidate_surface_slabs(jlong handle_ptr, JNIEnv* env) noexcept;
void set_surface_pair_wake(
    jlong handle_ptr,
    JNIEnv* env,
    jobject wake_global_ref,
    jmethodID wake_method) noexcept;
void release_surface_pair_wake(jlong handle_ptr, JNIEnv* env) noexcept;

} // namespace nve::android_jni

namespace {

std::string read_jstring_utf8(JNIEnv* env, jstring str) {
  if (env == nullptr || str == nullptr) {
    return {};
  }
  const char* raw = env->GetStringUTFChars(str, nullptr);
  if (raw == nullptr) {
    return {};
  }
  std::string out(raw);
  env->ReleaseStringUTFChars(str, raw);
  return out;
}

} // namespace

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NvePairController_nativeConnectPaired(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jstring endpoint,
    jstring token,
    jstring transport,
    jobject wake_target) {
  const auto access = nve::android_jni::get_surface_access(handle_ptr);
  if (access.session == nullptr || access.config == nullptr || access.last_error == nullptr ||
      env == nullptr || endpoint == nullptr) {
    return JNI_FALSE;
  }

  const std::string endpoint_utf8 = read_jstring_utf8(env, endpoint);
  const std::string token_utf8 = read_jstring_utf8(env, token);
  const std::string transport_utf8 = read_jstring_utf8(env, transport);

  nve::PairClientConfig pair_cfg{};
  std::string parse_err;
  if (!nve::parse_pair_endpoint(endpoint_utf8, transport_utf8, pair_cfg, &parse_err)) {
    *access.last_error = std::move(parse_err);
    return JNI_FALSE;
  }
  pair_cfg.pairing_token = token_utf8;

  JavaVM* jvm = access.jvm;
  if (jvm == nullptr) {
    (void)env->GetJavaVM(&jvm);
  }

  jobject wake_global_ref = nullptr;
  jmethodID wake_method = nullptr;
  nve::PairClient::WakeCallback wake_cb;
  if (jvm != nullptr && wake_target != nullptr) {
    jclass target_cls = env->GetObjectClass(wake_target);
    if (target_cls != nullptr) {
      wake_method = env->GetMethodID(target_cls, "onNativePairFrameReady", "()V");
      if (env->ExceptionCheck()) {
        env->ExceptionClear();
        wake_method = nullptr;
      }
      env->DeleteLocalRef(target_cls);
    }
    if (wake_method != nullptr) {
      wake_global_ref = env->NewGlobalRef(wake_target);
      if (wake_global_ref != nullptr) {
        wake_cb = [jvm, wake_global_ref, wake_method]() {
          JNIEnv* cb_env = nullptr;
          bool did_attach = false;
          const jint get_rc = jvm->GetEnv(reinterpret_cast<void**>(&cb_env), JNI_VERSION_1_6);
          if (get_rc == JNI_EDETACHED) {
#if defined(__ANDROID__)
            if (jvm->AttachCurrentThread(&cb_env, nullptr) != JNI_OK || cb_env == nullptr) {
              return;
            }
#else
            if (jvm->AttachCurrentThread(reinterpret_cast<void**>(&cb_env), nullptr) != JNI_OK ||
                cb_env == nullptr) {
              return;
            }
#endif
            did_attach = true;
          } else if (get_rc != JNI_OK || cb_env == nullptr) {
            return;
          }

          cb_env->CallVoidMethod(wake_global_ref, wake_method);
          if (cb_env->ExceptionCheck()) {
            cb_env->ExceptionClear();
          }
          if (did_attach) {
            (void)jvm->DetachCurrentThread();
          }
        };
      }
    }
  }

  std::string connect_err;
  if (!access.session->connect_paired(pair_cfg, *access.config, std::move(wake_cb), &connect_err)) {
    if (wake_global_ref != nullptr) {
      env->DeleteGlobalRef(wake_global_ref);
    }
    *access.last_error =
        connect_err.empty() ? "Failed to connect paired thin-client" : std::move(connect_err);
    return JNI_FALSE;
  }

  // `connect_paired` already disconnected & joined any prior PairClient reader thread, so
  // swapping out any previous GlobalRef here is race-free.
  nve::android_jni::set_surface_pair_wake(handle_ptr, env, wake_global_ref, wake_method);
  nve::android_jni::invalidate_surface_slabs(handle_ptr, env);
  access.last_error->clear();
  return JNI_TRUE;
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NvePairController_nativePollPairedFrames(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr) {
  const auto access = nve::android_jni::get_surface_access(handle_ptr);
  if (access.session == nullptr || !access.session->is_paired()) {
    return 0;
  }
  return static_cast<jint>(access.session->poll_paired_frames(nullptr));
}

extern "C" JNIEXPORT void JNICALL
Java_dev_nve_android_NvePairController_nativeDisconnectPaired(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  const auto access = nve::android_jni::get_surface_access(handle_ptr);
  if (access.session == nullptr) {
    return;
  }
  access.session->disconnect_paired();
  nve::android_jni::release_surface_pair_wake(handle_ptr, env);
  nve::android_jni::invalidate_surface_slabs(handle_ptr, env);
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_android_NvePairController_nativeIsPaired(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr) {
  const auto access = nve::android_jni::get_surface_access(handle_ptr);
  return (access.session != nullptr && access.session->is_paired()) ? JNI_TRUE : JNI_FALSE;
}
