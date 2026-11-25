#pragma once

#include "NveQtCommon.hpp"
#include "nve_host_runtime.hpp"

#include <QPointF>
#include <QQuickItem>
#include <QRectF>
#include <QString>
#include <QtQml/qqmlregistration.h>

#include <cstdint>
#include <span>
#include <memory>
#include <optional>
#include <string_view>

class QSGNode;

namespace nve::qt {

/// Telemetry snapshot describing the most recent Qt Scene Graph synchronization pass.
struct SceneGraphStats {
  uint64_t syncCount{0};
  uint64_t sceneGeneration{0};
  uint32_t totalSGNodes{0};
  uint32_t geometryNodes{0};
  uint32_t clipNodes{0};
  uint32_t opacityNodes{0};
  uint32_t textTextureNodes{0};
  uint32_t imageTextureNodes{0};
  uint32_t canvasTextureNodes{0};
  uint64_t textRasterizations{0};
  uint64_t textCacheHits{0};
  uint64_t canvasRasterizations{0};
  uint64_t canvasCacheHits{0};
  uint32_t cachedTextTextures{0};
  uint32_t cachedImageTextures{0};
  uint32_t cachedCanvasTextures{0};
  uint64_t dirtyNodesUpdated{0};
  uint64_t retainedNodesReused{0};
};

/// Hardware-accelerated Qt Quick (`QQuickItem` / Scene Graph `QSGNode`) host for any Native View
/// Engine (`nve_get_interface`) app or component.
///
/// Reads `NativeHostSession::scene_view()` directly without snapshot materialization or
/// `SceneTableBuilder`, maintaining a truly retained incremental `QSGNode` tree keyed by
/// `node_id` so unchanged sibling `QSGNode*` subtrees survive across dirty frames.
class NveQtQuickItem : public QQuickItem {
  Q_OBJECT
  QML_ELEMENT

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

  /// Register `NveQtQuickItem` as `NveSurface` under QML module `"Nve" 1.0`.
  static int registerQmlType();

  /// Attach an in-process `nve_get_interface` entry point directly (no dynamic library owned).
  bool attachInterface(NveGetInterfaceFn getInterface, uint32_t surfaceId = 1);

  /// Load an NVE dynamic library (`cdylib` / `.dylib` / `.so` / `.dll`), mount its surface,
  /// synchronize the flat `NveSceneView`, and schedule a Scene Graph update.
  Q_INVOKABLE bool loadDylib(
      const QString& path,
      uint32_t surfaceId = 1,
      int* outErrorCode = nullptr,
      QString* outErrorMessage = nullptr);

  /// Atomically hot-reload a replacement NVE dynamic library while preserving app state via
  /// `NveDevApi::snapshot_state` / `restore_state`. Failed reloads leave the live app intact.
  Q_INVOKABLE bool hotReload(
      const QString& path,
      int* outErrorCode = nullptr,
      QString* outErrorMessage = nullptr);

  /// Alias for `hotReload` matching `NveQtWidget::reloadDylib`.
  bool reloadDylib(
      const QString& path,
      int* outErrorCode = nullptr,
      QString* outErrorMessage = nullptr) {
    return hotReload(path, outErrorCode, outErrorMessage);
  }

  /// Connect in paired thin-client mode (zero local cdylib required). Incoming `NVD1` frames wake
  /// the Qt GUI thread via `QMetaObject::invokeMethod(..., Qt::QueuedConnection)`; UI input events
  /// stream upstream as `TransportMsgKind::InputEvents = 11`.
  Q_INVOKABLE bool connectPaired(
      const QString& endpoint,
      const QString& token,
      const QString& transport = QString(),
      uint32_t surfaceId = 1,
      QString* outErrorMessage = nullptr);
  bool connectPaired(
      const nve::PairClientConfig& config,
      uint32_t surfaceId = 1,
      QString* outErrorMessage = nullptr);
  Q_INVOKABLE void disconnectPaired();
  size_t pollPairedFrames();
  bool isPaired() const noexcept;
  bool isPairedConnected() const noexcept;

  /// Zero-copy instant-boot Frame-0 replay from an `NVE_SNAP` file before any dylib loads.
  Q_INVOKABLE bool replaySnapshot(
      const QString& snapshotPath,
      uint32_t expectedSchemaVersion = 1,
      uint32_t surfaceId = 1,
      uint64_t* outRestoreUs = nullptr);

  /// Alias for `replaySnapshot` matching `NveQtWidget::replayFrame0Snapshot`.
  bool replayFrame0Snapshot(
      const QString& snapshotPath,
      uint32_t expectedSchemaVersion = 1,
      uint32_t surfaceId = 1,
      uint64_t* outRestoreUs = nullptr) {
    return replaySnapshot(snapshotPath, expectedSchemaVersion, surfaceId, outRestoreUs);
  }

  /// Freeze-dries the live shadow tree into a complete Frame-0 `CommandBuffer` + `snapshot_state()`
  /// and writes it atomically to `snapshotPath`.
  Q_INVOKABLE bool saveSnapshot(const QString& snapshotPath, uint32_t schemaVersion = 1) const;

  /// Forward UTF-8 JSON props to `NveComponentApi::set_props`, re-render if dirty, sync scene,
  /// and drain outbound events.
  bool applyPropsJson(std::string_view propsJson);

