// GTK4 keyboard dispatch (`GtkEventControllerKey`), Tab/Shift+Tab focus traversal, Enter/Space
// activation, focus sync, and `GtkIMContext` preedit composition lifecycle.
#include "NveGtkInternal.hpp"

#include <gdk/gdkkeysyms.h>

#include <algorithm>
#include <array>
#include <string>
#include <utility>
#include <vector>

namespace nve::gtk {

namespace {

void collect_tab_order(GtkWidget* container, std::vector<GtkNodeView*>& out) {
  for (GtkWidget* c = gtk_widget_get_first_child(container); c != nullptr; c = gtk_widget_get_next_sibling(c)) {
    if (!gtk_widget_get_visible(c)) continue;
    auto* v = static_cast<GtkNodeView*>(g_object_get_data(G_OBJECT(c), kViewKey));
    if (v != nullptr) {
      if (v->sem.disabled || v->sem.hidden) continue;
      if (v->focusable && (!v->tab_index.has_value() || *v->tab_index >= 0)) out.push_back(v);
      if (v->content != nullptr) collect_tab_order(v->content, out);
    } else {
      collect_tab_order(c, out);
    }
  }
}

GtkNodeView* find_topmost_modal(GtkWidget* container) {
  GtkNodeView* found = nullptr;
  for (GtkWidget* c = gtk_widget_get_first_child(container); c != nullptr; c = gtk_widget_get_next_sibling(c)) {
    if (!gtk_widget_get_visible(c)) continue;
    auto* v = static_cast<GtkNodeView*>(g_object_get_data(G_OBJECT(c), kViewKey));
    if (v != nullptr) {
      if (v->sem.hidden || v->sem.disabled) continue;
      if (v->sem.modal) found = v;
      if (v->content != nullptr) {
        if (GtkNodeView* nested = find_topmost_modal(v->content)) found = nested;
      }
    }
  }
  return found;
}

void open_link_widget(GtkWidget* widget, const std::string& href) {
  if (!host::openable_link(href)) return;
#if GTK_CHECK_VERSION(4, 10, 0)
  GtkUriLauncher* launcher = gtk_uri_launcher_new(href.c_str());
  GtkRoot* root = gtk_widget_get_root(widget);
  gtk_uri_launcher_launch(launcher, GTK_IS_WINDOW(root) ? GTK_WINDOW(root) : nullptr, nullptr, nullptr, nullptr);
  g_object_unref(launcher);
#else
  (void)widget;
#endif
}

}  // namespace

namespace gtk_internal {

MappedKey map_gdk_key(guint keyval, guint keycode, GdkModifierType state) {
  MappedKey out;
  out.key_code = keycode;
  if ((state & (GDK_META_MASK | GDK_SUPER_MASK)) != 0) out.flags |= NVE_EVENT_FLAG_MOD_META;
  if ((state & GDK_CONTROL_MASK) != 0) out.flags |= NVE_EVENT_FLAG_MOD_CTRL;
  if ((state & GDK_SHIFT_MASK) != 0) out.flags |= NVE_EVENT_FLAG_MOD_SHIFT;
  if ((state & GDK_ALT_MASK) != 0) out.flags |= NVE_EVENT_FLAG_MOD_ALT;

  switch (keyval) {
    case GDK_KEY_Tab:
    case GDK_KEY_KP_Tab:
      out.key = "Tab";
      out.code = "Tab";
      return out;
    case GDK_KEY_ISO_Left_Tab:
      out.key = "Tab";
      out.code = "Tab";
      out.flags |= NVE_EVENT_FLAG_MOD_SHIFT;
      return out;
    case GDK_KEY_Return:
      out.key = "Enter";
      out.code = "Enter";
      return out;
    case GDK_KEY_KP_Enter:
      out.key = "Enter";
      out.code = "NumpadEnter";
      return out;
    case GDK_KEY_Escape:
      out.key = "Escape";
      out.code = "Escape";
      return out;
    case GDK_KEY_BackSpace:
      out.key = "Backspace";
      out.code = "Backspace";
      return out;
    case GDK_KEY_Delete:
    case GDK_KEY_KP_Delete:
      out.key = "Delete";
      out.code = "Delete";
      return out;
    case GDK_KEY_space:
    case GDK_KEY_KP_Space:
      out.key = " ";
      out.code = "Space";
      return out;
    case GDK_KEY_Left:
    case GDK_KEY_KP_Left:
      out.key = "ArrowLeft";
      out.code = "ArrowLeft";
      return out;
    case GDK_KEY_Right:
    case GDK_KEY_KP_Right:
      out.key = "ArrowRight";
      out.code = "ArrowRight";
      return out;
    case GDK_KEY_Up:
    case GDK_KEY_KP_Up:
      out.key = "ArrowUp";
      out.code = "ArrowUp";
      return out;
    case GDK_KEY_Down:
    case GDK_KEY_KP_Down:
      out.key = "ArrowDown";
      out.code = "ArrowDown";
      return out;
    case GDK_KEY_Home:
    case GDK_KEY_KP_Home:
      out.key = "Home";
      out.code = "Home";
      return out;
    case GDK_KEY_End:
    case GDK_KEY_KP_End:
      out.key = "End";
      out.code = "End";
      return out;
    case GDK_KEY_Page_Up:
    case GDK_KEY_KP_Page_Up:
      out.key = "PageUp";
      out.code = "PageUp";
      return out;
    case GDK_KEY_Page_Down:
    case GDK_KEY_KP_Page_Down:
      out.key = "PageDown";
      out.code = "PageDown";
      return out;
    case GDK_KEY_Insert:
    case GDK_KEY_KP_Insert:
      out.key = "Insert";
      out.code = "Insert";
      return out;
    case GDK_KEY_Shift_L:
      out.key = "Shift";
      out.code = "ShiftLeft";
      return out;
    case GDK_KEY_Shift_R:
      out.key = "Shift";
      out.code = "ShiftRight";
      return out;
    case GDK_KEY_Control_L:
      out.key = "Control";
      out.code = "ControlLeft";
      return out;
    case GDK_KEY_Control_R:
      out.key = "Control";
      out.code = "ControlRight";
      return out;
    case GDK_KEY_Alt_L:
      out.key = "Alt";
      out.code = "AltLeft";
      return out;
    case GDK_KEY_Alt_R:
      out.key = "Alt";
      out.code = "AltRight";
      return out;
    case GDK_KEY_Super_L:
    case GDK_KEY_Meta_L:
      out.key = "Meta";
      out.code = "MetaLeft";
      return out;
    case GDK_KEY_Super_R:
    case GDK_KEY_Meta_R:
      out.key = "Meta";
      out.code = "MetaRight";
      return out;
    default: break;
  }
  if (keyval >= GDK_KEY_F1 && keyval <= GDK_KEY_F12) {
    const std::string fn = "F" + std::to_string((keyval - GDK_KEY_F1) + 1u);
    out.key = fn;
    out.code = fn;
    return out;
  }
  const gunichar ch = gdk_keyval_to_unicode(keyval);
  if (ch != 0u && !g_unichar_iscntrl(ch)) {
    std::array<char, 8> buf{};
    const gint len = g_unichar_to_utf8(ch, buf.data());
    if (len > 0) out.key.assign(buf.data(), static_cast<size_t>(len));
    if (ch >= 'a' && ch <= 'z') {
      out.code = std::string("Key") + static_cast<char>(ch - 'a' + 'A');
    } else if (ch >= 'A' && ch <= 'Z') {
      out.code = std::string("Key") + static_cast<char>(ch);
    } else if (ch >= '0' && ch <= '9') {
      out.code = std::string("Digit") + static_cast<char>(ch);
    }
  }
  return out;
}

GtkIMContext* find_widget_im_context(GtkWidget* widget, WidgetKind kind) noexcept {
  if (widget == nullptr) return nullptr;
  GtkWidget* owner = widget;
  if (kind == WidgetKind::Entry) {
    if (GtkEditable* del = gtk_editable_get_delegate(GTK_EDITABLE(widget)); del != nullptr && GTK_IS_WIDGET(del)) {
      owner = GTK_WIDGET(del);
    }
  }
  GListModel* controllers = gtk_widget_observe_controllers(owner);
  if (controllers == nullptr) return nullptr;
  GtkIMContext* found = nullptr;
  const guint n = g_list_model_get_n_items(controllers);
  for (guint i = 0u; i < n; ++i) {
    gpointer item = g_list_model_get_item(controllers, i);
    if (item != nullptr) {
      if (GTK_IS_EVENT_CONTROLLER_KEY(item)) {
        if (GtkIMContext* im = gtk_event_controller_key_get_im_context(GTK_EVENT_CONTROLLER_KEY(item))) {
          found = im;
        }
      }
      g_object_unref(item);
      if (found != nullptr) break;
    }
  }
  g_object_unref(controllers);
  return found;
}

}  // namespace gtk_internal

// --- IME composition ---------------------------------------------------------------------------

void NveGtkHost::on_im_preedit_start_cb(GtkIMContext*, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  v->host->on_im_preedit_start(*v);
}

void NveGtkHost::on_im_preedit_changed_cb(GtkIMContext*, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  v->host->on_im_preedit_changed(*v);
}

void NveGtkHost::on_im_preedit_end_cb(GtkIMContext*, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  v->host->on_im_preedit_end(*v);
}

void NveGtkHost::on_im_commit_cb(GtkIMContext*, const char* text, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  v->host->on_im_commit(*v, text);
}

void NveGtkHost::on_input_realize_cb(GtkWidget*, gpointer data) {
  auto* v = static_cast<GtkNodeView*>(data);
  if (v->im_context == nullptr) v->host->attach_im_context(*v);
}

void NveGtkHost::attach_im_context(GtkNodeView& view) {
  if (view.im_context != nullptr) return;
  GtkIMContext* im = gtk_internal::find_widget_im_context(view.widget, view.kind);
  if (im == nullptr) {
    if (view.realize_handler == 0u) {
      view.realize_handler =
          g_signal_connect(view.widget, "realize", gtk_detail::to_gcallback(&NveGtkHost::on_input_realize_cb), &view);
    }
    return;
  }
  if (view.realize_handler != 0u) {
    g_signal_handler_disconnect(view.widget, view.realize_handler);
    view.realize_handler = 0u;
  }
  view.im_context = GTK_IM_CONTEXT(g_object_ref(im));
  g_signal_connect(view.im_context, "preedit-start", gtk_detail::to_gcallback(&NveGtkHost::on_im_preedit_start_cb), &view);
  g_signal_connect(view.im_context, "preedit-changed", gtk_detail::to_gcallback(&NveGtkHost::on_im_preedit_changed_cb), &view);
  g_signal_connect(view.im_context, "preedit-end", gtk_detail::to_gcallback(&NveGtkHost::on_im_preedit_end_cb), &view);
  g_signal_connect(view.im_context, "commit", gtk_detail::to_gcallback(&NveGtkHost::on_im_commit_cb), &view);
}

void NveGtkHost::detach_im_context(GtkNodeView& view) {
  if (view.realize_handler != 0u) {
    g_signal_handler_disconnect(view.widget, view.realize_handler);
    view.realize_handler = 0u;
  }
  if (view.im_context != nullptr) {
    g_signal_handlers_disconnect_by_data(view.im_context, &view);
    g_object_unref(view.im_context);
    view.im_context = nullptr;
  }
  view.composing = false;
  view.pending_change_after_compose = false;
  view.preedit_ended_awaiting_commit = false;
  view.preedit.clear();
}

void NveGtkHost::on_im_preedit_start(GtkNodeView& view) {
  if (view.composing || view.sem.disabled || view.sem.read_only) return;
  const uint32_t id = view.id;
  const auto alive = alive_;
  if (std::exchange(view.preedit_ended_awaiting_commit, false)) {
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, {});
    if (!alive->load(std::memory_order_acquire)) return;
    GtkNodeView* const* found = tree_.find(id);
    if (found == nullptr || *found == nullptr) return;
    (*found)->composing = true;
    (*found)->pending_change_after_compose = false;
    (*found)->preedit.clear();
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_START, {});
    return;
  }
  view.composing = true;
  view.pending_change_after_compose = false;
  view.preedit.clear();
  (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_START, {});
}

