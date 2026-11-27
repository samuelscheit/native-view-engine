#include "NveQtQuickItemInternal.hpp"

#include <QMetaObject>
#include <QPointer>
#include <QQmlEngine>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <string>
#include <utility>

namespace nve::qt {

NveQtQuickItem::NveQtQuickItem(QQuickItem* parent)
    : QQuickItem(parent), impl_(std::make_unique<Impl>(this)) {
  setFlag(ItemHasContents, true);
  setFlag(ItemAcceptsInputMethod, true);
  setAcceptedMouseButtons(Qt::LeftButton | Qt::RightButton);
  setAcceptHoverEvents(true);
  setImplicitWidth(640.0);
  setImplicitHeight(420.0);
}

NveQtQuickItem::~NveQtQuickItem() = default;
int NveQtQuickItem::registerQmlType() { return qmlRegisterType<NveQtQuickItem>("Nve", 1, 0, "NveSurface"); }
nve::TextMeasurerCallback NveQtQuickItem::makeQtTextMeasurer() { return makeSharedQtTextMeasurer(); }

bool NveQtQuickItem::attachInterface(NveGetInterfaceFn getInterface, uint32_t surfaceId) {
  const bool was_loaded = impl_->session_.is_valid();
  if (!impl_->session_.attach_interface(getInterface, impl_->currentSurfaceConfig(surfaceId))) return false;
  impl_->finishSessionTransition(surfaceId, !was_loaded, nullptr, true);
  return true;
}

bool NveQtQuickItem::loadDylib(
    const QString& path, uint32_t surfaceId, int* outErrorCode, QString* outErrorMessage) {
  const bool was_loaded = impl_->session_.is_valid();
  const QByteArray utf8 = path.toUtf8();
  int err_code = 0;
  std::string err_msg;
  const bool ok = impl_->session_.load_dylib(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())),
      impl_->currentSurfaceConfig(surfaceId), &err_code, &err_msg);
  if (outErrorCode) *outErrorCode = err_code;
  if (outErrorMessage) *outErrorMessage = QString::fromStdString(err_msg);
  if (!ok) return false;
  impl_->finishSessionTransition(surfaceId, !was_loaded, &path, true);
  return true;
}

bool NveQtQuickItem::hotReload(const QString& path, int* outErrorCode, QString* outErrorMessage) {
  const QByteArray utf8 = path.toUtf8();
  if (!impl_->session_.is_valid()) {
    const auto cfg = impl_->currentSurfaceConfig(impl_->surface_id_);
    impl_->session_.set_surface_size(cfg.width, cfg.height, cfg.scale_factor);
  }
  int err_code = 0;
  std::string err_msg;
  const bool ok = impl_->session_.reload_dylib(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())), &err_code, &err_msg);
  if (outErrorCode) *outErrorCode = err_code;
  if (outErrorMessage) *outErrorMessage = QString::fromStdString(err_msg);
  if (!ok) return false;
  impl_->finishSessionTransition(impl_->surface_id_, false, &path, true);
  return true;
}

