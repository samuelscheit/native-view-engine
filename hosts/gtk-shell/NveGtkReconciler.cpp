#include "NveGtkInternal.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <unordered_set>
#include <utility>

namespace nve {

namespace gtk_internal {

void unparent_all_fixed_children(GtkWidget* fixed_widget) {
  if (fixed_widget == nullptr || !GTK_IS_FIXED(fixed_widget)) {
    return;
  }
  while (GtkWidget* child = gtk_widget_get_first_child(fixed_widget)) {
    gtk_fixed_remove(GTK_FIXED(fixed_widget), child);
  }
}

void detach_from_parent(GtkWidget* widget) {
  if (widget == nullptr) {
    return;
  }
  GtkWidget* parent = gtk_widget_get_parent(widget);
  if (parent == nullptr) {
    return;
  }
  if (GTK_IS_FIXED(parent)) {
    gtk_fixed_remove(GTK_FIXED(parent), widget);
  } else {
    gtk_widget_unparent(widget);
  }
}

} // namespace gtk_internal

// -----------------------------------------------------------------------------
// NveGtkHost incremental NveSceneView reconciliation
// -----------------------------------------------------------------------------

void NveGtkHost::refresh_raw_props_from_view(const NveSceneView& view) {
  const auto snapshots = session_.snapshot_nodes();
  const auto raw_props = gtk_internal::extract_raw_props_by_node(snapshots);

  std::unordered_map<uint32_t, GtkNodeSemantics> next_semantics;
  next_semantics.reserve(view.node_count);
  std::unordered_map<uint32_t, std::string> next_resize_modes;

  for (uint32_t i = 0u; i < view.node_count; ++i) {
    const NveRenderNode& node = view.nodes[i];
    const auto rp_it = raw_props.find(node.node_id);
    const RawPropMap* rp = (rp_it != raw_props.end()) ? &rp_it->second : nullptr;
    const std::string_view text = scene_node_text(view, node);
    GtkNodeSemantics sem = gtk_internal::extract_node_semantics(node, text, rp);

    const auto prev_sem_it = node_semantics_.find(node.node_id);
    const bool sem_changed = (prev_sem_it == node_semantics_.end() || prev_sem_it->second != sem);
    next_semantics.emplace(node.node_id, sem);

    std::string mode;
    bool mode_changed = false;
    if (node.kind == NVE_NODE_KIND_IMAGE) {
      mode = gtk_internal::extract_image_resize_mode(rp);
      const auto prev_mode_it = node_image_resize_modes_.find(node.node_id);
      mode_changed =
          (prev_mode_it == node_image_resize_modes_.end() || prev_mode_it->second != mode);
      next_resize_modes.emplace(node.node_id, mode);
    }

    if (sem_changed || mode_changed) {
      if (node.node_id == NVE_ROOT_NODE_ID) {
        apply_accessibility_to_widget(root_widget_, sem);
      } else if (GtkWidget** slot = reconciler_.find(node.node_id);
                 slot != nullptr && *slot != nullptr) {
        GtkWidget* w = *slot;
        if (sem_changed) {
          apply_accessibility_to_widget(w, sem);
          if (GTK_IS_ENTRY(w)) {
            gtk_entry_set_placeholder_text(
                GTK_ENTRY(w), sem.placeholder.empty() ? nullptr : sem.placeholder.c_str());
            gtk_entry_set_visibility(GTK_ENTRY(w), sem.secure_text_entry ? FALSE : TRUE);
          }
          const bool interactive =
              !sem.disabled && !gtk_internal::is_leaf_text_node(node) &&
              (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u);
          ensure_click_gesture(node.node_id, w, interactive);
        }
        if (mode_changed && GTK_IS_PICTURE(w)) {
          apply_picture_fit(w, mode);
        }
      }
    }
  }

  node_semantics_ = std::move(next_semantics);
  node_image_resize_modes_ = std::move(next_resize_modes);
}

void NveGtkHost::sync_views(bool check_raw_props) {
  NveFabricHostHandle host = session_.active_fabric_host();
  if (host == nullptr) {
    return;
  }

  const NveSceneView view = session_.scene_view();
  if (view.schema_hash != NVE_SCENE_SCHEMA_HASH) {
    return;
  }

  for (uint32_t i = 0u; i < view.removed_count; ++i) {
    const uint32_t rid = view.removed_ids[i];
    node_semantics_.erase(rid);
    node_image_srcs_.erase(rid);
    node_image_resize_modes_.erase(rid);
  }

  bool need_raw_props = check_raw_props || !raw_props_initialized_;
  if (!need_raw_props && view.nodes != nullptr) {
    for (uint32_t i = 0u; i < view.dirty_count; ++i) {
      const uint32_t idx = view.dirty_indices[i];
      if (idx < view.node_count) {
        const uint8_t k = view.nodes[idx].kind;
        if (k == NVE_NODE_KIND_IMAGE || k == NVE_NODE_KIND_TEXT_INPUT) {
          need_raw_props = true;
          break;
        }
      }
    }
  }

  syncing_views_ = true;
  if (need_raw_props) {
    refresh_raw_props_from_view(view);
    raw_props_initialized_ = true;
  }
  (void)reconciler_.sync(view, adapter_);
  flush_css_if_dirty();
  syncing_views_ = false;
}

NveGtkHost::ScrollBinding* NveGtkHost::ensure_scroll_binding(uint32_t node_id, GtkWidget* widget) {
  if (widget == nullptr || !GTK_IS_SCROLLED_WINDOW(widget)) {
    return nullptr;
  }
  const auto it = scroll_bindings_.find(node_id);
  if (it != scroll_bindings_.end() && it->second != nullptr && it->second->scrolled == widget) {
    return it->second.get();
  }
  remove_scroll_binding(node_id);

  auto binding = std::make_unique<ScrollBinding>();
  binding->host = this;
  binding->node_id = node_id;
  binding->scrolled = widget;
  binding->hadj = gtk_scrolled_window_get_hadjustment(GTK_SCROLLED_WINDOW(widget));
  binding->vadj = gtk_scrolled_window_get_vadjustment(GTK_SCROLLED_WINDOW(widget));
  if (binding->hadj != nullptr) {
    binding->h_handler = g_signal_connect(
        binding->hadj,
        "value-changed",
        gtk_detail::to_gcallback(&NveGtkHost::on_scroll_value_changed),
        binding.get());
  }
  if (binding->vadj != nullptr) {
    binding->v_handler = g_signal_connect(
        binding->vadj,
        "value-changed",
        gtk_detail::to_gcallback(&NveGtkHost::on_scroll_value_changed),
        binding.get());
  }

  ScrollBinding* raw = binding.get();
  scroll_bindings_[node_id] = std::move(binding);
  return raw;
}

void NveGtkHost::remove_scroll_binding(uint32_t node_id) {
  const auto it = scroll_bindings_.find(node_id);
  if (it == scroll_bindings_.end()) {
    return;
  }
  if (it->second != nullptr) {
    if (it->second->hadj != nullptr && it->second->h_handler != 0u &&
        g_signal_handler_is_connected(it->second->hadj, it->second->h_handler)) {
      g_signal_handler_disconnect(it->second->hadj, it->second->h_handler);
    }
    if (it->second->vadj != nullptr && it->second->v_handler != 0u &&
        g_signal_handler_is_connected(it->second->vadj, it->second->v_handler)) {
      g_signal_handler_disconnect(it->second->vadj, it->second->v_handler);
    }
  }
  scroll_bindings_.erase(it);
}

LayoutRect NveGtkHost::layout_for_node(uint32_t node_id) const noexcept {
  const auto it = node_layouts_.find(node_id);
  return (it != node_layouts_.end()) ? it->second : LayoutRect{};
}

void NveGtkHost::ensure_click_gesture(uint32_t node_id, GtkWidget* widget, bool interactive) {
  if (!interactive) {
    remove_click_gesture(node_id, widget);
    return;
  }
  const auto it = click_bindings_.find(node_id);
  if (it != click_bindings_.end() && it->second != nullptr && it->second->gesture != nullptr) {
    return;
  }
  auto binding = std::make_unique<ClickBinding>();
  binding->host = this;
  binding->node_id = node_id;
  GtkGesture* click = gtk_gesture_click_new();
  binding->gesture = click;
  g_signal_connect(
      click, "released", gtk_detail::to_gcallback(&NveGtkHost::on_gesture_released), binding.get());
  gtk_widget_add_controller(widget, GTK_EVENT_CONTROLLER(click));
  click_bindings_[node_id] = std::move(binding);
}

void NveGtkHost::remove_click_gesture(uint32_t node_id, GtkWidget* widget) {
  const auto it = click_bindings_.find(node_id);
  if (it == click_bindings_.end()) {
    return;
  }
  if (widget != nullptr && it->second != nullptr && it->second->gesture != nullptr) {
    gtk_widget_remove_controller(widget, GTK_EVENT_CONTROLLER(it->second->gesture));
  }
  click_bindings_.erase(it);
}

void NveGtkHost::GtkViewAdapter::update_root(
    const NveSceneView& /*view*/,
    const NveRenderNode& node) {
  host->node_layouts_[NVE_ROOT_NODE_ID] = LayoutRect{node.x, node.y, node.width, node.height};
  std::string next_root_css;
  if (node.bg_rgba != 0u) {
    next_root_css = "." + host->scoped_root_class() + " {\n  background-color: " +
                    gtk_internal::format_css_rgba(node.bg_rgba) + ";\n}\n";
  }
  if (host->root_css_ != next_root_css) {
    host->root_css_ = std::move(next_root_css);
    host->css_dirty_ = true;
  }
  if (const GtkNodeSemantics* sem = host->find_semantics(node.node_id)) {
    host->apply_accessibility_to_widget(host->root_widget_, *sem);
  }
}

GtkWidget* NveGtkHost::GtkViewAdapter::create_view(
    const NveSceneView& view,
    const NveRenderNode& node) {
  const std::string_view text = scene_node_text(view, node);
  const GtkNodeSemantics default_sem = gtk_internal::extract_node_semantics(node, text, nullptr);
  const GtkNodeSemantics& sem =
      host->find_semantics(node.node_id) ? *host->find_semantics(node.node_id) : default_sem;

  GtkWidget* widget = nullptr;
  if (gtk_internal::is_leaf_text_node(node)) {
    widget = GTK_WIDGET(g_object_new(
        GTK_TYPE_LABEL, "label", "", "accessible-role", sem.gtk_role, nullptr));
    gtk_label_set_wrap(GTK_LABEL(widget), TRUE);
    gtk_label_set_wrap_mode(GTK_LABEL(widget), PANGO_WRAP_WORD_CHAR);
    gtk_label_set_xalign(GTK_LABEL(widget), 0.0f);
    gtk_label_set_yalign(GTK_LABEL(widget), 0.0f);
    gtk_widget_set_can_target(widget, FALSE);
  } else if (node.kind == NVE_NODE_KIND_IMAGE) {
    widget = GTK_WIDGET(g_object_new(
        GTK_TYPE_PICTURE, "accessible-role", sem.gtk_role, nullptr));
    gtk_picture_set_can_shrink(GTK_PICTURE(widget), TRUE);
  } else if (node.kind == NVE_NODE_KIND_SKIA_CANVAS) {
    widget = GTK_WIDGET(g_object_new(
        GTK_TYPE_DRAWING_AREA, "accessible-role", sem.gtk_role, nullptr));
    (void)host->ensure_canvas_binding(node.node_id, widget);
  } else if (node.kind == NVE_NODE_KIND_SCROLL_VIEW) {
    widget = GTK_WIDGET(
        g_object_new(GTK_TYPE_SCROLLED_WINDOW, "accessible-role", sem.gtk_role, nullptr));
    GtkWidget* inner_fixed = gtk_fixed_new();
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(widget), inner_fixed);
    host->scroll_contents_[widget] = inner_fixed;
    (void)host->ensure_scroll_binding(node.node_id, widget);
  } else if (node.kind == NVE_NODE_KIND_TEXT_INPUT) {
    widget = GTK_WIDGET(g_object_new(GTK_TYPE_ENTRY, "accessible-role", sem.gtk_role, nullptr));
  } else {
    widget = GTK_WIDGET(g_object_new(GTK_TYPE_FIXED, "accessible-role", sem.gtk_role, nullptr));
  }

