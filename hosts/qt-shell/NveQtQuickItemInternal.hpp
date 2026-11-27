#pragma once

#include "NveQtCommon.hpp"
#include "NveQtQuickItem.hpp"
#include "NveQtSGBuilder.hpp"
#include "NveQtTextCache.hpp"

#include <QAccessible>
#include <QQuickWindow>

#include <cmath>
#include <string>
#include <unordered_map>
#include <unordered_set>

namespace nve::qt {

class NveQtQuickItem::Impl {
 public:
  explicit Impl(NveQtQuickItem* owner) : owner_(owner) {
    session_.set_text_measurer(NveQtQuickItem::makeQtTextMeasurer());
  }
  ~Impl() {
    session_.reset();
  }

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
    const uint32_t dirty = session_.dispatch_events(batch, /*auto_render=*/true);
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
    if (n > 0u) (void)dispatchBatch(NveEventBatch{evs, n, 0u});
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

} // namespace nve::qt
