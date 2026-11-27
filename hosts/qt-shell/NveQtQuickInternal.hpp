#pragma once

#include "NveQtPaint.hpp"
#include "NveQtQuickItem.hpp"

#include <nve/host/controlled_text.hpp>
#include <nve/host/retained_tree.hpp>

#include <QAccessible>

#include <cmath>
#include <cstdint>
#include <span>
#include <string>
#include <unordered_map>
#include <unordered_set>
#include <vector>

class QQuickWindow;

namespace nve::qt {

class NodeSG;

/// Text-input state of a Quick node (the item edits it; the app controls it through
/// `ControlledText`).
struct QuickInput {
  bool multiline = false;
  bool secure = false;
  bool read_only = false;
  uint32_t max_length = 0u;  // 0 = unlimited
  QString placeholder;
  QString edit;     // shown text
  int caret = 0;    // in UTF-16 units of `edit`
  QString preedit;  // live IME composition (painted, never dispatched)
  host::ControlledText control;
};

/// GUI-thread record of one node: everything `updatePaintNode` and input handling need. Built by
/// the `RetainedTree` adapter; read by the scene-graph builder while the GUI thread is blocked.
struct QuickNodeView {
  uint32_t id = 0u;
  QuickNodeView* parent = nullptr;
  std::vector<QuickNodeView*> children;  // scene order
  uint32_t order = 0u;
  int32_t z = 0;

  QRectF local;  // relative to the parent's box (before its scroll offset)
  NodePaint paint;
  qreal opacity = 1.0;
  bool visible = true;
  bool clips = false;  // `overflow` != visible, or a scroll container

  bool is_input = false;
  QuickInput input;

  bool scrollable = false;
  QPointF scroll;  // runtime offset (published `scroll_offsets`)
  float requested_x = NAN;
  float requested_y = NAN;
  bool request_pending = false;

  uint32_t event_mask = 0u;
  bool wants_press = false;
  bool focusable = false;
  bool disabled = false;
  uint8_t cursor = NVE_CURSOR_KIND_AUTO;
  std::string href;
  NodeSemantics sem;
  bool synced = false;

  AppliedResources applied;  // re-decode resources only on change
  uint64_t canvas_redraws = 0u;

  // Scene-graph bookkeeping (render side reads; GUI side bumps).
  uint64_t paint_generation = 1u;  // bumped when anything painted changes
  bool children_dirty = true;      // child list / paint order changed
  NodeSG* sg = nullptr;            // owned by the SG tree

  [[nodiscard]] bool wants(NveEventKind kind) const noexcept {
    return !disabled && (event_mask & host::event_bit(kind)) != 0u;
  }
  [[nodiscard]] bool activatable() const noexcept { return !disabled && (wants_press || !href.empty()); }
};

class NveQtQuickItem::Impl final : public QtHost {
 public:
  Impl(NveQtQuickItem* owner, QtHostConfig config);
  ~Impl() override;

  struct Adapter {
    using Handle = QuickNodeView*;
    Impl* host = nullptr;
    uint32_t tag(const host::NodeData&) const { return 0u; }  // one record type for every kind
    Handle create(const host::NodeData& node);
    void update(Handle& view, const host::NodeData& node);
    /// Geometry-only change (`read_geometry` data): new box, repainted only when its size changed.
    void move(Handle& view, const host::NodeData& node);
    void update_root(const host::NodeData& node);
    void destroy(uint32_t id, Handle& view);
    void set_children(const host::NodeData& parent, Handle* parent_view, std::span<Handle* const> kids);
    /// Canvas draw list: a republish under the same id re-runs `update`.
    uint32_t module_resource(const host::NodeData& node) const { return host->canvas_.at(node.index).draw_list; }
  };

