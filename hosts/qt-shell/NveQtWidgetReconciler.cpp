// `RetainedTree` adapter of `NveQtWidget`: node views created, updated, re-parented, stacked
// and destroyed in O(changed), plus the per-kind bindings (visuals, content, inputs, scroll).
#include "NveQtWidgetInternal.hpp"

#include <QGraphicsOpacityEffect>
#include <QScrollArea>
#include <QScrollBar>

#include <algorithm>
#include <cmath>
#include <string>

namespace nve::qt {

namespace {

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

/// Pixel rect of a (sealed) layout rect, rounding edges (not sizes) so adjacent nodes never gap.
QRect pixel_rect(const NveLayoutRect& r) noexcept {
  const auto px = [](float v) { return static_cast<int>(std::lround(v)); };
  const int x0 = px(r.x), y0 = px(r.y);
  return QRect(x0, y0, std::max(0, px(r.x + r.width) - x0), std::max(0, px(r.y + r.height) - y0));
}

/// Non-negative px for a style sheet (inputs are sealed by `read_node`).
qreal css_px(qreal v) noexcept { return std::max(0.0, v); }

QString css_color(const QColor& c) {
  return QStringLiteral("rgba(%1, %2, %3, %4)").arg(c.red()).arg(c.green()).arg(c.blue()).arg(c.alpha());
}

/// Style sheet of a text input: the node's box visuals, padding (content insets) and text color.
QString input_style_sheet(const BoxVisual& v, const host::NodeGeometry& node, const QColor& text) {
  const qreal bw = v.has_border() ? css_px(v.border_width) : 0.0;
  const qreal top = css_px(node.content.y - bw);
  const qreal left = css_px(node.content.x - bw);
  const qreal right = css_px(node.rect.width - node.content.x - node.content.width - bw);
  const qreal bottom = css_px(node.rect.height - node.content.y - node.content.height - bw);
  QString css = QStringLiteral("background-color: %1; border-radius: %2px; padding: %3px %4px %5px %6px; color: %7;")
                    .arg(css_color(v.background))
                    .arg(css_px(v.radius))
                    .arg(top)
                    .arg(right)
                    .arg(bottom)
                    .arg(left)
                    .arg(css_color(text));
  css += bw > 0.0 ? QStringLiteral(" border: %1px solid %2;").arg(bw).arg(css_color(v.border_color))
                  : QStringLiteral(" border: none;");
  return css;
}

std::string editable_text(const QtNodeView& v) {
  const QString text = v.line != nullptr ? v.line->text() : v.text_edit->toPlainText();
  return text.toStdString();
}

void set_editable_text(QtNodeView& v, std::string_view text) {
  const auto write = v.text.host_write();
  if (v.line != nullptr) {
    v.line->setText(scene_string(text));
  } else {
    v.text_edit->setPlainText(scene_string(text, true));
  }
}

bool composing(const QtNodeView& v) noexcept {
  return (v.line != nullptr && v.line->composing()) || (v.text_edit != nullptr && v.text_edit->composing());
}

}  // namespace

// --- Hooks from the node widgets ---------------------------------------------------------------

void view_activate(QtNodeView* view, QPointF pos) {
  if (view == nullptr || view->sem.disabled) return;
  const uint32_t id = view->id;
  const bool wants_press = view->wants_press;
  const std::string href = view->href;
  (void)view->host->activate(id, wants_press, href, static_cast<float>(pos.x()), static_cast<float>(pos.y()));
}

void view_hover(QtNodeView* view, NveEventKind kind, QPointF pos) {
  if (view == nullptr || view->sem.disabled) return;
  if (kind == NVE_EVENT_POINTER_ENTER || kind == NVE_EVENT_POINTER_LEAVE) {
    const bool enter = kind == NVE_EVENT_POINTER_ENTER;
    if (view->hovered == enter) return;
    view->hovered = enter;
  }
  if ((view->event_mask & host::event_bit(kind)) == 0u) return;
  (void)view->host->dispatch_pointer(view->id, kind, static_cast<float>(pos.x()), static_cast<float>(pos.y()));
}

void view_focus(QtNodeView* view, bool focused) {
  if (view == nullptr || !view->sem.focusable) return;
  (void)view->host->dispatch_focus(view->id, focused);
}

void NveQtWidget::Impl::on_text_edited(QtNodeView& v) {
  std::string current = editable_text(v);
  if (v.text_edit != nullptr && v.max_length > 0u) {
    const QString q = QString::fromStdString(current);
    if (static_cast<uint32_t>(q.size()) > v.max_length) {  // `QPlainTextEdit` has no max length
      current = q.left(static_cast<qsizetype>(v.max_length)).toStdString();
      set_editable_text(v, current);
      v.text_edit->moveCursor(QTextCursor::End);
    }
  }
  if (!v.text.user_edited(current)) return;  // a host write, or no change since the last dispatch
  // May destroy `v` (the app can remove the input in response): touch nothing afterwards.
  (void)dispatch_text_change(v.id, current);
}

void NveQtWidget::Impl::on_submit(QtNodeView& v) { (void)dispatch_submit(v.id, editable_text(v)); }

void NveQtWidget::Impl::on_native_scroll(QtNodeView& v) {
  if (syncing() || v.scrolling_programmatically) return;
  const QWidget* inner = v.scroll->widget();
  (void)dispatch_scroll(v.id, static_cast<float>(v.scroll->horizontalScrollBar()->value()),
                        static_cast<float>(v.scroll->verticalScrollBar()->value()), static_cast<float>(inner->width()),
                        static_cast<float>(inner->height()));
}

// --- Adapter -----------------------------------------------------------------------------------

uint32_t NveQtWidget::Impl::Adapter::tag(const host::NodeData& node) const {
  if (node.kind == host::NodeKind::TextInput) {
    return static_cast<uint32_t>(node.input.multiline ? NativeKind::TextEdit : NativeKind::LineEdit);
  }
  return static_cast<uint32_t>(node.scrollable() ? NativeKind::Scroll : NativeKind::Node);
}

QtNodeView* NveQtWidget::Impl::Adapter::create(const host::NodeData& node) {
  auto* v = new QtNodeView();
  v->host = host;
  v->id = node.id;
  v->kind = static_cast<NativeKind>(tag(node));
  switch (v->kind) {
    case NativeKind::Node:
      v->frame = new NveQtNodeWidget(v, nullptr);
      v->widget = v->content = v->frame;
      break;
    case NativeKind::Scroll: {
      v->frame = new NveQtNodeWidget(v, nullptr);
      v->widget = v->frame;
      v->scroll = new QScrollArea(v->frame);
      v->scroll->setFrameShape(QFrame::NoFrame);
      v->scroll->setWidgetResizable(false);
      v->scroll->setAutoFillBackground(false);
      v->scroll->viewport()->setAutoFillBackground(false);
      v->content = new QWidget();
      v->content->setAutoFillBackground(false);
      v->scroll->setWidget(v->content);
      for (QScrollBar* bar : {v->scroll->horizontalScrollBar(), v->scroll->verticalScrollBar()}) {
        QObject::connect(bar, &QScrollBar::valueChanged, v->scroll, [h = host, v](int) { h->on_native_scroll(*v); });
      }
      host->scroll_views_.insert(v);
      break;
    }
    case NativeKind::LineEdit:
      v->line = new NveQtLineEdit(v, nullptr);
      v->widget = v->content = v->line;
      break;
    case NativeKind::TextEdit:
      v->text_edit = new NveQtTextEdit(v, nullptr);
      v->widget = v->content = v->text_edit;
      break;
  }
  v->widget->setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node.id));
  return v;
}

