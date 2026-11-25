#include "NveQtQuickItem.hpp"
#include "NveQtCommon.hpp"
#include "NveQtSGBuilder.hpp"
#include "NveQtTextCache.hpp"

#include <QAccessible>
#include <QInputMethodEvent>
#include <QKeyEvent>
#include <QMetaObject>
#include <QMouseEvent>
#include <QPointer>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QWheelEvent>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <limits>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

namespace nve::qt {

class NveQtQuickItem::Impl {
 public:
  explicit Impl(NveQtQuickItem* owner) : owner_(owner) {
    session_.set_text_measurer(NveQtQuickItem::makeQtTextMeasurer());
  }
  ~Impl() { session_.reset(); }

  qreal effectiveDpr() const {
    if (QQuickWindow* win = owner_->window()) {
      const qreal dpr = win->effectiveDevicePixelRatio();
      if (dpr > 0.0 && std::isfinite(dpr)) return dpr;
    }
    return 1.0;
  }

  NveSurfaceConfig currentSurfaceConfig(uint32_t sid) const {
    return NveSurfaceConfig{
        .surface_id = sid,
        .width = static_cast<float>(owner_->width() > 0.0 ? owner_->width() : 640.0),
        .height = static_cast<float>(owner_->height() > 0.0 ? owner_->height() : 420.0),
        .scale_factor = static_cast<float>(effectiveDpr()),
    };
  }

  void bumpGeneration() {
    ++scene_generation_;
    owner_->update();
    Q_EMIT owner_->sceneChanged();
  }

  void markLocalDirty(uint32_t node_id) {
    if (!pending_full_sg_sync_ && node_id != 0u) pending_dirty_ids_.insert(node_id);
    bumpGeneration();
  }

  void clearSceneState() {
    id_to_index_.clear();
    node_semantics_.clear();
    image_resize_modes_.clear();
    scroll_offsets_.clear();
    text_input_buffers_.clear();
    ime_preedit_buffers_.clear();
    pending_dirty_ids_.clear();
    pending_removed_ids_.clear();
    pending_full_sg_sync_ = true;
    has_synced_scene_ = false;
    raw_props_initialized_ = false;
    last_synced_frame_id_ = 0u;
    pressed_node_id_ = 0u;
    focused_node_id_ = 0u;
    bumpGeneration();
  }

  void rebuildIndex(const NveSceneView& view) {
    id_to_index_.clear();
    if (view.nodes == nullptr || view.node_count == 0u) return;
    id_to_index_.reserve(view.node_count);
    for (uint32_t i = 0u; i < view.node_count; ++i) {
      id_to_index_.emplace(view.nodes[i].node_id, static_cast<size_t>(i));
    }
  }

  const NveRenderNode* findNode(const NveSceneView& view, uint32_t id) const noexcept {
    return lookupSceneNode(view, id_to_index_, id);
  }

  const NodeSemantics* findSemantics(uint32_t id) const noexcept {
    const auto it = node_semantics_.find(id);
    return (it != node_semantics_.end()) ? &it->second : nullptr;
  }

  QPointF scrollOffsetFor(uint32_t id) const {
    const auto it = scroll_offsets_.find(id);
    return (it != scroll_offsets_.end()) ? it->second : QPointF(0.0, 0.0);
  }

