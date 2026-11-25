#include <jni.h>

#include "nve_host_runtime.hpp"

#include <algorithm>
#include <array>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace {

enum SlabSlot : size_t {
  kSlabNodes = 0,
  kSlabText = 1,
  kSlabChildren = 2,
  kSlabDirtyIndices = 3,
  kSlabRemovedIds = 4,
  kSlabMetadata = 5,
  kSlabCount = 6,
};

constexpr size_t kMetaHeaderSize = 32u;
constexpr size_t kMetaRecordHeaderSize = 28u;
constexpr uint16_t kMetaFormatVersion = 1u;
constexpr size_t kMetaInitialCapacity = 4096u;

constexpr uint32_t kMetaFlagDisabled = 1u << 0;
constexpr uint32_t kMetaFlagSecureTextEntry = 1u << 1;
constexpr uint32_t kMetaFlagMultiline = 1u << 2;
constexpr uint32_t kMetaFlagHasDrawList = 1u << 3;
constexpr uint32_t kMetaFlagDrawListCorrupt = 1u << 4;
constexpr uint32_t kMetaFlagAccessibilityHidden = 1u << 5;
constexpr uint32_t kMetaResizeModeShift = 8u;
constexpr uint32_t kMetaResizeModeMask = 0x3u;

constexpr size_t kDrawHeaderSize = 15u;
constexpr uint16_t kDrawFormatVersion = 1u;
constexpr uint32_t kMaxDrawOps = 65536u;
constexpr uint32_t kMaxPolylinePoints = 16384u;
constexpr std::string_view kDrawMagic = "NVE_DRAW1";
constexpr std::string_view kDrawAttrPrefix = "nve-draw1;b64,";
constexpr std::string_view kMetaMagic = "NVE_MET1";

// Non-null sentinel passed to `NewDirectByteBuffer` with capacity 0 when a slab is empty,
// avoiding NULL address edge cases across JVMs / ART.
uint8_t g_empty_slab_sentinel = 0;

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
  std::array<CachedDirectBuffer, kSlabCount> slab_buffers{};
  uint32_t slab_epoch_mask{0u};

  // High-water-mark flat binary `NVE_MET1` buffer holding ONLY dirty/changed node metadata
  // for the current frame. Never shrinks until reload/destroy so `kSlabMetadata` DirectByteBuffer
  // GlobalRef stays pinned across steady-state frames with zero JNI allocation churn.
  std::vector<uint8_t> metadata_slab;
  uint32_t metadata_used_bytes{0u};
  uint32_t metadata_record_count{0u};
  uint64_t metadata_cache_frame{0u};
  bool metadata_cache_valid{false};
  std::unordered_map<uint32_t, NodeMetadataEntry> metadata_cache;
  std::unordered_map<uint32_t, int32_t> image_resize_modes;
};

