#include "NveQtWidgetInternal.hpp"

#include <QAccessible>
#include <QAccessibleWidget>
#include <QFocusEvent>
#include <QInputMethodEvent>

#include <mutex>
#include <utility>

namespace nve::qt {

namespace {

class NveQtAccessibleWidget final : public QAccessibleWidget {
 public:
  explicit NveQtAccessibleWidget(QWidget* w)
      : QAccessibleWidget(
            w,
            static_cast<QAccessible::Role>(
                w->property(kA11yRoleProperty).toInt())) {}

  QAccessible::Role role() const override {
    if (QWidget* w = widget()) {
      const QVariant v = w->property(kA11yRoleProperty);
      if (v.isValid()) {
        return static_cast<QAccessible::Role>(v.toInt());
      }
    }
    return QAccessibleWidget::role();
  }

  QString text(QAccessible::Text t) const override {
    if (QWidget* w = widget()) {
      if (t == QAccessible::Name && !w->accessibleName().isEmpty()) {
        return w->accessibleName();
      }
      if (t == QAccessible::Description && !w->accessibleDescription().isEmpty()) {
        return w->accessibleDescription();
      }
      if (t == QAccessible::Value) {
        const QVariant v = w->property(kA11yValueProperty);
        if (v.isValid()) {
          return v.toString();
        }
        if (auto* edit = qobject_cast<QLineEdit*>(w)) {
          return edit->text();
        }
      }
    }
    return QAccessibleWidget::text(t);
  }

  QAccessible::State state() const override {
    QAccessible::State st = QAccessibleWidget::state();
    if (QWidget* w = widget()) {
      st.disabled = !w->isEnabled();
    }
    return st;
  }
};

QAccessibleInterface* nveQtAccessibleFactory(const QString& /*classname*/, QObject* object) {
  auto* widget = qobject_cast<QWidget*>(object);
  if (widget != nullptr && widget->property(kA11yRoleProperty).isValid()) {
    return new NveQtAccessibleWidget(widget);
  }
  return nullptr;
}

} // namespace

void ensureAccessibleFactoryInstalled() {
  static std::once_flag flag;
  std::call_once(flag, []() { QAccessible::installFactory(&nveQtAccessibleFactory); });
}

void applyWidgetAccessibility(QWidget* widget, const NodeSemantics& sem) {
  if (widget == nullptr) {
    return;
  }
  const QString prev_name = widget->accessibleName();
  const QString prev_desc = widget->accessibleDescription();

  widget->setAccessibleName(sem.label);
  widget->setAccessibleDescription(sem.hint);
  widget->setProperty(kA11yRoleProperty, static_cast<int>(sem.accessibleRole));
  if (sem.hasExplicitValue) {
    widget->setProperty(kA11yValueProperty, sem.value);
  } else {
    widget->setProperty(kA11yValueProperty, QVariant());
  }
  if (!sem.testId.isEmpty() && widget->objectName() != sem.testId) {
    widget->setObjectName(sem.testId);
  }
  if (widget->isEnabled() == sem.disabled) {
    widget->setEnabled(!sem.disabled);
  }

  if (prev_name != sem.label) {
    QAccessibleEvent ev(widget, QAccessible::NameChanged);
    QAccessible::updateAccessibility(&ev);
  }
  if (prev_desc != sem.hint) {
    QAccessibleEvent ev(widget, QAccessible::DescriptionChanged);
    QAccessible::updateAccessibility(&ev);
  }
}

// -----------------------------------------------------------------------------
// NveQtLineEdit
// -----------------------------------------------------------------------------

NveQtLineEdit::NveQtLineEdit(
    uint32_t node_id,
    TextChangeHandler on_text_change,
    SubmitHandler on_submit,
    FocusHandler on_focus,
    QWidget* parent)
    : QLineEdit(parent),
      node_id_(node_id),
      on_text_change_(std::move(on_text_change)),
      on_submit_(std::move(on_submit)),
      on_focus_(std::move(on_focus)) {
  setAttribute(Qt::WA_InputMethodEnabled, true);
  setProperty(kNodeIdProperty, QVariant::fromValue<qulonglong>(node_id));

  connect(this, &QLineEdit::textEdited, this, [this](const QString&) {
    if (!updating_programmatically_ && !composing_ime_) {
      dispatchCurrentTextIfChanged();
    }
  });
  connect(this, &QLineEdit::returnPressed, this, [this]() {
    if (!composing_ime_ && on_submit_) {
      const QByteArray utf8 = text().toUtf8();
      auto cb = on_submit_;
      const uint32_t id = node_id_;
      cb(id, std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())));
    }
  });
}

void NveQtLineEdit::applySemantics(const NodeSemantics& sem) {
  if (placeholderText() != sem.placeholder) {
    setPlaceholderText(sem.placeholder);
  }
  const EchoMode target_mode = sem.secureTextEntry ? QLineEdit::Password : QLineEdit::Normal;
  if (echoMode() != target_mode) {
    setEchoMode(target_mode);
  }
}

void NveQtLineEdit::applySceneText(const QString& next_text) {
  // Never clobber live IME composition mid-preedit.
  if (composing_ime_) {
    return;
  }
  if (!has_synced_scene_ || (next_text != last_scene_text_ && next_text != text())) {
    updating_programmatically_ = true;
    setText(next_text);
    last_dispatched_text_ = next_text;
    updating_programmatically_ = false;
  }
  last_scene_text_ = next_text;
  has_synced_scene_ = true;
}

void NveQtLineEdit::inputMethodEvent(QInputMethodEvent* event) {
  if (event == nullptr) {
    QLineEdit::inputMethodEvent(event);
    return;
  }
  const QString preedit = event->preeditString();
  const QString commit = event->commitString();
  const bool has_commit = !commit.isEmpty() || event->replacementLength() > 0;
  composing_ime_ = !preedit.isEmpty();

  QLineEdit::inputMethodEvent(event);

  if (has_commit && !composing_ime_) {
    dispatchCurrentTextIfChanged();
  }
}

void NveQtLineEdit::focusInEvent(QFocusEvent* event) {
  QLineEdit::focusInEvent(event);
  if (on_focus_) {
    auto cb = on_focus_;
    const uint32_t id = node_id_;
    cb(id, true);
  }
}

void NveQtLineEdit::focusOutEvent(QFocusEvent* event) {
  QLineEdit::focusOutEvent(event);
  if (on_focus_) {
    auto cb = on_focus_;
    const uint32_t id = node_id_;
    cb(id, false);
  }
}

void NveQtLineEdit::dispatchCurrentTextIfChanged() {
  const QString current = text();
  if (current == last_dispatched_text_) {
    return;
  }
  last_dispatched_text_ = current;
  if (on_text_change_) {
    const QByteArray utf8 = current.toUtf8();
    auto cb = on_text_change_;
    const uint32_t id = node_id_;
    cb(id, std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())));
  }
}

} // namespace nve::qt
