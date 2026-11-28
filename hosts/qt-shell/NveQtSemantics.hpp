// Accessibility semantics of one node (`nve.accessibility`, `nve.text_input`, `nve.interaction`,
// node kind) mapped onto Qt's accessibility model, shared by the Widgets and Quick hosts.
#pragma once

#include <nve/host/scene_nodes.hpp>

#include <QAccessible>
#include <QString>

#include <cmath>
#include <cstdint>
#include <optional>

class QObject;

namespace nve::qt {

struct NodeSemantics {
  QAccessible::Role role = QAccessible::Grouping;
  QString label;        // name: explicit label, else text / image alt / input placeholder
  QString hint;         // description
  QString value_text;
  QString placeholder;
  uint8_t level = 0u;   // heading level
  std::optional<uint8_t> checked;
  std::optional<uint8_t> selected;
  std::optional<uint8_t> expanded;
  std::optional<uint8_t> pressed;
  float value_now = NAN;
  float value_min = NAN;
  float value_max = NAN;
  bool disabled = false;
  bool hidden = false;
  bool focusable = false;
  bool activatable = false;  // has a press action (subscribed to Press, or a link)
  bool secure = false;
  bool multiline = false;
  bool read_only = false;
  bool search = false;

  friend bool operator==(const NodeSemantics& a, const NodeSemantics& b) noexcept;
};

/// Qt role of `host::aria_role(node)` (explicit role, else the node kind's native role).
[[nodiscard]] QAccessible::Role accessible_role(const host::NodeData& node) noexcept;
[[nodiscard]] NodeSemantics node_semantics(const host::NodeData& node);

[[nodiscard]] QAccessible::State accessible_state(const NodeSemantics& s) noexcept;
/// Accessible value: `value_text`, else `value_now` when finite, else empty.
[[nodiscard]] QString accessible_value(const NodeSemantics& s);

/// Posts the accessibility events (name / description / value / state) for `before` → `after`
/// of `target` (only while accessibility is active).
void notify_semantics_change(QAccessibleInterface* target, const NodeSemantics& before, const NodeSemantics& after);
void notify_semantics_change(QObject* target, const NodeSemantics& before, const NodeSemantics& after);

/// Implemented by host widgets that expose a node's semantics through `NveQtAccessibleWidget`.
class SemanticsCarrier {
 public:
  virtual ~SemanticsCarrier() = default;
  [[nodiscard]] virtual const NodeSemantics& semantics() const noexcept = 0;
  /// The accessible press action (no-op when the node has none).
  virtual void activate() {}
};

/// The press action while `s` is activatable and enabled.
[[nodiscard]] QStringList press_action_names(const NodeSemantics& s);
[[nodiscard]] bool is_press_action(const NodeSemantics& s, const QString& name);

/// Value (`value_now` / min / max; app-controlled, read-only) and attributes (heading level)
/// interfaces over a node's semantics, shared by the widget and the Qt Quick node interfaces.
class SemanticsInterfaces : public QAccessibleValueInterface, public QAccessibleAttributesInterface {
 public:
  [[nodiscard]] virtual const NodeSemantics& accessible_semantics() const = 0;
  /// `this` as the Value (only with a finite `value_now`) / Attributes interface, else nullptr.
  [[nodiscard]] void* semantics_interface(QAccessible::InterfaceType t);

  QVariant currentValue() const override;
  void setCurrentValue(const QVariant&) override {}
  QVariant maximumValue() const override;
  QVariant minimumValue() const override;
  QVariant minimumStepSize() const override { return {}; }
  QList<QAccessible::Attribute> attributeKeys() const override;
  QVariant attributeValue(QAccessible::Attribute key) const override;
};

/// Installs (once) the accessible-interface factory for `SemanticsCarrier` widgets.
void install_accessible_factory();

}  // namespace nve::qt