void release_all_slab_buffers(AndroidSurfaceState* state, JNIEnv* env) noexcept {
  if (!state) return;
  for (auto& slot : state->slab_buffers) {
    slot.release(env);
  }
  state->slab_epoch_mask = 0u;
  state->metadata_slab.clear();
  state->metadata_slab.shrink_to_fit();
  state->metadata_used_bytes = 0u;
  state->metadata_record_count = 0u;
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

inline uint16_t read_u16_le(const uint8_t* p) noexcept {
  return static_cast<uint16_t>(p[0]) | (static_cast<uint16_t>(p[1]) << 8);
}

inline uint32_t read_u32_le(const uint8_t* p) noexcept {
  return static_cast<uint32_t>(p[0]) |
         (static_cast<uint32_t>(p[1]) << 8) |
         (static_cast<uint32_t>(p[2]) << 16) |
         (static_cast<uint32_t>(p[3]) << 24);
}

inline float read_f32_le(const uint8_t* p) noexcept {
  const uint32_t bits = read_u32_le(p);
  float v = 0.0f;
  std::memcpy(&v, &bits, sizeof(float));
  return v;
}

inline void write_u16_le(uint8_t* dst, uint16_t v) noexcept {
  dst[0] = static_cast<uint8_t>(v & 0xFFu);
  dst[1] = static_cast<uint8_t>((v >> 8) & 0xFFu);
}

inline void write_u32_le(uint8_t* dst, uint32_t v) noexcept {
  dst[0] = static_cast<uint8_t>(v & 0xFFu);
  dst[1] = static_cast<uint8_t>((v >> 8) & 0xFFu);
  dst[2] = static_cast<uint8_t>((v >> 16) & 0xFFu);
  dst[3] = static_cast<uint8_t>((v >> 24) & 0xFFu);
}

inline void write_u64_le(uint8_t* dst, uint64_t v) noexcept {
  for (size_t i = 0; i < 8u; ++i) {
    dst[i] = static_cast<uint8_t>((v >> (i * 8u)) & 0xFFu);
  }
}

bool is_finite_f32_range(const uint8_t* data, size_t pos, size_t count) noexcept {
  for (size_t i = 0; i < count; ++i) {
    if (!std::isfinite(read_f32_le(data + pos + i * 4u))) {
      return false;
    }
  }
  return true;
}

/// Strict zero-allocation validator for `NVE_DRAW1` binary draw-list streams matching
/// `crates/nve-components/src/canvas_ir.rs`.
bool validate_nve_draw1(const uint8_t* data, size_t len) noexcept {
  if (data == nullptr || len < kDrawHeaderSize) {
    return false;
  }
  if (std::memcmp(data, kDrawMagic.data(), kDrawMagic.size()) != 0) {
    return false;
  }
  const uint16_t version = read_u16_le(data + 9u);
  if (version != kDrawFormatVersion) {
    return false;
  }
  const uint32_t op_count = read_u32_le(data + 11u);
  if (op_count > kMaxDrawOps) {
    return false;
  }
  size_t pos = kDrawHeaderSize;
  if (len - pos < static_cast<size_t>(op_count)) {
    return false;
  }
  for (uint32_t i = 0; i < op_count; ++i) {
    if (pos >= len) {
      return false;
    }
    const uint8_t op = data[pos++];
    switch (op) {
      case 1u: // Clear
        if (len - pos < 4u) return false;
        pos += 4u;
        break;
      case 2u: // FillRect
        if (len - pos < 20u || !is_finite_f32_range(data, pos, 4u)) return false;
        pos += 20u;
        break;
      case 3u: // FillRoundedRect
        if (len - pos < 24u || !is_finite_f32_range(data, pos, 5u)) return false;
        pos += 24u;
        break;
      case 4u: // FillCircle
        if (len - pos < 16u || !is_finite_f32_range(data, pos, 3u)) return false;
        pos += 16u;
        break;
      case 5u: // StrokeLine
        if (len - pos < 24u || !is_finite_f32_range(data, pos, 5u)) return false;
        pos += 24u;
        break;
      case 6u: { // StrokePolyline
        if (len - pos < 12u || !std::isfinite(read_f32_le(data + pos))) return false;
        pos += 8u; // width + rgba
        const uint32_t pt_count = read_u32_le(data + pos);
        pos += 4u;
        if (pt_count > kMaxPolylinePoints) return false;
        const size_t needed = static_cast<size_t>(pt_count) * 8u;
        if (needed > len - pos) return false;
        if (!is_finite_f32_range(data, pos, static_cast<size_t>(pt_count) * 2u)) return false;
        pos += needed;
        break;
      }
      case 7u: // ClipRect
        if (len - pos < 16u || !is_finite_f32_range(data, pos, 4u)) return false;
        pos += 16u;
        break;
      case 8u: // RestoreClip
        break;
      default:
        return false;
    }
  }
  return pos == len;
}

bool decode_base64_strict(std::string_view input, std::vector<uint8_t>& out) {
  out.clear();
  auto decode_char = [](unsigned char c) noexcept -> int {
    if (c >= 'A' && c <= 'Z') return static_cast<int>(c - 'A');
    if (c >= 'a' && c <= 'z') return static_cast<int>(c - 'a' + 26);
    if (c >= '0' && c <= '9') return static_cast<int>(c - '0' + 52);
    if (c == '+') return 62;
    if (c == '/') return 63;
    if (c == '=') return -2;
    return -1;
  };

  size_t clean_len = 0u;
  for (unsigned char c : input) {
    if (!std::isspace(c)) ++clean_len;
  }
  if (clean_len == 0u || (clean_len & 3u) != 0u) {
    return false;
  }

  out.reserve((clean_len / 4u) * 3u);
  int quad[4] = {0, 0, 0, 0};
  int q = 0;
  bool saw_pad = false;
  for (unsigned char c : input) {
    if (std::isspace(c)) continue;
    const int v = decode_char(c);
    if (v == -1) return false;
    if (saw_pad && v != -2) return false;
    if (v == -2) saw_pad = true;
    quad[q++] = v;
    if (q == 4) {
      if (quad[0] < 0 || quad[1] < 0) return false;
      out.push_back(static_cast<uint8_t>((quad[0] << 2) | (quad[1] >> 4)));
      if (quad[2] >= 0) {
        out.push_back(static_cast<uint8_t>(((quad[1] & 0x0F) << 4) | (quad[2] >> 2)));
        if (quad[3] >= 0) {
          out.push_back(static_cast<uint8_t>(((quad[2] & 0x03) << 6) | quad[3]));
        }
      } else if (quad[3] != -2) {
        return false;
      }
      q = 0;
    }
  }
  return q == 0;
}

int32_t parse_resize_mode_code(std::string_view raw) noexcept {
  std::string lower;
  lower.reserve(raw.size());
  for (unsigned char c : raw) {
    lower.push_back(static_cast<char>(std::tolower(c)));
  }
  if (lower == "contain") return 1;
  if (lower == "stretch") return 2;
  if (lower == "center") return 3;
  return 0; // cover (default)
}

bool prop_as_bool(const nve::RawPropValue& v, bool fallback = false) noexcept {
  if (const auto* b = std::get_if<bool>(&v)) {
    return *b;
  }
  if (const auto* i = std::get_if<int64_t>(&v)) {
    return *i != 0;
  }
  if (const auto* s = std::get_if<std::string>(&v)) {
    return *s == "true" || *s == "1" || *s == "yes";
  }
  return fallback;
}

std::string_view find_raw_string(
    const nve::RawPropMap& props,
    std::string_view primary,
    std::string_view alias = {}) noexcept {
  auto it = props.find(std::string(primary));
  if (it == props.end() && !alias.empty()) {
    it = props.find(std::string(alias));
  }
  if (it == props.end()) {
    return {};
  }
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    return *s;
  }
  return {};
}

