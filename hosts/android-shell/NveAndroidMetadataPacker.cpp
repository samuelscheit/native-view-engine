#include "NveAndroidSurfaceInternal.hpp"

#include "nve_draw_list.hpp"

#include <algorithm>
#include <cctype>
#include <cstdint>
#include <cstring>
#include <limits>
#include <span>
#include <string>
#include <string_view>
#include <unordered_set>
#include <utility>
#include <vector>

namespace nve::android_internal {

namespace {

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

uint16_t clamp_u16(size_t n) noexcept {
  return static_cast<uint16_t>(std::min<size_t>(n, std::numeric_limits<uint16_t>::max()));
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
    if (n.text_len < nve::kDrawListHeaderSize) return false;
    const size_t off = static_cast<size_t>(n.text_offset);
    const size_t len = static_cast<size_t>(n.text_len);
    if (off > view.text_slab_len || len > view.text_slab_len - off) return false;
    return nve::validate_draw_list(std::span<const uint8_t>(view.text_slab + off, len)).ok();
  }
  return false;
}

bool decode_and_validate_draw_list_str(std::string_view raw, std::vector<uint8_t>& out) {
  std::string_view candidate = raw;
  std::string stripped;
  const bool has_ws = std::any_of(raw.begin(), raw.end(), [](unsigned char c) {
    return std::isspace(c) != 0;
  });
  if (has_ws) {
    stripped.reserve(raw.size());
    for (unsigned char c : raw) {
      if (!std::isspace(c)) {
        stripped.push_back(static_cast<char>(c));
      }
    }
    candidate = stripped;
  }
  if (nve::decode_draw_list_attr_bytes(candidate, out) != nve::DrawListError::Ok) {
    out.clear();
    return false;
  }
  if (!nve::validate_draw_list(out).ok()) {
    out.clear();
    return false;
  }
  return true;
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
          if (nve::validate_draw_list(*bytes).ok()) {
            entry.draw_list = *bytes;
          } else {
            entry.draw_list_corrupt = true;
          }
        }
      } else if (const auto* str = std::get_if<std::string>(&dl_it->second)) {
        if (!str->empty()) {
          std::vector<uint8_t> decoded;
          if (decode_and_validate_draw_list_str(*str, decoded)) {
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

} // namespace

void pack_dirty_metadata(AndroidSurfaceState* state, const NveSceneView& view) {
  if (!is_session_active(state)) {
    return;
  }
  if (state->metadata_cache_valid && state->metadata_cache_frame == view.frame_id) {
    return;
  }

  const bool is_full_sync =
      !state->metadata_cache_valid || view.frame_id < state->metadata_cache_frame;
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
    flags |= (static_cast<uint32_t>(entry.resize_mode) & kMetaResizeModeMask)
             << kMetaResizeModeShift;

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

} // namespace nve::android_internal