void NveGtkHost::on_im_preedit_changed(GtkNodeView& view) {
  if (view.im_context == nullptr || view.sem.disabled || view.sem.read_only) return;
  gchar* raw = nullptr;
  gtk_im_context_get_preedit_string(view.im_context, &raw, nullptr, nullptr);
  const std::string preedit = raw != nullptr ? raw : "";
  g_free(raw);

  const uint32_t id = view.id;
  const auto alive = alive_;
  if (!preedit.empty() && !view.composing) {
    if (std::exchange(view.preedit_ended_awaiting_commit, false)) {
      (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, {});
      if (!alive->load(std::memory_order_acquire)) return;
      GtkNodeView* const* found0 = tree_.find(id);
      if (found0 == nullptr || *found0 == nullptr) return;
      (*found0)->composing = true;
      (*found0)->pending_change_after_compose = false;
    } else {
      view.composing = true;
      view.pending_change_after_compose = false;
    }
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_START, {});
    if (!alive->load(std::memory_order_acquire)) return;
    GtkNodeView* const* found = tree_.find(id);
    if (found == nullptr || *found == nullptr) return;
    (*found)->preedit = preedit;
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE, preedit);
    return;
  }
  if (view.composing) {
    view.preedit = preedit;
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE, preedit);
  }
}

