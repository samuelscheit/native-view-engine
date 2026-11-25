#include "fabric_internal.hpp"
#include "nve_draw_list.hpp"

#include <cstdlib>

namespace nve {

namespace {

uint8_t parse_align_self_prop(const RawPropValue& val) noexcept {
  if (std::holds_alternative<std::nullptr_t>(val)) {
    return kAlignSelfAuto;
  }
  if (const auto* i = std::get_if<int64_t>(&val)) {
    if (*i >= 0 && *i <= 4) {
      return static_cast<uint8_t>(*i);
    }
    return kAlignSelfAuto;
  }
  if (const auto* s = std::get_if<std::string>(&val)) {
    if (*s == "stretch") return 0u;
    if (*s == "flex-start" || *s == "flexStart" || *s == "start") return 1u;
    if (*s == "center") return 2u;
    if (*s == "flex-end" || *s == "flexEnd" || *s == "end") return 3u;
    if (*s == "baseline") return 4u;
    return kAlignSelfAuto;
  }
  return kAlignSelfAuto;
}

Dimension parse_flex_basis_prop(const RawPropValue& val) noexcept {
  if (std::holds_alternative<std::nullptr_t>(val)) {
    return Dimension{0u, 0.0f};
  }
  if (const auto* i = std::get_if<int64_t>(&val)) {
    const float v = static_cast<float>(*i);
    if (is_valid_scalar(v) && v >= 0.0f) {
      return Dimension{1u, v};
    }
    return Dimension{0u, 0.0f};
  }
  if (const auto* d = std::get_if<double>(&val)) {
    const float v = static_cast<float>(*d);
    if (is_valid_scalar(v) && v >= 0.0f) {
      return Dimension{1u, v};
    }
    return Dimension{0u, 0.0f};
  }
  if (const auto* s = std::get_if<std::string>(&val)) {
    if (s->empty() || *s == "auto") {
      return Dimension{0u, 0.0f};
    }
    const bool is_percent = (s->back() == '%');
    const std::string num_str = is_percent ? s->substr(0, s->size() - 1) : *s;
    if (num_str.empty()) {
      return Dimension{0u, 0.0f};
    }
    char* end_ptr = nullptr;
    const float v = std::strtof(num_str.c_str(), &end_ptr);
    if (end_ptr != nullptr && *end_ptr == '\0' && is_valid_scalar(v) && v >= 0.0f) {
      return Dimension{is_percent ? uint8_t{2u} : uint8_t{1u}, v};
    }
  }
  return Dimension{0u, 0.0f};
}

bool is_image_src_prop_key(std::string_view key) noexcept {
  return key == "src" || key == "source" || key == "uri" || key == "image_src" ||
         key == "imageSrc";
}

bool is_draw_list_prop_key(std::string_view key) noexcept {
  return key == "drawList" || key == "draw_list";
}

} // namespace

FabricHost::FabricHost(NveSurfaceConfig config) : config_(config) {
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  yg_config_ = YGConfigNew();
  YGConfigSetContext(yg_config_, this);
  if (config_.scale_factor > 0.0f) {
    YGConfigSetPointScaleFactor(yg_config_, config_.scale_factor);
  }
#endif
  ShadowNodeEntry& inserted = nodes_.emplace_fresh(NVE_ROOT_NODE_ID, NVE_COMP_ROOT);
  init_node_yoga(inserted);
  new_nodes_pending_layout_.push_back(NVE_ROOT_NODE_ID);
  mark_node_scene_dirty(inserted, kSceneDirtyAll);
  publish_scene_frame(0u);
  event_ring_.clear();
}

FabricHost::~FabricHost() {
#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
  detach_rn_uimanager();
#endif
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  for (auto& entry : nodes_.entries()) {
    if (entry.yg_node) {
      YGNodeRemoveAllChildren(entry.yg_node);
    }
  }
  for (auto& entry : nodes_.entries()) {
    if (entry.yg_node) {
      YGNodeFree(entry.yg_node);
      entry.yg_node = nullptr;
    }
  }
  if (yg_config_) {
    YGConfigFree(yg_config_);
    yg_config_ = nullptr;
  }
#endif
}

void FabricHost::mark_node_scene_dirty(ShadowNodeEntry& entry, uint8_t mask) {
  if (entry.scene_dirty_gen != scene_dirty_epoch_) {
    entry.scene_dirty_gen = scene_dirty_epoch_;
    entry.scene_dirty_mask = mask;
    frame_dirty_nodes_.push_back(entry.node_id);
  } else {
    entry.scene_dirty_mask |= mask;
  }
}

void FabricHost::mark_layout_dirty_upward(uint32_t id) {
  layout_dirty_ = true;
  uint32_t cur_id = id;
  const size_t max_hops = nodes_.size() + 1u;
  for (size_t hops = 0u; hops < max_hops; ++hops) {
    ShadowNodeEntry* node = nodes_.find(cur_id);
    if (node == nullptr) {
      break;
    }
    const bool already_dirty = node->subtree_layout_dirty;
    node->subtree_layout_dirty = true;
#if !(defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA)
    node->measure_gen = 0u;
    node->baseline_gen = 0u;
#endif
    if (already_dirty && hops > 0u) {
      break;
    }
    if (!node->has_parent) {
      break;
    }
    cur_id = node->parent_id;
  }
}

void FabricHost::destroy_node_if_exists(uint32_t id) {
  ShadowNodeEntry* node = nodes_.find(id);
  if (node == nullptr) {
    return;
  }

  const bool was_text_container = (node->kind == NVE_COMP_TEXT && !node->children.empty());
  const bool had_parent = node->has_parent;
  const uint32_t old_parent_id = node->parent_id;
  std::vector<uint32_t> old_children = std::move(node->children);

#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
  if (YGNodeRef yg = node->yg_node) {
    if (YGNodeRef owner = YGNodeGetOwner(yg)) {
      YGNodeRemoveChild(owner, yg);
    }
    YGNodeRemoveAllChildren(yg);
    YGNodeFree(yg);
    node->yg_node = nullptr;
  }
#endif

  nodes_.erase(id);
  frame_removed_nodes_.push_back(id);
  layout_dirty_ = true;
  if (was_text_container) {
    text_style_dirty_ = true;
  }

  if (had_parent) {
    if (ShadowNodeEntry* parent = nodes_.find(old_parent_id)) {
      std::erase(parent->children, id);
      mark_node_scene_dirty(*parent, kSceneDirtyChildren);
      mark_layout_dirty_upward(old_parent_id);
    }
  }
  for (uint32_t cid : old_children) {
    if (ShadowNodeEntry* child = nodes_.find(cid)) {
      if (child->has_parent && child->parent_id == id) {
        child->has_parent = false;
        child->parent_id = 0u;
      }
    }
  }
}

void FabricHost::propagate_text_styles() {
  if (!text_style_dirty_) {
    return;
  }
  text_style_dirty_ = false;

  const uint64_t pass_gen = ++gen_counter_;
  text_style_stack_.clear();
  if (text_style_stack_.capacity() < 64u) {
    text_style_stack_.reserve(64u);
  }
  text_style_stack_.push_back(TextStyleFrame{NVE_ROOT_NODE_ID, 0xFF000000u, 16.0f, false});

  while (!text_style_stack_.empty()) {
    const TextStyleFrame f = text_style_stack_.back();
    text_style_stack_.pop_back();

    ShadowNodeEntry* entry = nodes_.find(f.node_id);
    if (entry == nullptr || entry->visit_gen == pass_gen) {
      continue;
    }
    entry->visit_gen = pass_gen;

    uint32_t next_color = f.inherited_color;
    float next_font_size = f.inherited_font_size;
    bool next_has_inherited = f.has_inherited;

    if (entry->kind == NVE_COMP_TEXT) {
      if (!entry->has_explicit_style) {
        const uint32_t target_color = f.has_inherited ? f.inherited_color : 0xFF000000u;
        const float target_font_size = f.has_inherited ? f.inherited_font_size : 16.0f;
        const bool color_changed = (entry->style.text_color_rgba != target_color);
        const bool font_changed = (std::fabs(entry->style.font_size - target_font_size) > 1e-3f);
        if (color_changed || font_changed) {
          entry->style.text_color_rgba = target_color;
          entry->style.font_size = target_font_size;
          mark_node_scene_dirty(*entry, kSceneDirtyStyle);
          if (font_changed) {
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
            if (entry->yg_node && YGNodeHasMeasureFunc(entry->yg_node)) {
              YGNodeMarkDirty(entry->yg_node);
            }
#endif
            mark_layout_dirty_upward(entry->node_id);
          }
        }
      }
      next_color = entry->style.text_color_rgba;
      next_font_size = entry->style.font_size;
      next_has_inherited = true;
    }

    for (auto rit = entry->children.rbegin(); rit != entry->children.rend(); ++rit) {
      text_style_stack_.push_back(TextStyleFrame{*rit, next_color, next_font_size, next_has_inherited});
    }
  }
}

void FabricHost::flush_custom_component_props() {
  if (dirty_custom_nodes_.empty()) {
    return;
  }
  for (uint32_t id : dirty_custom_nodes_) {
    ShadowNodeEntry* entry = nodes_.find(id);
    if (entry == nullptr || !entry->raw_props_dirty || entry->custom_name.empty()) {
      continue;
    }
    entry->raw_props_dirty = false;
    auto it = custom_components_.find(entry->custom_name);
    if (it != custom_components_.end() && it->second.on_update_props && entry->raw_props) {
      it->second.on_update_props(entry->node_id, *entry->raw_props);
    }
  }
  dirty_custom_nodes_.clear();
}

int32_t FabricHost::submit_frame(NveBufferSlice slice, NveFrameStats* out_stats) {
  using Clock = std::chrono::steady_clock;
  const auto t0 = Clock::now();

  if (slice.ptr == nullptr && slice.len > 0) {
    return -1;
  }

  BinaryReader r(slice.ptr, slice.len);
  Header h{};
  if (!r.read(h.magic) || !r.read(h.version) || !r.read(h.flags) || !r.read(h.surface_id) ||
      !r.read(h.frame_id) || !r.read(h.op_count)) {
    return -1;
  }
  if (h.magic != NVE_ABI_MAGIC || h.version != NVE_ABI_VERSION) {
    return -2;
  }

  if (nodes_.size() <= 1u && h.op_count >= 16u) {
    const size_t est = std::min<size_t>(kDenseIdCap, static_cast<size_t>(h.op_count) / 3u + 16u);
    nodes_.reserve(est);
    frame_dirty_nodes_.reserve(est);
    new_nodes_pending_layout_.reserve(est);
    scene_.reserve(est);
  }

  uint32_t mutations = 0;
  bool committed = false;
  bool sync_mount = false;

  for (uint32_t i = 0; i < h.op_count; ++i) {
    uint8_t op = 0;
    if (!r.read(op)) return -3;
    switch (static_cast<NveOpcode>(op)) {
      case NVE_OP_CREATE_NODE: {
        uint32_t id = 0;
        uint16_t kind = 0;
        uint16_t name_len = 0;
        std::string_view name;
        if (!r.read(id) || !r.read(kind) || !r.read(name_len) || !r.read_string(name_len, name)) {
          return -4;
        }
        if (!is_valid_component_kind(kind)) {
          return -6;
        }
        destroy_node_if_exists(id);
        ShadowNodeEntry& inserted = nodes_.emplace_fresh(id, kind, name);
        init_node_yoga(inserted);
        new_nodes_pending_layout_.push_back(id);
        mark_node_scene_dirty(inserted, kSceneDirtyAll);
        layout_dirty_ = true;
        if (kind == NVE_COMP_TEXT) {
          text_style_dirty_ = true;
        }
        ++mutations;
        break;
      }
      case NVE_OP_REMOVE_NODE: {
        uint32_t id = 0;
        if (!r.read(id)) return -4;
        if (id == NVE_ROOT_NODE_ID) {
          return -5;
        }
        destroy_node_if_exists(id);
        ++mutations;
        break;
      }
      case NVE_OP_SET_CHILDREN: {
        uint32_t parent_id = 0;
        uint32_t count = 0;
        if (!r.read(parent_id) || !r.read(count)) return -4;
        if (!r.read_u32_array(count, tmp_children_)) return -4;
        ShadowNodeEntry* parent = nodes_.find(parent_id);
        if (parent == nullptr) return -5;

        const uint64_t ancestor_gen = ++gen_counter_;
        uint32_t walk_id = parent_id;
        bool walking = true;
        for (size_t hops = 0; walking && hops <= nodes_.size(); ++hops) {
          ShadowNodeEntry* w = nodes_.find(walk_id);
          if (w == nullptr) break;
          w->visit_gen = ancestor_gen;
          walking = w->has_parent;
          walk_id = w->parent_id;
        }

        const uint64_t dup_gen = ++gen_counter_;
        for (uint32_t cid : tmp_children_) {
          if (cid == parent_id || cid == NVE_ROOT_NODE_ID) {
            return -6;
          }
          if (ShadowNodeEntry* c = nodes_.find(cid)) {
            if (c->visit_gen == ancestor_gen || c->visit_gen == dup_gen) {
              return -6;
            }
            c->visit_gen = dup_gen;
          }
        }

        for (uint32_t old_cid : parent->children) {
          if (ShadowNodeEntry* oc = nodes_.find(old_cid)) {
            if (oc->has_parent && oc->parent_id == parent_id) {
              oc->has_parent = false;
              oc->parent_id = 0u;
            }
          }
        }
        for (uint32_t cid : tmp_children_) {
          if (ShadowNodeEntry* c = nodes_.find(cid)) {
            if (c->has_parent && c->parent_id != parent_id) {
              const uint32_t prev_pid = c->parent_id;
              if (ShadowNodeEntry* prev_parent = nodes_.find(prev_pid)) {
                std::erase(prev_parent->children, cid);
                mark_node_scene_dirty(*prev_parent, kSceneDirtyChildren);
                mark_layout_dirty_upward(prev_pid);
              }
            }
            c->has_parent = true;
            c->parent_id = parent_id;
          }
        }

        parent->children.assign(tmp_children_.begin(), tmp_children_.end());
        sync_children_yoga(*parent);
        mark_node_scene_dirty(*parent, kSceneDirtyChildren);
        mark_layout_dirty_upward(parent_id);
        text_style_dirty_ = true;
        ++mutations;
        break;
      }
      case NVE_OP_SET_TEXT: {
        uint32_t id = 0;
        uint32_t len = 0;
        std::string_view text;
        if (!r.read(id) || !r.read(len) || !r.read_string(len, text)) return -4;
        ShadowNodeEntry* entry = nodes_.find(id);
        if (entry == nullptr) return -5;
        if (entry->text != text) {
          entry->text.assign(text.data(), text.size());
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
          if (entry->yg_node && YGNodeHasMeasureFunc(entry->yg_node)) {
            YGNodeMarkDirty(entry->yg_node);
          }
#endif
          mark_layout_dirty_upward(id);
          mark_node_scene_dirty(*entry, kSceneDirtyText);
        }
        ++mutations;
        break;
      }
      case NVE_OP_SET_LAYOUT: {
        uint32_t id = 0;
        LayoutProps layout{};
        if (!r.read(id) || !r.read_layout(layout)) return -4;
        if (!is_valid_layout(layout)) return -6;
        ShadowNodeEntry* entry = nodes_.find(id);
        if (entry == nullptr) return -5;
        entry->layout = layout;
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
        if (entry->yg_node) {
          apply_yg_layout_props(entry->yg_node, layout);
        }
#endif
        mark_layout_dirty_upward(id);
        ++mutations;
        break;
      }
      case NVE_OP_SET_STYLE: {
        uint32_t id = 0;
        StyleProps style{};
        if (!r.read(id) || !r.read_style(style)) return -4;
        if (!is_valid_style(style)) return -6;
        ShadowNodeEntry* entry = nodes_.find(id);
        if (entry == nullptr) return -5;
        const StyleProps prev_style = entry->style;
        const bool had_explicit = entry->has_explicit_style;
        entry->style = style;
        entry->has_explicit_style = true;
        mark_node_scene_dirty(*entry, kSceneDirtyStyle);

        const bool font_changed = std::fabs(prev_style.font_size - style.font_size) > 1e-5f;
        const bool border_changed = std::fabs(prev_style.border_width - style.border_width) > 1e-5f;
        const bool text_color_changed = (prev_style.text_color_rgba != style.text_color_rgba);

        if (entry->kind == NVE_COMP_TEXT) {
          if (font_changed || border_changed) {
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
            if (entry->yg_node && YGNodeHasMeasureFunc(entry->yg_node)) {
              YGNodeMarkDirty(entry->yg_node);
            }
#endif
            mark_layout_dirty_upward(id);
          }
          if (!entry->children.empty() &&
              (!had_explicit || font_changed || text_color_changed)) {
            text_style_dirty_ = true;
          }
        }
        ++mutations;
        break;
      }
      case NVE_OP_SET_RAW_PROP: {
        uint32_t id = 0;
        uint16_t key_len = 0;
        std::string_view key;
        uint8_t tag = 0;
        if (!r.read(id) || !r.read(key_len) || !r.read_string(key_len, key) || !r.read(tag)) {
          return -4;
        }
        RawPropValue prop_val{nullptr};
        switch (tag) {
          case 0:
            prop_val = nullptr;
            break;
          case 1: {
            uint8_t b = 0;
            if (!r.read(b)) return -4;
            prop_val = (b != 0);
            break;
          }
          case 2: {
            int64_t v = 0;
            if (!r.read(v)) return -4;
            prop_val = v;
            break;
          }
          case 3: {
            double v = 0.0;
            if (!r.read(v)) return -4;
            prop_val = v;
            break;
          }
          case 4: {
            uint32_t vlen = 0;
            std::string_view sv;
            if (!r.read(vlen) || !r.read_string(vlen, sv)) return -4;
            prop_val = std::string(sv);
            break;
          }
          case 5: {
            uint32_t vlen = 0;
            std::vector<uint8_t> bytes;
            if (!r.read(vlen) || !r.read_bytes(vlen, bytes)) return -4;
            prop_val = std::move(bytes);
            break;
          }
          default:
            return -6;
        }
        ShadowNodeEntry* entry = nodes_.find(id);
        if (entry == nullptr) return -5;
        if (key == "alignSelf" || key == "align_self") {
          entry->align_self = parse_align_self_prop(prop_val);
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
          if (entry->yg_node) {
            apply_yg_align_self(entry->yg_node, entry->align_self);
          }
#endif
          mark_layout_dirty_upward(id);
        } else if (key == "flexBasis" || key == "flex_basis") {
          entry->flex_basis = parse_flex_basis_prop(prop_val);
#if defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA
          if (entry->yg_node) {
            apply_yg_flex_basis(entry->yg_node, entry->flex_basis);
          }
#endif
          mark_layout_dirty_upward(id);
        } else if (is_image_src_prop_key(key)) {
          if (const auto* s = std::get_if<std::string>(&prop_val)) {
            entry->image_src.assign(s->data(), s->size());
          } else if (std::holds_alternative<std::nullptr_t>(prop_val)) {
            entry->image_src.clear();
          }
          if (entry->kind == NVE_COMP_IMAGE) {
            mark_node_scene_dirty(*entry, kSceneDirtyText);
          }
        } else if (is_draw_list_prop_key(key)) {
          bool changed = false;
          if (const auto* bytes = std::get_if<std::vector<uint8_t>>(&prop_val)) {
            if (entry->draw_list != *bytes) {
              entry->draw_list = *bytes;
              changed = true;
            }
          } else if (const auto* s = std::get_if<std::string>(&prop_val)) {
            std::vector<uint8_t> decoded;
            if (decode_draw_list_attr_bytes(*s, decoded) == DrawListError::Ok) {
              if (entry->draw_list != decoded) {
                entry->draw_list = std::move(decoded);
                changed = true;
              }
            } else if (!entry->draw_list.empty()) {
              entry->draw_list.clear();
              changed = true;
            }
          } else if (std::holds_alternative<std::nullptr_t>(prop_val)) {
            if (!entry->draw_list.empty()) {
              entry->draw_list.clear();
              changed = true;
            }
          }
          if (entry->kind == NVE_COMP_SKIA_CANVAS && changed) {
            mark_node_scene_dirty(*entry, kSceneDirtyText);
          }
        }
        if (!entry->raw_props) {
          entry->raw_props = std::make_unique<RawPropMap>();
        }
        (*entry->raw_props)[std::string(key)] = std::move(prop_val);
        if (!entry->custom_name.empty() && !entry->raw_props_dirty) {
          entry->raw_props_dirty = true;
          dirty_custom_nodes_.push_back(id);
        }
        ++mutations;
        break;
      }
      case NVE_OP_SET_EVENT_MASK: {
        uint32_t id = 0;
        uint64_t mask = 0;
        if (!r.read(id) || !r.read(mask)) return -4;
        ShadowNodeEntry* entry = nodes_.find(id);
        if (entry == nullptr) return -5;
        if (entry->event_mask != mask) {
          entry->event_mask = mask;
          mark_node_scene_dirty(*entry, kSceneDirtyStyle);
        }
        ++mutations;
        break;
      }
      case NVE_OP_DISPATCH_COMMAND: {
        uint32_t id = 0;
        uint16_t nlen = 0;
        uint32_t plen = 0;
        std::string_view name;
        std::vector<uint8_t> payload;
        if (!r.read(id) || !r.read(nlen) || !r.read_string(nlen, name) || !r.read(plen) ||
            !r.read_bytes(plen, payload)) {
          return -4;
        }
        ShadowNodeEntry* entry = nodes_.find(id);
        if (entry == nullptr) return -5;
        auto comp_it = custom_components_.find(entry->custom_name);
        if (comp_it != custom_components_.end() && comp_it->second.on_dispatch_command) {
          comp_it->second.on_dispatch_command(
              id, name, NveBufferSlice{payload.data(), payload.size()});
        }
        break;
      }
      case NVE_OP_COMMIT: {
        uint8_t sync_flag = 0;
        if (!r.read(sync_flag)) return -4;
        sync_mount = (sync_flag != 0);
        committed = true;
        break;
      }
      default:
        return -7;
    }
  }

  if (r.remaining() != 0) {
    return -8;
  }

  const auto t_decode = Clock::now();
  const auto decode_us = static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::microseconds>(t_decode - t0).count());

