#include "NveGtkInternal.hpp"

#include <algorithm>
#include <cmath>
#include <limits>
#include <string>
#include <utility>

namespace nve::gtk {

using gtk_internal::px;

namespace {

constexpr const char* kScrollOwnerKey = "nve-gtk-scroll-owner";  // on a scroll view's content `GtkFixed`

std::string editable_text(const GtkNodeView& v) {
  if (v.kind == WidgetKind::TextView) {
    GtkTextBuffer* buf = gtk_text_view_get_buffer(GTK_TEXT_VIEW(v.widget));
    GtkTextIter start, end;
    gtk_text_buffer_get_bounds(buf, &start, &end);
    gchar* raw = gtk_text_buffer_get_text(buf, &start, &end, FALSE);
    std::string out = raw != nullptr ? raw : "";
    g_free(raw);
    return out;
  }
  const char* raw = gtk_editable_get_text(GTK_EDITABLE(v.widget));
  return raw != nullptr ? raw : "";
}

void set_editable_text(GtkNodeView& v, const std::string& text) {
  const auto write = v.text.host_write();
  if (v.kind == WidgetKind::TextView) {
    gtk_text_buffer_set_text(gtk_text_view_get_buffer(GTK_TEXT_VIEW(v.widget)), text.c_str(), -1);
  } else {
    gtk_editable_set_text(GTK_EDITABLE(v.widget), text.c_str());
  }
}

void on_text_changed(GtkNodeView* v) {
  if (v->text.writing()) return;
  std::string current = editable_text(*v);
  if (!v->text.user_edited(current)) return;
  // May destroy `v` (the app can remove the input in response): touch nothing afterwards.
  (void)v->host->dispatch_text_change(v->id, current);
}

/// Disconnects every handler bound to `v` before dropping the widget's controller: a controller
/// mid-emission (or still referenced by GTK) must never call back into a freed view.
void remove_controller(GtkNodeView& v, GtkEventController* controller) {
  g_signal_handlers_disconnect_by_data(controller, &v);
  gtk_widget_remove_controller(v.widget, controller);
}

void on_entry_changed(GtkEditable*, gpointer data) { on_text_changed(static_cast<GtkNodeView*>(data)); }
void on_buffer_changed(GtkTextBuffer*, gpointer data) { on_text_changed(static_cast<GtkNodeView*>(data)); }

void on_entry_activate(GtkEntry*, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  (void)v->host->dispatch_submit(v->id, editable_text(*v));
}

void on_focus_enter(GtkEventControllerFocus*, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  (void)v->host->dispatch_focus(v->id, true);
}

void on_focus_leave(GtkEventControllerFocus*, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  (void)v->host->dispatch_focus(v->id, false);
}

void on_click_released(GtkGestureClick*, gint, gdouble x, gdouble y, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  if (v->wants_press) {
    (void)v->host->simulate_press(v->id, static_cast<float>(x), static_cast<float>(y));
    return;
  }
  if (host::openable_link(v->href)) {  // a link nobody intercepts: open it natively (http/https/mailto)
#if GTK_CHECK_VERSION(4, 10, 0)
    GtkUriLauncher* launcher = gtk_uri_launcher_new(v->href.c_str());
    GtkRoot* root = gtk_widget_get_root(v->widget);
    gtk_uri_launcher_launch(launcher, GTK_IS_WINDOW(root) ? GTK_WINDOW(root) : nullptr, nullptr, nullptr, nullptr);
    g_object_unref(launcher);
#endif
  }
}

void on_scroll_value_changed(GtkAdjustment*, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  if (v->scrolling_programmatically) return;
  GtkScrolledWindow* sw = GTK_SCROLLED_WINDOW(v->widget);
  const auto x = static_cast<float>(gtk_adjustment_get_value(gtk_scrolled_window_get_hadjustment(sw)));
  const auto y = static_cast<float>(gtk_adjustment_get_value(gtk_scrolled_window_get_vadjustment(sw)));
  (void)v->host->dispatch_scroll(v->id, x, y, v->content_width(), v->content_height());
}

void on_canvas_draw(GtkDrawingArea*, cairo_t* cr, int width, int height, gpointer data) {
  const auto* v = static_cast<const GtkNodeView*>(data);
  (void)render_draw_list_to_cairo(cr, static_cast<double>(width), static_cast<double>(height), v->draw_list);
}

GtkWidget* new_widget(WidgetKind kind, GtkAccessibleRole role) {
  switch (kind) {
    case WidgetKind::Label: {
      GtkWidget* w = GTK_WIDGET(g_object_new(GTK_TYPE_LABEL, "label", "", "accessible-role", role, nullptr));
      gtk_label_set_xalign(GTK_LABEL(w), 0.0f);
      gtk_label_set_yalign(GTK_LABEL(w), 0.0f);
      gtk_widget_set_can_target(w, FALSE);
      return w;
    }
    case WidgetKind::Picture: {
      GtkWidget* w = GTK_WIDGET(g_object_new(GTK_TYPE_PICTURE, "accessible-role", role, nullptr));
      gtk_picture_set_can_shrink(GTK_PICTURE(w), TRUE);
      return w;
    }
    case WidgetKind::Entry: return GTK_WIDGET(g_object_new(GTK_TYPE_ENTRY, "accessible-role", role, nullptr));
    case WidgetKind::TextView: {
      GtkWidget* w = GTK_WIDGET(g_object_new(GTK_TYPE_TEXT_VIEW, "accessible-role", role, nullptr));
      gtk_text_view_set_wrap_mode(GTK_TEXT_VIEW(w), GTK_WRAP_WORD_CHAR);
      return w;
    }
    case WidgetKind::Scrolled: return GTK_WIDGET(g_object_new(GTK_TYPE_SCROLLED_WINDOW, "accessible-role", role, nullptr));
    case WidgetKind::Canvas: return GTK_WIDGET(g_object_new(GTK_TYPE_DRAWING_AREA, "accessible-role", role, nullptr));
    case WidgetKind::Fixed: break;
  }
  return GTK_WIDGET(g_object_new(GTK_TYPE_FIXED, "accessible-role", role, nullptr));
}

#if GTK_CHECK_VERSION(4, 8, 0)
GtkContentFit content_fit(uint8_t fit) noexcept {
  switch (fit) {
    case NVE_IMAGE_FIT_CONTAIN: return GTK_CONTENT_FIT_CONTAIN;
    case NVE_IMAGE_FIT_FILL: return GTK_CONTENT_FIT_FILL;
    case NVE_IMAGE_FIT_NONE:
    case NVE_IMAGE_FIT_SCALE_DOWN: return GTK_CONTENT_FIT_SCALE_DOWN;
    default: return GTK_CONTENT_FIT_COVER;
  }
}
#endif

}  // namespace

float GtkNodeView::content_width() const noexcept {
  return content != nullptr ? static_cast<float>(std::max(0, gtk_widget_get_width(content))) : 0.0f;
}
float GtkNodeView::content_height() const noexcept {
  return content != nullptr ? static_cast<float>(std::max(0, gtk_widget_get_height(content))) : 0.0f;
}

// --- Adapter ---------------------------------------------------------------------------------

uint32_t NveGtkHost::Adapter::tag(const host::NodeData& node) const {
  const WidgetKind kind = gtk_internal::widget_kind(node, host->canvas_.has(node.index));
  return static_cast<uint32_t>(kind) | (static_cast<uint32_t>(gtk_internal::accessible_role(node)) << 8u);
}

GtkNodeView* NveGtkHost::Adapter::create(const host::NodeData& node) {
  auto* v = new GtkNodeView();
  v->host = host;
  v->id = node.id;
  v->kind = gtk_internal::widget_kind(node, host->canvas_.has(node.index));
  v->widget = new_widget(v->kind, gtk_internal::accessible_role(node));
  g_object_ref_sink(v->widget);
  g_object_set_data(G_OBJECT(v->widget), kViewKey, v);
  gtk_widget_add_css_class(v->widget, host->scoped_class(node.id).c_str());
  switch (v->kind) {
    case WidgetKind::Fixed: v->content = v->widget; break;
    case WidgetKind::Scrolled: {
      v->content = gtk_fixed_new();
      g_object_set_data(G_OBJECT(v->content), kScrollOwnerKey, v);
      gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(v->widget), v->content);
      GtkScrolledWindow* sw = GTK_SCROLLED_WINDOW(v->widget);
      v->h_handler = g_signal_connect(gtk_scrolled_window_get_hadjustment(sw), "value-changed",
                                      gtk_detail::to_gcallback(&on_scroll_value_changed), v);
      v->v_handler = g_signal_connect(gtk_scrolled_window_get_vadjustment(sw), "value-changed",
                                      gtk_detail::to_gcallback(&on_scroll_value_changed), v);
      break;
    }
    case WidgetKind::Entry:
    case WidgetKind::TextView: {
      if (v->kind == WidgetKind::Entry) {
        v->changed_handler = g_signal_connect(v->widget, "changed", gtk_detail::to_gcallback(&on_entry_changed), v);
        v->activate_handler = g_signal_connect(v->widget, "activate", gtk_detail::to_gcallback(&on_entry_activate), v);
      } else {
        v->changed_handler = g_signal_connect(gtk_text_view_get_buffer(GTK_TEXT_VIEW(v->widget)), "changed",
                                              gtk_detail::to_gcallback(&on_buffer_changed), v);
      }
      v->focus = gtk_event_controller_focus_new();
      g_signal_connect(v->focus, "enter", gtk_detail::to_gcallback(&on_focus_enter), v);
      g_signal_connect(v->focus, "leave", gtk_detail::to_gcallback(&on_focus_leave), v);
      gtk_widget_add_controller(v->widget, v->focus);
      break;
    }
    case WidgetKind::Canvas:
      gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(v->widget), &on_canvas_draw, v, nullptr);
      break;
    case WidgetKind::Label:
    case WidgetKind::Picture: break;
  }
  return v;
}