  [[nodiscard]] QuickNodeView* find(uint32_t id) noexcept;
  [[nodiscard]] const QuickNodeView* find(uint32_t id) const noexcept;
  /// Marks `v` for the next `updatePaintNode` and schedules it.
  void mark_dirty(QuickNodeView& v);
  void bump_paint(QuickNodeView& v);
  /// Item-local origin of `v`'s box (ancestors' positions and scroll offsets applied).
  [[nodiscard]] QPointF origin_of(const QuickNodeView& v) const;
  /// Deepest visible record under an item-local point, in reverse paint order.
  [[nodiscard]] QuickNodeView* pick(QPointF item_pos);

  // --- Input (NveQtQuickInput.cpp) -----------------------------------------------------------
  void press(QPointF pos);
  void release(QPointF pos);
  void hover(QPointF pos, bool inside);
  bool wheel(QPointF pos, QPointF delta);
  bool key(QKeyEvent& event);
  void ime(QInputMethodEvent& event);
  [[nodiscard]] QVariant ime_query(Qt::InputMethodQuery query) const;
  void set_focus(uint32_t id);
  void edit_input(QuickNodeView& v, const QString& text, int caret);

  // --- Accessibility (NveQtQuickAccessible.cpp) ------------------------------------------------
  /// Registered accessible interface of node `id` (created on first use; ROOT = the item's).
  [[nodiscard]] QAccessibleInterface* a11y_node(uint32_t id);
  void a11y_node_destroyed(uint32_t id);
  void a11y_semantics_changed(QuickNodeView& v, const NodeSemantics& before);
  void a11y_focused(uint32_t id);

  NveQtQuickItem* owner_;
  QString dylib_path_;
  host::SceneTables tables_;
  canvas::CanvasTable canvas_;
  host::RetainedTree<Adapter> tree_;
  Adapter adapter_{this};
  QuickNodeView root_;  // the ROOT node: background, semantics, top-level children
  std::unordered_set<QuickNodeView*> scrollers_;
  std::vector<QuickNodeView*> sg_dirty_;  // records to update in the next `updatePaintNode`
  std::vector<NodeSG*> sg_dead_;          // wrappers of destroyed records (deleted render-side)
  std::unordered_map<uint32_t, QAccessible::Id> a11y_ids_;
  SceneGraphStats stats_;
  uint64_t frame_id_ = 0u;
  bool scene_changed_ = false;
  bool tearing_down_ = false;

  // Input state.
  uint32_t focused_ = 0u;
  uint32_t pressed_ = 0u;           // press target (activation on release over it)
  std::vector<uint32_t> hovered_;   // hovered path, root-most first

 protected:
  void clear_views() override;
  size_t sync_views(const host::RuntimeSceneGuard& scene) override;
  bool tree_resource_unchanged(const host::RuntimeSceneReader& scene, uint32_t id,
                               uint32_t applied) const noexcept override {
    return tree_.resource_unchanged(scene, id, applied);
  }
  void after_sync() override;

 private:
  void move_record(QuickNodeView& v, const host::NodeData& node);
  void update_record(QuickNodeView& v, const host::NodeData& node);
  void update_content(QuickNodeView& v, const host::NodeData& node);
  void update_input(QuickNodeView& v, const host::NodeData& node);
};

/// Installs (once) the factory exposing `NveQtQuickItem` and its nodes to accessibility.
void install_quick_accessible_factory();

// --- Scene graph (NveQtSGBuilder.cpp; render thread, GUI thread blocked) ----------------------

/// Builds / updates the retained `QSGNode` tree from the records: deletes wrappers of destroyed
/// records, updates dirty records (O(changed)), and relinks child lists whose order changed.
/// Returns the root (`old_root` reused when non-null; null = Qt dropped the old tree: rebuild).
QSGNode* sync_scene_graph(QSGNode* old_root, NveQtQuickItem::Impl& impl, QQuickWindow& window);
[[nodiscard]] const QSGNode* find_node_sg(const QSGNode* root, uint32_t node_id) noexcept;

/// Paints a text input's content (edited text or placeholder, secure bullets, IME preedit,
/// caret when focused) into its content box.
void paint_input(QPainter& p, const QRectF& box, const QuickNodeView& v, bool focused);

}  // namespace nve::qt