void NveQtWidget::Impl::Adapter::update(QtNodeView*& v, const host::NodeData& node) {
  ++host->reconcile_update_count_;
  host->update_geometry(*v, node);
  host->update_visual(*v, node);
  if (v->frame != nullptr) host->update_content(*v, node);
  if (v->line != nullptr || v->text_edit != nullptr) host->update_text_input(*v, node);
  if (v->scroll != nullptr) host->update_scroll(*v, node);
  host->update_interaction(*v, node);
  host->apply_semantics(*v, node);
  v->synced = true;
}

void NveQtWidget::Impl::Adapter::move(QtNodeView*& v, const host::NodeGeometry& node) {
  ++host->reconcile_move_count_;
  host->place(*v, node);
  host->apply_box(*v, node);
}

void NveQtWidget::Impl::Adapter::update_root(const host::NodeData& node) {
  const QColor bg = qcolor(node.visual.background);
  if (bg != host->root_background_) {
    host->root_background_ = bg;
    host->owner_->update();
  }
  NodeSemantics sem = node_semantics(node);
  notify_semantics_change(host->owner_, host->root_sem_, sem);
  host->root_sem_ = std::move(sem);
}

void NveQtWidget::Impl::Adapter::destroy(uint32_t, QtNodeView*& v) {
  if (v == nullptr) return;
  host->scroll_views_.erase(v);
  host->restack_.erase(v->content);
  // Child views may survive (moved to another parent): hand them back before this widget dies.
  for (QObject* child : v->content->children()) {
    auto* w = qobject_cast<QWidget*>(child);
    if (w != nullptr && (w->property(kNodeIdProperty).isValid() || dynamic_cast<NveQtShadowWidget*>(w) != nullptr)) {
      w->hide();
      w->setParent(nullptr);
    }
  }
  if (v->frame != nullptr) v->frame->detach();
  if (v->line != nullptr) {
    v->line->detach();
    v->line->disconnect(v->line);  // its own edit / return lambdas
  }
  if (v->text_edit != nullptr) {
    v->text_edit->detach();
    v->text_edit->disconnect(v->text_edit);
  }
  if (v->scroll != nullptr) {
    v->scroll->horizontalScrollBar()->disconnect(v->scroll);
    v->scroll->verticalScrollBar()->disconnect(v->scroll);
  }
  delete v->shadow;
  v->widget->hide();
  v->widget->setParent(nullptr);
  if (host->tearing_down_) {
    delete v->widget;
  } else {
    v->widget->deleteLater();  // may be inside its own event handler (press -> app removes it)
  }
  delete v;
  v = nullptr;
}