bool NveQtQuickItem::connectPaired(
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

bool NveQtQuickItem::connectPaired(
    const nve::PairClientConfig& config,
    uint32_t surfaceId,
    QString* outErrorMessage) {
  const bool was_loaded = isAppLoaded();
  QPointer<NveQtQuickItem> weak_self(this);
  auto wake_cb = [weak_self]() {
    if (!weak_self) return;
    QMetaObject::invokeMethod(
        weak_self.data(),
        [weak_self]() {
          if (weak_self) {
            (void)weak_self->pollPairedFrames();
          }
        },
        Qt::QueuedConnection);
  };
  std::string err_msg;
  const bool ok = impl_->session_.connect_paired(
      config, impl_->currentSurfaceConfig(surfaceId), std::move(wake_cb), &err_msg);
  if (outErrorMessage != nullptr) {
    *outErrorMessage = QString::fromStdString(err_msg);
  }
  if (!ok) return false;
  impl_->finishSessionTransition(surfaceId, !was_loaded, nullptr, false);
  return true;
}

void NveQtQuickItem::disconnectPaired() {
  const bool was_loaded = isAppLoaded();
  impl_->session_.disconnect_paired();
  if (was_loaded && !isAppLoaded()) {
    Q_EMIT appLoadedChanged();
  }
}

size_t NveQtQuickItem::pollPairedFrames() {
  if (!impl_->session_.is_paired()) return 0u;
  const size_t applied = impl_->session_.poll_paired_frames(nullptr);
  if (applied > 0u) {
    impl_->syncScene(false);
  }
  return applied;
}

bool NveQtQuickItem::isPaired() const noexcept { return impl_->session_.is_paired(); }
bool NveQtQuickItem::isPairedConnected() const noexcept { return impl_->session_.is_paired_connected(); }

bool NveQtQuickItem::applyProps(const nve::PropMap& props) {
  return impl_->session_.is_valid() && impl_->finishPropUpdate(impl_->session_.update_props(props, true));
}
bool NveQtQuickItem::applyProps(const NveMapEntry* entries, uint32_t count) {
  return impl_->session_.is_valid() && impl_->finishPropUpdate(impl_->session_.update_props(entries, count, true));
}
bool NveQtQuickItem::applyHotPatch(std::span<const uint8_t> patchNvp1) {
  return impl_->session_.is_valid() && impl_->session_.bridge() != nullptr &&
         impl_->finishPropUpdate(impl_->session_.bridge()->apply_hot_patch(patchNvp1, true));
}

int32_t NveQtQuickItem::renderFrame() {
  const int32_t rc = impl_->session_.render(nullptr);
  if (rc == 0) impl_->syncScene(false);
  (void)impl_->session_.drain_events();
  return rc;
}

uint32_t NveQtQuickItem::dispatchDirectEvents(NveEventBatch batch) { return impl_->dispatchBatch(batch); }
uint32_t NveQtQuickItem::dispatchPress(uint32_t nodeId, float x, float y) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, nve::EventKind::Press, 1u);
  ev.x = x; ev.y = y;
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u, 0u});
}
uint32_t NveQtQuickItem::dispatchPress(uint32_t nodeId, const QPointF& p) {
  return dispatchPress(nodeId, static_cast<float>(p.x()), static_cast<float>(p.y()));
}
uint32_t NveQtQuickItem::dispatchScroll(uint32_t nodeId, float ox, float oy, float cw, float ch) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, nve::EventKind::Scroll, 2u);
  ev.x = ox; ev.y = oy; ev.width = cw; ev.height = ch;
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u, 0u});
}
uint32_t NveQtQuickItem::dispatchTextChange(uint32_t nodeId, std::string_view text) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, nve::EventKind::TextChange, 3u);
  ev.text = nve::make_str_slice(text);
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u, 0u});
}
uint32_t NveQtQuickItem::dispatchSubmit(uint32_t nodeId, std::string_view text) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, nve::EventKind::Custom, 5u);
  ev.name = nve::make_str_slice("onSubmit");
  ev.text = nve::make_str_slice(text);
  ev.value = nve::make_str_value(text);
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u, 0u});
}
uint32_t NveQtQuickItem::dispatchFocus(uint32_t nodeId, bool focused) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, focused ? nve::EventKind::Focus : nve::EventKind::Blur, 0u);
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u, 0u});
}

