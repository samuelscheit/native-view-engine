// Packs an acquired `NveRuntimeSceneView` into `dev.nve.host.RuntimeSceneReader`'s frame
// (`long[] header` + `ByteBuffer[] slabs`) with cached zero-copy `DirectByteBuffer`s, and serves
// published resource bytes.
#include "NveAndroidSurface.hpp"


using nve::android::AndroidSurface;
using nve::android::CachedDirectBuffer;
using nve::android::status_of;
using nve::android::surface_of;

namespace nve::android {

jobject CachedDirectBuffer::sync(JNIEnv* env, const void* ptr, size_t len) {
  if (ptr == nullptr || len == 0u) {
    release(env);
    return nullptr;
  }
  if (ref_ != nullptr && ptr_ == ptr && len_ == len) return ref_;
  release(env);
  jobject local = env->NewDirectByteBuffer(const_cast<void*>(ptr), static_cast<jlong>(len));
  if (local == nullptr) {
    if (env->ExceptionCheck()) env->ExceptionClear();
    return nullptr;
  }
  ref_ = env->NewGlobalRef(local);
  env->DeleteLocalRef(local);
  if (ref_ == nullptr) return nullptr;
  ptr_ = ptr;
  len_ = len;
  return ref_;
}

void CachedDirectBuffer::release(JNIEnv* env) noexcept {
  if (ref_ != nullptr && env != nullptr) env->DeleteGlobalRef(ref_);
  ref_ = nullptr;
  ptr_ = nullptr;
  len_ = 0u;
}

void JavaCallback::release(JNIEnv* env) noexcept {
  if (target != nullptr && env != nullptr) env->DeleteGlobalRef(target);
  target = nullptr;
  method = nullptr;
}

}  // namespace nve::android

namespace {

using nve::android::kHeaderLen;
using nve::android::kHeaderTables;
using nve::android::kHeaderTableStride;
using nve::android::kSlabsLen;
using nve::android::kSlabTables;
using nve::android::kSlabTableStride;

template <class T>
size_t bytes_of(std::span<const T> s) noexcept {
  return s.size() * sizeof(T);
}

}  // namespace