void NveQtWidget::Impl::Adapter::set_children(const host::NodeGeometry&, QtNodeView** parent_view,
                                              std::span<QtNodeView** const> kids) {
  QWidget* container = parent_view != nullptr ? (*parent_view)->content : host->owner_;
  std::unordered_set<const QWidget*> wanted;
  for (uint32_t i = 0; i < kids.size(); ++i) {
    QtNodeView* k = *kids[i];
    k->order = i;
    wanted.insert(k->widget);
    if (k->widget->parentWidget() != container) {
      k->widget->setParent(container);
      if (k->shadow != nullptr) k->shadow->setParent(container);
    }
    k->widget->setGeometry(pixel_rect(k->local));
    k->widget->setVisible(k->visible);
    if (k->shadow != nullptr) k->shadow->setVisible(k->visible);
  }
  // Node widgets no longer in this parent's scene children are stale here.
  for (QObject* child : container->children()) {
    auto* w = qobject_cast<QWidget*>(child);
    if (w == nullptr || !w->property(kNodeIdProperty).isValid() || wanted.contains(w)) continue;
    if (QtNodeView* stale = host->find(static_cast<uint32_t>(w->property(kNodeIdProperty).toULongLong()))) {
      if (stale->shadow != nullptr) {
        stale->shadow->hide();
        stale->shadow->setParent(nullptr);
      }
    }
    w->hide();
    w->setParent(nullptr);
  }
  host->restack_.insert(container);
}

