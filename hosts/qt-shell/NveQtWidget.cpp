#include "NveQtWidgetInternal.hpp"

#include <QCloseEvent>
#include <QMetaObject>
#include <QPainter>
#include <QPointer>
#include <QResizeEvent>
#include <QScrollArea>
#include <QScrollBar>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <string>
#include <utility>

namespace nve::qt {

// -----------------------------------------------------------------------------
// NveQtWidget::Impl lifecycle & event dispatch
// -----------------------------------------------------------------------------

NveQtWidget::Impl::Impl(NveQtWidget* owner) : owner_(owner), adapter_{this} {
  ensureAccessibleFactoryInstalled();
  session_.set_text_measurer(NveQtWidget::makeQtTextMeasurer());
}

NveQtWidget::Impl::~Impl() {
  destroying_ = true;
  session_.disconnect_paired();
  reconciler_.clear(adapter_);
  node_semantics_.clear();
  node_resize_modes_.clear();
  node_interactive_.clear();
  session_.reset();
}

NveSurfaceConfig NveQtWidget::Impl::currentSurfaceConfig(uint32_t surface_id) const {
  NveSurfaceConfig cfg{};
  cfg.surface_id = surface_id;
  cfg.width = static_cast<float>(owner_->width() > 0 ? owner_->width() : 480);
  cfg.height = static_cast<float>(owner_->height() > 0 ? owner_->height() : 320);
  const qreal dpr = owner_->devicePixelRatioF();
  cfg.scale_factor = static_cast<float>(dpr > 0.0 ? dpr : 1.0);
  return cfg;
}

void NveQtWidget::Impl::handlePressFromWidget(uint32_t node_id, float x, float y) {
  (void)dispatchPress(node_id, x, y);
}

void NveQtWidget::Impl::handleScrollFromWidget(uint32_t node_id, QScrollArea* scroll) {
  if (in_sync_views_ || destroying_ || scroll == nullptr) {
    return;
  }
  const QScrollBar* hbar = scroll->horizontalScrollBar();
  const QScrollBar* vbar = scroll->verticalScrollBar();
  const float ox = (hbar != nullptr) ? static_cast<float>(hbar->value()) : 0.0f;
  const float oy = (vbar != nullptr) ? static_cast<float>(vbar->value()) : 0.0f;
  const QWidget* inner = scroll->widget();
  const float cw = static_cast<float>(
      (inner != nullptr) ? std::max(inner->width(), scroll->width()) : scroll->width());
  const float ch = static_cast<float>(
      (inner != nullptr) ? std::max(inner->height(), scroll->height()) : scroll->height());
  (void)dispatchScroll(node_id, ox, oy, cw, ch);
}

NveDirectEvent NveQtWidget::Impl::makeBaseEvent(
    uint32_t node_id,
    nve::EventKind kind,
    uint8_t payload_tag) const {
  NveDirectEvent ev{};
  ev.surface_id = surface_id_;
  ev.node_id = node_id;
  ev.timestamp_us = nve::current_timestamp_us();
  ev.kind = static_cast<uint8_t>(kind);
  ev.payload_tag = payload_tag;
  ev.value = nve::make_null_value();
  return ev;
}

uint32_t NveQtWidget::Impl::dispatchSingleDirectEvent(const NveDirectEvent& ev) {
  if (!session_.is_valid() && !session_.is_paired()) {
    return 0;
  }
  const uint32_t dirty =
      session_.dispatch_events(NveEventBatch{&ev, 1u, 0u}, /*auto_render=*/true);
  if (dirty != 0 && !session_.is_paired()) {
    syncViews(/*check_raw_props=*/false);
  }
  (void)session_.drain_events();
  return dirty;
}

uint32_t NveQtWidget::Impl::dispatchPress(uint32_t node_id, float x, float y) {
  if (!session_.is_valid() && !session_.is_paired()) {
    return 0;
  }
  const uint32_t dirty = session_.dispatch_press(node_id, x, y, /*auto_render=*/true);
  if (dirty != 0 && !session_.is_paired()) {
    syncViews(/*check_raw_props=*/false);
  }
  (void)session_.drain_events();
  return dirty;
}

bool NveQtWidget::Impl::connectPairedConfig(
    const nve::PairClientConfig& config,
    uint32_t surface_id,
    QString* out_error_message) {
  QPointer<NveQtWidget> weak_owner(owner_);
  auto wake_cb = [weak_owner]() {
    if (!weak_owner) {
      return;
    }
    QMetaObject::invokeMethod(
        weak_owner.data(),
        [weak_owner]() {
          if (weak_owner) {
            (void)weak_owner->pollPairedFrames();
          }
        },
        Qt::QueuedConnection);
  };
  std::string err_msg;
  const bool ok =
      session_.connect_paired(config, currentSurfaceConfig(surface_id), std::move(wake_cb), &err_msg);
  if (out_error_message != nullptr) {
    *out_error_message = QString::fromStdString(err_msg);
  }
  if (!ok) {
    return false;
  }
  surface_id_ = surface_id;
  clearViews();
  syncViews(/*check_raw_props=*/true);
  return true;
}

size_t NveQtWidget::Impl::pollPairedFramesImpl() {
  if (destroying_ || !session_.is_paired()) {
    return 0u;
  }
  const size_t applied = session_.poll_paired_frames(nullptr);
  if (applied > 0u) {
    syncViews(/*check_raw_props=*/false);
  }
  return applied;
}

uint32_t NveQtWidget::Impl::dispatchScroll(
    uint32_t node_id,
    float offset_x,
    float offset_y,
    float content_width,
    float content_height) {
  NveDirectEvent ev = makeBaseEvent(node_id, nve::EventKind::Scroll, 2u);
  ev.x = offset_x;
  ev.y = offset_y;
  ev.width = content_width;
  ev.height = content_height;
  return dispatchSingleDirectEvent(ev);
}

uint32_t NveQtWidget::Impl::dispatchTextChange(uint32_t node_id, std::string_view text) {
  NveDirectEvent ev = makeBaseEvent(node_id, nve::EventKind::TextChange, 3u);
  ev.text = nve::make_str_slice(text);
  return dispatchSingleDirectEvent(ev);
}

uint32_t NveQtWidget::Impl::dispatchSubmit(uint32_t node_id, std::string_view text) {
  NveDirectEvent ev = makeBaseEvent(node_id, nve::EventKind::Custom, 5u);
  ev.name = nve::make_str_slice("onSubmit");
  ev.text = nve::make_str_slice(text);
  ev.value = nve::make_str_value(text);
  return dispatchSingleDirectEvent(ev);
}

uint32_t NveQtWidget::Impl::dispatchFocus(uint32_t node_id, bool focused) {
  NveDirectEvent ev =
      makeBaseEvent(node_id, focused ? nve::EventKind::Focus : nve::EventKind::Blur, 0u);
  return dispatchSingleDirectEvent(ev);
}

// -----------------------------------------------------------------------------
// NveQtWidget public API
// -----------------------------------------------------------------------------

NveQtWidget::NveQtWidget(QWidget* parent) : QWidget(parent), impl_(std::make_unique<Impl>(this)) {
  setAutoFillBackground(false);
}

NveQtWidget::~NveQtWidget() = default;

nve::TextMeasurerCallback NveQtWidget::makeQtTextMeasurer() {
  return makeSharedQtTextMeasurer();
}

bool NveQtWidget::attachInterface(NveGetInterfaceFn getInterface, uint32_t surfaceId) {
  const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(surfaceId);
  if (!impl_->session_.attach_interface(getInterface, cfg)) {
    return false;
  }
  impl_->surface_id_ = surfaceId;
  impl_->clearViews();
  impl_->syncViews(/*check_raw_props=*/true);
  (void)impl_->session_.drain_events();
  return true;
}

bool NveQtWidget::loadDylib(
    const QString& path,
    uint32_t surfaceId,
    int* outErrorCode,
    QString* outErrorMessage) {
  const QByteArray utf8 = path.toUtf8();
  const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(surfaceId);
  int err_code = 0;
  std::string err_msg;
  const bool ok = impl_->session_.load_dylib(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      cfg,
      &err_code,
      &err_msg);
  if (outErrorCode != nullptr) {
    *outErrorCode = err_code;
  }
  if (outErrorMessage != nullptr) {
    *outErrorMessage = QString::fromStdString(err_msg);
  }
  if (!ok) {
    return false;
  }
  impl_->surface_id_ = surfaceId;
  impl_->clearViews();
  impl_->syncViews(/*check_raw_props=*/true);
  (void)impl_->session_.drain_events();
  return true;
}

bool NveQtWidget::reloadDylib(
    const QString& path,
    int* outErrorCode,
    QString* outErrorMessage) {
  const QByteArray utf8 = path.toUtf8();
  if (!impl_->session_.is_valid()) {
    const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(impl_->surface_id_);
    impl_->session_.set_surface_size(cfg.width, cfg.height, cfg.scale_factor);
  }
  int err_code = 0;
  std::string err_msg;
  const bool ok = impl_->session_.reload_dylib(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      &err_code,
      &err_msg);
  if (outErrorCode != nullptr) {
    *outErrorCode = err_code;
  }
  if (outErrorMessage != nullptr) {
    *outErrorMessage = QString::fromStdString(err_msg);
  }
  if (!ok) {
    return false;
  }
  impl_->clearViews();
  impl_->syncViews(/*check_raw_props=*/true);
  (void)impl_->session_.drain_events();
  return true;
}

bool NveQtWidget::connectPaired(
    const QString& endpoint,
    const QString& token,
    const QString& transport,
    uint32_t surfaceId,
    QString* outErrorMessage) {
  const QByteArray ep_utf8 = endpoint.toUtf8();
  const QByteArray tok_utf8 = token.toUtf8();
  const QByteArray tr_utf8 = transport.toUtf8();
  nve::PairClientConfig cfg{};
  std::string parse_err;
  if (!nve::parse_pair_endpoint(
          std::string_view(ep_utf8.constData(), static_cast<size_t>(ep_utf8.size())),
          std::string_view(tr_utf8.constData(), static_cast<size_t>(tr_utf8.size())),
          cfg,
          &parse_err)) {
    if (outErrorMessage != nullptr) {
      *outErrorMessage = QString::fromStdString(parse_err);
    }
    return false;
  }
  cfg.pairing_token.assign(tok_utf8.constData(), static_cast<size_t>(tok_utf8.size()));
  return connectPaired(cfg, surfaceId, outErrorMessage);
}

bool NveQtWidget::connectPaired(
    const nve::PairClientConfig& config,
    uint32_t surfaceId,
    QString* outErrorMessage) {
  return impl_->connectPairedConfig(config, surfaceId, outErrorMessage);
}

void NveQtWidget::disconnectPaired() {
  impl_->session_.disconnect_paired();
}

size_t NveQtWidget::pollPairedFrames() {
  return impl_->pollPairedFramesImpl();
}

bool NveQtWidget::isPaired() const noexcept {
  return impl_->session_.is_paired();
}

bool NveQtWidget::isPairedConnected() const noexcept {
  return impl_->session_.is_paired_connected();
}

bool NveQtWidget::applyProps(const nve::PropMap& props) {
  if (!impl_->session_.is_valid()) {
    return false;
  }
  const bool dirty = impl_->session_.update_props(props, /*auto_render=*/true);
  if (dirty) {
    impl_->syncViews(/*check_raw_props=*/true);
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

bool NveQtWidget::applyProps(const NveMapEntry* entries, uint32_t count) {
  if (!impl_->session_.is_valid()) {
    return false;
  }
  const bool dirty = impl_->session_.update_props(entries, count, /*auto_render=*/true);
  if (dirty) {
    impl_->syncViews(/*check_raw_props=*/true);
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

bool NveQtWidget::applyHotPatch(std::span<const uint8_t> patchNvp1) {
  if (!impl_->session_.is_valid() || impl_->session_.bridge() == nullptr) {
    return false;
  }
  const bool dirty = impl_->session_.bridge()->apply_hot_patch(patchNvp1, /*auto_render=*/true);
  if (dirty) {
    impl_->syncViews(/*check_raw_props=*/true);
  }
  (void)impl_->session_.drain_events();
  return dirty;
}

int32_t NveQtWidget::renderFrame() {
  const int32_t rc = impl_->session_.render(nullptr);
  if (rc == 0) {
    impl_->syncViews(/*check_raw_props=*/false);
  }
  (void)impl_->session_.drain_events();
  return rc;
}

uint32_t NveQtWidget::simulatePressOnNodeId(uint32_t nodeId, float x, float y) {
  return impl_->dispatchPress(nodeId, x, y);
}

uint32_t NveQtWidget::simulatePressOnNodeId(uint32_t nodeId, const QPointF& point) {
  return impl_->dispatchPress(
      nodeId, static_cast<float>(point.x()), static_cast<float>(point.y()));
}

uint32_t NveQtWidget::dispatchTextChangeOnNodeId(uint32_t nodeId, std::string_view text) {
  return impl_->dispatchTextChange(nodeId, text);
}

uint32_t NveQtWidget::dispatchSubmitOnNodeId(uint32_t nodeId, std::string_view text) {
  return impl_->dispatchSubmit(nodeId, text);
}

uint32_t NveQtWidget::dispatchFocusOnNodeId(uint32_t nodeId, bool focused) {
  return impl_->dispatchFocus(nodeId, focused);
}

uint32_t NveQtWidget::dispatchScrollOnNodeId(
    uint32_t nodeId,
    float offsetX,
    float offsetY,
    float contentWidth,
    float contentHeight) {
  return impl_->dispatchScroll(nodeId, offsetX, offsetY, contentWidth, contentHeight);
}

uint64_t NveQtWidget::reconcileUpdateCount() const noexcept {
  return impl_->reconcile_update_count_;
}

std::optional<NodeSemantics> NveQtWidget::accessibilityInfoForNodeId(uint32_t nodeId) const {
  if (const NodeSemantics* sem = impl_->findSemantics(nodeId)) {
    return *sem;
  }
  return std::nullopt;
}

QAccessible::Role NveQtWidget::accessibleRoleForNodeId(uint32_t nodeId) const {
  if (const NodeSemantics* sem = impl_->findSemantics(nodeId)) {
    return sem->accessibleRole;
  }
  return QAccessible::NoRole;
}

QWidget* NveQtWidget::nativeWidgetForNodeId(uint32_t nodeId) const {
  if (nodeId == NVE_ROOT_NODE_ID) {
    return const_cast<NveQtWidget*>(this);
  }
  QWidget* const* slot = impl_->reconciler_.find(nodeId);
  return (slot != nullptr) ? *slot : nullptr;
}

uint32_t NveQtWidget::nodeIdForNativeWidget(const QWidget* widget) const {
  if (widget == nullptr) {
    return 0;
  }
  if (widget == this) {
    return NVE_ROOT_NODE_ID;
  }
  const QVariant v = widget->property(kNodeIdProperty);
  if (v.isValid()) {
    return static_cast<uint32_t>(v.toULongLong());
  }
  return 0;
}

uint32_t NveQtWidget::nodeCount() const {
  return impl_->session_.node_count();
}

uint32_t NveQtWidget::surfaceId() const noexcept {
  return impl_->surface_id_;
}

bool NveQtWidget::isAppLoaded() const noexcept {
  return impl_->session_.is_valid() || impl_->session_.is_paired();
}

void NveQtWidget::setOnNveEvent(nve::EventCallback callback) {
  impl_->session_.set_event_callback(std::move(callback));
}

void NveQtWidget::setTextMeasurer(nve::TextMeasurerCallback measurer) {
  impl_->session_.set_text_measurer(std::move(measurer));
  if (impl_->session_.active_fabric_host() != nullptr) {
    const NveSurfaceConfig cfg = impl_->currentSurfaceConfig(impl_->surface_id_);
    impl_->session_.set_surface_size(cfg.width, cfg.height, cfg.scale_factor);
    impl_->syncViews(/*check_raw_props=*/false);
  }
}

nve::NativeHostSession& NveQtWidget::session() noexcept {
  return impl_->session_;
}

const nve::NativeHostSession& NveQtWidget::session() const noexcept {
  return impl_->session_;
}

QSize NveQtWidget::sizeHint() const {
  if (NveFabricHostHandle host = impl_->session_.active_fabric_host()) {
    nve::LayoutRect root_rect{};
    if (nve::fabric_host_get_node_layout(host, NVE_ROOT_NODE_ID, &root_rect) &&
        root_rect.width > 0.0f && root_rect.height > 0.0f) {
      return QSize(
          static_cast<int>(std::lround(root_rect.width)),
          static_cast<int>(std::lround(root_rect.height)));
    }
  }
  return QSize(640, 420);
}

void NveQtWidget::resizeEvent(QResizeEvent* event) {
  QWidget::resizeEvent(event);
  const QSize sz = event->size();
  if (sz.width() > 0 && sz.height() > 0) {
    const qreal dpr = devicePixelRatioF();
    impl_->session_.set_surface_size(
        static_cast<float>(sz.width()),
        static_cast<float>(sz.height()),
        static_cast<float>(dpr > 0.0 ? dpr : 1.0));
    if (impl_->session_.active_fabric_host() != nullptr) {
      impl_->syncViews(/*check_raw_props=*/false);
    }
  }
}

void NveQtWidget::paintEvent(QPaintEvent* /*event*/) {
  if (impl_->root_bg_color_.alpha() > 0) {
    QPainter painter(this);
    painter.fillRect(rect(), impl_->root_bg_color_);
  }
}

} // namespace nve::qt
