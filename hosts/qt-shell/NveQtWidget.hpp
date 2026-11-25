#pragma once

#include "nve_host_runtime.hpp"

#include <QPointF>
#include <QString>
#include <QWidget>

#include <cstdint>
#include <memory>
#include <string_view>

namespace nve::qt {

/// Embeddable Qt 6 (`QWidget`) host for any Native View Engine (`nve_get_interface`) app or component.
///
/// Built directly on `nve::NativeHostSession` and `nve::RetainedViewReconciler` from
/// `cxx/nve-fabric-bridge/include/nve_host_runtime.hpp`, mapping shadow nodes to retained native
/// Qt widgets (`NveQtBoxWidget`, `QLabel`, `QScrollArea`, `QLineEdit`) with `QFontMetricsF` text
/// measurement, interactive press routing, live resize relayout, and atomic state-preserving hot reload.
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

  /// Zero-copy instant-boot Frame-0 replay from an `NVE_SNAP` file before any dylib loads.
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

  /// Forward UTF-8 JSON props to `NveComponentApi::set_props`, re-render if dirty, sync Qt views,
  /// and drain outbound events.
  bool applyPropsJson(std::string_view propsJson);

  /// Apply a Tier-1 RSX hot-patch JSON payload (`NveDevApi::apply_hot_patch`) and sync Qt views.
  bool applyHotPatchJson(std::string_view patchJson);

  /// Render one frame from the loaded NVE app and reconcile retained Qt widgets.
  int32_t renderFrame();

  /// Enqueue and dispatch a press event on `nodeId` at local coordinates `(x, y)` and sync views.
  uint32_t simulatePressOnNodeId(uint32_t nodeId, float x = 0.0f, float y = 0.0f);
  uint32_t simulatePressOnNodeId(uint32_t nodeId, const QPointF& point);

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

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

} // namespace nve::qt
