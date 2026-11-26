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

  size_t total_blob_bytes = 0u;
  for (const auto& [nid, entry] : changed) {
    (void)nid;
    total_blob_bytes += static_cast<size_t>(clamp_u16(entry.label.size())) +
                        static_cast<size_t>(clamp_u16(entry.role.size())) +
                        static_cast<size_t>(clamp_u16(entry.hint.size())) +
                        static_cast<size_t>(clamp_u16(entry.state_desc.size())) +
                        static_cast<size_t>(clamp_u16(entry.placeholder.size())) +
                        static_cast<size_t>(clamp_u16(entry.test_id.size())) +
                        entry.draw_list.size();
  }

  const size_t min_records = std::max(kMetaRecordsInitialCapacity, changed.size());
  if (state->meta_records.size() < min_records) {
    size_t next_cap = std::max(kMetaRecordsInitialCapacity, state->meta_records.size());
    while (next_cap < min_records) {
      next_cap *= 2u;
    }
    state->meta_records.resize(next_cap);
  }

  const size_t min_blob = std::max(kMetaBlobInitialCapacity, total_blob_bytes);
  if (state->meta_blob.size() < min_blob) {
    size_t next_cap = std::max(kMetaBlobInitialCapacity, state->meta_blob.size());
    while (next_cap < min_blob) {
      next_cap *= 2u;
    }
    state->meta_blob.resize(next_cap, 0u);
  }

  uint8_t* blob_dst = state->meta_blob.data();
  size_t blob_pos = 0u;
  size_t rec_idx = 0u;
  for (const auto& [nid, entry] : changed) {
    uint32_t flags = 0u;
    if (entry.disabled) flags |= NVE_META_FLAG_DISABLED;
    if (entry.secure_text_entry) flags |= NVE_META_FLAG_SECURE_TEXT_ENTRY;
    if (entry.multiline) flags |= NVE_META_FLAG_MULTILINE;
    if (!entry.draw_list.empty()) flags |= NVE_META_FLAG_HAS_DRAW_LIST;
    if (entry.draw_list_corrupt) flags |= NVE_META_FLAG_DRAW_LIST_CORRUPT;
    if (entry.accessibility_hidden) flags |= NVE_META_FLAG_ACCESSIBILITY_HIDDEN;
    flags |= (static_cast<uint32_t>(entry.resize_mode) & NVE_META_RESIZE_MODE_MASK)
             << NVE_META_RESIZE_MODE_SHIFT;

    auto append_blob = [&](const void* src, size_t n) -> uint32_t {
      if (n == 0u) {
        return 0u;
      }
      const uint32_t offset = static_cast<uint32_t>(blob_pos);
      std::memcpy(blob_dst + blob_pos, src, n);
      blob_pos += n;
      return offset;
    };

    const uint16_t label_len = clamp_u16(entry.label.size());
    const uint16_t role_len = clamp_u16(entry.role.size());
    const uint16_t hint_len = clamp_u16(entry.hint.size());
    const uint16_t state_len = clamp_u16(entry.state_desc.size());
    const uint16_t placeholder_len = clamp_u16(entry.placeholder.size());
    const uint16_t test_id_len = clamp_u16(entry.test_id.size());
    const uint32_t draw_len = static_cast<uint32_t>(entry.draw_list.size());

    NveNodeMeta rec{};
    rec.node_id = nid;
    rec.flags = flags;
    rec.label_offset = append_blob(entry.label.data(), label_len);
    rec.label_len = label_len;
    rec.role_len = role_len;
    rec.role_offset = append_blob(entry.role.data(), role_len);
    rec.hint_offset = append_blob(entry.hint.data(), hint_len);
    rec.hint_len = hint_len;
    rec.state_desc_len = state_len;
    rec.state_desc_offset = append_blob(entry.state_desc.data(), state_len);
    rec.placeholder_offset = append_blob(entry.placeholder.data(), placeholder_len);
    rec.placeholder_len = placeholder_len;
    rec.test_id_len = test_id_len;
    rec.test_id_offset = append_blob(entry.test_id.data(), test_id_len);
    rec.draw_list_offset = append_blob(entry.draw_list.data(), draw_len);
    rec.draw_list_len = draw_len;
    state->meta_records[rec_idx++] = rec;
  }

  state->meta_record_count = static_cast<uint32_t>(rec_idx);
  state->meta_blob_used_bytes = static_cast<uint32_t>(blob_pos);
  state->metadata_cache_frame = view.frame_id;
  state->metadata_cache_valid = true;
}

} // namespace nve::android_internal
