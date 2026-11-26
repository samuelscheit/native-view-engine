#pragma once

#include <jni.h>

#include "nve_host_runtime.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace nve::android_internal {

enum SlabSlot : size_t {
  kSlabNodes = 0,
  kSlabText = 1,
  kSlabChildren = 2,
  kSlabDirtyIndices = 3,
  kSlabRemovedIds = 4,
  kSlabMetaRecords = 5,
  kSlabMetaBlob = 6,
  kSlabCount = 7,
};

inline constexpr size_t kMetaRecordsInitialCapacity = 64u;
inline constexpr size_t kMetaBlobInitialCapacity = 4096u;

// Non-null sentinel passed to `NewDirectByteBuffer` with capacity 0 when a slab is empty,
// avoiding NULL address edge cases across JVMs / ART.
inline uint8_t g_empty_slab_sentinel = 0;

struct CachedDirectBuffer {
  const void* ptr{nullptr};
  size_t byte_len{0};
  jobject global_ref{nullptr};

  void release(JNIEnv* env) noexcept {
    if (global_ref != nullptr && env != nullptr) {
      env->DeleteGlobalRef(global_ref);
    }
    global_ref = nullptr;
    ptr = nullptr;
    byte_len = 0u;
  }

  /// Replaces the cached `GlobalRef` `DirectByteBuffer` ONLY when `(ptr, byte_len)` actually changes.
  /// Returns `true` if a new `DirectByteBuffer` was created.
  bool sync(JNIEnv* env, const void* next_ptr, size_t next_len) {
    const void* effective_ptr = (next_ptr != nullptr && next_len > 0u)
                                    ? next_ptr
                                    : static_cast<const void*>(&g_empty_slab_sentinel);
    const size_t effective_len = (next_ptr != nullptr && next_len > 0u) ? next_len : 0u;

    if (global_ref != nullptr && ptr == effective_ptr && byte_len == effective_len) {
      return false;
    }

    if (global_ref != nullptr && env != nullptr) {
      env->DeleteGlobalRef(global_ref);
      global_ref = nullptr;
    }
    ptr = effective_ptr;
    byte_len = effective_len;

    if (env == nullptr) {
      return true;
    }
    jobject local = env->NewDirectByteBuffer(
        const_cast<void*>(effective_ptr), static_cast<jlong>(effective_len));
    if (local != nullptr) {
      global_ref = env->NewGlobalRef(local);
      env->DeleteLocalRef(local);
    }
    return true;
  }
};

struct NodeMetadataEntry {
  std::string label;
  std::string role;
  std::string hint;
  std::string state_desc;
  std::string placeholder;
  std::string test_id;
  std::vector<uint8_t> draw_list;
  bool disabled{false};
  bool accessibility_hidden{false};
  bool secure_text_entry{false};
  bool multiline{false};
  bool draw_list_corrupt{false};
  uint8_t resize_mode{0u};

  bool operator==(const NodeMetadataEntry& other) const = default;

  bool is_default() const noexcept {
    return label.empty() && role.empty() && hint.empty() && state_desc.empty() &&
           placeholder.empty() && test_id.empty() && draw_list.empty() && !disabled &&
           !accessibility_hidden && !secure_text_entry && !multiline && !draw_list_corrupt &&
           resize_mode == 0u;
  }
};

struct AndroidSurfaceState {
  nve::NativeHostSession session;
  NveSurfaceConfig config{};
  std::string last_error;
  JavaVM* jvm{nullptr};
  jclass activity_class{nullptr};
  jmethodID measure_text_method{nullptr};
  jobject pair_wake_target{nullptr};
  jmethodID pair_wake_method{nullptr};
  std::array<CachedDirectBuffer, kSlabCount> slab_buffers{};
  uint32_t slab_epoch_mask{0u};

  // High-water-mark fixed-stride 64-byte `NveNodeMeta` array + flat companion byte blob holding
  // ONLY dirty/changed node metadata for the current frame. Neither buffer shrinks until
  // reload/destroy so `kSlabMetaRecords` and `kSlabMetaBlob` DirectByteBuffer GlobalRefs stay
  // pinned across steady-state frames with zero JNI allocation churn.
  std::vector<NveNodeMeta> meta_records;
  std::vector<uint8_t> meta_blob;
  uint32_t meta_record_count{0u};
  uint32_t meta_blob_used_bytes{0u};
  uint64_t metadata_cache_frame{0u};
  bool metadata_cache_valid{false};
  std::unordered_map<uint32_t, NodeMetadataEntry> metadata_cache;
  std::unordered_map<uint32_t, int32_t> image_resize_modes;
};

inline bool is_session_active(const AndroidSurfaceState* state) noexcept {
  return state != nullptr && (state->session.is_valid() || state->session.is_paired());
}

/// Packs accessibility, `TextInput`, `Image` (`resizeMode`), and `SkiaCanvas` (`drawList`)
/// metadata for ONLY dirty/changed nodes into fixed-stride `NveNodeMeta` records + companion blob.
void pack_dirty_metadata(AndroidSurfaceState* state, const NveSceneView& view);

} // namespace nve::android_internal
