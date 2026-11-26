#pragma once

#include "NveQtCommon.hpp"
#include "nve_host_runtime.hpp"

#include <QPointF>
#include <QString>
#include <QWidget>

#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <span>
#include <string_view>
#include <vector>

namespace nve::qt {

/// Native `QWidget` for `NVE_NODE_KIND_SKIA_CANVAS` caching raw `NVE_DRAW1` bytes from
/// `NveSceneView::text_slab`, repainting only when bytes or visual style change, and painting
/// via the shared `renderDrawListToPainter` helper.
class NveQtCanvasWidget final : public QWidget {
  Q_OBJECT

 public:
  using PressHandler = std::function<void(uint32_t, float, float)>;

  explicit NveQtCanvasWidget(
      uint32_t nodeId,
      PressHandler onPress = {},
      QWidget* parent = nullptr);
  ~NveQtCanvasWidget() override = default;

  void applyNode(
      const NveRenderNode& node,
      std::span<const uint8_t> drawListBytes,
      bool interactive);

  bool setDrawListBytes(std::span<const uint8_t> bytes);
  void setInteractive(bool interactive);

  std::span<const uint8_t> drawListBytes() const noexcept { return draw_list_bytes_; }
  uint64_t redrawRequestCount() const noexcept { return redraw_request_count_; }

 protected:
  void paintEvent(QPaintEvent* event) override;
  void mousePressEvent(QMouseEvent* event) override;
  void mouseReleaseEvent(QMouseEvent* event) override;

 private:
  uint32_t node_id_{0};
  PressHandler on_press_;
  QColor bg_color_{0, 0, 0, 0};
  QColor border_color_{0, 0, 0, 0};
  float border_width_{0.0f};
  float border_radius_{0.0f};
  float opacity_{1.0f};
  bool interactive_{false};
  bool pressed_{false};
  std::vector<uint8_t> draw_list_bytes_;
  uint64_t redraw_request_count_{0};
};

/// Embeddable Qt 6 (`QWidget`) host for any Native View Engine (`nve_get_interface`) app or component.
///
/// Built directly on `nve::NativeHostSession` and `nve::IncrementalSceneReconciler` from
/// `cxx/nve-fabric-bridge/include/nve_host_runtime.hpp`, mapping flat 64-byte `NveSceneView`
/// nodes in O(changed) time to retained native Qt widgets (`NveQtBoxWidget`, `NveQtImageWidget`,
/// `QLabel`, `QScrollArea`, `QLineEdit`) with `QFontMetricsF` text measurement, interactive press
/// & scroll routing, live resize relayout, and atomic state-preserving hot reload.
class NveQtWidget : public QWidget {
  Q_OBJECT

 public:
  explicit NveQtWidget(QWidget* parent = nullptr);
  ~NveQtWidget() override;

  NveQtWidget(const NveQtWidget&) = delete;
  NveQtWidget& operator=(const NveQtWidget&) = delete;

  /// Attach an in-process `nve_get_interface` entry point directly (no dynamic library owned).
  bool attachInterface(NveGetInterfaceFn getInterface, uint32_t surfaceId = 1);

  /// Load an NVE dynamic library (`cdylib` / `.dylib` / `.so` / `.dll`), mount its surface, and
  /// synchronize retained Qt widgets.
  ///
  /// On failure returns `false` and sets optional `outErrorCode`:
  /// - `1`: empty path or OS library open failed
  /// - `2`: missing `nve_get_interface` symbol
  /// - `3`: missing `NVE_IFACE_SURFACE` or surface creation failed
  bool loadDylib(
      const QString& path,
      uint32_t surfaceId = 1,
      int* outErrorCode = nullptr,
      QString* outErrorMessage = nullptr);

  /// Atomically hot-reload a replacement NVE dynamic library while preserving app state via
  /// `NveDevApi::snapshot_state` / `restore_state`. Failed reloads leave the live app intact.
  bool reloadDylib(
      const QString& path,
      int* outErrorCode = nullptr,
      QString* outErrorMessage = nullptr);

  /// Connect in paired thin-client mode (zero local cdylib required). Incoming `NVD1` frames wake
  /// the Qt GUI thread via `QMetaObject::invokeMethod(..., Qt::QueuedConnection)`; UI input events
  /// stream upstream as `WireKind::EventBatch`.
  bool connectPaired(
      const QString& endpoint,
      const QString& token,
      const QString& transport = QString(),
      uint32_t surfaceId = 1,
      QString* outErrorMessage = nullptr);
  bool connectPaired(
      const nve::PairClientConfig& config,
      uint32_t surfaceId = 1,
      QString* outErrorMessage = nullptr);
  void disconnectPaired();
  size_t pollPairedFrames();
  bool isPaired() const noexcept;
  bool isPairedConnected() const noexcept;