// --- Per-kind bindings -------------------------------------------------------------------------

void NveQtWidget::Impl::place(QtNodeView& v, const host::NodeGeometry& node) {
  v.local = node.local;
  v.widget->setGeometry(pixel_rect(node.local));
  if (v.scroll != nullptr) v.scroll->setGeometry(0, 0, v.widget->width(), v.widget->height());
}

void NveQtWidget::Impl::apply_box(QtNodeView& v, const host::NodeGeometry& node) {
  if (v.frame != nullptr) {
    v.frame->set_visual(v.visual, QRectF(node.content.x, node.content.y, node.content.width, node.content.height),
                        v.clips);
  } else {
    const QString css = input_style_sheet(v.visual, node, v.text_color);
    if (v.widget->styleSheet() != css) v.widget->setStyleSheet(css);
  }
  if (v.shadow != nullptr) {
    v.shadow->set_shadow(v.visual, QRectF(node.local.x, node.local.y, node.local.width, node.local.height));
  }
}

void NveQtWidget::Impl::update_geometry(QtNodeView& v, const host::NodeData& node) {
  place(v, node);
  QWidget* parent = v.widget->parentWidget();
  if (v.z != node.visual.z_index) {
    v.z = node.visual.z_index;
    if (parent != nullptr) restack_.insert(parent);
  }
  v.visible = node.visual.visibility != NVE_VISIBILITY_HIDDEN;
  if (parent != nullptr) v.widget->setVisible(v.visible);
  const float opacity = node.visual.opacity;  // sealed to [0, 1] by `read_node`
  auto* effect = qobject_cast<QGraphicsOpacityEffect*>(v.widget->graphicsEffect());
  if (opacity >= 0.999f) {
    if (effect != nullptr) v.widget->setGraphicsEffect(nullptr);
  } else {
    if (effect == nullptr) {
      effect = new QGraphicsOpacityEffect(v.widget);
      v.widget->setGraphicsEffect(effect);
    }
    effect->setOpacity(static_cast<qreal>(opacity));
  }
}

void NveQtWidget::Impl::update_visual(QtNodeView& v, const host::NodeData& node) {
  v.visual = resolve_visual(node, v.applied, v.visual);
  v.clips = node.visual.overflow_x != NVE_OVERFLOW_VISIBLE || node.visual.overflow_y != NVE_OVERFLOW_VISIBLE;
  if (v.frame == nullptr) v.text_color = text_style(node.text_style).color;
  QWidget* parent = v.widget->parentWidget();
  if (!v.visual.has_outer_shadow()) {
    delete v.shadow;
    v.shadow = nullptr;
  } else if (v.shadow == nullptr) {
    v.shadow = new NveQtShadowWidget(parent);
    if (parent != nullptr) restack_.insert(parent);
  }
  apply_box(v, node);
  if (v.shadow == nullptr) return;
  v.shadow->setVisible(parent != nullptr && v.visible);
  auto* effect = qobject_cast<QGraphicsOpacityEffect*>(v.widget->graphicsEffect());
  v.shadow->setWindowOpacity(1.0);
  if (effect != nullptr) {
    auto* shadow_effect = qobject_cast<QGraphicsOpacityEffect*>(v.shadow->graphicsEffect());
    if (shadow_effect == nullptr) {
      shadow_effect = new QGraphicsOpacityEffect(v.shadow);
      v.shadow->setGraphicsEffect(shadow_effect);
    }
    shadow_effect->setOpacity(effect->opacity());
  } else {
    v.shadow->setGraphicsEffect(nullptr);
  }
}

void NveQtWidget::Impl::update_content(QtNodeView& v, const host::NodeData& node) {
  const std::optional<NodeContent> repainted = resolve_content(v.frame->edit_paint(), v.applied, node, canvas_);
  if (!repainted) return;
  if (*repainted == NodeContent::Canvas) ++v.redraw_count;
  v.frame->update();
}