  void syncScene(bool check_raw_props = false) {
    if (session_.active_fabric_host() == nullptr) return;
    const NveSceneView view = session_.scene_view();
    if (view.schema_hash != NVE_SCENE_SCHEMA_HASH) return;

    const QString prev_root_name = findSemantics(NVE_ROOT_NODE_ID)
                                       ? findSemantics(NVE_ROOT_NODE_ID)->label
                                       : QString();
    rebuildIndex(view);

    if (!has_synced_scene_ || view.frame_id < last_synced_frame_id_) {
      pending_full_sg_sync_ = true;
      pending_dirty_ids_.clear();
      pending_removed_ids_.clear();
    } else {
      for (uint32_t i = 0u; i < view.removed_count; ++i) {
        const uint32_t rid = view.removed_ids[i];
        pending_removed_ids_.insert(rid);
        pending_dirty_ids_.erase(rid);
        node_semantics_.erase(rid);
        image_resize_modes_.erase(rid);
      }
      for (uint32_t i = 0u; i < view.dirty_count; ++i) {
        const uint32_t idx = view.dirty_indices[i];
        if (idx < view.node_count) {
          const uint32_t nid = view.nodes[idx].node_id;
          pending_removed_ids_.erase(nid);
          pending_dirty_ids_.insert(nid);
        }
      }
    }

    bool need_rp = check_raw_props || !raw_props_initialized_;
    if (!need_rp && view.nodes != nullptr) {
      for (uint32_t i = 0u; i < view.dirty_count; ++i) {
        const uint32_t idx = view.dirty_indices[i];
        if (idx < view.node_count &&
            (view.nodes[idx].kind == NVE_NODE_KIND_IMAGE ||
             view.nodes[idx].kind == NVE_NODE_KIND_TEXT_INPUT)) {
          need_rp = true;
          break;
        }
      }
    }
    if (need_rp) {
      refreshQuickNodeMetadata(
          view, session_.snapshot_nodes(), node_semantics_, image_resize_modes_,
          pending_full_sg_sync_, pending_dirty_ids_);
      raw_props_initialized_ = true;
    } else if (view.nodes != nullptr) {
      for (uint32_t i = 0u; i < view.dirty_count; ++i) {
        const uint32_t idx = view.dirty_indices[i];
        if (idx >= view.node_count) continue;
        const NveRenderNode& n = view.nodes[idx];
        auto sit = node_semantics_.find(n.node_id);
        if (sit == node_semantics_.end()) {
          node_semantics_.emplace(n.node_id, extractNodeSemantics(n, nve::scene_node_text(view, n), nullptr));
        } else if (!sit->second.hasExplicitLabel && (n.flags & NVE_NODE_LEAF_TEXT) != 0u) {
          sit->second.label = detail::qstringFromUtf8(nve::scene_node_text(view, n));
        }
      }
    }

    has_synced_scene_ = true;
    last_synced_frame_id_ = view.frame_id;
    auto prune_map = [&](auto& m) {
      for (auto it = m.begin(); it != m.end();) {
        it = (id_to_index_.find(it->first) == id_to_index_.end()) ? m.erase(it) : std::next(it);
      }
    };
    prune_map(scroll_offsets_);
    prune_map(text_input_buffers_);
    prune_map(ime_preedit_buffers_);
    if (focused_node_id_ != 0u && id_to_index_.find(focused_node_id_) == id_to_index_.end()) {
      focused_node_id_ = 0u;
    }
    if (pressed_node_id_ != 0u && id_to_index_.find(pressed_node_id_) == id_to_index_.end()) {
      pressed_node_id_ = 0u;
    }
    bumpGeneration();

    const QString next_root_name = findSemantics(NVE_ROOT_NODE_ID)
                                       ? findSemantics(NVE_ROOT_NODE_ID)->label
                                       : QString();
    if (next_root_name != prev_root_name) {
      QAccessibleEvent ev(owner_, QAccessible::NameChanged);
      QAccessible::updateAccessibility(&ev);
    }
  }

  NveDirectEvent makeEvent(uint32_t node_id, nve::EventKind kind, uint8_t tag) const {
    NveDirectEvent ev{};
    ev.surface_id = surface_id_;
    ev.node_id = node_id;
    ev.timestamp_us = nve::current_timestamp_us();
    ev.kind = static_cast<uint8_t>(kind);
    ev.payload_tag = tag;
    ev.value = nve::make_null_value();
    return ev;
  }

  uint32_t dispatchBatch(NveEventBatch batch) {
    if ((!session_.is_valid() && !session_.is_paired()) || batch.ptr == nullptr || batch.len == 0u) {
      return 0u;
    }
    const uint32_t dirty = session_.dispatch_direct_events(batch, /*auto_render=*/true);
    if (dirty != 0u && !session_.is_paired()) syncScene(/*check_raw_props=*/false);
    (void)session_.drain_events();
    return dirty;
  }

  void setFocusedNode(uint32_t next_focus) {
    if (focused_node_id_ == next_focus) return;
    const uint32_t prev_focus = focused_node_id_;
    if (prev_focus != 0u) ime_preedit_buffers_.erase(prev_focus);
    focused_node_id_ = next_focus;
    NveDirectEvent evs[2]{};
    uint32_t n = 0u;
    if (prev_focus != 0u) evs[n++] = makeEvent(prev_focus, nve::EventKind::Blur, 0u);
    if (next_focus != 0u) evs[n++] = makeEvent(next_focus, nve::EventKind::Focus, 0u);
    if (n > 0u) (void)dispatchBatch(NveEventBatch{evs, n});
    if (!pending_full_sg_sync_) {
      if (prev_focus != 0u) pending_dirty_ids_.insert(prev_focus);
      if (next_focus != 0u) pending_dirty_ids_.insert(next_focus);
    }
    bumpGeneration();
    QAccessibleEvent a11y_ev(owner_, QAccessible::Focus);
    QAccessible::updateAccessibility(&a11y_ev);
  }