extern "C" {

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_acquireScene(JNIEnv* env, jclass, jlong handle,
                                                                   jlongArray header, jobjectArray slabs) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr || header == nullptr || slabs == nullptr) return NVE_RUNTIME_INVALID_ARG;
  if (static_cast<size_t>(env->GetArrayLength(header)) < kHeaderLen ||
      static_cast<size_t>(env->GetArrayLength(slabs)) < kSlabsLen) {
    s->last_error = "acquireScene needs RuntimeSceneReader.HEADER_LEN / SLABS_LEN arrays";
    return NVE_RUNTIME_INVALID_ARG;
  }
  s->scene.reset();
  NveRuntimeStatus st = NVE_RUNTIME_OK;
  nve::host::SceneError scene_error = nve::host::SceneError::None;
  s->scene = s->session.acquire_scene(&st, &scene_error);
  if (!s->scene) {
    if (st != NVE_RUNTIME_OK) return status_of(*s, st);
    s->last_error = "published scene failed validation (SceneError " +
                    std::to_string(static_cast<int>(scene_error)) + ")";
    return NVE_RUNTIME_ERROR;
  }
  const nve::host::RuntimeSceneReader& r = s->scene->reader();
  const NveRuntimeSceneView& v = s->scene->raw();
  if (v.table_count > nve::android::kMaxTables) {
    s->scene.reset();
    s->last_error = "scene publishes more than 128 tables";
    return NVE_RUNTIME_ERROR;
  }

  jlong h[kHeaderLen];
  h[0] = static_cast<jlong>(v.schema_hash);
  h[1] = static_cast<jlong>(v.frame_id);
  h[2] = v.node_capacity;
  h[3] = v.dirty_count;
  h[4] = v.removed_count;
  h[5] = v.text_slab_len;
  h[6] = v.children_slab_len;
  h[7] = v.table_count;
  h[nve::android::kHeaderSceneEpoch] = v.scene_epoch;
  h[nve::android::kHeaderDirtyResourceCount] = v.dirty_resource_count;
  auto set_slab = [&](size_t slot, const void* ptr, size_t len) {
    env->SetObjectArrayElement(slabs, static_cast<jsize>(slot), s->slabs[slot].sync(env, ptr, len));
  };
  set_slab(nve::android::kSlabRows, r.rows().data(), bytes_of(r.rows()));
  set_slab(nve::android::kSlabRects, r.rects().data(), bytes_of(r.rects()));
  set_slab(nve::android::kSlabText, r.text_slab().data(), r.text_slab().size());
  set_slab(nve::android::kSlabChildren, r.children_slab().data(), bytes_of(r.children_slab()));
  set_slab(nve::android::kSlabDirty, r.dirty_indices().data(), bytes_of(r.dirty_indices()));
  set_slab(nve::android::kSlabRemoved, r.removed_ids().data(), bytes_of(r.removed_ids()));
  set_slab(nve::android::kSlabDirtyResources, r.dirty_resources().data(), bytes_of(r.dirty_resources()));
  for (uint32_t t = 0u; t < v.table_count; ++t) {
    const NveTableView& tv = v.tables[t];
    jlong* th = h + kHeaderTables + t * kHeaderTableStride;
    th[0] = static_cast<jlong>((static_cast<uint64_t>(tv.key_hi) << 32u) | tv.key_lo);
    th[1] = tv.slot;
    th[2] = tv.words;
    th[3] = tv.row_count;
    th[4] = tv.dirty_count;
    const size_t base = kSlabTables + t * kSlabTableStride;
    const size_t row_words = static_cast<size_t>(tv.row_count) * tv.words;
    set_slab(base, tv.rows, row_words * sizeof(uint32_t));
    set_slab(base + 1u, tv.owners, tv.owners != nullptr ? tv.row_count * sizeof(uint32_t) : 0u);
    set_slab(base + 2u, tv.dirty_rows, tv.dirty_count * sizeof(uint32_t));
  }
  // Tables that disappeared since the last frame drop their stale buffers.
  for (size_t slot = kSlabTables + v.table_count * kSlabTableStride; slot < kSlabsLen; ++slot) {
    set_slab(slot, nullptr, 0u);
  }
  env->SetLongArrayRegion(header, 0, static_cast<jsize>(kHeaderTables + v.table_count * kHeaderTableStride), h);
  if (env->ExceptionCheck()) {  // e.g. a slab array of the wrong element type
    env->ExceptionClear();
    s->scene.reset();
    s->last_error = "acquireScene could not fill the packed frame arrays";
    return NVE_RUNTIME_INVALID_ARG;
  }
  s->last_error.clear();
  return NVE_RUNTIME_OK;
}

JNIEXPORT void JNICALL Java_dev_nve_host_NativeRuntime_releaseScene(JNIEnv*, jclass, jlong handle) {
  AndroidSurface* s = surface_of(handle);
  if (s != nullptr) nve::android::release_scene(*s);
}

JNIEXPORT jobject JNICALL Java_dev_nve_host_NativeRuntime_resource(JNIEnv* env, jclass, jlong handle,
                                                                  jint resource_id, jlongArray out_kind) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr || !s->scene || resource_id == 0) return nullptr;
  const std::optional<nve::host::RuntimeResource> res = s->scene->resource(static_cast<uint32_t>(resource_id));
  if (!res) return nullptr;
  if (out_kind != nullptr && env->GetArrayLength(out_kind) >= 1) {
    const jlong kind = static_cast<jlong>(res->kind.value);
    env->SetLongArrayRegion(out_kind, 0, 1, &kind);
  }
  // A fresh local view per call: nothing outlives the scene guard on the native side, and the
  // Java side never keeps it past `releaseScene` (RuntimeSurface copies or decodes in-frame).
  static uint8_t empty = 0u;  // non-null address: some JVMs reject `NewDirectByteBuffer(NULL, 0)`
  void* data = res->bytes.empty() ? &empty : const_cast<uint8_t*>(res->bytes.data());
  jobject buf = env->NewDirectByteBuffer(data, static_cast<jlong>(res->bytes.size()));
  if (buf == nullptr && env->ExceptionCheck()) env->ExceptionClear();
  return buf;
}

}  // extern "C"