void NveGtkHost::Adapter::update(GtkNodeView*& v, const host::NodeData& node) {
  ++host->reconcile_update_count_;
  GtkWidget* w = v->widget;
  host->apply_geometry(*v, node);
  gtk_widget_set_opacity(w, static_cast<double>(std::clamp(node.visual.opacity, 0.0f, 1.0f)));
  gtk_widget_set_visible(w, node.visual.visibility != NVE_VISIBILITY_HIDDEN);
  const bool clips = node.visual.overflow_x != NVE_OVERFLOW_VISIBLE || node.visual.overflow_y != NVE_OVERFLOW_VISIBLE ||
                     node.visual.border_radius > 0.0f;
  gtk_widget_set_overflow(w, clips ? GTK_OVERFLOW_HIDDEN : GTK_OVERFLOW_VISIBLE);
  gtk_widget_set_cursor_from_name(w, gtk_internal::cursor_name(node.interaction.cursor));

  switch (v->kind) {
    case WidgetKind::Label: {
      const std::string text(node.text);
      const char* current = gtk_label_get_text(GTK_LABEL(w));
      if (current == nullptr || text != current) gtk_label_set_text(GTK_LABEL(w), text.c_str());
      const host::TextStyle& t = node.text_style;
      const bool wraps = t.white_space != NVE_WHITE_SPACE_NO_WRAP && t.white_space != NVE_WHITE_SPACE_PRE;
      gtk_label_set_wrap(GTK_LABEL(w), wraps ? TRUE : FALSE);
      gtk_label_set_wrap_mode(GTK_LABEL(w), PANGO_WRAP_WORD_CHAR);
      const auto lines = static_cast<int>(std::min<uint32_t>(t.max_lines, std::numeric_limits<int>::max()));
      gtk_label_set_lines(GTK_LABEL(w), lines > 0 ? lines : -1);
      gtk_label_set_ellipsize(GTK_LABEL(w), t.max_lines > 0u ? PANGO_ELLIPSIZE_END : PANGO_ELLIPSIZE_NONE);
      const float xalign = t.text_align == NVE_TEXT_ALIGN_CENTER                                   ? 0.5f
                           : (t.text_align == NVE_TEXT_ALIGN_END || t.text_align == NVE_TEXT_ALIGN_RIGHT) ? 1.0f
                                                                                                 : 0.0f;
      gtk_label_set_xalign(GTK_LABEL(w), xalign);
      gtk_label_set_justify(GTK_LABEL(w), t.text_align == NVE_TEXT_ALIGN_JUSTIFY ? GTK_JUSTIFY_FILL
                                          : xalign == 0.5f                       ? GTK_JUSTIFY_CENTER
                                          : xalign == 1.0f                       ? GTK_JUSTIFY_RIGHT
                                                                                 : GTK_JUSTIFY_LEFT);
      break;
    }
    case WidgetKind::Picture: host->update_image(*v, node); break;
    case WidgetKind::Canvas: host->update_canvas(*v, node.index); break;
    case WidgetKind::Entry:
    case WidgetKind::TextView: host->update_text_input(*v, node); break;
    case WidgetKind::Scrolled: host->update_scroll(*v, node); break;
    case WidgetKind::Fixed: break;
  }
  v->href.assign(node.href);
  host->update_press(*v, node);
  host->apply_semantics(*v, node);
  host->apply_css(*v, node);
  v->synced = true;
}

