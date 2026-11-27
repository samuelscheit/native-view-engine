#pragma once

#include "NveQtHost.hpp"
#include "NveQtSemantics.hpp"

#include <QWidget>

#include <cstdint>
#include <memory>
#include <optional>
#include <span>

namespace nve::qt {

/// Embeddable Qt 6 `QWidget` host: one `libnve_runtime` surface (`QtHost`) mirrored into
/// retained widgets by `RetainedTree` (O(changed) per frame). Every node is an `NveQtNodeWidget`
/// painting its visuals (background, border, radius, inset shadows; outer shadows on a sibling
/// below it) and content (typography-styled text, fitted / tinted images, `nve.canvas` draw lists);
/// text inputs are `QLineEdit` / `QPlainTextEdit`, scroll containers host a `QScrollArea`.
/// Press / link / hover / focus / text / submit / scroll events, cursors, z-order, opacity,
/// visibility, clipping, accessibility, live resize, and the `QtHost` app lifecycles.
class NveQtWidget : public QWidget, public SemanticsCarrier {
  Q_OBJECT

 public:
  explicit NveQtWidget(QtHostConfig config = {}, QWidget* parent = nullptr);
  ~NveQtWidget() override;
  NveQtWidget(const NveQtWidget&) = delete;
  NveQtWidget& operator=(const NveQtWidget&) = delete;

  /// The runtime surface: app sources, state, props, hot patches, input dispatch, pumping.
  [[nodiscard]] QtHost& host() noexcept;
  [[nodiscard]] const QtHost& host() const noexcept;

  /// Retained widget of packed node id `nodeId` (the root id maps to `this`; `nullptr` when absent).
  [[nodiscard]] QWidget* nativeWidgetForNodeId(uint32_t nodeId) const;
  /// Packed node id of a mounted widget (`0` when it is not a node view).
  [[nodiscard]] uint32_t nodeIdForNativeWidget(const QWidget* widget) const;
  [[nodiscard]] std::optional<NodeSemantics> accessibilityInfoForNodeId(uint32_t nodeId) const;
  /// Copied draw list painted by a canvas node (empty when none).
  [[nodiscard]] std::span<const uint8_t> canvasDrawListForNodeId(uint32_t nodeId) const;
  [[nodiscard]] uint64_t canvasRedrawCountForNodeId(uint32_t nodeId) const;
  [[nodiscard]] size_t viewCount() const;
  [[nodiscard]] uint64_t reconcileUpdateCount() const noexcept;
  /// Geometry-only `move`s (nodes whose box alone changed: repositioned, nothing decoded).
  [[nodiscard]] uint64_t reconcileMoveCount() const noexcept;

  [[nodiscard]] const NodeSemantics& semantics() const noexcept override;
  [[nodiscard]] QSize sizeHint() const override;

  class Impl;  // NveQtWidgetInternal.hpp

 protected:
  void resizeEvent(QResizeEvent* event) override;
  void paintEvent(QPaintEvent* event) override;

 private:
  std::unique_ptr<Impl> impl_;
};

}  // namespace nve::qt
