// `NveQtQuickItem`: the `RetainedTree` adapter building GUI-thread node records, the item's QML
// API, and the scene-graph entry points (`updatePaintNode` -> NveQtSGBuilder.cpp).
#include "NveQtQuickInternal.hpp"

#include <QQuickWindow>
#include <QtQml/qqml.h>

#include <algorithm>

namespace nve::qt {

namespace {

QRectF qrect(const NveLayoutRect& r) { return QRectF(r.x, r.y, r.width, r.height); }

}  // namespace

// --- Impl --------------------------------------------------------------------------------------

NveQtQuickItem::Impl::Impl(NveQtQuickItem* owner, QtHostConfig config) : QtHost(owner, config), owner_(owner) {
  root_.id = host::kRootIndex;
  root_.paint.visual.background = Qt::transparent;
}

NveQtQuickItem::Impl::~Impl() {
  tearing_down_ = true;
  shutdown();
}

QuickNodeView* NveQtQuickItem::Impl::find(uint32_t id) noexcept {
  if (host::node_index(id) == host::kRootIndex) return &root_;
  QuickNodeView** v = tree_.find(id);
  return v != nullptr ? *v : nullptr;
}

const QuickNodeView* NveQtQuickItem::Impl::find(uint32_t id) const noexcept {
  return const_cast<Impl*>(this)->find(id);
}

void NveQtQuickItem::Impl::mark_dirty(QuickNodeView& v) {
  if (std::find(sg_dirty_.begin(), sg_dirty_.end(), &v) == sg_dirty_.end()) sg_dirty_.push_back(&v);
  scene_changed_ = true;
  if (!tearing_down_) owner_->update();
}

void NveQtQuickItem::Impl::bump_paint(QuickNodeView& v) {
  ++v.paint_generation;
  mark_dirty(v);
}

QPointF NveQtQuickItem::Impl::origin_of(const QuickNodeView& v) const {
  QPointF origin;
  for (const QuickNodeView* n = &v; n != nullptr && n != &root_; n = n->parent) {
    origin += n->local.topLeft();
    if (n->parent != nullptr) origin -= n->parent->scroll;
  }
  return origin;
}

void NveQtQuickItem::Impl::clear_views() {
  tree_.clear(adapter_);
  root_.children.clear();
  root_.children_dirty = true;
  scrollers_.clear();
  focused_ = pressed_ = 0u;
  hovered_.clear();
  mark_dirty(root_);
}

size_t NveQtQuickItem::Impl::sync_views(const host::RuntimeSceneGuard& scene) {
  canvas_.bind(scene.reader());
  const size_t touched = tree_.sync(scene.reader(), tables_, adapter_) + tree_.moved();
  // Scroll offsets change without dirtying their node: read them every frame.
  for (QuickNodeView* v : scrollers_) {
    const NveScrollOffset off = scene.reader().scroll_offset(v->id);
    const QPointF now(off.x, off.y);
    if (now != v->scroll) {
      v->scroll = now;
      mark_dirty(*v);
    }
  }
  if (scene.reader().frame_id() != frame_id_) {
    frame_id_ = scene.reader().frame_id();
    scene_changed_ = scene_changed_ || touched > 0u;
  }
  return touched;
}

void NveQtQuickItem::Impl::after_sync() {
  // Programmatic scroll targets, once each: the runtime clamps and publishes the offset.
  std::vector<std::pair<uint32_t, QPointF>> requests;
  for (QuickNodeView* v : scrollers_) {
    if (!v->request_pending) continue;
    v->request_pending = false;
    requests.emplace_back(v->id, QPointF(std::isnan(v->requested_x) ? v->scroll.x() : v->requested_x,
                                         std::isnan(v->requested_y) ? v->scroll.y() : v->requested_y));
  }
  for (const auto& [id, target] : requests) {
    if (const auto now = set_scroll_offset(id, static_cast<float>(target.x()), static_cast<float>(target.y()))) {
      if (QuickNodeView* v = find(id); v != nullptr && *now != v->scroll) {
        v->scroll = *now;
        mark_dirty(*v);
      }
    }
  }
  if (scene_changed_ && !tearing_down_) {
    scene_changed_ = false;
    Q_EMIT owner_->sceneChanged();
  }
}

// --- Adapter -----------------------------------------------------------------------------------

QuickNodeView* NveQtQuickItem::Impl::Adapter::create(const host::NodeData& node) {
  auto* v = new QuickNodeView();
  v->id = node.id;
  return v;
}

void NveQtQuickItem::Impl::Adapter::update(QuickNodeView*& v, const host::NodeData& node) {
  host->update_record(*v, node);
}

void NveQtQuickItem::Impl::Adapter::move(QuickNodeView*& v, const host::NodeGeometry& node) {
  ++host->stats_.nodesMoved;
  host->move_record(*v, node);
}

void NveQtQuickItem::Impl::Adapter::update_root(const host::NodeData& node) {
  QuickNodeView& r = host->root_;
  const QColor bg = qcolor(node.visual.background);
  if (bg != r.paint.visual.background || !r.synced) {
    r.paint.visual.background = bg;
    r.synced = true;
    host->bump_paint(r);
  }
  NodeSemantics sem = node_semantics(node);
  if (!(sem == r.sem)) {
    const NodeSemantics before = std::move(r.sem);
    r.sem = std::move(sem);
    host->a11y_semantics_changed(r, before);
  }
}

void NveQtQuickItem::Impl::Adapter::destroy(uint32_t id, QuickNodeView*& v) {
  if (v == nullptr) return;
  host->a11y_node_destroyed(id);
  host->scrollers_.erase(v);
  std::erase(host->sg_dirty_, v);
  if (v->sg != nullptr) host->sg_dead_.push_back(v->sg);
  for (QuickNodeView* c : v->children) {
    if (c->parent == v) c->parent = nullptr;  // survivors are relinked by their new parent
  }
  if (v->parent != nullptr) {
    std::erase(v->parent->children, v);
    v->parent->children_dirty = true;
    host->mark_dirty(*v->parent);
  }
  if (host->focused_ == id) host->focused_ = 0u;
  if (host->pressed_ == id) host->pressed_ = 0u;
  std::erase(host->hovered_, id);
  delete v;
  v = nullptr;
  if (!host->tearing_down_) host->owner_->update();
}

void NveQtQuickItem::Impl::Adapter::set_children(const host::NodeGeometry&, QuickNodeView** parent_view,
                                                 std::span<QuickNodeView** const> kids) {
  QuickNodeView& parent = parent_view != nullptr ? **parent_view : host->root_;
  parent.children.clear();
  for (uint32_t i = 0; i < kids.size(); ++i) {
    QuickNodeView* k = *kids[i];
    if (k->parent != nullptr && k->parent != &parent) {
      std::erase(k->parent->children, k);
      k->parent->children_dirty = true;
      host->mark_dirty(*k->parent);
    }
    k->parent = &parent;
    k->order = i;
    parent.children.push_back(k);
  }
  parent.children_dirty = true;
  host->mark_dirty(parent);
}

void NveQtQuickItem::Impl::move_record(QuickNodeView& v, const host::NodeGeometry& node) {
  const QRectF local = qrect(node.local);
  const QRectF content = qrect(node.content);
  const bool repaint = local.size() != v.local.size() || content != v.paint.content_box;
  v.local = local;
  v.paint.content_box = content;
  if (repaint) {
    bump_paint(v);  // re-rasterized at the new size (text re-wraps in the new content box)
  } else {
    mark_dirty(v);  // the transform only
  }
}

void NveQtQuickItem::Impl::update_record(QuickNodeView& v, const host::NodeData& node) {
  bool repaint = !v.synced;
  const QRectF local = qrect(node.local);
  if (local.size() != v.local.size()) repaint = true;
  v.local = local;
  if (v.parent != nullptr && v.z != node.visual.z_index) {  // paint order of the siblings changed
    v.parent->children_dirty = true;
    mark_dirty(*v.parent);
  }
  v.z = node.visual.z_index;
  v.visible = node.visual.visibility != NVE_VISIBILITY_HIDDEN;
  v.opacity = node.visual.opacity;

  BoxVisual visual = resolve_visual(node, v.applied, v.paint.visual);
  const QRectF content = qrect(node.content);
  const bool overflow_clips =
      node.visual.overflow_x != NVE_OVERFLOW_VISIBLE || node.visual.overflow_y != NVE_OVERFLOW_VISIBLE;
  if (!(visual == v.paint.visual) || content != v.paint.content_box || overflow_clips != v.paint.clip_rounded) {
    v.paint.visual = std::move(visual);
    v.paint.content_box = content;
    v.paint.clip_rounded = overflow_clips;
    repaint = true;
  }
  if (const std::optional<NodeContent> repainted = resolve_content(v.paint, v.applied, node, canvas_)) {
    repaint = true;
    if (*repainted == NodeContent::Canvas) ++v.canvas_redraws;
  }

  v.scrollable = node.scrollable();
  v.clips = overflow_clips || v.scrollable;
  if (v.scrollable) {
    scrollers_.insert(&v);
    const auto take = [&](float target, float& last) {
      if (std::isnan(target) || target == last) return;
      last = target;
      v.request_pending = true;
    };
    take(node.scroll.requested_x, v.requested_x);
    take(node.scroll.requested_y, v.requested_y);
  } else {
    scrollers_.erase(&v);
    v.scroll = QPointF();
  }

  const bool was_input = v.is_input;
  v.is_input = node.kind == host::NodeKind::TextInput;
  if (v.is_input || was_input) {
    if (v.is_input) update_input(v, node);
    repaint = true;
  }

  v.event_mask = node.interaction.event_mask;
  v.wants_press = node.wants(NVE_EVENT_PRESS);
  v.href.assign(node.href);
  v.disabled = node.interaction.disabled;
  v.focusable = !v.disabled && (node.interaction.focusable || v.is_input);
  v.cursor = node.interaction.cursor;
  NodeSemantics sem = node_semantics(node);
  if (!v.synced || !(sem == v.sem)) {
    const NodeSemantics before = std::move(v.sem);
    v.sem = std::move(sem);
    if (v.synced) a11y_semantics_changed(v, before);
  }
  v.synced = true;
  if (repaint) {
    bump_paint(v);
  } else {
    mark_dirty(v);  // transform / opacity / clip / visibility
  }
}

void NveQtQuickItem::Impl::update_input(QuickNodeView& v, const host::NodeData& node) {
  QuickInput& in = v.input;
  in.multiline = node.input.multiline;
  in.secure = node.input.secure || node.input.input_kind == NVE_INPUT_KIND_PASSWORD;
  in.read_only = node.input.read_only;
  in.max_length = node.input.max_length;
  in.placeholder = scene_string(node.input.placeholder);
  v.paint.style = text_style(node.text_style);
  if (!in.preedit.isEmpty()) return;  // never clobber a live IME composition
  const std::string shown = in.edit.toStdString();
  if (in.control.app_published(node.input.value, shown)) {
    const auto write = in.control.host_write();
    in.edit = scene_string(node.input.value, in.multiline);
    in.caret = static_cast<int>(in.edit.size());
  }
}

// --- NveQtQuickItem ----------------------------------------------------------------------------

NveQtQuickItem::NveQtQuickItem(QQuickItem* parent)
    : QQuickItem(parent), impl_(std::make_unique<Impl>(this, QtHostConfig{})) {
  setFlag(ItemHasContents, true);
  setFlag(ItemIsFocusScope, true);
  setFlag(ItemAcceptsInputMethod, true);
  setAcceptedMouseButtons(Qt::LeftButton);
  setAcceptHoverEvents(true);
  setActiveFocusOnTab(true);
  install_quick_accessible_factory();
}

NveQtQuickItem::~NveQtQuickItem() = default;

int NveQtQuickItem::registerQmlType() { return qmlRegisterType<NveQtQuickItem>("Nve", 1, 0, "NveSurface"); }

QtHost& NveQtQuickItem::host() noexcept { return *impl_; }

bool NveQtQuickItem::loadDylib(const QString& path) {
  const bool was_loaded = impl_->is_loaded();
  if (!impl_->load_dylib(path.toStdString())) return false;
  if (impl_->dylib_path_ != path) {
    impl_->dylib_path_ = path;
    Q_EMIT dylibPathChanged();
  }
  if (!was_loaded) Q_EMIT appLoadedChanged();
  return true;
}

bool NveQtQuickItem::hotReload(const QString& path) {
  const QString target = path.isEmpty() ? impl_->dylib_path_ : path;
  return !target.isEmpty() && impl_->reload_dylib(target.toStdString());
}

bool NveQtQuickItem::connectPaired(const QString& uri, const QString& token) {
  const bool was_loaded = impl_->is_loaded();
  if (!impl_->connect_paired(uri.toStdString(), token.toStdString())) return false;
  if (was_loaded != impl_->is_loaded()) Q_EMIT appLoadedChanged();
  return true;
}

void NveQtQuickItem::disconnectPaired() {
  const bool was_loaded = impl_->is_loaded();
  impl_->disconnect_paired();
  if (was_loaded != impl_->is_loaded()) Q_EMIT appLoadedChanged();
}

QString NveQtQuickItem::dylibPath() const { return impl_->dylib_path_; }
void NveQtQuickItem::setDylibPath(const QString& path) {
  if (path != impl_->dylib_path_ && !path.isEmpty()) (void)loadDylib(path);
}

quint32 NveQtQuickItem::surfaceId() const noexcept { return impl_->surface_id(); }
void NveQtQuickItem::setSurfaceId(quint32 id) {
  if (id == 0u || id == impl_->surface_id()) return;
  const bool was_loaded = impl_->is_loaded();
  impl_->set_surface_id(id);
  impl_->dylib_path_.clear();
  Q_EMIT surfaceIdChanged();
  if (was_loaded) Q_EMIT appLoadedChanged();
}

quint32 NveQtQuickItem::nodeCount() const { return static_cast<quint32>(impl_->tree_.size()); }
bool NveQtQuickItem::isAppLoaded() const noexcept { return impl_->is_loaded(); }
quint64 NveQtQuickItem::sceneGeneration() const noexcept { return impl_->frame_id_; }

QRectF NveQtQuickItem::nodeRect(uint32_t node_id) const {
  const QuickNodeView* v = impl_->find(node_id);
  if (v == nullptr || v == &impl_->root_) return v != nullptr ? boundingRect() : QRectF();
  return QRectF(impl_->origin_of(*v), v->local.size());
}

std::optional<NodeSemantics> NveQtQuickItem::accessibilityInfoForNodeId(uint32_t node_id) const {
  const QuickNodeView* v = impl_->find(node_id);
  return v != nullptr ? std::optional<NodeSemantics>(v->sem) : std::nullopt;
}

QString NveQtQuickItem::nodeText(uint32_t node_id) const {
  const QuickNodeView* v = impl_->find(node_id);
  if (v == nullptr) return {};
  return v->is_input ? v->input.edit : v->paint.text;
}

QPointF NveQtQuickItem::scrollOffsetForNode(uint32_t node_id) const {
  const QuickNodeView* v = impl_->find(node_id);
  return v != nullptr ? v->scroll : QPointF();
}

uint32_t NveQtQuickItem::focusedNodeId() const noexcept { return impl_->focused_; }

SceneGraphStats NveQtQuickItem::sceneGraphStats() const noexcept {
  SceneGraphStats s = impl_->stats_;
  s.sceneGeneration = impl_->frame_id_;
  return s;
}

const QSGNode* NveQtQuickItem::retainedSGNodeForNodeId(const QSGNode* root, uint32_t node_id) noexcept {
  return find_node_sg(root, node_id);
}

QSGNode* NveQtQuickItem::updatePaintNode(QSGNode* old_node, UpdatePaintNodeData*) {
  if (window() == nullptr) return old_node;
  return sync_scene_graph(old_node, *impl_, *window());
}

void NveQtQuickItem::geometryChange(const QRectF& new_geometry, const QRectF& old_geometry) {
  QQuickItem::geometryChange(new_geometry, old_geometry);
  if (new_geometry.size() != old_geometry.size() && new_geometry.width() > 0.0 && new_geometry.height() > 0.0) {
    const qreal dpr = window() != nullptr ? window()->effectiveDevicePixelRatio() : 0.0;
    impl_->set_surface_size(static_cast<float>(new_geometry.width()), static_cast<float>(new_geometry.height()),
                            static_cast<float>(dpr));
  }
}

void NveQtQuickItem::itemChange(ItemChange change, const ItemChangeData& value) {
  QQuickItem::itemChange(change, value);
  if (change == ItemDevicePixelRatioHasChanged || change == ItemSceneChange) {
    // Textures are rasterized per device pixel ratio: repaint everything on the new screen.
    const auto repaint = [this](auto& self, QuickNodeView& v) -> void {
      impl_->bump_paint(v);
      for (QuickNodeView* c : v.children) self(self, *c);
    };
    repaint(repaint, impl_->root_);
    if (window() != nullptr && width() > 0.0 && height() > 0.0) {
      impl_->set_surface_size(static_cast<float>(width()), static_cast<float>(height()),
                              static_cast<float>(window()->effectiveDevicePixelRatio()));
    }
  }
}

}  // namespace nve::qt
