// The widgets a `NveQtWidget` node view is made of: the painted node frame, the outer-shadow
// sibling, and the two text inputs.
#include "NveQtWidgetInternal.hpp"

#include <QFocusEvent>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPainter>
#include <QPointer>
#include <QRegion>

#include <algorithm>

namespace nve::qt {

namespace {

const NodeSemantics& no_semantics() {
  static const NodeSemantics kNone;
  return kNone;
}

bool wants(const QtNodeView* view, NveInteractionEvent kind) noexcept {
  return view != nullptr && !view->sem.disabled && (view->interaction_events & host::event_set_bit(kind)) != 0u;
}

}  // namespace

// --- NveQtNodeWidget ---------------------------------------------------------------------------

NveQtNodeWidget::NveQtNodeWidget(QtNodeView* view, QWidget* parent) : QWidget(parent), view_(view) {
  setAttribute(Qt::WA_NoSystemBackground, true);
  setAutoFillBackground(false);
  setFocusPolicy(Qt::NoFocus);
}

void NveQtNodeWidget::set_visual(const BoxVisual& visual, const QRectF& content_box, bool clip_rounded) {
  NodePaint& n = paint_;
  if (visual == n.visual && content_box == n.content_box && clip_rounded == n.clip_rounded) return;
  const bool mask_changed = clip_rounded != n.clip_rounded || visual.radius != n.visual.radius;
  n.visual = visual;
  n.content_box = content_box;
  n.clip_rounded = clip_rounded;
  if (mask_changed) update_mask();
  update();
}

const NodeSemantics& NveQtNodeWidget::semantics() const noexcept {
  return view_ != nullptr ? view_->sem : no_semantics();
}

void NveQtNodeWidget::activate() {
  if (view_ != nullptr) view_activate(view_, QPointF(width() * 0.5, height() * 0.5));
}

QList<QPair<QAccessibleInterface*, QAccessible::Relation>> NveQtNodeWidget::accessible_relations(
    QAccessible::Relation match) const {
  return view_ != nullptr ? view_->host->accessible_relations_for(*view_, match)
                          : QList<QPair<QAccessibleInterface*, QAccessible::Relation>>{};
}

QAccessibleInterface* NveQtNodeWidget::active_descendant_interface() const {
  return view_ != nullptr ? view_->host->active_descendant_for(*view_) : nullptr;
}

void NveQtNodeWidget::update_mask() {
  // `overflow: hidden` with rounded corners clips the node's own painting and its children.
  if (paint_.clip_rounded && paint_.visual.radius > 0.0 && width() > 0 && height() > 0) {
    setMask(QRegion(rounded_path(QRectF(rect()), paint_.visual.radius).toFillPolygon().toPolygon()));
  } else {
    clearMask();
  }
}

void NveQtNodeWidget::resizeEvent(QResizeEvent* event) {
  QWidget::resizeEvent(event);
  if (paint_.clip_rounded) update_mask();
}

void NveQtNodeWidget::paintEvent(QPaintEvent*) {
  QPainter p(this);
  paint_node(p, QRectF(rect()), paint_, &inset_cache_);
}

void NveQtNodeWidget::mousePressEvent(QMouseEvent* event) {
  const bool activatable = view_ != nullptr && view_->sem.activatable && !view_->sem.disabled;
  if (event->button() != Qt::LeftButton || !activatable) {
    event->ignore();  // bubbles to the nearest pressable ancestor widget
    return;
  }
  pressed_ = true;
  if (view_->sem.focusable) setFocus(Qt::MouseFocusReason);
  event->accept();
}

void NveQtNodeWidget::mouseReleaseEvent(QMouseEvent* event) {
  if (event->button() != Qt::LeftButton || !pressed_) {
    event->ignore();
    return;
  }
  pressed_ = false;
  event->accept();
  if (view_ != nullptr && rect().contains(event->position().toPoint())) view_activate(view_, event->position());
}

void NveQtNodeWidget::mouseMoveEvent(QMouseEvent* event) {
  if (wants(view_, NVE_INTERACTION_EVENT_POINTER_MOVE)) {
    view_hover(view_, NVE_INTERACTION_EVENT_POINTER_MOVE, event->position());
  }
  QWidget::mouseMoveEvent(event);
}

void NveQtNodeWidget::enterEvent(QEnterEvent* event) {
  view_hover(view_, NVE_INTERACTION_EVENT_POINTER_ENTER, event->position());
  QWidget::enterEvent(event);
}

void NveQtNodeWidget::leaveEvent(QEvent* event) {
  view_hover(view_, NVE_INTERACTION_EVENT_POINTER_LEAVE, QPointF());
  QWidget::leaveEvent(event);
}

void NveQtNodeWidget::focusInEvent(QFocusEvent* event) {
  QWidget::focusInEvent(event);
  view_focus(view_, true);
}

void NveQtNodeWidget::focusOutEvent(QFocusEvent* event) {
  QWidget::focusOutEvent(event);
  view_focus(view_, false);
}

void NveQtNodeWidget::keyPressEvent(QKeyEvent* event) {
  const QPointer<NveQtNodeWidget> self(this);
  if (view_ != nullptr) view_key(view_, NVE_INTERACTION_EVENT_KEY_DOWN, *event, false);
  if (self.isNull()) return;
  const bool activation_key =
      event->key() == Qt::Key_Return || event->key() == Qt::Key_Enter || event->key() == Qt::Key_Space;
  if (activation_key && view_ != nullptr && view_->sem.activatable && !view_->sem.disabled) {
    event->accept();
    activate();
    return;
  }
  QWidget::keyPressEvent(event);
}

void NveQtNodeWidget::keyReleaseEvent(QKeyEvent* event) {
  const QPointer<NveQtNodeWidget> self(this);
  if (view_ != nullptr) view_key(view_, NVE_INTERACTION_EVENT_KEY_UP, *event, false);
  if (self.isNull()) return;
  QWidget::keyReleaseEvent(event);
}

// --- NveQtShadowWidget -------------------------------------------------------------------------

NveQtShadowWidget::NveQtShadowWidget(QWidget* parent) : QWidget(parent) {
  setAttribute(Qt::WA_TransparentForMouseEvents, true);
  setAttribute(Qt::WA_NoSystemBackground, true);
  setFocusPolicy(Qt::NoFocus);
}

void NveQtShadowWidget::set_shadow(const BoxVisual& visual, const QRectF& box) {
  const QRectF area = box.marginsAdded(outer_shadow_margins(visual.shadows));
  const QRect px(QPoint(static_cast<int>(std::floor(area.left())), static_cast<int>(std::floor(area.top()))),
                 QPoint(static_cast<int>(std::ceil(area.right())) - 1, static_cast<int>(std::ceil(area.bottom())) - 1));
  if (visual == visual_ && box == box_ && geometry() == px) return;
  visual_ = visual;
  box_ = box;
  setGeometry(px);
  update();
}

void NveQtShadowWidget::paintEvent(QPaintEvent*) {
  QPainter p(this);
  cache_.paint(p, box_.translated(-QPointF(pos())), visual_.radius, visual_.shadows, false);
}

// --- Text inputs -------------------------------------------------------------------------------

NveQtLineEdit::NveQtLineEdit(QtNodeView* view, QWidget* parent) : QLineEdit(parent), view_(view) {
  setAttribute(Qt::WA_InputMethodEnabled, true);
  setFrame(false);
  connect(this, &QLineEdit::textEdited, this, [this](const QString&) {
    if (view_ != nullptr && !composing_ && !in_ime_) view_->host->on_text_edited(*view_);
  });
  connect(this, &QLineEdit::returnPressed, this, [this] {
    if (view_ != nullptr && !composing_) view_->host->on_submit(*view_);
  });
}

void NveQtLineEdit::inputMethodEvent(QInputMethodEvent* event) {
  const QPointer<NveQtLineEdit> self(this);
  const bool was_composing = composing_;
  const QString preedit = event->preeditString();
  const QString commit = event->commitString();
  const bool now_composing = !preedit.isEmpty();
  const bool commits = !commit.isEmpty() || event->replacementLength() > 0;
  composing_ = now_composing;
  if (!was_composing && now_composing && view_ != nullptr) {
    (void)view_->host->dispatch_composition(view_->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_START, {});
    if (self.isNull()) return;
  }
  if (now_composing && view_ != nullptr) {
    (void)view_->host->dispatch_composition(view_->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE, preedit.toStdString());
    if (self.isNull()) return;
  }
  in_ime_ = true;
  QLineEdit::inputMethodEvent(event);
  if (self.isNull()) return;
  in_ime_ = false;
  if (was_composing && !now_composing && view_ != nullptr) {
    (void)view_->host->dispatch_composition(view_->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, commit.toStdString());
    if (self.isNull()) return;
  }
  if (commits && composing_) {
    pending_change_after_compose_ = true;
  } else if (!composing_ && (commits || std::exchange(pending_change_after_compose_, false)) && view_ != nullptr) {
    view_->host->on_text_edited(*view_);
  }
}

void NveQtLineEdit::keyPressEvent(QKeyEvent* event) {
  const QPointer<NveQtLineEdit> self(this);
  if (view_ != nullptr) view_key(view_, NVE_INTERACTION_EVENT_KEY_DOWN, *event, composing_);
  if (self.isNull()) return;
  QLineEdit::keyPressEvent(event);
}

void NveQtLineEdit::keyReleaseEvent(QKeyEvent* event) {
  const QPointer<NveQtLineEdit> self(this);
  if (view_ != nullptr) view_key(view_, NVE_INTERACTION_EVENT_KEY_UP, *event, composing_);
  if (self.isNull()) return;
  QLineEdit::keyReleaseEvent(event);
}

void NveQtLineEdit::focusInEvent(QFocusEvent* event) {
  QLineEdit::focusInEvent(event);
  view_focus(view_, true);
}

void NveQtLineEdit::focusOutEvent(QFocusEvent* event) {
  const QPointer<NveQtLineEdit> self(this);
  const bool was_composing = std::exchange(composing_, false);
  const bool flush_pending = std::exchange(pending_change_after_compose_, false);
  QLineEdit::focusOutEvent(event);
  if (self.isNull()) return;
  if (was_composing && view_ != nullptr) {
    (void)view_->host->dispatch_composition(view_->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, {});
    if (self.isNull()) return;
  }
  if (flush_pending && view_ != nullptr) {
    view_->host->on_text_edited(*view_);
    if (self.isNull()) return;
  }
  view_focus(view_, false);
}

void NveQtLineEdit::enterEvent(QEnterEvent* event) {
  view_hover(view_, NVE_INTERACTION_EVENT_POINTER_ENTER, event->position());
  QLineEdit::enterEvent(event);
}

void NveQtLineEdit::leaveEvent(QEvent* event) {
  view_hover(view_, NVE_INTERACTION_EVENT_POINTER_LEAVE, QPointF());
  QLineEdit::leaveEvent(event);
}

NveQtTextEdit::NveQtTextEdit(QtNodeView* view, QWidget* parent) : QPlainTextEdit(parent), view_(view) {
  setAttribute(Qt::WA_InputMethodEnabled, true);
  setFrameShape(QFrame::NoFrame);
  setLineWrapMode(QPlainTextEdit::WidgetWidth);
  connect(this, &QPlainTextEdit::textChanged, this, [this] {
    if (view_ != nullptr && !composing_ && !in_ime_ && !view_->text.writing()) view_->host->on_text_edited(*view_);
  });
}

void NveQtTextEdit::inputMethodEvent(QInputMethodEvent* event) {
  const QPointer<NveQtTextEdit> self(this);
  const bool was_composing = composing_;
  const QString preedit = event->preeditString();
  const QString commit = event->commitString();
  const bool now_composing = !preedit.isEmpty();
  const bool commits = !commit.isEmpty() || event->replacementLength() > 0;
  composing_ = now_composing;
  if (!was_composing && now_composing && view_ != nullptr) {
    (void)view_->host->dispatch_composition(view_->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_START, {});
    if (self.isNull()) return;
  }
  if (now_composing && view_ != nullptr) {
    (void)view_->host->dispatch_composition(view_->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_UPDATE, preedit.toStdString());
    if (self.isNull()) return;
  }
  in_ime_ = true;
  QPlainTextEdit::inputMethodEvent(event);
  if (self.isNull()) return;
  in_ime_ = false;
  if (was_composing && !now_composing && view_ != nullptr) {
    (void)view_->host->dispatch_composition(view_->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, commit.toStdString());
    if (self.isNull()) return;
  }
  if (commits && composing_) {
    pending_change_after_compose_ = true;
  } else if (!composing_ && (commits || std::exchange(pending_change_after_compose_, false)) && view_ != nullptr) {
    view_->host->on_text_edited(*view_);
  }
}

void NveQtTextEdit::keyPressEvent(QKeyEvent* event) {
  const QPointer<NveQtTextEdit> self(this);
  if (view_ != nullptr) view_key(view_, NVE_INTERACTION_EVENT_KEY_DOWN, *event, composing_);
  if (self.isNull()) return;
  QPlainTextEdit::keyPressEvent(event);
}

void NveQtTextEdit::keyReleaseEvent(QKeyEvent* event) {
  const QPointer<NveQtTextEdit> self(this);
  if (view_ != nullptr) view_key(view_, NVE_INTERACTION_EVENT_KEY_UP, *event, composing_);
  if (self.isNull()) return;
  QPlainTextEdit::keyReleaseEvent(event);
}

void NveQtTextEdit::focusInEvent(QFocusEvent* event) {
  QPlainTextEdit::focusInEvent(event);
  view_focus(view_, true);
}

void NveQtTextEdit::focusOutEvent(QFocusEvent* event) {
  const QPointer<NveQtTextEdit> self(this);
  const bool was_composing = std::exchange(composing_, false);
  const bool flush_pending = std::exchange(pending_change_after_compose_, false);
  QPlainTextEdit::focusOutEvent(event);
  if (self.isNull()) return;
  if (was_composing && view_ != nullptr) {
    (void)view_->host->dispatch_composition(view_->id, NVE_TEXT_INPUT_EVENT_COMPOSITION_END, {});
    if (self.isNull()) return;
  }
  if (flush_pending && view_ != nullptr) {
    view_->host->on_text_edited(*view_);
    if (self.isNull()) return;
  }
  view_focus(view_, false);
}

void NveQtTextEdit::enterEvent(QEnterEvent* event) {
  view_hover(view_, NVE_INTERACTION_EVENT_POINTER_ENTER, event->position());
  QPlainTextEdit::enterEvent(event);
}

void NveQtTextEdit::leaveEvent(QEvent* event) {
  view_hover(view_, NVE_INTERACTION_EVENT_POINTER_LEAVE, QPointF());
  QPlainTextEdit::leaveEvent(event);
}

}  // namespace nve::qt