bool find_raw_bool(
    const nve::RawPropMap& props,
    std::string_view primary,
    std::string_view alias = {}) noexcept {
  auto it = props.find(std::string(primary));
  if (it == props.end() && !alias.empty()) {
    it = props.find(std::string(alias));
  }
  if (it == props.end()) {
    return false;
  }
  return prop_as_bool(it->second, false);
}

bool text_slab_has_draw_list_for_node(const NveSceneView& view, uint32_t node_id) noexcept {
  if (view.nodes == nullptr || view.text_slab == nullptr) {
    return false;
  }
  for (uint32_t i = 0; i < view.node_count; ++i) {
    const NveRenderNode& n = view.nodes[i];
    if (n.node_id != node_id) continue;
    if (n.text_len < kDrawHeaderSize) return false;
    const size_t off = static_cast<size_t>(n.text_offset);
    const size_t len = static_cast<size_t>(n.text_len);
    if (off > view.text_slab_len || len > view.text_slab_len - off) return false;
    return validate_nve_draw1(view.text_slab + off, len);
  }
  return false;
}

NodeMetadataEntry extract_node_metadata(
    const nve::RenderedNodeSnapshot& snap,
    const NveSceneView& view) {
  NodeMetadataEntry entry{};
  if (snap.raw_props.empty()) {
    return entry;
  }
  const auto& props = snap.raw_props;

  entry.label = std::string(find_raw_string(props, "accessibilityLabel", "aria-label"));
  entry.role = std::string(find_raw_string(props, "accessibilityRole", "role"));
  entry.hint = std::string(find_raw_string(props, "accessibilityHint"));
  entry.state_desc =
      std::string(find_raw_string(props, "accessibilityStateDescription", "accessibilityValue"));
  entry.placeholder = std::string(find_raw_string(props, "placeholder"));
  entry.test_id = std::string(find_raw_string(props, "testID", "data-testid"));

  entry.disabled = find_raw_bool(props, "disabled", "accessibilityDisabled");
  entry.secure_text_entry = find_raw_bool(props, "secureTextEntry");
  entry.multiline = find_raw_bool(props, "multiline");
  entry.accessibility_hidden =
      find_raw_bool(props, "accessibilityElementsHidden", "aria-hidden");
  if (!entry.accessibility_hidden) {
    const std::string_view imp = find_raw_string(props, "importantForAccessibility");
    if (imp == "no" || imp == "no-hide-descendants") {
      entry.accessibility_hidden = true;
    }
  }

  if (snap.kind == NVE_COMP_IMAGE) {
    const std::string_view rm = find_raw_string(props, "resizeMode", "resize_mode");
    if (!rm.empty()) {
      entry.resize_mode = static_cast<uint8_t>(parse_resize_mode_code(rm) & 0x3);
    }
  }

  // Only pack `drawList` bytes from raw_props when not already present in `text_slab`.
  auto dl_it = props.find("drawList");
  if (dl_it == props.end()) {
    dl_it = props.find("draw_list");
  }
  if (dl_it != props.end() && !std::holds_alternative<std::nullptr_t>(dl_it->second)) {
    const bool already_in_text_slab =
        (snap.kind == NVE_COMP_SKIA_CANVAS) && text_slab_has_draw_list_for_node(view, snap.node_id);
    if (!already_in_text_slab) {
      if (const auto* bytes = std::get_if<std::vector<uint8_t>>(&dl_it->second)) {
        if (!bytes->empty()) {
          if (validate_nve_draw1(bytes->data(), bytes->size())) {
            entry.draw_list = *bytes;
          } else {
            entry.draw_list_corrupt = true;
          }
        }
      } else if (const auto* str = std::get_if<std::string>(&dl_it->second)) {
        if (!str->empty()) {
          std::string_view b64 = *str;
          if (b64.rfind(kDrawAttrPrefix, 0) == 0) {
            b64.remove_prefix(kDrawAttrPrefix.size());
          }
          std::vector<uint8_t> decoded;
          if (decode_base64_strict(b64, decoded) &&
              validate_nve_draw1(decoded.data(), decoded.size())) {
            entry.draw_list = std::move(decoded);
          } else {
            entry.draw_list_corrupt = true;
          }
        }
      } else {
        entry.draw_list_corrupt = true;
      }
    }
  }

  return entry;
}