void NveGtkHost::on_im_preedit_end(GtkNodeView& view) {
  if (!view.composing) return;
  // IBus / Wayland fire `preedit-end` synchronously right before `commit` on the same keypress.
  // Defer emitting empty `COMPOSITION_END` to an idle tick (or focus-leave / next key) so an
  // immediately following `commit` produces a single `COMPOSITION_END(committed)` + `TextChange`.
  const uint32_t id = view.id;
  const bool flush_edit = std::exchange(view.pending_change_after_compose, false);
  view.composing = false;
  view.preedit_ended_awaiting_commit = true;
  view.preedit.clear();
  struct PendingCancel {
    std::shared_ptr<std::atomic<bool>> alive;
    NveGtkHost* host = nullptr;
    uint32_t node_id = 0u;
  };
  auto* pending = new PendingCancel{alive_, this, id};
  g_idle_add_full(
      G_PRIORITY_HIGH_IDLE,
      [](gpointer data) -> gboolean {
        auto* p = static_cast<PendingCancel*>(data);
        if (p->alive->load(std::memory_order_acquire)) {
          if (GtkNodeView* const* found = p->host->tree_.find(p->node_id);
              found != nullptr && *found != nullptr &&
              std::exchange((*found)->preedit_ended_awaiting_commit, false)) {
            (void)p->host->dispatch_composition(p->node_id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, {});
          }
        }
        return G_SOURCE_REMOVE;
      },
      pending, [](gpointer data) { delete static_cast<PendingCancel*>(data); });
  if (!flush_edit) return;
  const auto alive = alive_;
  if (alive->load(std::memory_order_acquire)) {
    if (GtkNodeView* const* found = tree_.find(id); found != nullptr && *found != nullptr) {
      edit_text(**found);
    }
  }
}

