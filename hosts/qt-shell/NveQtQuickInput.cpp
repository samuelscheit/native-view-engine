// `NveQtQuickItem` input: picking in paint order, press / link activation, pointer events with
// hover enter / leave per node, cursors, wheel scrolling (with chaining to outer scroll views),
// focus traversal, keyboard activation, and controlled text editing with IME composition.
#include "NveQtQuickInternal.hpp"

#include <QCursor>
#include <QGuiApplication>
#include <QInputMethod>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QWheelEvent>

#include <algorithm>

namespace nve::qt {

namespace {

/// Children in reverse paint order (topmost first).
std::vector<QuickNodeView*> topmost_first(const QuickNodeView& v) {
  std::vector<QuickNodeView*> kids = v.children;
  std::sort(kids.begin(), kids.end(), paints_before);
  std::reverse(kids.begin(), kids.end());
  return kids;
}

/// `p` in `parent`'s content coordinates (its box origin, plus its scroll offset).
QuickNodeView* pick_in(QuickNodeView& parent, QPointF p) {
  for (QuickNodeView* v : topmost_first(parent)) {
    if (!v->visible) continue;
    const QPointF local = p - v->local.topLeft();
    const bool inside = local.x() >= 0.0 && local.y() >= 0.0 && local.x() <= v->local.width() &&
                        local.y() <= v->local.height() && v->local.width() > 0.0 && v->local.height() > 0.0;
    if (!inside && v->clips) continue;
    if (QuickNodeView* hit = pick_in(*v, local + v->scroll)) return hit;
    if (inside) return v;
  }
  return nullptr;
}

/// Nearest of `v` and its ancestors (root excluded) satisfying `pred`.
template <class Pred>
QuickNodeView* nearest(QuickNodeView* v, Pred&& pred) {
  for (; v != nullptr && v->parent != nullptr; v = v->parent) {
    if (pred(*v)) return v;
  }
  return nullptr;
}

Qt::CursorShape cursor_shape(uint8_t cursor) noexcept {
  switch (cursor) {
    case NVE_CURSOR_KIND_POINTER: return Qt::PointingHandCursor;
    case NVE_CURSOR_KIND_TEXT: return Qt::IBeamCursor;
    case NVE_CURSOR_KIND_GRAB: return Qt::OpenHandCursor;
    case NVE_CURSOR_KIND_GRABBING: return Qt::ClosedHandCursor;
    case NVE_CURSOR_KIND_NOT_ALLOWED: return Qt::ForbiddenCursor;
    case NVE_CURSOR_KIND_COL_RESIZE: return Qt::SplitHCursor;
    case NVE_CURSOR_KIND_ROW_RESIZE: return Qt::SplitVCursor;
    case NVE_CURSOR_KIND_MOVE: return Qt::SizeAllCursor;
    case NVE_CURSOR_KIND_CROSSHAIR: return Qt::CrossCursor;
    case NVE_CURSOR_KIND_PROGRESS: return Qt::BusyCursor;
    case NVE_CURSOR_KIND_WAIT: return Qt::WaitCursor;
    default: return Qt::ArrowCursor;
  }
}

}  // namespace

QuickNodeView* NveQtQuickItem::Impl::pick(QPointF item_pos) { return pick_in(root_, item_pos); }

// --- Pointer -----------------------------------------------------------------------------------

void NveQtQuickItem::Impl::press(QPointF pos) {
  QuickNodeView* hit = pick(pos);
  QuickNodeView* focus = nearest(hit, [](const QuickNodeView& v) { return v.focusable; });
  QuickNodeView* target = nearest(hit, [](const QuickNodeView& v) { return v.activatable(); });
  pressed_ = target != nullptr ? target->id : 0u;
  if (focus != nullptr && focus->is_input) {
    focus->input.caret = static_cast<int>(focus->input.edit.size());
  }
  const uint32_t down = [&] {
    QuickNodeView* d = nearest(hit, [](const QuickNodeView& v) { return v.wants(NVE_INTERACTION_EVENT_POINTER_DOWN); });
    return d != nullptr ? d->id : 0u;
  }();
  set_focus(focus != nullptr ? focus->id : 0u);  // may dispatch: records can change
  if (const QuickNodeView* d = find(down); d != nullptr && down != 0u) {
    const QPointF local = pos - origin_of(*d);
    (void)dispatch_pointer(down, NVE_INTERACTION_EVENT_POINTER_DOWN, static_cast<float>(local.x()),
                           static_cast<float>(local.y()));
  }
}

void NveQtQuickItem::Impl::release(QPointF pos) {
  QuickNodeView* hit = pick(pos);
  if (QuickNodeView* up =
          nearest(hit, [](const QuickNodeView& v) { return v.wants(NVE_INTERACTION_EVENT_POINTER_UP); })) {
    const QPointF local = pos - origin_of(*up);
    (void)dispatch_pointer(up->id, NVE_INTERACTION_EVENT_POINTER_UP, static_cast<float>(local.x()),
                           static_cast<float>(local.y()));
    hit = pick(pos);  // the dispatch may have changed the scene
  }
  const uint32_t pressed = std::exchange(pressed_, 0u);
  QuickNodeView* target = nearest(hit, [pressed](const QuickNodeView& v) { return v.id == pressed; });
  if (pressed == 0u || target == nullptr || !target->activatable()) return;  // released elsewhere
  const QPointF local = pos - origin_of(*target);
  const std::string href = target->href;
  // May destroy records: touch nothing afterwards.
  (void)activate(pressed, target->wants_press, href, static_cast<float>(local.x()), static_cast<float>(local.y()));
}

void NveQtQuickItem::Impl::hover(QPointF pos, bool inside) {
  QuickNodeView* hit = inside ? pick(pos) : nullptr;
  std::vector<uint32_t> path;
  for (QuickNodeView* v = hit; v != nullptr && v->parent != nullptr; v = v->parent) path.insert(path.begin(), v->id);
  // Leave nodes no longer hovered (deepest first), enter new ones (outermost first).
  const std::vector<uint32_t> before = std::exchange(hovered_, path);
  for (auto it = before.rbegin(); it != before.rend(); ++it) {
    if (std::find(path.begin(), path.end(), *it) != path.end()) continue;
    if (const QuickNodeView* v = find(*it); v != nullptr && v->wants(NVE_INTERACTION_EVENT_POINTER_LEAVE)) {
      (void)dispatch_pointer(*it, NVE_INTERACTION_EVENT_POINTER_LEAVE, 0.0f, 0.0f);
    }
  }
  for (const uint32_t id : path) {
    if (std::find(before.begin(), before.end(), id) != before.end()) continue;
    if (const QuickNodeView* v = find(id); v != nullptr && v->wants(NVE_INTERACTION_EVENT_POINTER_ENTER)) {
      const QPointF local = pos - origin_of(*v);
      (void)dispatch_pointer(id, NVE_INTERACTION_EVENT_POINTER_ENTER, static_cast<float>(local.x()),
                             static_cast<float>(local.y()));
    }
  }
  hit = inside ? pick(pos) : nullptr;
  if (QuickNodeView* mv =
          nearest(hit, [](const QuickNodeView& v) { return v.wants(NVE_INTERACTION_EVENT_POINTER_MOVE); })) {
    const QPointF local = pos - origin_of(*mv);
    (void)dispatch_pointer(mv->id, NVE_INTERACTION_EVENT_POINTER_MOVE, static_cast<float>(local.x()),
                           static_cast<float>(local.y()));
    hit = inside ? pick(pos) : nullptr;
  }
  // Cursor: the nearest explicit one, else a hand over pressables, an I-beam over inputs.
  Qt::CursorShape shape = Qt::ArrowCursor;
  for (const QuickNodeView* v = hit; v != nullptr && v->parent != nullptr; v = v->parent) {
    if (v->cursor != NVE_CURSOR_KIND_AUTO) {
      shape = cursor_shape(v->cursor);
      break;
    }
    if (v->activatable()) {
      shape = Qt::PointingHandCursor;
      break;
    }
    if (v->is_input) {
      shape = Qt::IBeamCursor;
      break;
    }
  }
  if (owner_->cursor().shape() != shape) owner_->setCursor(shape);
}

bool NveQtQuickItem::Impl::wheel(QPointF pos, QPointF delta) {
  // The innermost scroll container that can still move takes the delta (else its ancestors).
  for (QuickNodeView* v = pick(pos); v != nullptr && v->parent != nullptr; v = v->parent) {
    if (!v->scrollable) continue;
    const std::optional<QPointF> now =
        scroll_by(v->id, static_cast<float>(-delta.x()), static_cast<float>(-delta.y()));
    if (!now || *now == v->scroll) continue;
    v->scroll = *now;
    mark_dirty(*v);
    QSizeF extent = v->local.size();
    for (const QuickNodeView* c : v->children) {
      extent = extent.expandedTo(QSizeF(c->local.right(), c->local.bottom()));
    }
    (void)dispatch_scroll(v->id, static_cast<float>(now->x()), static_cast<float>(now->y()),
                          static_cast<float>(extent.width()), static_cast<float>(extent.height()));
    return true;
  }
  return false;
}

// --- Focus & keyboard --------------------------------------------------------------------------

void NveQtQuickItem::Impl::set_focus(uint32_t id) {
  if (id == focused_) return;
  const uint32_t before = std::exchange(focused_, id);
  bool end_comp = false;
  bool flush_pending = false;
  for (const uint32_t changed : {before, id}) {
    if (QuickNodeView* v = find(changed); v != nullptr && changed != 0u && v->is_input) {
      if (changed == before) {
        if (v->input.composing || !v->input.preedit.isEmpty()) end_comp = true;
        flush_pending = std::exchange(v->input.pending_change_after_compose, false);
      }
      v->input.composing = false;
      v->input.preedit.clear();
      bump_paint(*v);  // caret
    }
  }
  if (id != 0u && !owner_->hasActiveFocus()) owner_->forceActiveFocus(Qt::MouseFocusReason);
  QGuiApplication::inputMethod()->update(Qt::ImQueryAll);
  if (id != 0u) a11y_focused(id);
  if (end_comp && before != 0u && find(before) != nullptr) {
    (void)dispatch_composition(before, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, {});
  }
  if (flush_pending && before != 0u) {
    if (QuickNodeView* prev = find(before); prev != nullptr && prev->is_input) {
      edit_input(*prev, prev->input.edit, prev->input.caret);
    }
  }
  if (before != 0u && find(before) != nullptr) (void)dispatch_focus(before, false);
  if (id != 0u && find(id) != nullptr) (void)dispatch_focus(id, true);
}

void NveQtQuickItem::Impl::edit_input(QuickNodeView& v, const QString& text, int caret) {
  QString next = text;
  if (v.input.max_length > 0u && static_cast<uint32_t>(next.size()) > v.input.max_length) {
    next.truncate(static_cast<qsizetype>(v.input.max_length));
  }
  v.input.edit = next;
  v.input.caret = std::clamp(caret, 0, static_cast<int>(next.size()));
  bump_paint(v);
  QGuiApplication::inputMethod()->update(Qt::ImSurroundingText | Qt::ImCursorPosition);
  const std::string utf8 = next.toStdString();
  // May destroy `v`: touch nothing afterwards.
  edit_text<QuickNodeView>(
      v.input.control, utf8, v.id,
      [this](uint32_t id) -> host::ControlledAnswer<QuickNodeView> {
        QuickNodeView* field = find(id);
        if (field == nullptr || !field->is_input) return {};
        return {field, &field->input.control, field->input.control.app_value()};
      },
      [this](QuickNodeView& field, std::string_view value) {
        // A transformed or rejected edit: show the app's value (caret kept where it fits).
        QuickInput& in = field.input;
        in.edit = scene_string(value, in.multiline);
        in.caret = std::min(in.caret, static_cast<int>(in.edit.size()));
        bump_paint(field);
        QGuiApplication::inputMethod()->update(Qt::ImSurroundingText | Qt::ImCursorPosition);
      });
}

bool NveQtQuickItem::Impl::key(QKeyEvent& event) {
  QuickNodeView* f = find(focused_);
  if (f == nullptr || focused_ == 0u) f = nullptr;
  const bool composing = f != nullptr && f->is_input && f->input.composing;
  const MappedQtKey mapped = map_qt_key(event, composing);
  bool key_dispatched = false;
  if (!mapped.key.empty()) {
    QuickNodeView* listener =
        f != nullptr ? nearest(f, [](const QuickNodeView& v) { return v.wants(NVE_INTERACTION_EVENT_KEY_DOWN); })
                     : nullptr;
    if (listener == nullptr && root_.wants(NVE_INTERACTION_EVENT_KEY_DOWN)) listener = &root_;
    if (listener != nullptr) {
      key_dispatched = dispatch_key(listener->id, NVE_INTERACTION_EVENT_KEY_DOWN, mapped.key, mapped.code,
                                    mapped.flags, mapped.key_code);
      f = find(focused_);
      if (f == nullptr || focused_ == 0u) f = nullptr;
    }
  }
  const int k = event.key();
  // Never hijack Tab/Enter/Space/text edits while an IME composition is active (matches GTK4 & Win32).
  if (composing) return false;
  if (k == Qt::Key_Tab || k == Qt::Key_Backtab) {  // focus traversal (positive tab_index, then DFS; modal-scoped)
    QuickNodeView* modal = nullptr;
    const auto find_modal = [&](auto&& self, QuickNodeView& cur) -> void {
      std::vector<QuickNodeView*> kids = cur.children;
      std::sort(kids.begin(), kids.end(), [](const QuickNodeView* a, const QuickNodeView* b) { return a->order < b->order; });
      for (QuickNodeView* c : kids) {
        if (!c->visible || c->disabled || c->sem.hidden) continue;
        if (c->sem.modal) modal = c;
        self(self, *c);
      }
    };
    find_modal(find_modal, root_);

    std::vector<QuickNodeView*> order;
    const auto collect_tab = [&](auto&& self, QuickNodeView& cur, bool include_self) -> void {
      if (include_self) {
        if (!cur.visible || cur.disabled || cur.sem.hidden) return;
        if (cur.focusable && !(cur.tab_index.has_value() && *cur.tab_index < 0)) order.push_back(&cur);
      }
      std::vector<QuickNodeView*> kids = cur.children;
      std::sort(kids.begin(), kids.end(), [](const QuickNodeView* a, const QuickNodeView* b) { return a->order < b->order; });
      for (QuickNodeView* c : kids) self(self, *c, true);
    };
    if (modal != nullptr) {
      collect_tab(collect_tab, *modal, true);
    } else {
      collect_tab(collect_tab, root_, false);
    }
    if (order.empty()) return modal != nullptr || key_dispatched;

    std::stable_sort(order.begin(), order.end(), [](const QuickNodeView* a, const QuickNodeView* b) {
      const bool a_pos = a->tab_index.has_value() && *a->tab_index > 0;
      const bool b_pos = b->tab_index.has_value() && *b->tab_index > 0;
      if (a_pos != b_pos) return a_pos;
      if (a_pos && *a->tab_index != *b->tab_index) return *a->tab_index < *b->tab_index;
      return false;
    });
    const auto at = std::find(order.begin(), order.end(), f);
    const ptrdiff_t n = static_cast<ptrdiff_t>(order.size());
    const ptrdiff_t i = at == order.end() ? (k == Qt::Key_Tab ? -1 : 0) : at - order.begin();
    set_focus(order[static_cast<size_t>(((i + (k == Qt::Key_Tab ? 1 : -1)) % n + n) % n)]->id);
    return true;
  }
  if (f == nullptr) return key_dispatched;
  if (!f->is_input) {
    const bool is_enter = k == Qt::Key_Return || k == Qt::Key_Enter;
    if (f->activatable() && (is_enter || (k == Qt::Key_Space && f->wants_press))) {
      (void)activate(f->id, f->wants_press, f->href, static_cast<float>(f->local.width() * 0.5),
                     static_cast<float>(f->local.height() * 0.5));
      return true;
    }
    return key_dispatched;
  }
  QuickInput& in = f->input;
  if (in.read_only || f->disabled) return k != Qt::Key_Escape || key_dispatched;
  QString text = in.edit;
  int caret = std::clamp(in.caret, 0, static_cast<int>(text.size()));
  switch (k) {
    case Qt::Key_Left: in.caret = std::max(0, caret - 1); bump_paint(*f); return true;
    case Qt::Key_Right: in.caret = std::min(static_cast<int>(text.size()), caret + 1); bump_paint(*f); return true;
    case Qt::Key_Home: in.caret = 0; bump_paint(*f); return true;
    case Qt::Key_End: in.caret = static_cast<int>(text.size()); bump_paint(*f); return true;
    case Qt::Key_Backspace:
      if (caret == 0) return true;
      text.remove(caret - 1, 1);
      edit_input(*f, text, caret - 1);
      return true;
    case Qt::Key_Delete:
      if (caret >= text.size()) return true;
      text.remove(caret, 1);
      edit_input(*f, text, caret);
      return true;
    case Qt::Key_Return:
    case Qt::Key_Enter:
      if (in.multiline) {
        text.insert(caret, QChar(u'\n'));
        edit_input(*f, text, caret + 1);
      } else {
        (void)dispatch_submit(f->id, text.toStdString());
      }
      return true;
    default: break;
  }
  const QString typed = scene_string(event.text().toStdString());
  if (typed.isEmpty()) return key_dispatched;
  text.insert(caret, typed);
  edit_input(*f, text, caret + static_cast<int>(typed.size()));
  return true;
}

bool NveQtQuickItem::Impl::key_release(QKeyEvent& event) {
  QuickNodeView* f = find(focused_);
  if (f == nullptr || focused_ == 0u) f = nullptr;
  const MappedQtKey mapped = map_qt_key(event, f != nullptr && f->is_input && f->input.composing);
  if (mapped.key.empty()) return false;
  QuickNodeView* listener =
      f != nullptr ? nearest(f, [](const QuickNodeView& v) { return v.wants(NVE_INTERACTION_EVENT_KEY_UP); })
                   : nullptr;
  if (listener == nullptr && root_.wants(NVE_INTERACTION_EVENT_KEY_UP)) listener = &root_;
  if (listener == nullptr) return false;
  return dispatch_key(listener->id, NVE_INTERACTION_EVENT_KEY_UP, mapped.key, mapped.code, mapped.flags,
                      mapped.key_code);
}

void NveQtQuickItem::Impl::ime(QInputMethodEvent& event) {
  QuickNodeView* f = find(focused_);
  if (f == nullptr || focused_ == 0u || !f->is_input || f->input.read_only || f->disabled) return;
  const uint32_t id = f->id;
  const bool was_composing = f->input.composing;
  const QString preedit = event.preeditString();
  const QString commit = scene_string(event.commitString().toStdString(), f->input.multiline);
  const bool now_composing = !preedit.isEmpty();
  const int repl_start_rel = event.replacementStart();
  const int repl_len_raw = event.replacementLength();
  const bool commits = !commit.isEmpty() || repl_len_raw > 0;
  f->input.preedit = preedit;
  f->input.composing = now_composing;
  if (!was_composing && now_composing) {
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_START, {});
    f = find(id);
    if (f == nullptr || !f->is_input) return;
  }
  if (now_composing) {
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE, preedit.toStdString());
    f = find(id);
    if (f == nullptr || !f->is_input) return;
  }
  if (was_composing && !now_composing) {
    (void)dispatch_composition(id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, commit.toStdString());
    f = find(id);
    if (f == nullptr || !f->is_input) return;
  }
  if (commits) {
    QString text = f->input.edit;
    const int len = static_cast<int>(text.size());
    const int caret = std::clamp(f->input.caret, 0, len);
    const int repl_start = std::clamp(caret + repl_start_rel, 0, len);
    const int repl_len = std::clamp(repl_len_raw, 0, len - repl_start);
    if (repl_len > 0) text.remove(repl_start, repl_len);
    text.insert(repl_start, commit);
    if (f->input.max_length > 0u && static_cast<uint32_t>(text.size()) > f->input.max_length) {
      text.truncate(static_cast<qsizetype>(f->input.max_length));
    }
    const int next_caret = std::clamp(repl_start + static_cast<int>(commit.size()), 0, static_cast<int>(text.size()));
    if (now_composing) {
      // Simultaneous commit + next preedit: retain committed text locally, suppress mid-composition
      // `TextChange`, and flush once composition ends (matches `NveQtCustomWidgets`).
      f->input.edit = text;
      f->input.caret = next_caret;
      f->input.pending_change_after_compose = true;
      bump_paint(*f);
      QGuiApplication::inputMethod()->update(Qt::ImSurroundingText | Qt::ImCursorPosition);
      return;
    }
    f->input.pending_change_after_compose = false;
    edit_input(*f, text, next_caret);
    return;
  }
  if (!now_composing && std::exchange(f->input.pending_change_after_compose, false)) {
    edit_input(*f, f->input.edit, f->input.caret);
    return;
  }
  bump_paint(*f);
}

