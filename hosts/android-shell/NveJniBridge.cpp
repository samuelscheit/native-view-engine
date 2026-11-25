#include <jni.h>
#include <dlfcn.h>
#include <string>
#include <vector>

#include "nve_abi.h"

namespace {

struct AndroidHostState {
  NveFabricHostHandle fabric_host{nullptr};
  void* dylib{nullptr};
  NveModule module{};
  NveAppHandle app{nullptr};
  uint32_t surface_id{1};
  uint64_t frame_id{1};
};

} // namespace

extern "C" JNIEXPORT jlong JNICALL
Java_dev_nve_host_NveNativeBridge_nativeCreateHost(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jint surface_id,
    jfloat width,
    jfloat height,
    jfloat scale_factor) {
  auto* state = new AndroidHostState();
  state->surface_id = static_cast<uint32_t>(surface_id);
  state->fabric_host = nve_fabric_host_create(NveSurfaceConfig{
      .surface_id = state->surface_id,
      .width = width,
      .height = height,
      .scale_factor = scale_factor,
  });
  return reinterpret_cast<jlong>(state);
}

extern "C" JNIEXPORT void JNICALL
Java_dev_nve_host_NveNativeBridge_nativeDestroyHost(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidHostState*>(handle_ptr);
  if (!state) return;
  if (state->app && state->module.surface && state->module.surface->destroy) {
    state->module.surface->destroy(state->app);
    state->app = nullptr;
  }
  state->module = NveModule{};
  if (state->dylib) {
    dlclose(state->dylib);
    state->dylib = nullptr;
  }
  if (state->fabric_host) {
    nve_fabric_host_destroy(state->fabric_host);
    state->fabric_host = nullptr;
  }
  delete state;
}

extern "C" JNIEXPORT jboolean JNICALL
Java_dev_nve_host_NveNativeBridge_nativeHotSwapApp(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jstring so_path) {
  auto* state = reinterpret_cast<AndroidHostState*>(handle_ptr);
  if (!state || !so_path) return JNI_FALSE;

  // Validate and open the replacement `.so` BEFORE tearing down the live app so a failed
  // reload leaves the currently running app and its state intact.
  const char* path = env->GetStringUTFChars(so_path, nullptr);
  if (!path) return JNI_FALSE;
  void* lib = dlopen(path, RTLD_NOW | RTLD_LOCAL);
  env->ReleaseStringUTFChars(so_path, path);
  if (!lib) return JNI_FALSE;

  auto get_iface = reinterpret_cast<NveGetInterfaceFn>(dlsym(lib, "nve_get_interface"));
  if (!get_iface) {
    dlclose(lib);
    return JNI_FALSE;
  }
  NveModule mod = nve_resolve_module(get_iface);
  if (!mod.surface || !mod.surface->create || !mod.surface->render) {
    dlclose(lib);
    return JNI_FALSE;
  }

  NveAppHandle next_app = mod.surface->create(state->surface_id);
  if (!next_app) {
    dlclose(lib);
    return JNI_FALSE;
  }

  std::vector<uint8_t> saved;
  if (state->app && state->module.dev && state->module.dev->snapshot_state) {
    NveBufferSlice s = state->module.dev->snapshot_state(state->app);
    if (s.ptr && s.len > 0) saved.assign(s.ptr, s.ptr + s.len);
  }
  if (state->app && state->module.surface && state->module.surface->destroy) {
    state->module.surface->destroy(state->app);
    state->app = nullptr;
  }
  if (state->dylib) {
    dlclose(state->dylib);
    state->dylib = nullptr;
  }

  state->dylib = lib;
  state->module = mod;
  state->app = next_app;
  if (!saved.empty() && mod.dev && mod.dev->restore_state) {
    mod.dev->restore_state(state->app, NveBufferSlice{saved.data(), saved.size()});
  }
  NveBufferSlice frame = mod.surface->render(state->app, state->frame_id++);
  NveFrameStats stats{};
  nve_fabric_host_submit_frame(state->fabric_host, frame, &stats);
  return JNI_TRUE;
}