uint16_t clamp_u16(size_t n) noexcept {
  return static_cast<uint16_t>(std::min<size_t>(n, std::numeric_limits<uint16_t>::max()));
}

/// Packs accessibility, `TextInput`, `Image` (`resizeMode`), and `SkiaCanvas` (`drawList`)
/// metadata for ONLY dirty/changed nodes into ONE reusable flat `NVE_MET1` binary buffer.
void pack_dirty_metadata(AndroidSurfaceState* state, const NveSceneView& view) {
  if (!state || !state->session.is_valid()) {
    return;
  }
  if (state->metadata_cache_valid && state->metadata_cache_frame == view.frame_id) {
    return;
  }

  const bool is_full_sync = !state->metadata_cache_valid || view.frame_id < state->metadata_cache_frame;
  if (is_full_sync) {
    state->metadata_cache.clear();
    state->image_resize_modes.clear();
  } else if (view.removed_ids != nullptr && view.removed_count > 0u) {
    for (uint32_t r = 0; r < view.removed_count; ++r) {
      const uint32_t rid = view.removed_ids[r];
      state->metadata_cache.erase(rid);
      state->image_resize_modes.erase(rid);
    }
  }

  const auto snapshots = state->session.snapshot_nodes();
  std::vector<std::pair<uint32_t, NodeMetadataEntry>> changed;
  changed.reserve(8u);

  const bool check_orphans = !state->metadata_cache.empty();
  std::unordered_set<uint32_t> live_ids;
  if (check_orphans) {
    live_ids.reserve(snapshots.size());
  }

  for (const auto& snap : snapshots) {
    if (check_orphans) {
      live_ids.insert(snap.node_id);
    }
    NodeMetadataEntry next = extract_node_metadata(snap, view);
    if (snap.kind == NVE_COMP_IMAGE) {
      if (next.resize_mode != 0u) {
        state->image_resize_modes[snap.node_id] = static_cast<int32_t>(next.resize_mode);
      } else {
        state->image_resize_modes.erase(snap.node_id);
      }
    }

    auto prev_it = state->metadata_cache.find(snap.node_id);
    if (next.is_default()) {
      if (prev_it != state->metadata_cache.end()) {
        state->metadata_cache.erase(prev_it);
        changed.emplace_back(snap.node_id, std::move(next));
      }
    } else if (prev_it == state->metadata_cache.end() || !(prev_it->second == next)) {
      state->metadata_cache[snap.node_id] = next;
      changed.emplace_back(snap.node_id, std::move(next));
    }
  }

  if (check_orphans) {
    for (auto it = state->metadata_cache.begin(); it != state->metadata_cache.end();) {
      if (live_ids.find(it->first) == live_ids.end()) {
        state->image_resize_modes.erase(it->first);
        it = state->metadata_cache.erase(it);
      } else {
        ++it;
      }
    }
  }

  size_t total_bytes = kMetaHeaderSize;
  for (const auto& [nid, entry] : changed) {
    (void)nid;
    total_bytes += kMetaRecordHeaderSize +
                   static_cast<size_t>(clamp_u16(entry.label.size())) +
                   static_cast<size_t>(clamp_u16(entry.role.size())) +
                   static_cast<size_t>(clamp_u16(entry.hint.size())) +
                   static_cast<size_t>(clamp_u16(entry.state_desc.size())) +
                   static_cast<size_t>(clamp_u16(entry.placeholder.size())) +
                   static_cast<size_t>(clamp_u16(entry.test_id.size())) +
                   entry.draw_list.size();
  }

  const size_t min_backing = std::max(kMetaInitialCapacity, total_bytes);
  if (state->metadata_slab.size() < min_backing) {
    size_t next_cap = std::max(kMetaInitialCapacity, state->metadata_slab.size());
    while (next_cap < min_backing) {
      next_cap *= 2u;
    }
    state->metadata_slab.resize(next_cap, 0u);
  }

  uint8_t* dst = state->metadata_slab.data();
  std::memcpy(dst, kMetaMagic.data(), kMetaMagic.size());
  write_u16_le(dst + 8u, kMetaFormatVersion);
  write_u16_le(dst + 10u, is_full_sync ? 1u : 0u);
  write_u32_le(dst + 12u, static_cast<uint32_t>(changed.size()));
  write_u64_le(dst + 16u, view.frame_id);
  write_u32_le(dst + 24u, static_cast<uint32_t>(total_bytes));
  write_u32_le(dst + 28u, 0u);

  size_t pos = kMetaHeaderSize;
  for (const auto& [nid, entry] : changed) {
    uint32_t flags = 0u;
    if (entry.disabled) flags |= kMetaFlagDisabled;
    if (entry.secure_text_entry) flags |= kMetaFlagSecureTextEntry;
    if (entry.multiline) flags |= kMetaFlagMultiline;
    if (!entry.draw_list.empty()) flags |= kMetaFlagHasDrawList;
    if (entry.draw_list_corrupt) flags |= kMetaFlagDrawListCorrupt;
    if (entry.accessibility_hidden) flags |= kMetaFlagAccessibilityHidden;
    flags |= (static_cast<uint32_t>(entry.resize_mode) & kMetaResizeModeMask) << kMetaResizeModeShift;

    const uint16_t label_len = clamp_u16(entry.label.size());
    const uint16_t role_len = clamp_u16(entry.role.size());
    const uint16_t hint_len = clamp_u16(entry.hint.size());
    const uint16_t state_len = clamp_u16(entry.state_desc.size());
    const uint16_t placeholder_len = clamp_u16(entry.placeholder.size());
    const uint16_t test_id_len = clamp_u16(entry.test_id.size());
    const uint32_t draw_len = static_cast<uint32_t>(entry.draw_list.size());

    write_u32_le(dst + pos + 0u, nid);
    write_u32_le(dst + pos + 4u, flags);
    write_u16_le(dst + pos + 8u, label_len);
    write_u16_le(dst + pos + 10u, role_len);
    write_u16_le(dst + pos + 12u, hint_len);
    write_u16_le(dst + pos + 14u, state_len);
    write_u16_le(dst + pos + 16u, placeholder_len);
    write_u16_le(dst + pos + 18u, test_id_len);
    write_u32_le(dst + pos + 20u, draw_len);
    write_u32_le(dst + pos + 24u, 0u);
    pos += kMetaRecordHeaderSize;

    auto copy_slice = [&](const void* src, size_t n) {
      if (n > 0u) {
        std::memcpy(dst + pos, src, n);
        pos += n;
      }
    };
    copy_slice(entry.label.data(), label_len);
    copy_slice(entry.role.data(), role_len);
    copy_slice(entry.hint.data(), hint_len);
    copy_slice(entry.state_desc.data(), state_len);
    copy_slice(entry.placeholder.data(), placeholder_len);
    copy_slice(entry.test_id.data(), test_id_len);
    copy_slice(entry.draw_list.data(), draw_len);
  }

  state->metadata_used_bytes = static_cast<uint32_t>(pos);
  state->metadata_record_count = static_cast<uint32_t>(changed.size());
  state->metadata_cache_frame = view.frame_id;
  state->metadata_cache_valid = true;
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
  release_all_slab_buffers(state, env);
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
  if (text != nullptr && env != nullptr) {
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

extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeDispatchCustomEvent(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint node_id,
    jstring name,
    jstring payload) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state || !state->session.is_valid() || node_id <= 0 || name == nullptr || env == nullptr) {
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

  auto* bridge = state->session.bridge();
  if (!bridge) {
    return 0;
  }
  nve::fabric_host_enqueue_custom_event(
      bridge->fabric_host(),
      static_cast<uint32_t>(node_id),
      nve::current_timestamp_us(),
      event_name,
      reinterpret_cast<const uint8_t*>(utf8_payload.data()),
      utf8_payload.size());
  const NveBufferSlice inbound = nve_fabric_host_drain_events(bridge->fabric_host());
  const uint32_t dirty = bridge->dispatch_native_events(inbound, /*auto_render=*/true);
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
  if (!state || !state->session.is_valid() || node_id <= 0) {
    return 0;
  }
  NveDirectEvent ev{};
  ev.surface_id = state->config.surface_id;
  ev.node_id = static_cast<uint32_t>(node_id);
  ev.timestamp_us = nve::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(nve::EventKind::Scroll);
  ev.payload_tag = NVE_EVENT_PAYLOAD_SCROLL;
  ev.x = sanitize_finite(scroll_x);
  ev.y = sanitize_finite(scroll_y);
  ev.width = 0.0f;
  ev.height = 0.0f;
  ev.value = nve::make_null_value();
  const uint32_t dirty =
      state->session.dispatch_direct_events(NveEventBatch{&ev, 1u}, /*auto_render=*/true);
  (void)state->session.drain_events();
  return static_cast<jint>(dirty);
}

/// Reads the live zero-copy `NveSceneView` directly from `FabricHost`, packs dirty/changed
/// accessibility/`TextInput`/`Image`/`SkiaCanvas` metadata into ONE flat binary `NVE_MET1` buffer,
/// updates cached JNI `GlobalRef` `DirectByteBuffer` slabs ONLY when `(ptr, byte_len)` changes,
/// and writes `[schema_hash, frame_id, node_count, dirty_count, removed_count, text_slab_len,
/// children_slab_len (, metadata_used_bytes)]` into `out_meta`.
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
  if (state->slab_buffers[kSlabMetadata].sync(
          env, state->metadata_slab.data(), state->metadata_slab.size())) {
    mask |= (1u << kSlabMetadata);
  }
  state->slab_epoch_mask = mask;

  if (env != nullptr && out_meta != nullptr) {
    const jsize arr_len = env->GetArrayLength(out_meta);
    if (arr_len >= 7) {
      const jlong meta[8] = {
          static_cast<jlong>(view.schema_hash),
          static_cast<jlong>(view.frame_id),
          static_cast<jlong>(view.node_count),
          static_cast<jlong>(view.dirty_count),
          static_cast<jlong>(view.removed_count),
          static_cast<jlong>(view.text_slab_len),
          static_cast<jlong>(view.children_slab_len),
          static_cast<jlong>(state->metadata_used_bytes),
      };
      env->SetLongArrayRegion(out_meta, 0, arr_len >= 8 ? 8 : 7, meta);
    }
  }
  return static_cast<jint>(view.node_count);
}