void NveGtkHost::on_im_commit(GtkNodeView& view, const char* text) {
  const bool was_composing = std::exchange(view.composing, false);
  const bool after_preedit_end = std::exchange(view.preedit_ended_awaiting_commit, false);
  if (!was_composing && !after_preedit_end) return;
  const uint32_t id = view.id;
  const bool flush_edit = std::exchange(view.pending_change_after_compose, false);
  view.preedit.clear();
  const auto alive = alive_;
  const std::string committed = text != nullptr ? text : "";
  (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, committed);
  if (!flush_edit || !alive->load(std::memory_order_acquire)) return;
  if (GtkNodeView* const* found = tree_.find(id); found != nullptr && *found != nullptr) {
    edit_text(**found);
  }
}

GtkIMContext* NveGtkHost::im_context_for_node_id(uint32_t node_id) const noexcept {
  GtkNodeView* const* v = const_cast<host::RetainedTree<Adapter>&>(tree_).find(node_id);
  return v != nullptr && *v != nullptr ? (*v)->im_context : nullptr;
}

bool NveGtkHost::is_composing_node(uint32_t node_id) const noexcept {
  GtkNodeView* const* v = const_cast<host::RetainedTree<Adapter>&>(tree_).find(node_id);
  return v != nullptr && *v != nullptr && (*v)->composing;
}

// --- Focus & keyboard --------------------------------------------------------------------------

void NveGtkHost::on_widget_focus_change(GtkNodeView& view, bool focused) {
  const uint32_t id = view.id;
  const auto alive = alive_;
  if (focused) {
    if (focused_node_id_ == id) return;
    focused_node_id_ = id;
    (void)dispatch_focus(id, true);
    return;
  }
  const bool was_composing =
      std::exchange(view.composing, false) || std::exchange(view.preedit_ended_awaiting_commit, false);
  view.pending_change_after_compose = false;
  view.preedit.clear();
  if (focused_node_id_ == id) focused_node_id_ = 0u;
  if (was_composing) {
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, {});
    if (!alive->load(std::memory_order_acquire)) return;
  }
  (void)dispatch_focus(id, false);
}