  g_object_ref_sink(widget);
  const std::string generic_cls = "nve-node-" + std::to_string(node.node_id);
  const std::string scoped_cls = host->scoped_node_class(node.node_id);
  gtk_widget_add_css_class(widget, generic_cls.c_str());
  gtk_widget_add_css_class(widget, scoped_cls.c_str());
  return widget;
}

void NveGtkHost::GtkViewAdapter::update_view(
    const NveSceneView& view,
    GtkWidget*& widget,
    const NveRenderNode& node) {
  if (widget == nullptr) {
    return;
  }
  ++host->reconcile_update_count_;

  const std::string_view text = scene_node_text(view, node);
  const GtkNodeSemantics default_sem = gtk_internal::extract_node_semantics(node, text, nullptr);
  const GtkNodeSemantics& sem =
      host->find_semantics(node.node_id) ? *host->find_semantics(node.node_id) : default_sem;

  host->node_layouts_[node.node_id] = LayoutRect{node.x, node.y, node.width, node.height};

  const int req_w = (std::isfinite(node.width) && node.width > 0.0f)
                        ? static_cast<int>(std::lround(node.width))
                        : -1;
  const int req_h = (std::isfinite(node.height) && node.height > 0.0f)
                        ? static_cast<int>(std::lround(node.height))
                        : -1;
  gtk_widget_set_size_request(widget, req_w, req_h);

  const double clamped_opacity = static_cast<double>(node.opacity_u8) / 255.0;
  gtk_widget_set_opacity(widget, clamped_opacity);

  if (GTK_IS_LABEL(widget)) {
    const char* current = gtk_label_get_text(GTK_LABEL(widget));
    if (current == nullptr || text != std::string_view(current)) {
      const std::string owned(text);
      gtk_label_set_text(GTK_LABEL(widget), owned.c_str());
    }
    gtk_widget_set_can_target(widget, FALSE);
  } else if (GTK_IS_PICTURE(widget)) {
    const auto mode_it = host->node_image_resize_modes_.find(node.node_id);
    const std::string mode = (mode_it != host->node_image_resize_modes_.end())
                                 ? mode_it->second
                                 : std::string("cover");
    host->apply_picture_fit(widget, mode);

    const bool clip = std::isfinite(node.border_radius) && node.border_radius > 0.0f;
    gtk_widget_set_overflow(widget, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);

    const auto src_it = host->node_image_srcs_.find(node.node_id);
    if (src_it == host->node_image_srcs_.end() || src_it->second != text) {
      host->node_image_srcs_[node.node_id] = std::string(text);
      GdkTexture* tex = host->texture_cache_.get(text);
      gtk_picture_set_paintable(
          GTK_PICTURE(widget), tex != nullptr ? GDK_PAINTABLE(tex) : nullptr);
    }
  } else if (GTK_IS_DRAWING_AREA(widget)) {
    const bool clip = std::isfinite(node.border_radius) && node.border_radius > 0.0f;
    gtk_widget_set_overflow(widget, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);
    gtk_drawing_area_set_content_width(GTK_DRAWING_AREA(widget), std::max(0, req_w));
    gtk_drawing_area_set_content_height(GTK_DRAWING_AREA(widget), std::max(0, req_h));

    CanvasBinding* binding = host->ensure_canvas_binding(node.node_id, widget);
    if (binding != nullptr) {
      const auto* raw_ptr = reinterpret_cast<const uint8_t*>(text.data());
      const size_t raw_len = text.size();
      const bool same =
          binding->draw_list_bytes.size() == raw_len &&
          (raw_len == 0u ||
           std::memcmp(binding->draw_list_bytes.data(), raw_ptr, raw_len) == 0);
      if (!same) {
        binding->draw_list_bytes.assign(raw_ptr, raw_ptr + raw_len);
        ++binding->redraw_count;
        gtk_widget_queue_draw(widget);
      }
    }
  } else if (GTK_IS_ENTRY(widget)) {
    EntryBinding* binding = host->ensure_entry_binding(node.node_id, widget);
    gtk_entry_set_placeholder_text(
        GTK_ENTRY(widget), sem.placeholder.empty() ? nullptr : sem.placeholder.c_str());
    gtk_entry_set_visibility(GTK_ENTRY(widget), sem.secure_text_entry ? FALSE : TRUE);

    if (binding != nullptr && !binding->user_editing) {
      const char* current = gtk_editable_get_text(GTK_EDITABLE(widget));
      const std::string_view current_sv =
          (current != nullptr) ? std::string_view(current) : std::string_view{};
      if (!binding->has_synced_snapshot ||
          (text != binding->last_snapshot_text && text != current_sv)) {
        binding->updating_programmatically = true;
        const std::string owned(text);
        gtk_editable_set_text(GTK_EDITABLE(widget), owned.c_str());
        binding->last_dispatched_text = owned;
        binding->updating_programmatically = false;
      }
      binding->last_snapshot_text.assign(text.data(), text.size());
      binding->has_synced_snapshot = true;
    }
  } else if (GTK_IS_FIXED(widget)) {
    const bool clip = std::isfinite(node.border_radius) && node.border_radius > 0.0f;
    gtk_widget_set_overflow(widget, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);
  } else if (GTK_IS_SCROLLED_WINDOW(widget)) {
    (void)host->ensure_scroll_binding(node.node_id, widget);
    const bool clip = std::isfinite(node.border_radius) && node.border_radius > 0.0f;
    gtk_widget_set_overflow(widget, clip ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);
  }

  host->apply_accessibility_to_widget(widget, sem);

  const bool interactive =
      !sem.disabled && !gtk_internal::is_leaf_text_node(node) &&
      (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u);
  host->ensure_click_gesture(node.node_id, widget, interactive);

  host->set_node_css(
      node.node_id, gtk_internal::build_node_css_rule(host->scoped_node_class(node.node_id), node));
}

