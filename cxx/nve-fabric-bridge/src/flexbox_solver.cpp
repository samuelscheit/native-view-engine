#include "fabric_internal.hpp"

namespace nve {

namespace {

#if !(defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA)

constexpr uint8_t kChildFlagCrossAuto = 1u << 0;
constexpr uint8_t kChildFlagDeferredStretch = 1u << 1;
constexpr uint8_t kChildFlagMeasuredExactW = 1u << 2;
constexpr uint8_t kChildFlagFrozen = 1u << 3;

inline uint8_t resolve_child_align(
    uint8_t parent_align_items,
    uint8_t child_align_self,
    bool is_row) noexcept {
  const uint8_t align = (child_align_self <= 4u) ? child_align_self : parent_align_items;
  if (!is_row && align == 4u /* Baseline */) {
    return 1u /* FlexStart */;
  }
  return align;
}

inline float sanitize_non_negative(float v) noexcept {
  return (std::isfinite(v) && v > 0.0f) ? v : 0.0f;
}

inline float resolve_definite_dim(const Dimension& d, float parent_size) noexcept {
  if (!std::isfinite(d.value)) return -1.0f;
  switch (d.tag) {
    case 1:
      return std::max(0.0f, d.value);
    case 2:
      return (std::isfinite(parent_size) && parent_size >= 0.0f)
                 ? std::max(0.0f, parent_size * (d.value * 0.01f))
                 : -1.0f;
    default:
      return -1.0f;
  }
}

inline float clamp_axis_size(
    float val,
    const Dimension& min_d,
    const Dimension& max_d,
    float parent_size) noexcept {
  val = sanitize_non_negative(val);
  const float max_v = resolve_definite_dim(max_d, parent_size);
  if (max_v >= 0.0f) {
    val = std::min(val, max_v);
  }
  const float min_v = resolve_definite_dim(min_d, parent_size);
  if (min_v >= 0.0f) {
    val = std::max(val, min_v);
  }
  return sanitize_non_negative(val);
}

#endif // !(defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA)

} // namespace

#if !(defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA)

void FabricHost::run_standalone_layout_pass(ShadowNodeEntry& root) {
  const uint64_t ts = now_us();
  const uint64_t measure_gen = measure_epoch_;
  const bool surf_w_ok = std::isfinite(config_.width) && config_.width >= 0.0f;
  const bool surf_h_ok = std::isfinite(config_.height) && config_.height >= 0.0f;
  const float surf_w_base = surf_w_ok ? config_.width : kUnconstrained;
  const float surf_h_base = surf_h_ok ? config_.height : kUnconstrained;
  float root_w = resolve_definite_dim(root.layout.width, surf_w_base);
  float root_h = resolve_definite_dim(root.layout.height, surf_h_base);
  if (root_w < 0.0f && surf_w_ok) {
    root_w = config_.width;
  }
  if (root_h < 0.0f && surf_h_ok) {
    root_h = config_.height;
  }
  if (root_w < 0.0f || root_h < 0.0f) {
    const NveIntrinsicSize root_measured = measure_intrinsic_size(
        NVE_ROOT_NODE_ID,
        (root_w >= 0.0f) ? root_w : 0.0f,
        (root_w >= 0.0f) ? MeasureMode::Exact : MeasureMode::Unconstrained,
        (root_h >= 0.0f) ? root_h : 0.0f,
        (root_h >= 0.0f) ? MeasureMode::Exact : MeasureMode::Unconstrained,
        measure_gen,
        0u);
    if (root_w < 0.0f) root_w = root_measured.width;
    if (root_h < 0.0f) root_h = root_measured.height;
  }
  compute_fallback_layout(
      NVE_ROOT_NODE_ID, 0.0f, 0.0f, root_w, root_h, ts, measure_gen, force_full_layout_, 0u);
}

NveIntrinsicSize FabricHost::measure_text_fallback(
    const ShadowNodeEntry& entry,
    float max_w,
    MeasureMode w_mode) const {
  const float font_size =
      (std::isfinite(entry.style.font_size) && entry.style.font_size > 0.0f) ? entry.style.font_size
                                                                             : 16.0f;
  const bool has_bound =
      (w_mode != MeasureMode::Unconstrained) && std::isfinite(max_w) && max_w >= 0.0f;
  const float bound_w = has_bound ? max_w : 0.0f;
  const size_t len = entry.text.size();
  if (len == 0) {
    return NveIntrinsicSize{(w_mode == MeasureMode::Exact && has_bound) ? bound_w : 0.0f, 0.0f};
  }

  auto clamp_w = [&](float w) noexcept -> float {
    if (w_mode == MeasureMode::Exact && has_bound) return bound_w;
    if (w_mode == MeasureMode::AtMost && has_bound) return std::min(w, bound_w);
    return w;
  };
  if (text_measurer_) {
    const NveIntrinsicSize m = text_measurer_(entry.text, font_size, bound_w);
    return NveIntrinsicSize{clamp_w(sanitize_non_negative(m.width)), sanitize_non_negative(m.height)};
  }

  const float line_height = font_size * 1.3f;
  const float intrinsic_w = static_cast<float>(len) * (font_size * 0.55f) + 4.0f;
  const float out_w = clamp_w(intrinsic_w);
  const float out_h =
      (out_w > 0.0f && intrinsic_w > out_w) ? (std::ceil(intrinsic_w / out_w) * line_height) : line_height;
  return NveIntrinsicSize{out_w, out_h};
}

float FabricHost::compute_node_baseline(
    uint32_t node_id,
    float w,
    float h,
    uint64_t measure_gen,
    uint32_t depth) {
  const float safe_w = sanitize_non_negative(w);
  const float safe_h = sanitize_non_negative(h);
  if (depth > 512u) return safe_h;
  ShadowNodeEntry* node = nodes_.find(node_id);
  if (node == nullptr) return safe_h;
  if (node->kind == NVE_COMP_TEXT && node->children.empty()) {
    return compute_text_leaf_baseline(*node, safe_h);
  }
  if (node->children.empty()) return safe_h;
  if (node->baseline_gen == measure_gen && std::fabs(node->cached_baseline_w - safe_w) <= 1e-3f &&
      std::fabs(node->cached_baseline_h - safe_h) <= 1e-3f) {
    return node->cached_baseline;
  }

  const LayoutProps& nl = node->layout;
  const float pad_w =
      sanitize_non_negative(nl.padding.left) + sanitize_non_negative(nl.padding.right);
  const float pad_h =
      sanitize_non_negative(nl.padding.top) + sanitize_non_negative(nl.padding.bottom);
  const float inner_w = std::max(0.0f, safe_w - pad_w);
  const float inner_h = std::max(0.0f, safe_h - pad_h);

  FlexScratchArena::Scope line_scope(flex_arena_);
  const FlexLineResult line = resolve_flex_line(
      *node, inner_w, MeasureMode::Exact, inner_h, MeasureMode::Exact, true, measure_gen, depth + 1u);

  float result = safe_h;
  if (line.count > 0u) {
    const bool is_row = (nl.flex_direction == 1 || nl.flex_direction == 3);
    size_t baseline_idx = 0u;
    for (size_t i = 0u; i < line.count; ++i) {
      const ShadowNodeEntry* child = nodes_.find(line.live_children[i]);
      if (child != nullptr &&
          resolve_child_align(nl.align_items, child->align_self, is_row) == 4u) {
        baseline_idx = i;
        break;
      }
    }
    const float child_w = is_row ? line.child_main[baseline_idx] : line.child_cross[baseline_idx];
    const float child_h = is_row ? line.child_cross[baseline_idx] : line.child_main[baseline_idx];
    const float child_b = compute_node_baseline(
        line.live_children[baseline_idx], child_w, child_h, measure_gen, depth + 1u);
    result = line.child_y[baseline_idx] + child_b;
  }

  node->baseline_gen = measure_gen;
  node->cached_baseline_w = safe_w;
  node->cached_baseline_h = safe_h;
  node->cached_baseline = result;
  return result;
}

FabricHost::FlexLineResult FabricHost::resolve_flex_line(
    const ShadowNodeEntry& node,
    float inner_w,
    MeasureMode inner_w_mode,
    float inner_h,
    MeasureMode inner_h_mode,
    bool compute_positions,
    uint64_t measure_gen,
    uint32_t depth) {
  FlexLineResult res{};
  if (depth > 512u || node.children.empty()) return res;

  const size_t raw_count = node.children.size();
  res.live_children = flex_arena_.alloc<uint32_t>(raw_count);
  size_t count = 0u;
  for (uint32_t cid : node.children) {
    if (nodes_.find(cid) != nullptr) {
      res.live_children[count++] = cid;
    }
  }
  res.count = count;
  if (count == 0u) return res;

  res.child_main = flex_arena_.alloc<float>(count);
  res.child_cross = flex_arena_.alloc<float>(count);
  if (compute_positions) {
    res.child_x = flex_arena_.alloc<float>(count);
    res.child_y = flex_arena_.alloc<float>(count);
  }

  FlexScratchArena::Scope scratch(flex_arena_);
  const ShadowNodeEntry** child_nodes = flex_arena_.alloc<const ShadowNodeEntry*>(count);
  uint8_t* child_align = flex_arena_.alloc<uint8_t>(count);
  uint8_t* child_flags = flex_arena_.alloc_zeroed<uint8_t>(count);
  float* measured_w = flex_arena_.alloc<float>(count);

  const LayoutProps& nl = node.layout;
  const bool is_row = (nl.flex_direction == 1 || nl.flex_direction == 3);
  const bool is_reverse = (nl.flex_direction == 2 || nl.flex_direction == 3);
  const float pad_l = sanitize_non_negative(nl.padding.left);
  const float pad_t = sanitize_non_negative(nl.padding.top);
  const float gap = sanitize_non_negative(nl.gap);
  const float total_gap = (count > 1u) ? gap * static_cast<float>(count - 1u) : 0.0f;

  const bool w_bounded = (inner_w_mode != MeasureMode::Unconstrained) && std::isfinite(inner_w);
  const bool h_bounded = (inner_h_mode != MeasureMode::Unconstrained) && std::isfinite(inner_h);
  const MeasureMode eff_w_mode = w_bounded ? inner_w_mode : MeasureMode::Unconstrained;
  const MeasureMode eff_h_mode = h_bounded ? inner_h_mode : MeasureMode::Unconstrained;
  const float safe_inner_w = w_bounded ? std::max(0.0f, inner_w) : 0.0f;
  const float safe_inner_h = h_bounded ? std::max(0.0f, inner_h) : 0.0f;

  const MeasureMode main_mode = is_row ? eff_w_mode : eff_h_mode;
  const MeasureMode cross_mode = is_row ? eff_h_mode : eff_w_mode;
  const float main_avail = is_row ? safe_inner_w : safe_inner_h;
  const float cross_avail = is_row ? safe_inner_h : safe_inner_w;
  const float main_pct_base = (main_mode == MeasureMode::Exact) ? main_avail : kUnconstrained;
  const float cross_pct_base = (cross_mode == MeasureMode::Exact) ? cross_avail : kUnconstrained;

  for (size_t i = 0u; i < count; ++i) {
    const ShadowNodeEntry* child = nodes_.find(res.live_children[i]);
    child_nodes[i] = child;
    child_align[i] = resolve_child_align(nl.align_items, child->align_self, is_row);
    if (is_row && child_align[i] == 4u) {
      res.is_baseline_layout = true;
    }
  }

  float fixed_main = 0.0f;
  for (size_t i = 0u; i < count; ++i) {
    const ShadowNodeEntry& child = *child_nodes[i];
    const LayoutProps& cl = child.layout;
    const float c_mw = cl.margin.left + cl.margin.right;
    const float c_mh = cl.margin.top + cl.margin.bottom;
    const float m_margin = is_row ? c_mw : c_mh;
    const float c_margin = is_row ? c_mh : c_mw;
    const Dimension& m_dim = is_row ? cl.width : cl.height;
    const Dimension& c_dim = is_row ? cl.height : cl.width;
    const Dimension& m_min = is_row ? cl.min_width : cl.min_height;
    const Dimension& m_max = is_row ? cl.max_width : cl.max_height;
    const Dimension& c_min = is_row ? cl.min_height : cl.min_width;
    const Dimension& c_max = is_row ? cl.max_height : cl.max_width;

    const float def_basis = (main_mode != MeasureMode::Unconstrained)
                                ? resolve_definite_dim(child.flex_basis, main_pct_base)
                                : kUnconstrained;
    const float def_main =
        (def_basis >= 0.0f) ? def_basis : resolve_definite_dim(m_dim, main_pct_base);
    const float def_cross = resolve_definite_dim(c_dim, cross_pct_base);

    const bool is_stretch = (def_cross < 0.0f && child_align[i] == 0u);
    const bool cross_stretched =
        is_stretch && (cross_mode == MeasureMode::Exact) && !res.is_baseline_layout;
    if (is_stretch && res.is_baseline_layout) child_flags[i] |= kChildFlagDeferredStretch;
    if (def_cross < 0.0f && !cross_stretched) child_flags[i] |= kChildFlagCrossAuto;

    float resolved_main =
        (def_main >= 0.0f) ? clamp_axis_size(def_main, m_min, m_max, main_pct_base) : kUnconstrained;
    float resolved_cross =
        (def_cross >= 0.0f)
            ? clamp_axis_size(def_cross, c_min, c_max, cross_pct_base)
            : (cross_stretched
                   ? clamp_axis_size(std::max(0.0f, cross_avail - c_margin), c_min, c_max, cross_pct_base)
                   : kUnconstrained);

    if (resolved_main < 0.0f || resolved_cross < 0.0f) {
      float c_avail_w = 0.0f;
      MeasureMode c_w_mode = MeasureMode::Unconstrained;
      float c_avail_h = 0.0f;
      MeasureMode c_h_mode = MeasureMode::Unconstrained;

      if (is_row) {
        if (resolved_main >= 0.0f) {
          c_avail_w = resolved_main;
          c_w_mode = MeasureMode::Exact;
        } else if (eff_w_mode != MeasureMode::Unconstrained) {
          c_avail_w = std::max(0.0f, safe_inner_w - c_mw);
          c_w_mode = MeasureMode::AtMost;
        }
        if (resolved_cross >= 0.0f) {
          c_avail_h = resolved_cross;
          c_h_mode = MeasureMode::Exact;
        } else if (eff_h_mode != MeasureMode::Unconstrained) {
          c_avail_h = std::max(0.0f, safe_inner_h - c_mh);
          c_h_mode = MeasureMode::AtMost;
        }
      } else {
        if (resolved_cross >= 0.0f) {
          c_avail_w = resolved_cross;
          c_w_mode = MeasureMode::Exact;
        } else if (eff_w_mode != MeasureMode::Unconstrained) {
          c_avail_w = std::max(0.0f, safe_inner_w - c_mw);
          c_w_mode = MeasureMode::AtMost;
        }
        if (resolved_main >= 0.0f) {
          c_avail_h = resolved_main;
          c_h_mode = MeasureMode::Exact;
        }
      }

      const NveIntrinsicSize measured = measure_intrinsic_size(
          res.live_children[i], c_avail_w, c_w_mode, c_avail_h, c_h_mode, measure_gen, depth + 1u);
      measured_w[i] = measured.width;
      if (c_w_mode == MeasureMode::Exact) child_flags[i] |= kChildFlagMeasuredExactW;

      if (resolved_cross < 0.0f) {
        resolved_cross =
            clamp_axis_size(is_row ? measured.height : measured.width, c_min, c_max, cross_pct_base);
      }
      if (resolved_main < 0.0f) {
        if (!is_row && std::fabs(resolved_cross - measured.width) > 1e-3f) {
          const NveIntrinsicSize remeasured = measure_intrinsic_size(
              res.live_children[i],
              resolved_cross,
              MeasureMode::Exact,
              0.0f,
              MeasureMode::Unconstrained,
              measure_gen,
              depth + 1u);
          measured_w[i] = resolved_cross;
          child_flags[i] |= kChildFlagMeasuredExactW;
          resolved_main = clamp_axis_size(remeasured.height, m_min, m_max, main_pct_base);
        } else {
          resolved_main =
              clamp_axis_size(is_row ? measured.width : measured.height, m_min, m_max, main_pct_base);
        }
      }
    } else {
      measured_w[i] = is_row ? resolved_main : resolved_cross;
      child_flags[i] |= kChildFlagMeasuredExactW;
    }

    res.child_main[i] = resolved_main;
    res.child_cross[i] = resolved_cross;
    fixed_main += resolved_main + m_margin;
  }

  // Iterative CSS Flexbox grow/shrink resolution with min/max clamping freeze rounds.
  const bool can_grow = (main_mode == MeasureMode::Exact);
  const bool can_shrink = (main_mode != MeasureMode::Unconstrained);
  const float initial_free = main_avail - fixed_main - total_gap;
  const bool growing = can_grow && (initial_free > 1e-4f);
  const bool shrinking = can_shrink && (initial_free < -1e-4f);

  if (growing || shrinking) {
    float* base_main = flex_arena_.alloc<float>(count);
    std::memcpy(base_main, res.child_main, count * sizeof(float));
    for (size_t i = 0u; i < count; ++i) {
      const LayoutProps& cl = child_nodes[i]->layout;
      const float factor =
          growing ? sanitize_non_negative(cl.flex_grow) : sanitize_non_negative(cl.flex_shrink);
      if (factor <= 0.0f || (shrinking && base_main[i] <= 0.0f)) {
        child_flags[i] |= kChildFlagFrozen;
      }
    }

    for (size_t iter = 0u; iter < count; ++iter) {
      float used_by_frozen = total_gap;
      float base_unfrozen = 0.0f;
      float total_weight = 0.0f;
      for (size_t i = 0u; i < count; ++i) {
        const LayoutProps& cl = child_nodes[i]->layout;
        const float m_margin =
            is_row ? (cl.margin.left + cl.margin.right) : (cl.margin.top + cl.margin.bottom);
        if ((child_flags[i] & kChildFlagFrozen) != 0u) {
          used_by_frozen += res.child_main[i] + m_margin;
        } else {
          base_unfrozen += base_main[i] + m_margin;
          total_weight += growing ? sanitize_non_negative(cl.flex_grow)
                                  : (sanitize_non_negative(cl.flex_shrink) * base_main[i]);
        }
      }
      if (total_weight <= 0.0f) break;

      const float remaining = main_avail - used_by_frozen - base_unfrozen;
      if ((growing && remaining <= 1e-4f) || (shrinking && remaining >= -1e-4f)) {
        for (size_t i = 0u; i < count; ++i) {
          if ((child_flags[i] & kChildFlagFrozen) == 0u) res.child_main[i] = base_main[i];
        }
        break;
      }

      bool any_clamped = false;
      for (size_t i = 0u; i < count; ++i) {
        if ((child_flags[i] & kChildFlagFrozen) != 0u) continue;
        const LayoutProps& cl = child_nodes[i]->layout;
        const Dimension& m_min = is_row ? cl.min_width : cl.min_height;
        const Dimension& m_max = is_row ? cl.max_width : cl.max_height;
        const float weight = growing ? sanitize_non_negative(cl.flex_grow)
                                     : (sanitize_non_negative(cl.flex_shrink) * base_main[i]);
        const float unclamped = base_main[i] + remaining * (weight / total_weight);
        const float clamped = clamp_axis_size(unclamped, m_min, m_max, main_pct_base);
        res.child_main[i] = clamped;
        if (std::fabs(clamped - unclamped) > 1e-4f) {
          child_flags[i] |= kChildFlagFrozen;
          any_clamped = true;
        }
      }
      if (!any_clamped) break;
    }
  }

  // Post-flex wrapped text & nested container re-measurement at resolved final width.
  if (is_row) {
    for (size_t i = 0u; i < count; ++i) {
      if ((child_flags[i] & kChildFlagCrossAuto) == 0u) continue;
      const ShadowNodeEntry& child = *child_nodes[i];
      const bool width_changed = std::fabs(res.child_main[i] - measured_w[i]) > 1e-3f;
      const bool container_needs_exact =
          (!child.children.empty() && (child_flags[i] & kChildFlagMeasuredExactW) == 0u);
      if (width_changed || container_needs_exact) {
        const LayoutProps& cl = child.layout;
        const NveIntrinsicSize remeasured = measure_intrinsic_size(
            res.live_children[i],
            res.child_main[i],
            MeasureMode::Exact,
            0.0f,
            MeasureMode::Unconstrained,
            measure_gen,
            depth + 1u);
        res.child_cross[i] =
            clamp_axis_size(remeasured.height, cl.min_height, cl.max_height, cross_pct_base);
      }
    }
  }

  // Baseline computation & line cross-size calculation.
  float max_baseline_ascent = 0.0f;
  float max_baseline_descent = 0.0f;
  float max_non_baseline_cross = 0.0f;
  float* child_baseline = res.is_baseline_layout ? flex_arena_.alloc_zeroed<float>(count) : nullptr;

  for (size_t i = 0u; i < count; ++i) {
    const LayoutProps& cl = child_nodes[i]->layout;
    const float c_margin =
        is_row ? (cl.margin.top + cl.margin.bottom) : (cl.margin.left + cl.margin.right);
    if (res.is_baseline_layout && child_align[i] == 4u) {
      const float b = compute_node_baseline(
          res.live_children[i], res.child_main[i], res.child_cross[i], measure_gen, depth + 1u);
      child_baseline[i] = b;
      max_baseline_ascent = std::max(max_baseline_ascent, b + cl.margin.top);
      max_baseline_descent =
          std::max(max_baseline_descent, (res.child_cross[i] - b) + cl.margin.bottom);
    } else {
      max_non_baseline_cross = std::max(max_non_baseline_cross, res.child_cross[i] + c_margin);
    }
  }

  const float baseline_line_cross =
      res.is_baseline_layout ? std::max(0.0f, max_baseline_ascent + max_baseline_descent) : 0.0f;
  const float natural_line_cross = std::max(max_non_baseline_cross, baseline_line_cross);

  if (res.is_baseline_layout) {
    for (size_t i = 0u; i < count; ++i) {
      if ((child_flags[i] & kChildFlagDeferredStretch) != 0u) {
        const LayoutProps& cl = child_nodes[i]->layout;
        const float c_margin = cl.margin.top + cl.margin.bottom;
        res.child_cross[i] = clamp_axis_size(
            std::max(0.0f, natural_line_cross - c_margin),
            cl.min_height,
            cl.max_height,
            cross_pct_base);
      }
    }
  }

  float used_main = total_gap;
  for (size_t i = 0u; i < count; ++i) {
    const LayoutProps& cl = child_nodes[i]->layout;
    const float m_margin =
        is_row ? (cl.margin.left + cl.margin.right) : (cl.margin.top + cl.margin.bottom);
    used_main += res.child_main[i] + m_margin;
  }
  res.content_main = std::max(0.0f, used_main);
  res.content_cross = std::max(0.0f, natural_line_cross);

  if (!compute_positions) return res;

  const float rem_main = main_avail - used_main;
  float leading_main = 0.0f;
  float between_extra = 0.0f;
  switch (nl.justify_content) {
    case 1: leading_main = rem_main * 0.5f; break;
    case 2: leading_main = rem_main; break;
    case 3:
      if (count > 1u && rem_main > 0.0f) between_extra = rem_main / static_cast<float>(count - 1u);
      break;
    case 4:
      if (rem_main > 0.0f) {
        between_extra = rem_main / static_cast<float>(count);
        leading_main = between_extra * 0.5f;
      } else {
        leading_main = rem_main * 0.5f;
      }
      break;
    case 5:
      if (rem_main > 0.0f) {
        between_extra = rem_main / static_cast<float>(count + 1u);
        leading_main = between_extra;
      } else {
        leading_main = rem_main * 0.5f;
      }
      break;
    case 0:
    default: break;
  }

  const float cross_box = res.is_baseline_layout ? natural_line_cross : cross_avail;
  float cursor = is_reverse
      ? ((is_row ? (pad_l + safe_inner_w) : (pad_t + safe_inner_h)) - leading_main)
      : ((is_row ? pad_l : pad_t) + leading_main);

  for (size_t i = 0u; i < count; ++i) {
    const LayoutProps& cl = child_nodes[i]->layout;
    const float m_margin_start = is_row ? cl.margin.left : cl.margin.top;
    const float m_margin_end = is_row ? cl.margin.right : cl.margin.bottom;
    const float c_margin_start = is_row ? cl.margin.top : cl.margin.left;
    const float c_margin_end = is_row ? cl.margin.bottom : cl.margin.right;

    const float rem_cross = cross_box - (res.child_cross[i] + c_margin_start + c_margin_end);
    float cross_offset = c_margin_start;
    switch (child_align[i]) {
      case 2: cross_offset = c_margin_start + rem_cross * 0.5f; break;
      case 3: cross_offset = c_margin_start + rem_cross; break;
      case 4: cross_offset = max_baseline_ascent - child_baseline[i]; break;
      case 0:
      case 1:
      default: cross_offset = c_margin_start; break;
    }

    float main_pos = 0.0f;
    if (is_reverse) {
      cursor -= m_margin_end + res.child_main[i];
      main_pos = cursor;
      cursor -= m_margin_start + gap + between_extra;
    } else {
      main_pos = cursor + m_margin_start;
      cursor += m_margin_start + res.child_main[i] + m_margin_end + gap + between_extra;
    }

    res.child_x[i] = is_row ? main_pos : (pad_l + cross_offset);
    res.child_y[i] = is_row ? (pad_t + cross_offset) : main_pos;
  }

  return res;
}

NveIntrinsicSize FabricHost::measure_intrinsic_size(
    uint32_t node_id,
    float avail_w,
    MeasureMode w_mode,
    float avail_h,
    MeasureMode h_mode,
    uint64_t measure_gen,
    uint32_t depth) {
  if (depth > 512u) return NveIntrinsicSize{0.0f, 0.0f};
  ShadowNodeEntry* node = nodes_.find(node_id);
  if (node == nullptr) return NveIntrinsicSize{0.0f, 0.0f};

  const bool w_ok = (w_mode != MeasureMode::Unconstrained) && std::isfinite(avail_w);
  const bool h_ok = (h_mode != MeasureMode::Unconstrained) && std::isfinite(avail_h);
  const MeasureMode eff_w_mode = w_ok ? w_mode : MeasureMode::Unconstrained;
  const MeasureMode eff_h_mode = h_ok ? h_mode : MeasureMode::Unconstrained;
  const float norm_avail_w = w_ok ? std::max(0.0f, avail_w) : 0.0f;
  const float norm_avail_h = h_ok ? std::max(0.0f, avail_h) : 0.0f;

  if (node->measure_gen == measure_gen &&
      node->cached_w_mode == static_cast<uint8_t>(eff_w_mode) &&
      node->cached_h_mode == static_cast<uint8_t>(eff_h_mode) &&
      (eff_w_mode == MeasureMode::Unconstrained ||
       std::fabs(node->cached_avail_w - norm_avail_w) <= 1e-3f) &&
      (eff_h_mode == MeasureMode::Unconstrained ||
       std::fabs(node->cached_avail_h - norm_avail_h) <= 1e-3f)) {
    return node->cached_measure;
  }

  const LayoutProps& l = node->layout;
  const float pct_base_w = (eff_w_mode == MeasureMode::Exact) ? norm_avail_w : kUnconstrained;
  const float pct_base_h = (eff_h_mode == MeasureMode::Exact) ? norm_avail_h : kUnconstrained;

  float known_w = (eff_w_mode == MeasureMode::Exact)
                      ? norm_avail_w
                      : resolve_definite_dim(l.width, pct_base_w);
  float known_h = (eff_h_mode == MeasureMode::Exact)
                      ? norm_avail_h
                      : resolve_definite_dim(l.height, pct_base_h);
  if (eff_w_mode != MeasureMode::Exact && known_w >= 0.0f) {
    known_w = clamp_axis_size(known_w, l.min_width, l.max_width, pct_base_w);
  }
  if (eff_h_mode != MeasureMode::Exact && known_h >= 0.0f) {
    known_h = clamp_axis_size(known_h, l.min_height, l.max_height, pct_base_h);
  }

  const float pad_w =
      sanitize_non_negative(l.padding.left) + sanitize_non_negative(l.padding.right);
  const float pad_h =
      sanitize_non_negative(l.padding.top) + sanitize_non_negative(l.padding.bottom);

  auto resolve_outer_bound = [](float known,
                                MeasureMode avail_mode,
                                float norm_avail,
                                const Dimension& max_dim,
                                float pct_base,
                                bool use_at_most_avail,
                                float& out_bound) -> MeasureMode {
    if (known >= 0.0f) {
      out_bound = known;
      return MeasureMode::Exact;
    }
    MeasureMode mode = MeasureMode::Unconstrained;
    out_bound = 0.0f;
    if (use_at_most_avail && avail_mode == MeasureMode::AtMost) {
      mode = MeasureMode::AtMost;
      out_bound = norm_avail;
    }
    const float max_v = resolve_definite_dim(max_dim, pct_base);
    if (max_v >= 0.0f) {
      out_bound = (mode == MeasureMode::AtMost) ? std::min(out_bound, max_v) : max_v;
      mode = MeasureMode::AtMost;
    }
    return mode;
  };

  NveIntrinsicSize result{0.0f, 0.0f};
  if (known_w >= 0.0f && known_h >= 0.0f) {
    result = NveIntrinsicSize{known_w, known_h};
  } else if (node->kind == NVE_COMP_TEXT && node->children.empty()) {
    float outer_bound_w = 0.0f;
    const MeasureMode text_w_mode =
        resolve_outer_bound(known_w, eff_w_mode, norm_avail_w, l.max_width, pct_base_w, true, outer_bound_w);
    const float inner_text_w =
        (text_w_mode != MeasureMode::Unconstrained) ? std::max(0.0f, outer_bound_w - pad_w) : 0.0f;
    NveIntrinsicSize measured = measure_text_fallback(*node, inner_text_w, text_w_mode);

    result.width = (known_w >= 0.0f)
                       ? known_w
                       : clamp_axis_size(measured.width + pad_w, l.min_width, l.max_width, pct_base_w);
    if (known_h < 0.0f && text_w_mode != MeasureMode::Unconstrained &&
        result.width > outer_bound_w + 1e-3f) {
      measured = measure_text_fallback(*node, std::max(0.0f, result.width - pad_w), MeasureMode::Exact);
    }
    result.height = (known_h >= 0.0f)
                        ? known_h
                        : clamp_axis_size(measured.height + pad_h, l.min_height, l.max_height, pct_base_h);
  } else if (node->children.empty()) {
    result.width = (known_w >= 0.0f)
                       ? known_w
                       : clamp_axis_size(pad_w, l.min_width, l.max_width, pct_base_w);
    result.height = (known_h >= 0.0f)
                        ? known_h
                        : clamp_axis_size(pad_h, l.min_height, l.max_height, pct_base_h);
  } else {
    float outer_bound_w = 0.0f;
    float outer_bound_h = 0.0f;
    const MeasureMode inner_w_mode =
        resolve_outer_bound(known_w, eff_w_mode, norm_avail_w, l.max_width, pct_base_w, true, outer_bound_w);
    const MeasureMode inner_h_mode =
        resolve_outer_bound(known_h, eff_h_mode, norm_avail_h, l.max_height, pct_base_h, false, outer_bound_h);
    const float inner_avail_w =
        (inner_w_mode != MeasureMode::Unconstrained) ? std::max(0.0f, outer_bound_w - pad_w) : 0.0f;
    const float inner_avail_h =
        (inner_h_mode != MeasureMode::Unconstrained) ? std::max(0.0f, outer_bound_h - pad_h) : 0.0f;

    FlexScratchArena::Scope line_scope(flex_arena_);
    const FlexLineResult line = resolve_flex_line(
        *node, inner_avail_w, inner_w_mode, inner_avail_h, inner_h_mode, false, measure_gen, depth + 1u);

    const bool is_row = (l.flex_direction == 1 || l.flex_direction == 3);
    const float content_w = pad_w + (is_row ? line.content_main : line.content_cross);
    const float content_h = pad_h + (is_row ? line.content_cross : line.content_main);

    result.width = (known_w >= 0.0f)
                       ? known_w
                       : clamp_axis_size(content_w, l.min_width, l.max_width, pct_base_w);
    result.height = (known_h >= 0.0f)
                        ? known_h
                        : clamp_axis_size(content_h, l.min_height, l.max_height, pct_base_h);
  }

  node->measure_gen = measure_gen;
  node->cached_avail_w = norm_avail_w;
  node->cached_avail_h = norm_avail_h;
  node->cached_w_mode = static_cast<uint8_t>(eff_w_mode);
  node->cached_h_mode = static_cast<uint8_t>(eff_h_mode);
  node->cached_measure = result;
  return result;
}

void FabricHost::compute_fallback_layout(
    uint32_t node_id,
    float x,
    float y,
    float w,
    float h,
    uint64_t ts,
    uint64_t measure_gen,
    bool force_subtree,
    uint32_t depth) {
  if (depth > 512u) return;
  ShadowNodeEntry* node = nodes_.find(node_id);
  if (node == nullptr) return;

  const float safe_x = std::isfinite(x) ? x : 0.0f;
  const float safe_y = std::isfinite(y) ? y : 0.0f;
  const float safe_w = sanitize_non_negative(w);
  const float safe_h = sanitize_non_negative(h);

  const bool size_same = node->has_computed_layout &&
                         std::fabs(node->computed_layout.width - safe_w) <= 1e-4f &&
                         std::fabs(node->computed_layout.height - safe_h) <= 1e-4f;

  const LayoutRect next_rect{safe_x, safe_y, safe_w, safe_h};
  const bool changed = !node->has_computed_layout || rect_changed(node->computed_layout, next_rect);
  node->computed_layout = next_rect;
  node->has_computed_layout = true;
  if (changed) {
    mark_node_scene_dirty(*node, kSceneDirtyLayout);
  }
  if ((node->event_mask & kEventMaskLayout) != 0 && changed) {
    std::lock_guard<std::mutex> lock(event_mutex_);
    event_ring_.push_layout(
        config_.surface_id,
        node->node_id,
        ts,
        next_rect.x,
        next_rect.y,
        next_rect.width,
        next_rect.height);
  }

  if (node->children.empty()) {
    node->subtree_layout_dirty = false;
    return;
  }

  if (!force_subtree && !node->subtree_layout_dirty && size_same) {
    return;
  }

  const LayoutProps& nl = node->layout;
  const bool is_row = (nl.flex_direction == 1 || nl.flex_direction == 3);
  const float pad_w =
      sanitize_non_negative(nl.padding.left) + sanitize_non_negative(nl.padding.right);
  const float pad_h =
      sanitize_non_negative(nl.padding.top) + sanitize_non_negative(nl.padding.bottom);
  const float inner_w = std::max(0.0f, safe_w - pad_w);
  const float inner_h = std::max(0.0f, safe_h - pad_h);

  FlexScratchArena::Scope line_scope(flex_arena_);
  const FlexLineResult line = resolve_flex_line(
      *node, inner_w, MeasureMode::Exact, inner_h, MeasureMode::Exact, true, measure_gen, depth);

  for (size_t i = 0u; i < line.count; ++i) {
    const float cw = is_row ? line.child_main[i] : line.child_cross[i];
    const float ch = is_row ? line.child_cross[i] : line.child_main[i];
    compute_fallback_layout(
        line.live_children[i],
        line.child_x[i],
        line.child_y[i],
        cw,
        ch,
        ts,
        measure_gen,
        force_subtree,
        depth + 1u);
  }
  node->subtree_layout_dirty = false;
}

#endif // !(defined(NVE_WITH_RN_YOGA) && NVE_WITH_RN_YOGA)

} // namespace nve