  uint64_t layout_us = 0;
  uint64_t diff_us = 0;
  uint64_t mount_us = 0;

  if (committed) {
    const auto t_layout_start = Clock::now();
    run_layout_pass();
    const auto t_layout_end = Clock::now();
    layout_us = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t_layout_end - t_layout_start)
            .count());

    const auto t_diff_start = Clock::now();
    publish_scene_frame(h.frame_id);
    const auto t_diff_end = Clock::now();
    diff_us = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t_diff_end - t_diff_start).count());

    flush_custom_component_props();

#if defined(NVE_WITH_RN_FABRIC) && NVE_WITH_RN_FABRIC
    const auto t_mount_start = Clock::now();
    commit_rn_shadow_tree(sync_mount);
    const auto t_mount_end = Clock::now();
    mount_us = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(t_mount_end - t_mount_start).count());
#else
    (void)sync_mount;
#endif
  }

  if (out_stats != nullptr) {
    out_stats->op_count = h.op_count;
    out_stats->mutation_count = mutations;
    out_stats->decode_us = decode_us;
    out_stats->layout_us = layout_us;
    out_stats->diff_us = diff_us;
    out_stats->mount_us = mount_us;
  }
  return 0;
}

std::vector<uint8_t> FabricHost::serialize_frame0(uint64_t frame_id) const {
  std::vector<uint32_t> ids;
  ids.reserve(nodes_.size());
  for (const auto& entry : nodes_.entries()) {
    ids.push_back(entry.node_id);
  }
  std::sort(ids.begin(), ids.end());

  CommandBufferEncoder enc(config_.surface_id, frame_id);
  enc.reserve(ids.size() * 160u + 64u);

  for (uint32_t id : ids) {
    if (id == NVE_ROOT_NODE_ID) {
      continue;
    }
    if (const ShadowNodeEntry* entry = nodes_.find(id)) {
      enc.create_node(entry->node_id, entry->kind, entry->custom_name);
    }
  }

  for (uint32_t id : ids) {
    if (const ShadowNodeEntry* entry = nodes_.find(id); entry && !entry->children.empty()) {
      enc.set_children(entry->node_id, entry->children);
    }
  }

  for (uint32_t id : ids) {
    const ShadowNodeEntry* entry = nodes_.find(id);
    if (entry == nullptr) {
      continue;
    }
    if (id != NVE_ROOT_NODE_ID) {
      enc.set_layout(entry->node_id, entry->layout);
    }
    if (entry->has_explicit_style) {
      enc.set_style(entry->node_id, entry->style);
    }
    if (!entry->text.empty()) {
      enc.set_text(entry->node_id, entry->text);
    }
    if (entry->event_mask != 0u) {
      enc.set_event_mask(entry->node_id, entry->event_mask);
    }
    if (entry->raw_props && !entry->raw_props->empty()) {
      std::vector<std::string_view> keys;
      keys.reserve(entry->raw_props->size());
      for (const auto& [k, _] : *entry->raw_props) {
        keys.push_back(k);
      }
      std::sort(keys.begin(), keys.end());
      for (std::string_view key : keys) {
        const RawPropValue& val = entry->raw_props->at(std::string(key));
        std::visit(
            [&](const auto& v) {
              using T = std::decay_t<decltype(v)>;
              if constexpr (std::is_same_v<T, std::nullptr_t>) {
                enc.set_raw_prop_null(entry->node_id, key);
              } else if constexpr (std::is_same_v<T, bool>) {
                enc.set_raw_prop_bool(entry->node_id, key, v);
              } else if constexpr (std::is_same_v<T, int64_t>) {
                enc.set_raw_prop_i64(entry->node_id, key, v);
              } else if constexpr (std::is_same_v<T, double>) {
                enc.set_raw_prop_f64(entry->node_id, key, v);
              } else if constexpr (std::is_same_v<T, std::string>) {
                enc.set_raw_prop_string(entry->node_id, key, v);
              } else if constexpr (std::is_same_v<T, std::vector<uint8_t>>) {
                enc.set_raw_prop_bytes(entry->node_id, key, v);
              }
            },
            val);
      }
    }
  }

  enc.commit(true);
  (void)enc.finish();
  return std::move(enc.raw());
}

} // namespace nve