void NveGtkHost::Adapter::move(GtkNodeView*& v, const host::NodeGeometry& node) {
  ++host->reconcile_move_count_;
  host->apply_geometry(*v, node);
}

void NveGtkHost::Adapter::update_root(const host::NodeData& node) {
  const std::string rule = "." + host->scoped_class(node.id) + " {\n  background-color: " +
                           (node.visual.background != 0u ? gtk_internal::css_rgba(node.visual.background)
                                                         : std::string("transparent")) +
                           ";\n}\n";
  if (!gtk_widget_has_css_class(host->root_widget_, host->scoped_class(node.id).c_str())) {
    gtk_widget_add_css_class(host->root_widget_, host->scoped_class(node.id).c_str());
    gtk_widget_add_css_class(host->viewport_widget_, host->scoped_class(node.id).c_str());
  }
  host->set_css_rule(node.id, rule);
  gtk_internal::apply_accessibility(host->root_widget_, gtk_internal::node_semantics(node));
}

void NveGtkHost::Adapter::destroy(uint32_t id, GtkNodeView*& v) {
  if (v == nullptr) return;
  host->css_dirty_ = host->css_rules_.erase(id) > 0u || host->css_dirty_;
  if (v->extents_dirty) std::erase(host->dirty_scroll_extents_, v);
  host->mark_scroll_extents(gtk_widget_get_parent(v->widget));
  if (v->kind == WidgetKind::Scrolled) {
    GtkScrolledWindow* sw = GTK_SCROLLED_WINDOW(v->widget);
    g_signal_handler_disconnect(gtk_scrolled_window_get_hadjustment(sw), v->h_handler);
    g_signal_handler_disconnect(gtk_scrolled_window_get_vadjustment(sw), v->v_handler);
    // The content may outlive this view (GTK refs): it must not lead back to freed memory.
    g_object_set_data(G_OBJECT(v->content), kScrollOwnerKey, nullptr);
    gtk_internal::unparent_children(v->content);
    gtk_scrolled_window_set_child(sw, nullptr);
  } else if (v->kind == WidgetKind::Fixed) {
    gtk_internal::unparent_children(v->content);
  } else if (v->kind == WidgetKind::Entry || v->kind == WidgetKind::TextView) {
    GObject* source = v->kind == WidgetKind::Entry ? G_OBJECT(v->widget)
                                                   : G_OBJECT(gtk_text_view_get_buffer(GTK_TEXT_VIEW(v->widget)));
    g_signal_handler_disconnect(source, v->changed_handler);
    if (v->activate_handler != 0u) g_signal_handler_disconnect(v->widget, v->activate_handler);
    remove_controller(*v, v->focus);
  } else if (v->kind == WidgetKind::Canvas) {
    gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(v->widget), nullptr, nullptr, nullptr);
  }
  if (v->click != nullptr) remove_controller(*v, GTK_EVENT_CONTROLLER(v->click));
  gtk_internal::detach_from_parent(v->widget);
  g_object_set_data(G_OBJECT(v->widget), kViewKey, nullptr);
  g_object_unref(v->widget);
  delete v;
  v = nullptr;
}