void NveGtkHost::GtkViewAdapter::destroy_view(uint32_t node_id, GtkWidget*& view) {
  host->remove_click_gesture(node_id, view);
  host->remove_scroll_binding(node_id);
  host->remove_entry_binding(node_id, view);
  host->remove_canvas_binding(node_id, view);
  host->node_layouts_.erase(node_id);
  host->node_semantics_.erase(node_id);
  host->node_image_srcs_.erase(node_id);
  host->node_image_resize_modes_.erase(node_id);
  host->erase_node_css(node_id);

  if (view == nullptr) {
    return;
  }

  auto scroll_it = host->scroll_contents_.find(view);
  if (scroll_it != host->scroll_contents_.end()) {
    gtk_internal::unparent_all_fixed_children(scroll_it->second);
    gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(view), nullptr);
    host->scroll_contents_.erase(scroll_it);
  } else if (GTK_IS_FIXED(view)) {
    gtk_internal::unparent_all_fixed_children(view);
  }

  gtk_internal::detach_from_parent(view);
  g_object_unref(view);
  view = nullptr;
}

void NveGtkHost::sync_children_impl(
    uint32_t parent_id,
    GtkWidget** parent_view,
    const std::vector<std::pair<uint32_t, GtkWidget*>>& resolved_children) {
  GtkWidget* container = nullptr;
  bool is_scroll_container = false;

  if (parent_id == NVE_ROOT_NODE_ID) {
    container = root_widget_;
  } else if (parent_view != nullptr && *parent_view != nullptr) {
    GtkWidget* parent = *parent_view;
    auto scroll_it = scroll_contents_.find(parent);
    if (scroll_it != scroll_contents_.end()) {
      container = scroll_it->second;
      is_scroll_container = true;
    } else if (GTK_IS_FIXED(parent)) {
      container = parent;
    }
  }

  if (container == nullptr || !GTK_IS_FIXED(container)) {
    return;
  }

  std::unordered_set<GtkWidget*> desired_set;
  desired_set.reserve(resolved_children.size());
  for (const auto& [_, child] : resolved_children) {
    desired_set.insert(child);
  }

  GtkWidget* existing = gtk_widget_get_first_child(container);
  while (existing != nullptr) {
    GtkWidget* next = gtk_widget_get_next_sibling(existing);
    if (desired_set.find(existing) == desired_set.end()) {
      gtk_fixed_remove(GTK_FIXED(container), existing);
    }
    existing = next;
  }

  float content_max_w = 0.0f;
  float content_max_h = 0.0f;
  GtkWidget* prev_sibling = nullptr;
  std::unordered_set<GtkWidget*> mounted_this_pass;
  mounted_this_pass.reserve(resolved_children.size());

  for (const auto& [cid, child] : resolved_children) {
    if (!mounted_this_pass.insert(child).second) {
      continue;
    }
    const LayoutRect rect = layout_for_node(cid);
    const double x = std::isfinite(rect.x) ? static_cast<double>(rect.x) : 0.0;
    const double y = std::isfinite(rect.y) ? static_cast<double>(rect.y) : 0.0;

    if (std::isfinite(rect.x) && std::isfinite(rect.width)) {
      content_max_w = std::max(content_max_w, rect.x + rect.width);
    }
    if (std::isfinite(rect.y) && std::isfinite(rect.height)) {
      content_max_h = std::max(content_max_h, rect.y + rect.height);
    }

    GtkWidget* current_parent = gtk_widget_get_parent(child);
    if (current_parent != container) {
      gtk_internal::detach_from_parent(child);
      gtk_fixed_put(GTK_FIXED(container), child, x, y);
    } else {
      gtk_fixed_move(GTK_FIXED(container), child, x, y);
    }

    if (gtk_widget_get_prev_sibling(child) != prev_sibling) {
      gtk_widget_insert_after(child, container, prev_sibling);
      gtk_fixed_move(GTK_FIXED(container), child, x, y);
    }
    prev_sibling = child;
  }

  if (is_scroll_container) {
    const LayoutRect parent_rect = layout_for_node(parent_id);
    const float eff_w = std::max(content_max_w, parent_rect.width);
    const float eff_h = std::max(content_max_h, parent_rect.height);
    const int cw = static_cast<int>(std::lround(eff_w));
    const int ch = static_cast<int>(std::lround(eff_h));
    gtk_widget_set_size_request(container, cw > 0 ? cw : -1, ch > 0 ? ch : -1);

    const auto sbit = scroll_bindings_.find(parent_id);
    if (sbit != scroll_bindings_.end() && sbit->second != nullptr) {
      ScrollBinding& sb = *sbit->second;
      sb.content_width = eff_w;
      sb.content_height = eff_h;
      const double view_w = std::max<double>(0.0, static_cast<double>(parent_rect.width));
      const double view_h = std::max<double>(0.0, static_cast<double>(parent_rect.height));
      if (sb.hadj != nullptr) {
        const double cur_x = gtk_adjustment_get_value(sb.hadj);
        gtk_adjustment_configure(
            sb.hadj,
            std::clamp(cur_x, 0.0, std::max(0.0, static_cast<double>(eff_w) - view_w)),
            0.0,
            std::max(static_cast<double>(eff_w), view_w),
            20.0,
            view_w,
            view_w);
      }
      if (sb.vadj != nullptr) {
        const double cur_y = gtk_adjustment_get_value(sb.vadj);
        gtk_adjustment_configure(
            sb.vadj,
            std::clamp(cur_y, 0.0, std::max(0.0, static_cast<double>(eff_h) - view_h)),
            0.0,
            std::max(static_cast<double>(eff_h), view_h),
            20.0,
            view_h,
            view_h);
      }
    }
  }
}