std::optional<NodeSemantics> NveQtQuickItem::accessibilityInfoForNodeId(uint32_t nodeId) const {
  if (const NodeSemantics* s = impl_->findSemantics(nodeId)) return *s;
  return std::nullopt;
}
QAccessible::Role NveQtQuickItem::accessibleRoleForNodeId(uint32_t nodeId) const {
  const NodeSemantics* s = impl_->findSemantics(nodeId);
  return s ? s->accessibleRole : QAccessible::NoRole;
}
QString NveQtQuickItem::accessibleNameForNodeId(uint32_t nodeId) const {
  const NodeSemantics* s = impl_->findSemantics(nodeId);
  return s ? s->label : QString();
}
QString NveQtQuickItem::accessibleDescriptionForNodeId(uint32_t nodeId) const {
  const NodeSemantics* s = impl_->findSemantics(nodeId);
  return s ? s->hint : QString();
}
QString NveQtQuickItem::imePreeditText(uint32_t nodeId) const {
  const auto it = impl_->ime_preedit_buffers_.find(nodeId);
  return (it != impl_->ime_preedit_buffers_.end()) ? it->second : QString();
}

uint32_t NveQtQuickItem::hitTestNode(const QPointF& itemPos, bool interactiveOnly) const {
  return hitTestSceneView(
      impl_->session_.scene_view(), impl_->id_to_index_, impl_->node_semantics_,
      impl_->scroll_offsets_, width(), height(), itemPos, interactiveOnly, false);
}
uint32_t NveQtQuickItem::hitTestScrollableNode(const QPointF& itemPos) const {
  return hitTestSceneView(
      impl_->session_.scene_view(), impl_->id_to_index_, impl_->node_semantics_,
      impl_->scroll_offsets_, width(), height(), itemPos, false, true);
}
QRectF NveQtQuickItem::sceneNodeGlobalBounds(uint32_t nodeId) const {
  return findSceneNodeGlobalBounds(
      impl_->session_.scene_view(), impl_->id_to_index_, impl_->scroll_offsets_, nodeId);
}

NveSceneView NveQtQuickItem::sceneView() const noexcept { return impl_->session_.scene_view(); }
std::optional<NveRenderNode> NveQtQuickItem::sceneNode(uint32_t nodeId) const {
  if (const NveRenderNode* n = impl_->findNode(impl_->session_.scene_view(), nodeId)) return *n;
  return std::nullopt;
}
std::string_view NveQtQuickItem::sceneNodeText(uint32_t nodeId) const {
  const NveSceneView view = impl_->session_.scene_view();
  if (const NveRenderNode* n = impl_->findNode(view, nodeId)) {
    if (n->kind == NVE_NODE_KIND_TEXT_INPUT) {
      if (auto it = impl_->text_input_buffers_.find(nodeId); it != impl_->text_input_buffers_.end()) {
        return it->second;
      }
    }
    return nve::scene_node_text(view, *n);
  }
  return {};
}

QPointF NveQtQuickItem::scrollOffsetForNode(uint32_t nodeId) const { return impl_->scrollOffsetFor(nodeId); }
void NveQtQuickItem::setScrollOffsetForNode(uint32_t nodeId, const QPointF& offset) {
  const NveSceneView view = impl_->session_.scene_view();
  const NveRenderNode* node = impl_->findNode(view, nodeId);
  if (node == nullptr) return;
  const QSizeF content = computeScrollContentSize(view, impl_->id_to_index_, *node);
  const QPointF clamped(
      std::clamp(offset.x(), 0.0, std::max<qreal>(0.0, content.width() - node->width)),
      std::clamp(offset.y(), 0.0, std::max<qreal>(0.0, content.height() - node->height)));
  if (impl_->scrollOffsetFor(nodeId) != clamped) {
    impl_->scroll_offsets_[nodeId] = clamped;
    impl_->markLocalDirty(nodeId);
  }
}

uint32_t NveQtQuickItem::focusedNodeId() const noexcept { return impl_->focused_node_id_; }
SceneGraphStats NveQtQuickItem::sceneGraphStats() const noexcept { return impl_->sg_stats_; }