void NveGtkHost::Adapter::set_children(const host::NodeGeometry&, GtkNodeView** parent_view,
                                       std::span<GtkNodeView** const> kids) {
  GtkWidget* container = parent_view != nullptr ? (*parent_view)->content : host->root_widget_;
  if (container == nullptr) return;  // leaf widgets (label, picture, ...) render no children
  host->mark_scroll_extents(container);
  GtkWidget* prev = nullptr;
  for (GtkNodeView** kid : kids) {
    GtkNodeView* k = *kid;
    const bool moved_here = gtk_widget_get_parent(k->widget) != container;
    if (moved_here) {
      host->mark_scroll_extents(gtk_widget_get_parent(k->widget));
      gtk_internal::detach_from_parent(k->widget);
    }
    // Paint order == scene child order. (Re-inserting resets a `GtkFixed` child's position, so the
    // position is applied afterwards, always.)
    if (moved_here || gtk_widget_get_prev_sibling(k->widget) != prev) gtk_widget_insert_after(k->widget, container, prev);
    gtk_fixed_move(GTK_FIXED(container), k->widget, static_cast<double>(k->local.x), static_cast<double>(k->local.y));
    prev = k->widget;
  }
  // Anything after the last scene child is stale (moved away or removed from this parent).
  while (GtkWidget* extra = prev != nullptr ? gtk_widget_get_next_sibling(prev) : gtk_widget_get_first_child(container)) {
    gtk_fixed_remove(GTK_FIXED(container), extra);
  }
}