  std::string effectiveText(const NveSceneView& view, const NveRenderNode& node) const {
    if (node.kind == NVE_NODE_KIND_TEXT_INPUT) {
      if (auto it = text_input_buffers_.find(node.node_id); it != text_input_buffers_.end()) {
        return it->second;
      }
    }
    const std::string_view sv = nve::scene_node_text(view, node);
    return std::string(sv.data(), sv.size());
  }

  NveQtQuickItem* owner_{nullptr};
  nve::NativeHostSession session_;
  std::unordered_map<uint32_t, size_t> id_to_index_;
  std::unordered_map<uint32_t, NodeSemantics> node_semantics_;
  std::unordered_map<uint32_t, ImageResizeMode> image_resize_modes_;
  std::unordered_map<uint32_t, QPointF> scroll_offsets_;
  std::unordered_map<uint32_t, std::string> text_input_buffers_;
  std::unordered_map<uint32_t, QString> ime_preedit_buffers_;
  std::unordered_set<uint32_t> pending_dirty_ids_;
  std::unordered_set<uint32_t> pending_removed_ids_;
  void finishSessionTransition(
      uint32_t next_sid, bool emit_app_loaded, const QString* next_path, bool drain) {
    const bool path_changed = (next_path != nullptr && dylib_path_ != *next_path);
    const bool sid_changed = (surface_id_ != next_sid);
    if (next_path != nullptr) dylib_path_ = *next_path;
    surface_id_ = next_sid;
    clearSceneState();
    syncScene(true);
    if (drain) (void)session_.drain_events();
    if (path_changed) Q_EMIT owner_->dylibPathChanged();
    if (sid_changed) Q_EMIT owner_->surfaceIdChanged();
    if (emit_app_loaded) Q_EMIT owner_->appLoadedChanged();
  }

  bool finishPropUpdate(bool dirty) {
    if (dirty) syncScene(true);
    (void)session_.drain_events();
    return dirty;
  }

  bool pending_full_sg_sync_{true};
  bool has_synced_scene_{false};
  bool raw_props_initialized_{false};
  uint64_t last_synced_frame_id_{0};
  QString dylib_path_;
  uint32_t surface_id_{1};
  uint64_t scene_generation_{0};
  uint32_t pressed_node_id_{0};
  uint32_t focused_node_id_{0};
  SceneGraphStats sg_stats_{};
};

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

bool NveQtQuickItem::replaySnapshot(
    const QString& snapshotPath, uint32_t expectedSchemaVersion, uint32_t surfaceId, uint64_t* outRestoreUs) {
  const QByteArray utf8 = snapshotPath.toUtf8();
  impl_->session_.set_surface_config(impl_->currentSurfaceConfig(surfaceId));
  const auto t0 = std::chrono::steady_clock::now();
  if (!impl_->session_.replay_frame0_snapshot(
          std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())), expectedSchemaVersion)) {
    return false;
  }
  impl_->finishSessionTransition(surfaceId, false, nullptr, false);
  if (outRestoreUs) {
    *outRestoreUs = static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::steady_clock::now() - t0).count());
  }
  return true;
}

bool NveQtQuickItem::saveSnapshot(const QString& snapshotPath, uint32_t schemaVersion) const {
  const QByteArray utf8 = snapshotPath.toUtf8();
  return impl_->session_.save_snapshot(
      std::string_view(utf8.constData(), static_cast<size_t>(utf8.size())), schemaVersion);
}

