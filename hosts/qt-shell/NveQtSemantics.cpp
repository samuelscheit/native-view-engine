#include "NveQtSemantics.hpp"

#include "NveQtText.hpp"

#include <QAccessibleWidget>
#include <QWidget>

#include <mutex>

namespace nve::qt {

bool operator==(const NodeSemantics& a, const NodeSemantics& b) noexcept {
  const auto same = [](float x, float y) { return (std::isnan(x) && std::isnan(y)) || x == y; };
  return a.role == b.role && a.label == b.label && a.hint == b.hint && a.value_text == b.value_text &&
         a.placeholder == b.placeholder && a.level == b.level && a.checked == b.checked && a.selected == b.selected &&
         a.expanded == b.expanded && a.pressed == b.pressed && same(a.value_now, b.value_now) &&
         same(a.value_min, b.value_min) && same(a.value_max, b.value_max) && a.disabled == b.disabled &&
         a.hidden == b.hidden && a.focusable == b.focusable && a.activatable == b.activatable &&
         a.secure == b.secure && a.multiline == b.multiline && a.read_only == b.read_only && a.search == b.search;
}

namespace {

QAccessible::Role qt_role(uint8_t role) noexcept {
  switch (role) {
    case NVE_ARIA_ROLE_BUTTON: return QAccessible::Button;
    case NVE_ARIA_ROLE_LINK: return QAccessible::Link;
    case NVE_ARIA_ROLE_HEADING: return QAccessible::Heading;
    case NVE_ARIA_ROLE_TEXT:
    case NVE_ARIA_ROLE_CODE: return QAccessible::StaticText;
    case NVE_ARIA_ROLE_CODE_BLOCK: return QAccessible::Paragraph;
    case NVE_ARIA_ROLE_IMAGE: return QAccessible::Graphic;
    case NVE_ARIA_ROLE_TEXT_BOX:
    case NVE_ARIA_ROLE_SEARCH_BOX: return QAccessible::EditableText;
    case NVE_ARIA_ROLE_SCROLL_AREA:
    case NVE_ARIA_ROLE_TAB_PANEL: return QAccessible::Pane;
    case NVE_ARIA_ROLE_CHECKBOX: return QAccessible::CheckBox;
    case NVE_ARIA_ROLE_SWITCH: return QAccessible::Switch;
    case NVE_ARIA_ROLE_RADIO: return QAccessible::RadioButton;
    case NVE_ARIA_ROLE_SLIDER: return QAccessible::Slider;
    case NVE_ARIA_ROLE_TAB: return QAccessible::PageTab;
    case NVE_ARIA_ROLE_TAB_LIST: return QAccessible::PageTabList;
    case NVE_ARIA_ROLE_DIALOG: return QAccessible::Dialog;
    case NVE_ARIA_ROLE_ALERT_DIALOG:
    case NVE_ARIA_ROLE_ALERT: return QAccessible::AlertMessage;
    case NVE_ARIA_ROLE_STATUS: return QAccessible::StatusBar;
    case NVE_ARIA_ROLE_TOOLTIP: return QAccessible::ToolTip;
    case NVE_ARIA_ROLE_NAVIGATION:
    case NVE_ARIA_ROLE_MAIN:
    case NVE_ARIA_ROLE_BANNER:
    case NVE_ARIA_ROLE_REGION: return QAccessible::Section;
    case NVE_ARIA_ROLE_ARTICLE: return QAccessible::Document;
    case NVE_ARIA_ROLE_COMPLEMENTARY: return QAccessible::ComplementaryContent;
    case NVE_ARIA_ROLE_CONTENT_INFO: return QAccessible::Footer;
    case NVE_ARIA_ROLE_FORM: return QAccessible::Form;
    case NVE_ARIA_ROLE_GROUP: return QAccessible::Grouping;
    case NVE_ARIA_ROLE_LIST:
    case NVE_ARIA_ROLE_LIST_BOX: return QAccessible::List;
    case NVE_ARIA_ROLE_LIST_ITEM:
    case NVE_ARIA_ROLE_OPTION: return QAccessible::ListItem;
    case NVE_ARIA_ROLE_MENU: return QAccessible::PopupMenu;
    case NVE_ARIA_ROLE_MENU_ITEM: return QAccessible::MenuItem;
    case NVE_ARIA_ROLE_SEPARATOR: return QAccessible::Separator;
    case NVE_ARIA_ROLE_PRESENTATION: return QAccessible::NoRole;
    case NVE_ARIA_ROLE_TABLE:
    case NVE_ARIA_ROLE_GRID: return QAccessible::Table;
    case NVE_ARIA_ROLE_ROW: return QAccessible::Row;
    case NVE_ARIA_ROLE_CELL:
    case NVE_ARIA_ROLE_GRID_CELL: return QAccessible::Cell;
    case NVE_ARIA_ROLE_COLUMN_HEADER: return QAccessible::ColumnHeader;
    case NVE_ARIA_ROLE_ROW_HEADER: return QAccessible::RowHeader;
    case NVE_ARIA_ROLE_PROGRESS_BAR: return QAccessible::ProgressBar;
    case NVE_ARIA_ROLE_COMBO_BOX: return QAccessible::ComboBox;
    default: return QAccessible::Grouping;  // `NONE` and future roles
  }
}

QString qstr(std::string_view s) { return scene_string(s); }

/// Accessible interface of a `SemanticsCarrier` widget: role, name, description, value (with
/// range), heading level, ARIA states, and the press action.
class NveQtAccessibleWidget final : public QAccessibleWidget,
                                    public QAccessibleValueInterface,
                                    public QAccessibleAttributesInterface {
 public:
  NveQtAccessibleWidget(QWidget* w, SemanticsCarrier* carrier)
      : QAccessibleWidget(w, carrier->semantics().role), carrier_(carrier) {}

  QAccessible::Role role() const override { return carrier_->semantics().role; }

  QString text(QAccessible::Text t) const override {
    const NodeSemantics& s = carrier_->semantics();
    switch (t) {
      case QAccessible::Name: return s.label;
      case QAccessible::Description: return s.hint;
      case QAccessible::Value: return accessible_value(s);
      default: return QAccessibleWidget::text(t);
    }
  }

  QAccessible::State state() const override {
    QAccessible::State st = QAccessibleWidget::state();
    const QAccessible::State own = accessible_state(carrier_->semantics());
    st.disabled = st.disabled || own.disabled;
    st.invisible = st.invisible || own.invisible;
    st.focusable = st.focusable || own.focusable;
    st.checkable = own.checkable;
    st.checked = own.checked;
    st.checkStateMixed = own.checkStateMixed;
    st.selectable = own.selectable;
    st.selected = own.selected;
    st.expandable = own.expandable;
    st.expanded = own.expanded;
    st.collapsed = own.collapsed;
    st.pressed = own.pressed;
    st.readOnly = own.readOnly;
    st.multiLine = own.multiLine;
    st.passwordEdit = own.passwordEdit;
    st.searchEdit = own.searchEdit;
    st.editable = own.editable;
    return st;
  }

  QStringList actionNames() const override {
    return carrier_->semantics().activatable && !carrier_->semantics().disabled ? QStringList{pressAction()}
                                                                                : QStringList{};
  }

  void doAction(const QString& name) override {
    if (name == pressAction() && carrier_->semantics().activatable && !carrier_->semantics().disabled) {
      carrier_->activate();
    }
  }

  void* interface_cast(QAccessible::InterfaceType t) override {
    if (t == QAccessible::ValueInterface && std::isfinite(carrier_->semantics().value_now)) {
      return static_cast<QAccessibleValueInterface*>(this);
    }
    if (t == QAccessible::AttributesInterface) return static_cast<QAccessibleAttributesInterface*>(this);
    return QAccessibleWidget::interface_cast(t);
  }

  QVariant currentValue() const override { return static_cast<double>(carrier_->semantics().value_now); }
  void setCurrentValue(const QVariant&) override {}  // values are app-controlled
  QVariant maximumValue() const override { return finite_or_null(carrier_->semantics().value_max); }
  QVariant minimumValue() const override { return finite_or_null(carrier_->semantics().value_min); }
  QVariant minimumStepSize() const override { return QVariant(); }

  QList<QAccessible::Attribute> attributeKeys() const override {
    return carrier_->semantics().level != 0u ? QList<QAccessible::Attribute>{QAccessible::Attribute::Level}
                                             : QList<QAccessible::Attribute>{};
  }
  QVariant attributeValue(QAccessible::Attribute key) const override {
    const uint8_t level = carrier_->semantics().level;
    return key == QAccessible::Attribute::Level && level != 0u ? QVariant(static_cast<int>(level)) : QVariant();
  }

 private:
  static QVariant finite_or_null(float v) { return std::isfinite(v) ? QVariant(static_cast<double>(v)) : QVariant(); }

  SemanticsCarrier* carrier_;
};

QAccessibleInterface* accessible_factory(const QString&, QObject* object) {
  auto* widget = qobject_cast<QWidget*>(object);
  auto* carrier = dynamic_cast<SemanticsCarrier*>(object);
  return widget != nullptr && carrier != nullptr ? new NveQtAccessibleWidget(widget, carrier) : nullptr;
}

}  // namespace

QAccessible::Role accessible_role(const host::NodeData& node) noexcept { return qt_role(host::aria_role(node)); }

NodeSemantics node_semantics(const host::NodeData& node) {
  NodeSemantics s;
  s.role = accessible_role(node);
  const host::AccessibilityData& a = node.a11y;
  s.label = qstr(host::accessible_label(node));
  if (s.label.isEmpty() && node.kind == host::NodeKind::TextInput) s.label = qstr(node.input.placeholder);
  s.hint = qstr(a.hint);
  s.value_text = qstr(a.value_text);
  s.placeholder = qstr(node.input.placeholder);
  s.level = a.level;
  s.checked = a.checked;
  s.selected = a.selected;
  s.expanded = a.expanded;
  s.pressed = a.pressed;
  s.value_now = a.value_now;
  s.value_min = a.value_min;
  s.value_max = a.value_max;
  s.disabled = node.interaction.disabled;
  s.hidden = a.hidden || node.visual.visibility == NVE_VISIBILITY_HIDDEN;
  s.focusable = node.interaction.focusable || node.kind == host::NodeKind::TextInput;
  s.activatable = node.wants(NVE_EVENT_PRESS) || !node.href.empty();
  s.secure = node.input.secure || node.input.input_kind == NVE_INPUT_KIND_PASSWORD;
  s.multiline = node.input.multiline;
  s.read_only = node.input.read_only;
  s.search = node.input.input_kind == NVE_INPUT_KIND_SEARCH || (a.present && a.role == NVE_ARIA_ROLE_SEARCH_BOX);
  return s;
}

QAccessible::State accessible_state(const NodeSemantics& s) noexcept {
  QAccessible::State st;
  st.disabled = s.disabled;
  st.invisible = s.hidden;
  st.focusable = s.focusable;
  st.checkable = s.checked != NVE_ARIA_TRI_STATE_UNSET;
  st.checked = s.checked == NVE_ARIA_TRI_STATE_TRUE;
  st.checkStateMixed = s.checked == NVE_ARIA_TRI_STATE_MIXED;
  st.selectable = s.selected != NVE_ARIA_TRI_STATE_UNSET;
  st.selected = s.selected == NVE_ARIA_TRI_STATE_TRUE;
  st.expandable = s.expanded != NVE_ARIA_TRI_STATE_UNSET;
  st.expanded = s.expanded == NVE_ARIA_TRI_STATE_TRUE;
  st.collapsed = s.expanded == NVE_ARIA_TRI_STATE_FALSE;
  st.pressed = s.pressed == NVE_ARIA_TRI_STATE_TRUE;
  const bool editable = s.role == QAccessible::EditableText;
  st.editable = editable && !s.read_only;
  st.readOnly = s.read_only;
  st.multiLine = editable && s.multiline;
  st.passwordEdit = editable && s.secure;
  st.searchEdit = editable && s.search;
  return st;
}

QString accessible_value(const NodeSemantics& s) {
  if (!s.value_text.isEmpty()) return s.value_text;
  return std::isfinite(s.value_now) ? QString::number(static_cast<double>(s.value_now)) : QString();
}

void notify_semantics_change(QObject* target, int child, const NodeSemantics& before, const NodeSemantics& after) {
  if (target == nullptr || !QAccessible::isActive()) return;
  const auto post = [&](QAccessible::Event kind) {
    QAccessibleEvent ev(target, kind);
    ev.setChild(child);
    QAccessible::updateAccessibility(&ev);
  };
  if (before.label != after.label) post(QAccessible::NameChanged);
  if (before.hint != after.hint) post(QAccessible::DescriptionChanged);
  if (accessible_value(before) != accessible_value(after)) post(QAccessible::ValueChanged);
  const QAccessible::State a = accessible_state(before);
  const QAccessible::State b = accessible_state(after);
  QAccessible::State changed;
  changed.disabled = a.disabled != b.disabled;
  changed.invisible = a.invisible != b.invisible;
  changed.checked = a.checked != b.checked || a.checkStateMixed != b.checkStateMixed;
  changed.selected = a.selected != b.selected;
  changed.expanded = a.expanded != b.expanded || a.collapsed != b.collapsed;
  changed.pressed = a.pressed != b.pressed;
  changed.readOnly = a.readOnly != b.readOnly;
  if (changed.disabled || changed.invisible || changed.checked || changed.selected || changed.expanded ||
      changed.pressed || changed.readOnly) {
    QAccessibleStateChangeEvent ev(target, changed);
    ev.setChild(child);
    QAccessible::updateAccessibility(&ev);
  }
}

void install_accessible_factory() {
  static std::once_flag once;
  std::call_once(once, [] { QAccessible::installFactory(&accessible_factory); });
}

}  // namespace nve::qt
