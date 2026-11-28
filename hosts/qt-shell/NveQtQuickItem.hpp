#pragma once

#include "NveQtHost.hpp"
#include "NveQtSemantics.hpp"

#include <QPointF>
#include <QQuickItem>
#include <QRectF>
#include <QString>

#include <cstdint>
#include <memory>
#include <optional>

class QSGNode;

namespace nve::qt {

/// Telemetry of the retained Qt Scene Graph (cumulative, read on the GUI thread).
struct SceneGraphStats {
  uint64_t syncCount = 0u;          // `updatePaintNode` passes that had work
  uint64_t sceneGeneration = 0u;    // published runtime frame last mirrored
  uint32_t retainedNodes = 0u;      // live node wrappers in the SG tree
  uint64_t nodesUpdated = 0u;       // node wrappers whose properties were updated
  uint64_t nodesReused = 0u;        // ... of which already existed (retained across frames)
  uint64_t nodesMoved = 0u;         // records whose box alone changed (`move`: nothing decoded)
  uint64_t textRasterizations = 0u;
  uint64_t imageRasterizations = 0u;
  uint64_t canvasRasterizations = 0u;
  uint64_t boxRasterizations = 0u;  // boxes without content (rounded / border / shadows)
  uint64_t solidRectNodes = 0u;     // plain backgrounds drawn without a texture
  uint64_t texturesReused = 0u;     // dirty nodes whose painted texture was still valid
};

/// Qt Quick host (`QQuickItem` on the retained Qt Scene Graph) for any NVE app: one `QtHost`
/// runtime surface mirrored by `RetainedTree` into GUI-thread node records, which
/// `updatePaintNode` turns into a retained `QSGNode` tree (transform / opacity / clip / scroll
/// per node; plain backgrounds as rect nodes; boxes, text, images and canvases as textures painted
/// by the same code as the Widgets host, re-rasterized only when they change). Input: mouse
/// press / links, hover + cursors, wheel scrolling, focus, keyboard activation, text editing with
/// IME; accessibility exposes every node as a `QAccessibleInterface`.
class NveQtQuickItem : public QQuickItem {
  Q_OBJECT

  Q_PROPERTY(QString dylibPath READ dylibPath WRITE setDylibPath NOTIFY dylibPathChanged)
  Q_PROPERTY(quint32 surfaceId READ surfaceId WRITE setSurfaceId NOTIFY surfaceIdChanged)
  Q_PROPERTY(quint32 nodeCount READ nodeCount NOTIFY sceneChanged)
  Q_PROPERTY(bool appLoaded READ isAppLoaded NOTIFY appLoadedChanged)
  Q_PROPERTY(quint64 sceneGeneration READ sceneGeneration NOTIFY sceneChanged)

 public:
  explicit NveQtQuickItem(QQuickItem* parent = nullptr);
  ~NveQtQuickItem() override;
  NveQtQuickItem(const NveQtQuickItem&) = delete;
  NveQtQuickItem& operator=(const NveQtQuickItem&) = delete;

  /// Registers `NveSurface` in the QML module `Nve 1.0`.
  static int registerQmlType();

  /// The runtime surface: app sources, state, props, hot patches, input dispatch, pumping.
  [[nodiscard]] QtHost& host() noexcept;

  Q_INVOKABLE bool loadDylib(const QString& path);
  /// State-preserving hot reload of the loaded (or another) `cdylib`; a failure keeps the app.
  Q_INVOKABLE bool hotReload(const QString& path = {});
  Q_INVOKABLE bool connectPaired(const QString& uri, const QString& token);
  Q_INVOKABLE void disconnectPaired();

  [[nodiscard]] QString dylibPath() const;
  void setDylibPath(const QString& path);
  [[nodiscard]] quint32 surfaceId() const noexcept;
  void setSurfaceId(quint32 id);
  [[nodiscard]] quint32 nodeCount() const;
  [[nodiscard]] bool isAppLoaded() const noexcept;
  [[nodiscard]] quint64 sceneGeneration() const noexcept;

  /// Deepest visible node under an item-local point (paint order, z-index, clips, scroll offsets,
  /// visibility); `0` when none. `interactive_only`: the nearest node taking input instead.
  [[nodiscard]] uint32_t hitTestNode(QPointF item_pos, bool interactive_only = false) const;
  /// Item-local rect of a node (scroll offsets of its ancestors applied); empty when absent.
  [[nodiscard]] QRectF nodeRect(uint32_t node_id) const;
  [[nodiscard]] std::optional<NodeSemantics> accessibilityInfoForNodeId(uint32_t node_id) const;
  /// Text shown by a text / text-input node (input: the edited value).
  [[nodiscard]] QString nodeText(uint32_t node_id) const;
  [[nodiscard]] QPointF scrollOffsetForNode(uint32_t node_id) const;
  /// Focused node (`0` when none).
  [[nodiscard]] uint32_t focusedNodeId() const noexcept;
  [[nodiscard]] SceneGraphStats sceneGraphStats() const noexcept;
  /// Retained node wrapper of `node_id` in the tree `updatePaintNode` returned (tests).
  [[nodiscard]] static const QSGNode* retainedSGNodeForNodeId(const QSGNode* root, uint32_t node_id) noexcept;

  class Impl;  // NveQtQuickInternal.hpp
  /// Null while the item is being destroyed (accessibility may query nodes during teardown).
  [[nodiscard]] Impl* impl() noexcept { return impl_.get(); }

 Q_SIGNALS:
  void dylibPathChanged();
  void surfaceIdChanged();
  void appLoadedChanged();
  void sceneChanged();

 protected:
  /// GUI thread, before each scene-graph sync: rasterizes the dirty records (text, shadows,
  /// images) so the render thread only uploads textures.
  void updatePolish() override;
  QSGNode* updatePaintNode(QSGNode* old_node, UpdatePaintNodeData* data) override;
  /// The item leaves its window: Qt deletes the node tree; forget every wrapper pointer.
  void releaseResources() override;
  void geometryChange(const QRectF& new_geometry, const QRectF& old_geometry) override;
  void itemChange(ItemChange change, const ItemChangeData& value) override;
  void mousePressEvent(QMouseEvent* event) override;
  void mouseMoveEvent(QMouseEvent* event) override;
  void mouseReleaseEvent(QMouseEvent* event) override;
  void hoverMoveEvent(QHoverEvent* event) override;
  void hoverLeaveEvent(QHoverEvent* event) override;
  void wheelEvent(QWheelEvent* event) override;
  void keyPressEvent(QKeyEvent* event) override;
  void keyReleaseEvent(QKeyEvent* event) override;
  void inputMethodEvent(QInputMethodEvent* event) override;
  void focusOutEvent(QFocusEvent* event) override;

 public:
  [[nodiscard]] QVariant inputMethodQuery(Qt::InputMethodQuery query) const override;  // public in QQuickItem

 private:
  std::unique_ptr<Impl> impl_;
};

}  // namespace nve::qt