// --- Per-kind bindings -------------------------------------------------------------------------

void NveGtkHost::update_press(GtkNodeView& v, const host::NodeData& node) {
  v.wants_press = node.wants(NVE_EVENT_PRESS);
  const bool needs = !node.interaction.disabled && (v.wants_press || !v.href.empty());
  if (v.kind == WidgetKind::Label) gtk_widget_set_can_target(v.widget, needs ? TRUE : FALSE);  // text buttons
  if (needs && v.click == nullptr) {
    v.click = gtk_gesture_click_new();
    gtk_gesture_single_set_button(GTK_GESTURE_SINGLE(v.click), GDK_BUTTON_PRIMARY);
    g_signal_connect(v.click, "released", gtk_detail::to_gcallback(&on_click_released), &v);
    gtk_widget_add_controller(v.widget, GTK_EVENT_CONTROLLER(v.click));
  } else if (!needs && v.click != nullptr) {
    remove_controller(v, GTK_EVENT_CONTROLLER(v.click));
    v.click = nullptr;
  }
  gtk_widget_set_focusable(v.widget, (node.interaction.focusable || v.kind == WidgetKind::Entry ||
                                      v.kind == WidgetKind::TextView) ? TRUE : FALSE);
}

void NveGtkHost::update_image(GtkNodeView& v, const host::NodeData& node) {
#if GTK_CHECK_VERSION(4, 8, 0)
  gtk_picture_set_content_fit(GTK_PICTURE(v.widget), content_fit(node.image.fit));
#endif
  // Same id, incremental sync, bytes not redefined: the shown texture stays. Otherwise resolve
  // by content (the cache keys URL / encoded bytes). `GtkPicture` refs its paintable, so a later
  // cache eviction cannot free a shown texture.
  if (v.synced && tree_.resource_unchanged(scene_->reader(), node.image.src, v.image_src)) return;
  v.image_src = node.image.src;
  GdkTexture* tex = nullptr;
  if (const auto src = host::decode_image_src(scene_resource(node.image.src, host::kImageSrcKind))) {
    tex = textures_.get(*src);
  }
  GdkPaintable* paintable = tex != nullptr ? GDK_PAINTABLE(tex) : nullptr;
  if (gtk_picture_get_paintable(GTK_PICTURE(v.widget)) != paintable) {
    gtk_picture_set_paintable(GTK_PICTURE(v.widget), paintable);
  }
}

