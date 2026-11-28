// Packs an acquired `NveSceneView` into the generated `NveJniFrame` layout read by
// `dev.nve.host.SceneReader` (`long[]` header + `ByteBuffer[]` slabs) with cached zero-copy
// `DirectByteBuffer`s, and serves published resource bytes.
#include "NveAndroidSurface.hpp"

#include <span>

using nve::android::AndroidSurface;
using nve::android::CachedDirectBuffer;
using nve::android::clear_exception;
using nve::android::status_of;
using nve::android::surface_of;
namespace frame = nve::jni_frame;

namespace nve::android {

bool CachedDirectBuffer::sync(JNIEnv* env, const void* ptr, size_t len, jobject* out) {
  *out = nullptr;
  if (ptr == nullptr || len == 0u) {
    release(env);
    return true;
  }
  if (ref_ != nullptr && ptr_ == ptr && len_ == len) {
    *out = ref_;
    return true;
  }
  release(env);
  jobject local = env->NewDirectByteBuffer(const_cast<void*>(ptr), static_cast<jlong>(len));
  if (local == nullptr) {
    clear_exception(env);
    return false;
  }
  ref_ = env->NewGlobalRef(local);
  env->DeleteLocalRef(local);
  if (ref_ == nullptr) {  // OOM: `NewGlobalRef` may leave an OutOfMemoryError pending
    clear_exception(env);
    return false;
  }
  ptr_ = ptr;
  len_ = len;
  *out = ref_;
  return true;
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

template <class T>
size_t bytes_of(std::span<const T> s) noexcept {
  return s.size() * sizeof(T);
}

/// Fills one `slabs[slot]`; `false` (exception cleared) on any JNI failure.
bool set_slab(JNIEnv* env, AndroidSurface& s, jobjectArray slabs, size_t slot, const void* ptr, size_t len) {
  jobject buf = nullptr;
  if (!s.slabs[slot].sync(env, ptr, len, &buf)) return false;
  env->SetObjectArrayElement(slabs, static_cast<jsize>(slot), buf);
  return !clear_exception(env);  // e.g. ArrayStoreException: `slabs` is not a ByteBuffer[]
}

/// Packs the acquired scene; `false` on a JNI failure (the caller reports it).
bool pack(JNIEnv* env, AndroidSurface& s, jlongArray header, jobjectArray slabs) {
  const nve::host::SceneReader& r = s.scene->reader();
  const NveSceneView& v = s.scene->raw();
  jlong h[frame::HEADER_LEN];
  h[frame::H_SCHEMA] = static_cast<jlong>(v.schema_hash);
  h[frame::H_FRAME_ID] = static_cast<jlong>(v.frame_id);
  h[frame::H_NODE_CAPACITY] = v.node_capacity;
  h[frame::H_DIRTY_COUNT] = v.dirty_count;
  h[frame::H_MOVED_COUNT] = v.moved_count;
  h[frame::H_REMOVED_COUNT] = v.removed_count;
  h[frame::H_TEXT_LEN] = v.text_slab_len;
  h[frame::H_CHILDREN_LEN] = v.children_slab_len;
  h[frame::H_TABLE_COUNT] = v.table_count;
  h[frame::H_SCROLL_COUNT] = v.scroll_count;
  h[frame::H_SCENE_EPOCH] = v.scene_epoch;
  h[frame::H_DIRTY_RESOURCE_COUNT] = v.dirty_resource_count;
  const bool slabs_ok =
      set_slab(env, s, slabs, frame::S_ROWS, r.rows().data(), bytes_of(r.rows())) &&
      set_slab(env, s, slabs, frame::S_RECTS, r.rects().data(), bytes_of(r.rects())) &&
      set_slab(env, s, slabs, frame::S_TEXT, r.text_slab().data(), r.text_slab().size()) &&
      set_slab(env, s, slabs, frame::S_CHILDREN, r.children_slab().data(), bytes_of(r.children_slab())) &&
      set_slab(env, s, slabs, frame::S_DIRTY, r.dirty_indices().data(), bytes_of(r.dirty_indices())) &&
      set_slab(env, s, slabs, frame::S_MOVED, r.moved_indices().data(), bytes_of(r.moved_indices())) &&
      set_slab(env, s, slabs, frame::S_REMOVED, r.removed_ids().data(), bytes_of(r.removed_ids())) &&
      set_slab(env, s, slabs, frame::S_DIRTY_RESOURCES, r.dirty_resources().data(), bytes_of(r.dirty_resources())) &&
      set_slab(env, s, slabs, frame::S_SCROLL_OFFSETS, r.scroll_offsets().data(), bytes_of(r.scroll_offsets()));
  if (!slabs_ok) return false;
  for (uint32_t t = 0u; t < v.table_count; ++t) {
    const NveTableView& tv = v.tables[t];
    jlong* th = h + frame::H_TABLES + t * frame::TABLE_HEADER_STRIDE;
    th[frame::TH_KEY] = static_cast<jlong>((static_cast<uint64_t>(tv.key_hi) << 32u) | tv.key_lo);
    th[frame::TH_SLOT] = tv.slot;
    th[frame::TH_WORDS] = tv.words;
    th[frame::TH_ROW_COUNT] = tv.row_count;
    th[frame::TH_DIRTY_COUNT] = tv.dirty_count;
    th[frame::TH_ROW_INDEX_DIR_LEN] = tv.row_index_dir_len;
    th[frame::TH_ROW_INDEX_PAGE_COUNT] = tv.row_index_page_count;
    const size_t base = frame::S_TABLES + t * frame::TABLE_SLAB_STRIDE;
    const size_t word = sizeof(uint32_t);
    const size_t row_words = static_cast<size_t>(tv.row_count) * tv.words;
    const size_t page_words = static_cast<size_t>(tv.row_index_page_count) * NVE_ROW_INDEX_PAGE_LEN;
    const bool ok =
        set_slab(env, s, slabs, base + frame::TS_ROWS, tv.rows, row_words * word) &&
        set_slab(env, s, slabs, base + frame::TS_OWNERS, tv.owners, tv.owners != nullptr ? tv.row_count * word : 0u) &&
        set_slab(env, s, slabs, base + frame::TS_DIRTY_ROWS, tv.dirty_rows, tv.dirty_count * word) &&
        set_slab(env, s, slabs, base + frame::TS_ROW_INDEX_DIR, tv.row_index_dir, tv.row_index_dir_len * word) &&
        set_slab(env, s, slabs, base + frame::TS_ROW_INDEX_PAGES, tv.row_index_pages, page_words * word);
    if (!ok) return false;
  }
  // Only tables that disappeared since the previous frame still hold stale buffers.
  const size_t stale_end = frame::S_TABLES + static_cast<size_t>(s.prev_table_count) * frame::TABLE_SLAB_STRIDE;
  for (size_t slot = frame::S_TABLES + v.table_count * frame::TABLE_SLAB_STRIDE; slot < stale_end; ++slot) {
    if (!set_slab(env, s, slabs, slot, nullptr, 0u)) return false;
  }
  s.prev_table_count = v.table_count;
  env->SetLongArrayRegion(header, 0, static_cast<jsize>(frame::H_TABLES + v.table_count * frame::TABLE_HEADER_STRIDE), h);
  return !clear_exception(env);
}

}  // namespace

extern "C" {

JNIEXPORT jint JNICALL Java_dev_nve_host_NativeRuntime_acquireScene(JNIEnv* env, jclass, jlong handle,
                                                                   jlongArray header, jobjectArray slabs) {
  AndroidSurface* s = surface_of(handle);
  if (s == nullptr || header == nullptr || slabs == nullptr) return NVE_RUNTIME_INVALID_ARG;
  if (static_cast<size_t>(env->GetArrayLength(header)) < frame::HEADER_LEN ||
      static_cast<size_t>(env->GetArrayLength(slabs)) < frame::SLABS_LEN) {
    s->last_error = "acquireScene needs NveJniFrame.HEADER_LEN / SLABS_LEN arrays";
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
  if (s->scene->raw().table_count > frame::MAX_TABLES) {
    s->scene.reset();
    s->last_error = "scene publishes more than NveJniFrame.MAX_TABLES tables";
    return NVE_RUNTIME_ERROR;
  }
  if (!pack(env, *s, header, slabs)) {
    // A partially filled `slabs` array may reference buffers of the cache: drop them all so the
    // next acquire re-fills every slot from scratch.
    s->scene.reset();
    nve::android::release_buffers(*s, env);
    s->prev_table_count = static_cast<uint32_t>(frame::MAX_TABLES);
    s->last_error = "acquireScene could not fill the packed frame arrays (wrong array type or out of memory)";
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
  if (buf == nullptr) clear_exception(env);
  return buf;
}

}  // extern "C"