QSGNode* NveQtQuickItem::syncSceneGraphNode(QSGNode* oldNode) {
  const NveSceneView view = impl_->session_.scene_view();
  if (view.node_count > 0u && impl_->id_to_index_.size() != view.node_count) {
    impl_->rebuildIndex(view);
  }
  const qreal dpr = impl_->effectiveDpr();
  SGBuildContext ctx{
      .view = view,
      .id_to_index = &impl_->id_to_index_,
      .node_semantics = &impl_->node_semantics_,
      .image_resize_modes = &impl_->image_resize_modes_,
      .scroll_offsets = &impl_->scroll_offsets_,
      .text_input_buffers = &impl_->text_input_buffers_,
      .ime_preedit_buffers = &impl_->ime_preedit_buffers_,
      .window = window(),
      .item_width = width(),
      .item_height = height(),
      .dpr = dpr,
      .dpr_centis = std::max(1, static_cast<int>(std::lround(dpr * 100.0))),
      .scene_generation = impl_->scene_generation_,
      .focused_node_id = impl_->focused_node_id_,
      .full_resync = impl_->pending_full_sg_sync_,
      .dirty_ids = &impl_->pending_dirty_ids_,
      .removed_ids = &impl_->pending_removed_ids_,
  };
  QSGNode* result = syncRetainedSceneGraph(oldNode, ctx, impl_->sg_stats_);
  impl_->pending_full_sg_sync_ = false;
  impl_->pending_dirty_ids_.clear();
  impl_->pending_removed_ids_.clear();
  return result;
}

const QSGNode* NveQtQuickItem::retainedSGNodeForNodeId(const QSGNode* root, uint32_t nodeId) noexcept {
  return root ? static_cast<const NveSGRootNode*>(root)->findRetainedWrapper(nodeId) : nullptr;
}

QString NveQtQuickItem::dylibPath() const { return impl_->dylib_path_; }
void NveQtQuickItem::setDylibPath(const QString& path) {
  if (path == impl_->dylib_path_) return;
  if (path.isEmpty()) {
    impl_->dylib_path_.clear();
    impl_->session_.reset();
    impl_->clearSceneState();
    Q_EMIT dylibPathChanged();
    Q_EMIT appLoadedChanged();
    return;
  }
  (void)loadDylib(path, impl_->surface_id_);
}

uint32_t NveQtQuickItem::surfaceId() const noexcept { return impl_->surface_id_; }
void NveQtQuickItem::setSurfaceId(uint32_t id) {
  if (impl_->surface_id_ == id) return;
  impl_->surface_id_ = id;
  impl_->session_.set_surface_config(impl_->currentSurfaceConfig(id));
  Q_EMIT surfaceIdChanged();
}

uint32_t NveQtQuickItem::nodeCount() const { return impl_->session_.node_count(); }
bool NveQtQuickItem::isAppLoaded() const noexcept {
  return impl_->session_.is_valid() || impl_->session_.is_paired();
}
uint64_t NveQtQuickItem::sceneGeneration() const noexcept { return impl_->scene_generation_; }
void NveQtQuickItem::setOnNveEvent(nve::EventCallback cb) { impl_->session_.set_event_callback(std::move(cb)); }
void NveQtQuickItem::setTextMeasurer(nve::TextMeasurerCallback measurer) {
  impl_->session_.set_text_measurer(std::move(measurer));
  if (impl_->session_.active_fabric_host() != nullptr) {
    const auto cfg = impl_->currentSurfaceConfig(impl_->surface_id_);
    impl_->session_.set_surface_size(cfg.width, cfg.height, cfg.scale_factor);
    impl_->syncScene(false);
  }
}
nve::NativeHostSession& NveQtQuickItem::session() noexcept { return impl_->session_; }
const nve::NativeHostSession& NveQtQuickItem::session() const noexcept { return impl_->session_; }
QSGNode* NveQtQuickItem::updatePaintNode(QSGNode* oldNode, UpdatePaintNodeData*) {
  return syncSceneGraphNode(oldNode);
}

} // namespace nve::qt