void NveGtkHost::apply_geometry(GtkNodeView& v, const host::NodeGeometry& node) {
  GtkWidget* w = v.widget;
  v.local = node.local;
  gtk_widget_set_size_request(w, px(node.rect.width) > 0 ? px(node.rect.width) : -1,
                              px(node.rect.height) > 0 ? px(node.rect.height) : -1);
  GtkWidget* parent = gtk_widget_get_parent(w);
  if (parent != nullptr && GTK_IS_FIXED(parent)) {
    gtk_fixed_move(GTK_FIXED(parent), w, static_cast<double>(node.local.x), static_cast<double>(node.local.y));
  }
  mark_scroll_extents(parent);
  if (v.kind == WidgetKind::Canvas) {
    gtk_drawing_area_set_content_width(GTK_DRAWING_AREA(w), px(v.local.width));
    gtk_drawing_area_set_content_height(GTK_DRAWING_AREA(w), px(v.local.height));
  }
}

void NveGtkHost::mark_scroll_extents(GtkWidget* container) {
  if (container == nullptr) return;
  if (auto* owner = static_cast<GtkNodeView*>(g_object_get_data(G_OBJECT(container), kScrollOwnerKey))) {
    if (!owner->extents_dirty) {
      owner->extents_dirty = true;
      dirty_scroll_extents_.push_back(owner);
    }
  }
}

void NveGtkHost::update_canvas(GtkNodeView& v, uint32_t index) {
  const uint32_t id = canvas_.at(index).draw_list;
  if (v.synced && tree_.resource_unchanged(scene_->reader(), id, v.canvas_res)) return;
  v.canvas_res = id;
  // A new id (or redefined bytes) may still carry the shown draw list: redraw only on new bytes.
  const std::span<const uint8_t> bytes = scene_resource(id, canvas::kDrawListKind);
  if (v.synced && std::ranges::equal(bytes, v.draw_list)) return;
  v.draw_list.assign(bytes.begin(), bytes.end());  // drawn later, after the scene is released
  ++v.redraw_count;
  gtk_widget_queue_draw(v.widget);
}

void NveGtkHost::update_text_input(GtkNodeView& v, const host::NodeData& node) {
  if (v.kind == WidgetKind::Entry) {
    GtkEntry* e = GTK_ENTRY(v.widget);
    const std::string placeholder(node.input.placeholder);
    gtk_entry_set_placeholder_text(e, placeholder.empty() ? nullptr : placeholder.c_str());
    gtk_entry_set_visibility(e, gtk_internal::node_semantics(node).secure ? FALSE : TRUE);
    {
      // Shrinking the limit truncates the shown text and emits "changed": a host write, not an
      // edit. GTK accepts 0 (unlimited) ..= 65535.
      const auto write = v.text.host_write();
      gtk_entry_set_max_length(e, static_cast<int>(std::min<uint32_t>(node.input.max_length, GTK_ENTRY_BUFFER_MAX_SIZE)));
    }
    gtk_editable_set_editable(GTK_EDITABLE(e), node.input.read_only ? FALSE : TRUE);
  } else {
    gtk_text_view_set_editable(GTK_TEXT_VIEW(v.widget), node.input.read_only ? FALSE : TRUE);
  }
  // Controlled value: adopt the app's value unless it merely echoes what the user is typing.
  const std::string value(node.input.value);
  if (v.text.app_published(value, editable_text(v))) set_editable_text(v, value);
}