bool NveGtkHost::set_focused_node(uint32_t node_id) {
  if (node_id == focused_node_id_) return node_id != 0u;
  GtkNodeView* target = nullptr;
  if (node_id != 0u) {
    GtkNodeView* const* found = tree_.find(node_id);
    if (found == nullptr || *found == nullptr) return false;
    target = *found;
    if (!target->focusable || target->sem.disabled) return false;
  }

  const uint32_t prev_id = focused_node_id_;
  const auto alive = alive_;
  if (target != nullptr) (void)gtk_widget_grab_focus(target->widget);
  if (!alive->load(std::memory_order_acquire)) return false;

  // Headless / unmapped GTK widgets may not emit focus-controller signals on `grab_focus`:
  // ensure host state and Blur/Focus/CompositionEnd events always stay in lockstep.
  if (prev_id != 0u && focused_node_id_ == prev_id) {
    if (GtkNodeView* const* prev = tree_.find(prev_id); prev != nullptr && *prev != nullptr) {
      on_widget_focus_change(**prev, false);
      if (!alive->load(std::memory_order_acquire)) return false;
    } else {
      focused_node_id_ = 0u;
    }
  }
  if (node_id != 0u && focused_node_id_ != node_id) {
    if (GtkNodeView* const* now = tree_.find(node_id); now != nullptr && *now != nullptr) {
      on_widget_focus_change(**now, true);
    }
  }
  return node_id == 0u ? (focused_node_id_ == 0u) : (focused_node_id_ == node_id);
}

bool NveGtkHost::step_tab_focus(bool reverse) {
  GtkNodeView* modal = find_topmost_modal(root_widget_);
  std::vector<GtkNodeView*> raw;
  if (modal != nullptr) {
    if (!modal->sem.disabled && !modal->sem.hidden) {
      if (modal->focusable && (!modal->tab_index.has_value() || *modal->tab_index >= 0)) raw.push_back(modal);
      if (modal->content != nullptr) collect_tab_order(modal->content, raw);
    }
    if (raw.empty()) return true;  // active modal traps Tab even with zero focusable descendants
  } else {
    collect_tab_order(root_widget_, raw);
    if (raw.empty()) return false;
  }

  std::vector<GtkNodeView*> positive;
  std::vector<GtkNodeView*> natural;
  for (GtkNodeView* v : raw) {
    if (v->tab_index.has_value() && *v->tab_index > 0) {
      positive.push_back(v);
    } else {
      natural.push_back(v);
    }
  }
  std::stable_sort(positive.begin(), positive.end(),
                   [](const GtkNodeView* a, const GtkNodeView* b) { return *a->tab_index < *b->tab_index; });
  std::vector<GtkNodeView*> order;
  order.reserve(positive.size() + natural.size());
  order.insert(order.end(), positive.begin(), positive.end());
  order.insert(order.end(), natural.begin(), natural.end());

  const auto it = std::find_if(order.begin(), order.end(),
                               [&](const GtkNodeView* v) { return v->id == focused_node_id_; });
  size_t next_idx = 0u;
  if (it == order.end()) {
    next_idx = reverse ? order.size() - 1u : 0u;
  } else {
    const size_t cur = static_cast<size_t>(it - order.begin());
    next_idx = reverse ? (cur + order.size() - 1u) % order.size() : (cur + 1u) % order.size();
  }
  return set_focused_node(order[next_idx]->id);
}

uint32_t NveGtkHost::find_key_target(uint32_t start_id, NveInteractionEvent kind) const noexcept {
  const auto wants_kind = [kind](const GtkNodeView* v) {
    return v != nullptr && !v->sem.disabled &&
           (kind == NVE_INTERACTION_EVENT_KEY_DOWN ? v->wants_key_down : v->wants_key_up);
  };
  if (start_id != 0u) {
    auto& tree = const_cast<host::RetainedTree<Adapter>&>(tree_);
    if (GtkNodeView* const* found = tree.find(start_id); found != nullptr && *found != nullptr) {
      for (GtkWidget* w = (*found)->widget; w != nullptr && w != root_widget_; w = gtk_widget_get_parent(w)) {
        const auto* v = static_cast<const GtkNodeView*>(g_object_get_data(G_OBJECT(w), kViewKey));
        if (wants_kind(v)) return v->id;
      }
      if (!(*found)->sem.disabled) return (*found)->id;
      return 0u;
    }
  }
  uint32_t fallback = 0u;
  const_cast<host::RetainedTree<Adapter>&>(tree_).for_each_view([&](GtkNodeView* v) {
    if (fallback == 0u && wants_kind(v)) fallback = v->id;
  });
  return fallback != 0u ? fallback : 1u;  // root node id
}