void NveGtkHost::on_gesture_released(
    GtkGestureClick* gesture,
    gint n_press,
    gdouble x,
    gdouble y,
    gpointer user_data) {
  (void)gesture;
  (void)n_press;
  auto* binding = static_cast<ClickBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr) {
    return;
  }
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  (void)host->simulate_press(node_id, static_cast<float>(x), static_cast<float>(y));
}

void NveGtkHost::on_scroll_value_changed(GtkAdjustment* adjustment, gpointer user_data) {
  (void)adjustment;
  auto* binding = static_cast<ScrollBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr || binding->host->syncing_views_) {
    return;
  }
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  const float ox =
      (binding->hadj != nullptr) ? static_cast<float>(gtk_adjustment_get_value(binding->hadj)) : 0.0f;
  const float oy =
      (binding->vadj != nullptr) ? static_cast<float>(gtk_adjustment_get_value(binding->vadj)) : 0.0f;
  const float cw = std::max(
      binding->content_width,
      (binding->hadj != nullptr) ? static_cast<float>(gtk_adjustment_get_upper(binding->hadj)) : 0.0f);
  const float ch = std::max(
      binding->content_height,
      (binding->vadj != nullptr) ? static_cast<float>(gtk_adjustment_get_upper(binding->vadj)) : 0.0f);
  (void)host->dispatch_scroll(node_id, ox, oy, cw, ch);
}

} // namespace nve