QVariant NveQtQuickItem::Impl::ime_query(Qt::InputMethodQuery query) const {
  const QuickNodeView* f = find(focused_);
  const bool editing = f != nullptr && focused_ != 0u && f->is_input;
  switch (query) {
    case Qt::ImEnabled: return editing && !f->input.read_only;
    case Qt::ImSurroundingText: return editing ? QVariant(f->input.edit) : QVariant();
    case Qt::ImCursorPosition:
    case Qt::ImAnchorPosition: return editing ? QVariant(f->input.caret) : QVariant();
    case Qt::ImHints: {
      if (!editing) return {};
      Qt::InputMethodHints hints = Qt::ImhNone;
      if (f->input.secure) hints |= Qt::ImhHiddenText | Qt::ImhSensitiveData | Qt::ImhNoPredictiveText;
      if (f->input.multiline) hints |= Qt::ImhMultiLine;
      return static_cast<int>(hints);
    }
    case Qt::ImCursorRectangle: {
      if (!editing) return {};
      const QFontMetricsF fm(f->paint.style.font);
      const QPointF at = origin_of(*f) + f->paint.content_box.topLeft();
      return QRectF(at.x() + fm.horizontalAdvance(f->input.edit.left(f->input.caret)), at.y(), 1.0, fm.height());
    }
    default: return {};
  }
}