bool NveQtQuickItem::applyPropsJson(std::string_view propsJson) {
  return impl_->session_.is_valid() && impl_->finishPropUpdate(impl_->session_.update_props_json(propsJson, true));
}
bool NveQtQuickItem::applyPropsDirect(const NveMapEntry* entries, uint32_t count) {
  return impl_->session_.is_valid() && impl_->finishPropUpdate(impl_->session_.update_props_direct(entries, count, true));
}
bool NveQtQuickItem::applyHotPatchJson(std::string_view patchJson) {
  return impl_->session_.is_valid() && impl_->session_.bridge() != nullptr &&
         impl_->finishPropUpdate(impl_->session_.bridge()->apply_hot_patch_json(patchJson, true));
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
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u});
}
uint32_t NveQtQuickItem::dispatchPress(uint32_t nodeId, const QPointF& p) {
  return dispatchPress(nodeId, static_cast<float>(p.x()), static_cast<float>(p.y()));
}
uint32_t NveQtQuickItem::dispatchScroll(uint32_t nodeId, float ox, float oy, float cw, float ch) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, nve::EventKind::Scroll, 2u);
  ev.x = ox; ev.y = oy; ev.width = cw; ev.height = ch;
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u});
}
uint32_t NveQtQuickItem::dispatchTextChange(uint32_t nodeId, std::string_view text) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, nve::EventKind::TextChange, 3u);
  ev.text = nve::make_str_slice(text);
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u});
}
uint32_t NveQtQuickItem::dispatchSubmit(uint32_t nodeId, std::string_view text) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, nve::EventKind::Custom, 5u);
  ev.name = nve::make_str_slice("onSubmit");
  ev.text = nve::make_str_slice(text);
  ev.value = nve::make_str_value(text);
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u});
}
uint32_t NveQtQuickItem::dispatchFocus(uint32_t nodeId, bool focused) {
  NveDirectEvent ev = impl_->makeEvent(nodeId, focused ? nve::EventKind::Focus : nve::EventKind::Blur, 0u);
  return impl_->dispatchBatch(NveEventBatch{&ev, 1u});
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

void NveQtQuickItem::geometryChange(const QRectF& newG, const QRectF& oldG) {
  QQuickItem::geometryChange(newG, oldG);
  if (newG.size() != oldG.size() && newG.width() > 0.0 && newG.height() > 0.0) {
    impl_->session_.set_surface_size(
        static_cast<float>(newG.width()), static_cast<float>(newG.height()),
        static_cast<float>(impl_->effectiveDpr()));
    if (impl_->session_.active_fabric_host() != nullptr) impl_->syncScene(false);
  }
}

void NveQtQuickItem::mousePressEvent(QMouseEvent* event) {
  if (!event || event->button() != Qt::LeftButton) { QQuickItem::mousePressEvent(event); return; }
  const uint32_t hit = hitTestNode(event->position(), true);
  if (hit != 0u) {
    if (const NveRenderNode* n = impl_->findNode(impl_->session_.scene_view(), hit);
        n && n->kind == NVE_NODE_KIND_TEXT_INPUT) {
      forceActiveFocus(Qt::MouseFocusReason);
      impl_->setFocusedNode(hit);
    } else {
      impl_->setFocusedNode(0u);
    }
    impl_->pressed_node_id_ = hit;
    event->accept();
    return;
  }
  if (impl_->focused_node_id_ != 0u) impl_->setFocusedNode(0u);
  impl_->pressed_node_id_ = 0u;
  event->ignore();
}

void NveQtQuickItem::mouseMoveEvent(QMouseEvent* event) {
  if (!event) return;
  if (impl_->pressed_node_id_ != 0u && (event->buttons() & Qt::LeftButton)) {
    const QRectF bounds = sceneNodeGlobalBounds(impl_->pressed_node_id_);
    if (!bounds.isValid() || !bounds.contains(event->position())) impl_->pressed_node_id_ = 0u;
    event->accept();
    return;
  }
  event->ignore();
}

void NveQtQuickItem::mouseReleaseEvent(QMouseEvent* event) {
  if (!event || event->button() != Qt::LeftButton) { QQuickItem::mouseReleaseEvent(event); return; }
  const uint32_t target = impl_->pressed_node_id_;
  impl_->pressed_node_id_ = 0u;
  if (target == 0u) { event->ignore(); return; }
  const QPointF pos = event->position();
  const QRectF bounds = sceneNodeGlobalBounds(target);
  event->accept();
  if (bounds.isValid() && bounds.contains(pos)) {
    (void)dispatchPress(target, static_cast<float>(pos.x() - bounds.x()), static_cast<float>(pos.y() - bounds.y()));
  }
}

void NveQtQuickItem::wheelEvent(QWheelEvent* event) {
  if (!event) return;
  const uint32_t scroll_id = hitTestScrollableNode(event->position());
  if (scroll_id == 0u) { event->ignore(); return; }
  const NveSceneView view = impl_->session_.scene_view();
  const NveRenderNode* node = impl_->findNode(view, scroll_id);
  if (!node) { event->ignore(); return; }

  QPointF delta(event->pixelDelta());
  if (delta.isNull()) delta = QPointF(event->angleDelta()) * (45.0 / 120.0);
  const QSizeF content = computeScrollContentSize(view, impl_->id_to_index_, *node);
  const QPointF prev = impl_->scrollOffsetFor(scroll_id);
  const QPointF next(
      std::clamp(prev.x() - delta.x(), 0.0, std::max<qreal>(0.0, content.width() - node->width)),
      std::clamp(prev.y() - delta.y(), 0.0, std::max<qreal>(0.0, content.height() - node->height)));
  impl_->scroll_offsets_[scroll_id] = next;
  impl_->markLocalDirty(scroll_id);
  (void)dispatchScroll(
      scroll_id, static_cast<float>(next.x()), static_cast<float>(next.y()),
      static_cast<float>(content.width()), static_cast<float>(content.height()));
  event->accept();
}

void NveQtQuickItem::keyPressEvent(QKeyEvent* event) {
  if (!event || impl_->focused_node_id_ == 0u) { QQuickItem::keyPressEvent(event); return; }
  const uint32_t target_id = impl_->focused_node_id_;
  const NveSceneView view = impl_->session_.scene_view();
  const NveRenderNode* node = impl_->findNode(view, target_id);
  if (!node || node->kind != NVE_NODE_KIND_TEXT_INPUT) { impl_->setFocusedNode(0u); event->ignore(); return; }
  if (event->key() == Qt::Key_Escape) { impl_->setFocusedNode(0u); event->accept(); return; }

  std::string current = impl_->effectiveText(view, *node);
  const auto pre_it = impl_->ime_preedit_buffers_.find(target_id);
  const QString preedit = (pre_it != impl_->ime_preedit_buffers_.end()) ? pre_it->second : QString();
  const TextKeyAction act =
      applyTextInputKeyPress(*event, current, impl_->findSemantics(target_id), preedit);
  if (act == TextKeyAction::Submit) { (void)dispatchSubmit(target_id, current); event->accept(); return; }
  if (act == TextKeyAction::TextChanged) {
    impl_->text_input_buffers_[target_id] = current;
    impl_->markLocalDirty(target_id);
    (void)dispatchTextChange(target_id, current);
    event->accept();
    return;
  }
  event->ignore();
}

void NveQtQuickItem::inputMethodEvent(QInputMethodEvent* event) {
  if (!event || impl_->focused_node_id_ == 0u) { QQuickItem::inputMethodEvent(event); return; }
  const uint32_t target_id = impl_->focused_node_id_;
  const NveSceneView view = impl_->session_.scene_view();
  const NveRenderNode* node = impl_->findNode(view, target_id);
  if (!node || node->kind != NVE_NODE_KIND_TEXT_INPUT) { impl_->setFocusedNode(0u); event->ignore(); return; }
  std::string current = impl_->effectiveText(view, *node);
  if (applyTextInputImeEvent(*event, target_id, current, impl_->ime_preedit_buffers_)) {
    impl_->text_input_buffers_[target_id] = current;
    impl_->markLocalDirty(target_id);
    (void)dispatchTextChange(target_id, current);
  } else {
    impl_->markLocalDirty(target_id);
  }
  event->accept();
}

QVariant NveQtQuickItem::inputMethodQuery(Qt::InputMethodQuery query) const {
  const uint32_t focused = impl_->focused_node_id_;
  switch (query) {
    case Qt::ImEnabled: return focused != 0u;
    case Qt::ImCursorRectangle: return focused != 0u ? QVariant(sceneNodeGlobalBounds(focused)) : QVariant(QRectF());
    case Qt::ImSurroundingText: return focused == 0u ? QString() : detail::qstringFromUtf8(sceneNodeText(focused));
    case Qt::ImCursorPosition:
    case Qt::ImAnchorPosition: return focused == 0u ? 0 : static_cast<int>(detail::qstringFromUtf8(sceneNodeText(focused)).size());
    case Qt::ImHints: {
      const NodeSemantics* sem = impl_->findSemantics(focused);
      return static_cast<int>((sem && sem->secureTextEntry) ? (Qt::ImhHiddenText | Qt::ImhNoPredictiveText) : Qt::ImhNone);
    }
    default: return QQuickItem::inputMethodQuery(query);
  }
}

} // namespace nve::qt