  /// Forward structured zero-copy props to `NveComponentApi::set_props_direct`.
  bool applyPropsDirect(const NveMapEntry* entries, uint32_t count);

  /// Apply one binary `NVP1` Tier-1 hot-patch frame (`NveDevApi::apply_hot_patch`) and sync scene.
  bool applyHotPatch(std::span<const uint8_t> patchNvp1);

  /// Render one frame from the loaded NVE app and synchronize the flat `NveSceneView`.
  Q_INVOKABLE int32_t renderFrame();

  /// Dispatch zero-copy `NveDirectEvent`s into `NativeHostSession`.
  uint32_t dispatchDirectEvents(NveEventBatch batch);
  uint32_t dispatchPress(uint32_t nodeId, float x = 0.0f, float y = 0.0f);
  uint32_t dispatchPress(uint32_t nodeId, const QPointF& localPoint);
  uint32_t dispatchScroll(
      uint32_t nodeId,
      float offsetX,
      float offsetY,
      float contentWidth,
      float contentHeight);
  uint32_t dispatchTextChange(uint32_t nodeId, std::string_view text);
  uint32_t dispatchSubmit(uint32_t nodeId, std::string_view text);
  uint32_t dispatchFocus(uint32_t nodeId, bool focused);

  /// Per-node accessibility & TextInput metadata queries.
  std::optional<NodeSemantics> accessibilityInfoForNodeId(uint32_t nodeId) const;
  QAccessible::Role accessibleRoleForNodeId(uint32_t nodeId) const;
  QString accessibleNameForNodeId(uint32_t nodeId) const;
  QString accessibleDescriptionForNodeId(uint32_t nodeId) const;
  QString imePreeditText(uint32_t nodeId) const;
  QVariant inputMethodQuery(Qt::InputMethodQuery query) const override;

  /// Front-to-back hit-test against the active `NveSceneView`, honoring `z_index`, clip bounds,
  /// and scroll offsets. Returns `0` if no node matches.
  uint32_t hitTestNode(const QPointF& itemPos, bool interactiveOnly = false) const;

  /// Locate the deepest `NVE_NODE_KIND_SCROLL_VIEW` enclosing `itemPos`, or `0` if none.
  uint32_t hitTestScrollableNode(const QPointF& itemPos) const;

  /// Compute a node's resolved bounding rectangle in item-local coordinates.
  QRectF sceneNodeGlobalBounds(uint32_t nodeId) const;

  /// Zero-copy flat `NveSceneView` directly from `NativeHostSession::scene_view()`.
  NveSceneView sceneView() const noexcept;
  std::optional<NveRenderNode> sceneNode(uint32_t nodeId) const;
  std::string_view sceneNodeText(uint32_t nodeId) const;

  /// Scroll offset state for `NVE_NODE_KIND_SCROLL_VIEW` nodes.
  QPointF scrollOffsetForNode(uint32_t nodeId) const;
  void setScrollOffsetForNode(uint32_t nodeId, const QPointF& offset);

  /// Currently focused `NVE_NODE_KIND_TEXT_INPUT` node ID (`0` when unfocused).
  uint32_t focusedNodeId() const noexcept;

  /// Most recent Scene Graph synchronization statistics.
  SceneGraphStats sceneGraphStats() const noexcept;

  /// Build/incrementally update the retained Scene Graph node tree.
  QSGNode* syncSceneGraphNode(QSGNode* oldNode);

  /// Inspect the retained wrapper `QSGNode*` for `nodeId` inside `root` (for verification/tests).
  static const QSGNode* retainedSGNodeForNodeId(const QSGNode* root, uint32_t nodeId) noexcept;

  QString dylibPath() const;
  void setDylibPath(const QString& path);

  uint32_t surfaceId() const noexcept;
  void setSurfaceId(uint32_t id);

  uint32_t nodeCount() const;
  bool isAppLoaded() const noexcept;
  uint64_t sceneGeneration() const noexcept;

  void setOnNveEvent(nve::EventCallback callback);
  void setTextMeasurer(nve::TextMeasurerCallback measurer);
  static nve::TextMeasurerCallback makeQtTextMeasurer();

  nve::NativeHostSession& session() noexcept;
  const nve::NativeHostSession& session() const noexcept;

 Q_SIGNALS:
  void dylibPathChanged();
  void surfaceIdChanged();
  void appLoadedChanged();
  void sceneChanged();

 protected:
  QSGNode* updatePaintNode(QSGNode* oldNode, UpdatePaintNodeData* updatePaintNodeData) override;
  void geometryChange(const QRectF& newGeometry, const QRectF& oldGeometry) override;
  void mousePressEvent(QMouseEvent* event) override;
  void mouseMoveEvent(QMouseEvent* event) override;
  void mouseReleaseEvent(QMouseEvent* event) override;
  void wheelEvent(QWheelEvent* event) override;
  void keyPressEvent(QKeyEvent* event) override;
  void inputMethodEvent(QInputMethodEvent* event) override;

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

} // namespace nve::qt

namespace nve {
using qt::NveQtQuickItem;
using qt::SceneGraphStats;
} // namespace nve