/// Bitmask indicating which of the 6 cached DirectByteBuffer slabs changed on the last
/// `nativeRenderFrame` call:
/// bit 0 = nodes, bit 1 = text_slab, bit 2 = children_slab, bit 3 = dirty_indices,
/// bit 4 = removed_ids, bit 5 = dirty_metadata (`NVE_MET1`).
extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetSlabEpochMask(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  return state ? static_cast<jint>(state->slab_epoch_mask) : 0;
}

/// Reads cached `resizeMode` (`0=cover, 1=contain, 2=stretch, 3=center`) for an `Image` node
/// populated during `pack_dirty_metadata` (zero extra snapshot scans).
extern "C" JNIEXPORT jint JNICALL
Java_dev_nve_android_NveMainActivity_nativeGetImageResizeMode(
    JNIEnv* /*env*/,
    jclass /*clazz*/,
    jlong handle_ptr,
    jint node_id) {
  auto* state = reinterpret_cast<AndroidSurfaceState*>(handle_ptr);
  if (!state || !state->session.is_valid() || node_id <= 0) {
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
Java_dev_nve_android_NveMainActivity_nativeGetMetadataBuffer(
    JNIEnv* env,
    jclass /*clazz*/,
    jlong handle_ptr) {
  return return_cached_slab(
      reinterpret_cast<AndroidSurfaceState*>(handle_ptr), env, kSlabMetadata);
}