bool NveGtkHost::handle_gdk_key(bool pressed, guint keyval, guint keycode, GdkModifierType state) {
  const gtk_internal::MappedKey mapped = gtk_internal::map_gdk_key(keyval, keycode, state);
  if (mapped.key.empty()) return false;

  const auto alive = alive_;
  GtkNodeView* focused = nullptr;
  if (focused_node_id_ != 0u) {
    if (GtkNodeView* const* found = tree_.find(focused_node_id_); found != nullptr) focused = *found;
  }
  const bool composing = focused != nullptr && focused->composing;
  const uint32_t flags = mapped.flags | (composing ? NVE_EVENT_FLAG_COMPOSING : 0u);
  const NveInteractionEvent kind = pressed ? NVE_INTERACTION_EVENT_KEY_DOWN : NVE_INTERACTION_EVENT_KEY_UP;

  if (composing) {
    // Never hijack Tab/Enter/Space while IME composition is active; still notify key listeners.
    if (const uint32_t target = find_key_target(focused_node_id_, kind); target != 0u) {
      (void)dispatch_key(target, kind, mapped.key, mapped.code, flags, mapped.key_code);
    }
    return false;
  }

  const bool no_cmd_mods = (flags & (NVE_EVENT_FLAG_MOD_CTRL | NVE_EVENT_FLAG_MOD_ALT | NVE_EVENT_FLAG_MOD_META)) == 0u;
  const bool is_tab = mapped.key == "Tab" && no_cmd_mods;
  if (pressed && is_tab) {
    if (const uint32_t target = find_key_target(focused_node_id_, kind); target != 0u) {
      (void)dispatch_key(target, kind, mapped.key, mapped.code, flags, mapped.key_code);
      if (!alive->load(std::memory_order_acquire)) return true;
    }
    return step_tab_focus((flags & NVE_EVENT_FLAG_MOD_SHIFT) != 0u);
  }

  if (focused != nullptr && std::exchange(focused->preedit_ended_awaiting_commit, false)) {
    (void)dispatch_composition(focused->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, {});
    if (!alive->load(std::memory_order_acquire)) return true;
    focused = nullptr;
    if (focused_node_id_ != 0u) {
      if (GtkNodeView* const* found = tree_.find(focused_node_id_); found != nullptr) focused = *found;
    }
  }

  const uint32_t target = find_key_target(focused_node_id_, kind);
  bool listener_owned = false;
  if (target != 0u) {
    if (GtkNodeView* const* tv = tree_.find(target); tv != nullptr && *tv != nullptr) {
      listener_owned = !(*tv)->sem.disabled &&
                       (kind == NVE_INTERACTION_EVENT_KEY_DOWN ? (*tv)->wants_key_down : (*tv)->wants_key_up);
    }
  }
  const bool dispatched =
      target != 0u && dispatch_key(target, kind, mapped.key, mapped.code, flags, mapped.key_code);
  if (!alive->load(std::memory_order_acquire)) return true;

  // Re-fetch `focused` because `KeyDown` may have mutated or destroyed views.
  focused = nullptr;
  if (focused_node_id_ != 0u) {
    if (GtkNodeView* const* found = tree_.find(focused_node_id_); found != nullptr) focused = *found;
  }
  if (focused == nullptr || focused->sem.disabled) return listener_owned || dispatched;

  const bool is_input = focused->kind == WidgetKind::Entry || focused->kind == WidgetKind::TextView;
  if (pressed && !is_input && no_cmd_mods && (mapped.key == "Enter" || mapped.key == " ")) {
    if (focused->wants_press) {
      (void)simulate_press(focused->id);
      return true;
    }
    if (mapped.key == "Enter" && host::openable_link(focused->href)) {
      open_link_widget(focused->widget, focused->href);
      return true;
    }
  }
  // Let GTK's native Entry / TextView process non-Tab typing, caret movement, and IME.
  return !is_input && (listener_owned || dispatched);
}

gboolean NveGtkHost::on_key_pressed(GtkEventControllerKey*, guint keyval, guint keycode, GdkModifierType state,
                                    gpointer user_data) {
  return static_cast<NveGtkHost*>(user_data)->handle_gdk_key(true, keyval, keycode, state) ? TRUE : FALSE;
}

void NveGtkHost::on_key_released(GtkEventControllerKey*, guint keyval, guint keycode, GdkModifierType state,
                                 gpointer user_data) {
  (void)static_cast<NveGtkHost*>(user_data)->handle_gdk_key(false, keyval, keycode, state);
}

}  // namespace nve::gtk