  /// Zero-copy instant-boot Frame-0 replay from an `NVERELOC` file before any dylib loads.
  /// Immediately reconciles retained Qt widgets from the mapped Frame-0 buffer and caches the
  /// snapshot state for seamless restoration on the subsequent `loadDylib()` / `attachInterface()`.
  bool replayFrame0Snapshot(
      const QString& snapshotPath,
      uint32_t expectedSchemaVersion = 1,
      uint32_t surfaceId = 1,
      uint64_t* outRestoreUs = nullptr);

  /// Freeze-dries the live shadow tree into a complete Frame-0 `CommandBuffer` + `snapshot_state()`
  /// and writes it atomically to `snapshotPath`.
  bool saveSnapshot(const QString& snapshotPath, uint32_t schemaVersion = 1) const;

  /// Configures automatic atomic snapshot persistence on `closeEvent`. Pass empty path to disable.
  void setAutoSnapshotPath(const QString& snapshotPath, uint32_t schemaVersion = 1);

  /// Forward structured typed props to `NveComponentApi::set_props`, re-render if dirty,
  /// sync Qt views, and drain outbound events.
  bool applyProps(const nve::PropMap& props);
  bool applyProps(const NveMapEntry* entries, uint32_t count);

  /// Apply one binary `NVP1` Tier-1 hot-patch frame (`NveDevApi::apply_hot_patch`) and sync Qt views.
  bool applyHotPatch(std::span<const uint8_t> patchNvp1);

  /// Render one frame from the loaded NVE app and reconcile retained Qt widgets.
  int32_t renderFrame();

  /// Enqueue and dispatch a press event on `nodeId` at local coordinates `(x, y)` and sync views.
  uint32_t simulatePressOnNodeId(uint32_t nodeId, float x = 0.0f, float y = 0.0f);
  uint32_t simulatePressOnNodeId(uint32_t nodeId, const QPointF& point);

  /// Dispatch a UTF-8 `TextChange`, Return-key `onSubmit`, `Focus`/`Blur`, or `Scroll` event for `nodeId`.
  uint32_t dispatchTextChangeOnNodeId(uint32_t nodeId, std::string_view text);
  uint32_t dispatchSubmitOnNodeId(uint32_t nodeId, std::string_view text);
  uint32_t dispatchFocusOnNodeId(uint32_t nodeId, bool focused);
  uint32_t dispatchScrollOnNodeId(
      uint32_t nodeId,
      float offsetX,
      float offsetY,
      float contentWidth,
      float contentHeight);

  /// Total incremental widget update passes applied via `IncrementalSceneReconciler`.
  uint64_t reconcileUpdateCount() const noexcept;

  /// Query extracted per-node accessibility & TextInput metadata for `nodeId`.
  std::optional<NodeSemantics> accessibilityInfoForNodeId(uint32_t nodeId) const;
  QAccessible::Role accessibleRoleForNodeId(uint32_t nodeId) const;

  /// Lookup the retained `QWidget*` mounted for `nodeId` (`NVE_ROOT_NODE_ID` returns `this`).
  QWidget* nativeWidgetForNodeId(uint32_t nodeId) const;

  /// Lookup the NVE shadow node ID for a mounted native `QWidget*` (`0` if unknown).
  uint32_t nodeIdForNativeWidget(const QWidget* widget) const;

  /// Total shadow node count (including root) in the active host, or `0` if unloaded.
  uint32_t nodeCount() const;

  /// Active surface ID.
  uint32_t surfaceId() const noexcept;

  /// True when an NVE app is currently loaded and valid.
  bool isAppLoaded() const noexcept;

  /// Register a callback invoked for each outbound `nve::DecodedEvent` drained from the app.
  void setOnNveEvent(nve::EventCallback callback);

  /// Override the native text measurer (defaults to `makeQtTextMeasurer()`).
  void setTextMeasurer(nve::TextMeasurerCallback measurer);

  /// Build the default `QFontMetricsF` text measurer matching `NveQtWidget`'s `QLabel` font sizing.
  static nve::TextMeasurerCallback makeQtTextMeasurer();

  /// Access the underlying shared `nve::NativeHostSession`.
  nve::NativeHostSession& session() noexcept;
  const nve::NativeHostSession& session() const noexcept;

  QSize sizeHint() const override;

 protected:
  void resizeEvent(QResizeEvent* event) override;
  void paintEvent(QPaintEvent* event) override;
  void closeEvent(QCloseEvent* event) override;

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

} // namespace nve::qt