void NveQtWidget::Impl::update_text_input(QtNodeView& v, const host::NodeData& node) {
  const NodeSemantics sem = node_semantics(node);
  const QtTextStyle style = text_style(node.text_style);
  const QString placeholder = scene_string(node.input.placeholder);
  v.max_length = node.input.max_length;
  QPalette pal = v.widget->palette();
  QColor dim = style.color;
  dim.setAlpha(style.color.alpha() / 2);
  pal.setColor(QPalette::PlaceholderText, dim);
  v.widget->setPalette(pal);
  if (v.widget->font() != style.font) v.widget->setFont(style.font);
  if (v.line != nullptr) {
    QLineEdit& e = *v.line;
    if (e.placeholderText() != placeholder) e.setPlaceholderText(placeholder);
    // Setters only on change: `setMaxLength` re-sets the text (moving the caret to the end).
    const QLineEdit::EchoMode echo = sem.secure ? QLineEdit::Password : QLineEdit::Normal;
    if (e.echoMode() != echo) e.setEchoMode(echo);
    constexpr uint32_t kQtMaxLength = 32767u;  // QLineEdit's default and upper bound
    const int max_length = static_cast<int>(node.input.max_length > 0u ? std::min(node.input.max_length, kQtMaxLength)
                                                                        : kQtMaxLength);
    if (e.maxLength() != max_length) e.setMaxLength(max_length);
    if (e.isReadOnly() != node.input.read_only) e.setReadOnly(node.input.read_only);
    const Qt::Alignment align = (style.justify ? Qt::AlignLeft : style.align) | Qt::AlignVCenter;
    if (e.alignment() != align) e.setAlignment(align);
  } else {
    QPlainTextEdit& e = *v.text_edit;
    if (e.placeholderText() != placeholder) e.setPlaceholderText(placeholder);
    if (e.isReadOnly() != node.input.read_only) e.setReadOnly(node.input.read_only);
  }
  if (composing(v)) return;  // never clobber a live IME composition
  // Controlled value: adopt the app's value unless it merely echoes what the user is typing.
  if (v.text.app_published(node.input.value, editable_text(v))) set_editable_text(v, node.input.value);
}

void NveQtWidget::Impl::update_scroll(QtNodeView& v, const host::NodeData& node) {
  v.axes = node.kind == host::NodeKind::Scroll ? node.scroll.axes
           : (node.visual.overflow_x >= NVE_OVERFLOW_SCROLL && node.visual.overflow_y >= NVE_OVERFLOW_SCROLL)
               ? NVE_SCROLL_AXES_BOTH
           : node.visual.overflow_x >= NVE_OVERFLOW_SCROLL ? NVE_SCROLL_AXES_HORIZONTAL
                                                           : NVE_SCROLL_AXES_VERTICAL;
  const Qt::ScrollBarPolicy shown = node.scroll.scrollbars == NVE_SCROLLBARS_ALWAYS  ? Qt::ScrollBarAlwaysOn
                                    : node.scroll.scrollbars == NVE_SCROLLBARS_NEVER ? Qt::ScrollBarAlwaysOff
                                                                                     : Qt::ScrollBarAsNeeded;
  const bool h = v.axes == NVE_SCROLL_AXES_HORIZONTAL || v.axes == NVE_SCROLL_AXES_BOTH;
  const bool vert = v.axes == NVE_SCROLL_AXES_VERTICAL || v.axes == NVE_SCROLL_AXES_BOTH;
  v.scroll->setHorizontalScrollBarPolicy(h ? shown : Qt::ScrollBarAlwaysOff);
  v.scroll->setVerticalScrollBarPolicy(vert ? shown : Qt::ScrollBarAlwaysOff);
  // Programmatic scroll (`requested_scroll_*`): each new target applies once, after extents.
  const auto take = [&](float target, float& last) {
    if (std::isnan(target) || target == last) return;
    last = target;
    v.request_pending = true;
  };
  take(node.scroll.requested_x, v.requested_x);
  take(node.scroll.requested_y, v.requested_y);
}