void NveGtkHost::update_scroll(GtkNodeView& v, const host::NodeData& node) {
  GtkScrolledWindow* sw = GTK_SCROLLED_WINDOW(v.widget);
  const uint8_t axes = node.kind == host::NodeKind::Scroll ? node.scroll.axes
                       : (node.visual.overflow_x >= NVE_OVERFLOW_SCROLL && node.visual.overflow_y >= NVE_OVERFLOW_SCROLL)
                           ? NVE_SCROLL_AXES_BOTH
                       : node.visual.overflow_x >= NVE_OVERFLOW_SCROLL ? NVE_SCROLL_AXES_HORIZONTAL
                                                                       : NVE_SCROLL_AXES_VERTICAL;
  const GtkPolicyType shown = node.scroll.scrollbars == NVE_SCROLLBARS_ALWAYS  ? GTK_POLICY_ALWAYS
                              : node.scroll.scrollbars == NVE_SCROLLBARS_NEVER ? GTK_POLICY_EXTERNAL
                                                                               : GTK_POLICY_AUTOMATIC;
  const bool h = axes == NVE_SCROLL_AXES_HORIZONTAL || axes == NVE_SCROLL_AXES_BOTH;
  const bool vert = axes == NVE_SCROLL_AXES_VERTICAL || axes == NVE_SCROLL_AXES_BOTH;
  gtk_scrolled_window_set_policy(sw, h ? shown : GTK_POLICY_NEVER, vert ? shown : GTK_POLICY_NEVER);
  gtk_scrolled_window_set_kinetic_scrolling(sw, node.scroll.bounce ? TRUE : FALSE);
  // Programmatic scroll (`requested_scroll_*`): apply each new target once.
  const auto apply = [&](float target, float& last, GtkAdjustment* adj) {
    if (std::isnan(target) || target == last) return;
    last = target;
    const bool prev = std::exchange(v.scrolling_programmatically, true);
    gtk_adjustment_set_value(adj, static_cast<double>(target));
    v.scrolling_programmatically = prev;
  };
  apply(node.scroll.requested_x, v.requested_x, gtk_scrolled_window_get_hadjustment(sw));
  apply(node.scroll.requested_y, v.requested_y, gtk_scrolled_window_get_vadjustment(sw));
}

void NveGtkHost::update_scroll_extents() {
  // Content extent = union of the children's layout rects (our own record: GTK 4.24's
  // `gtk_fixed_get_child_position` reports 0,0, see docs/UPSTREAM.md). Only scroll views whose
  // children were placed, moved, resized or removed since the last sync are re-measured.
  for (GtkNodeView* v : dirty_scroll_extents_) {
    v->extents_dirty = false;
    float w = 0.0f;
    float h = 0.0f;
    for (GtkWidget* c = gtk_widget_get_first_child(v->content); c != nullptr; c = gtk_widget_get_next_sibling(c)) {
      if (const auto* child = static_cast<const GtkNodeView*>(g_object_get_data(G_OBJECT(c), kViewKey))) {
        w = std::max(w, child->local.x + child->local.width);
        h = std::max(h, child->local.y + child->local.height);
      }
    }
    gtk_widget_set_size_request(v->content, px(w), px(h));
  }
  dirty_scroll_extents_.clear();
}

std::span<const uint8_t> NveGtkHost::canvas_draw_list_for_node_id(uint32_t node_id) const noexcept {
  GtkNodeView* const* v = const_cast<host::RetainedTree<Adapter>&>(tree_).find(node_id);
  return v != nullptr && *v != nullptr ? std::span<const uint8_t>((*v)->draw_list) : std::span<const uint8_t>();
}

uint64_t NveGtkHost::canvas_redraw_count_for_node_id(uint32_t node_id) const noexcept {
  GtkNodeView* const* v = const_cast<host::RetainedTree<Adapter>&>(tree_).find(node_id);
  return v != nullptr && *v != nullptr ? (*v)->redraw_count : 0u;
}

}  // namespace nve::gtk