// --- NveQtQuickItem event handlers -------------------------------------------------------------

void NveQtQuickItem::mousePressEvent(QMouseEvent* event) {
  if (event->button() != Qt::LeftButton) {
    event->ignore();
    return;
  }
  event->accept();
  impl_->press(event->position());
}

void NveQtQuickItem::mouseMoveEvent(QMouseEvent* event) { impl_->hover(event->position(), contains(event->position())); }

void NveQtQuickItem::mouseReleaseEvent(QMouseEvent* event) {
  event->accept();
  impl_->release(event->position());
}

void NveQtQuickItem::hoverMoveEvent(QHoverEvent* event) { impl_->hover(event->position(), true); }

void NveQtQuickItem::hoverLeaveEvent(QHoverEvent* event) { impl_->hover(event->position(), false); }

void NveQtQuickItem::wheelEvent(QWheelEvent* event) {
  // Pixel deltas from touchpads; one wheel notch (120) scrolls 48 px.
  const QPointF delta = !event->pixelDelta().isNull() ? QPointF(event->pixelDelta())
                                                      : QPointF(event->angleDelta()) * (48.0 / 120.0);
  event->setAccepted(impl_->wheel(event->position(), delta));
}

void NveQtQuickItem::keyPressEvent(QKeyEvent* event) {
  if (!impl_->key(*event)) QQuickItem::keyPressEvent(event);
}

void NveQtQuickItem::keyReleaseEvent(QKeyEvent* event) {
  if (!impl_->key_release(*event)) QQuickItem::keyReleaseEvent(event);
}

void NveQtQuickItem::inputMethodEvent(QInputMethodEvent* event) {
  impl_->ime(*event);
  event->accept();
}

QVariant NveQtQuickItem::inputMethodQuery(Qt::InputMethodQuery query) const {
  const QVariant own = impl_->ime_query(query);
  return own.isValid() ? own : QQuickItem::inputMethodQuery(query);
}

void NveQtQuickItem::focusOutEvent(QFocusEvent* event) {
  QQuickItem::focusOutEvent(event);
  impl_->set_focus(0u);
}

uint32_t NveQtQuickItem::hitTestNode(QPointF item_pos, bool interactive_only) const {
  QuickNodeView* hit = impl_->pick(item_pos);
  if (interactive_only) {
    hit = nearest(hit, [](const QuickNodeView& v) {
      return v.interaction_events != 0u || v.activatable() || v.focusable || v.scrollable;
    });
  }
  return hit != nullptr ? hit->id : 0u;
}

}  // namespace nve::qt