void NveQtWidget::Impl::update_interaction(QtNodeView& v, const host::NodeData& node) {
  v.event_mask = node.interaction.event_mask;
  v.wants_press = node.wants(NVE_EVENT_PRESS);
  v.href.assign(node.href);
  const bool activatable = !node.interaction.disabled && (v.wants_press || !v.href.empty());
  if (node.interaction.cursor != NVE_CURSOR_KIND_AUTO) {
    v.widget->setCursor(cursor_shape(node.interaction.cursor));
  } else if (activatable) {
    v.widget->setCursor(Qt::PointingHandCursor);
  } else {
    v.widget->unsetCursor();
  }
  if (v.frame != nullptr) {
    v.frame->setFocusPolicy(node.interaction.focusable && !node.interaction.disabled ? Qt::StrongFocus : Qt::NoFocus);
    v.frame->setMouseTracking(node.wants(NVE_EVENT_POINTER_MOVE));
  }
}

void NveQtWidget::Impl::apply_semantics(QtNodeView& v, const host::NodeData& node) {
  NodeSemantics sem = node_semantics(node);
  if (v.synced && sem == v.sem) return;
  notify_semantics_change(v.widget, v.sem, sem);
  v.sem = std::move(sem);
  v.widget->setEnabled(!v.sem.disabled);
  if (v.frame == nullptr) {  // inputs keep Qt's own accessible text interface
    v.widget->setAccessibleName(v.sem.label);
    v.widget->setAccessibleDescription(v.sem.hint);
  }
}

void NveQtWidget::Impl::restack(QWidget* container) {
  std::vector<QtNodeView*> kids;
  for (QObject* child : container->children()) {
    auto* w = qobject_cast<QWidget*>(child);
    if (w == nullptr || !w->property(kNodeIdProperty).isValid()) continue;
    if (QtNodeView* v = find(static_cast<uint32_t>(w->property(kNodeIdProperty).toULongLong()))) kids.push_back(v);
  }
  // Paint order: z-index, then scene order; each outer shadow sits right below its node.
  std::stable_sort(kids.begin(), kids.end(), [](const QtNodeView* a, const QtNodeView* b) {
    return a->z != b->z ? a->z < b->z : a->order < b->order;
  });
  for (QtNodeView* v : kids) {
    if (v->shadow != nullptr) v->shadow->raise();
    v->widget->raise();
  }
}

void NveQtWidget::Impl::update_scroll_extents() {
  // Content extent = union of the children's layout rects, at least the viewport.
  for (QtNodeView* v : scroll_views_) {
    const QSize viewport = v->scroll->maximumViewportSize();
    float w = 0.0f;
    float h = 0.0f;
    for (QObject* child : v->content->children()) {
      auto* cw = qobject_cast<QWidget*>(child);
      if (cw == nullptr || !cw->property(kNodeIdProperty).isValid()) continue;
      if (const QtNodeView* c = find(static_cast<uint32_t>(cw->property(kNodeIdProperty).toULongLong()))) {
        w = std::max(w, c->local.x + c->local.width);
        h = std::max(h, c->local.y + c->local.height);
      }
    }
    const bool horizontal = v->axes == NVE_SCROLL_AXES_HORIZONTAL || v->axes == NVE_SCROLL_AXES_BOTH;
    const bool vertical = v->axes == NVE_SCROLL_AXES_VERTICAL || v->axes == NVE_SCROLL_AXES_BOTH;
    const int cw = horizontal ? std::max(viewport.width(), static_cast<int>(std::ceil(w))) : viewport.width();
    const int ch = vertical ? std::max(viewport.height(), static_cast<int>(std::ceil(h))) : viewport.height();
    v->scrolling_programmatically = true;  // a shrinking extent clamps the bars
    v->content->resize(cw, ch);
    v->scrolling_programmatically = false;
  }
}

}  // namespace nve::qt
