// Accessibility semantics of one node (`nve.accessibility`, `nve.text_input`, `nve.interaction`,
// node kind) mapped onto Qt's accessibility model, shared by the Widgets and Quick hosts.
#pragma once

#include <nve/host/scene_nodes.hpp>

#include <QAccessible>
#include <QString>

#include <cmath>
#include <cstdint>

class QObject;

namespace nve::qt {

struct NodeSemantics {
  QAccessible::Role role = QAccessible::Grouping;
  QString label;        // name: explicit label, else text / image alt / input placeholder
  QString hint;         // description
  QString value_text;
  QString placeholder;
  uint8_t level = 0u;   // heading level
  uint8_t checked = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t selected = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t expanded = NVE_ARIA_TRI_STATE_UNSET;
  uint8_t pressed = NVE_ARIA_TRI_STATE_UNSET;
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
/// of `target` (`child` = -1 for the object itself).
void notify_semantics_change(QObject* target, int child, const NodeSemantics& before, const NodeSemantics& after);

/// Implemented by host widgets that expose a node's semantics through `NveQtAccessibleWidget`.
class SemanticsCarrier {
 public:
  virtual ~SemanticsCarrier() = default;
  [[nodiscard]] virtual const NodeSemantics& semantics() const noexcept = 0;
  /// The accessible press action (no-op when the node has none).
  virtual void activate() {}
};

/// Installs (once) the accessible-interface factory for `SemanticsCarrier` widgets.
void install_accessible_factory();

}  // namespace nve::qt
