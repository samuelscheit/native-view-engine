#pragma once

#include "NveQtPaint.hpp"
#include "NveQtText.hpp"
#include "NveQtWidget.hpp"

#include <nve/host/controlled_text.hpp>
#include <nve/host/retained_tree.hpp>

#include <QImage>
#include <QLineEdit>
#include <QPlainTextEdit>
#include <QString>

#include <cmath>
#include <cstdint>
#include <span>
#include <string>
#include <unordered_set>
#include <vector>

class QScrollArea;

namespace nve::qt {

struct QtNodeView;

/// Native widget family of a node; the `RetainedTree` tag, so a change recreates the view.
enum class NativeKind : uint32_t { Node, LineEdit, TextEdit, Scroll };

inline constexpr const char* kNodeIdProperty = "nveNodeId";
/// On a scroll view's content widget: its `QtNodeView*` (as `quintptr`).
inline constexpr const char* kScrollOwnerProperty = "nveScrollOwner";

// --- Hooks the node widgets call (NveQtWidgetReconciler.cpp) -----------------------------------
void view_hover(QtNodeView* view, NveEventKind kind, QPointF pos);
void view_focus(QtNodeView* view, bool focused);
/// Press / link activation (node-local point); may destroy the view: touch nothing afterwards.
void view_activate(QtNodeView* view, QPointF pos);

/// Custom-painted frame of one node: background, inset shadows, content, border; press, hover,
/// focus and keyboard activation; accessible through its node's semantics.
class NveQtNodeWidget final : public QWidget, public SemanticsCarrier {
 public:
  NveQtNodeWidget(QtNodeView* view, QWidget* parent);
  /// Called when the view is destroyed (the widget may outlive it until `deleteLater`).
  void detach() noexcept { view_ = nullptr; }

  void set_visual(const BoxVisual& visual, const QRectF& content_box, bool clip_rounded);
  /// Content (`QtHost::resolve_content`); call `update()` after changing it.
  [[nodiscard]] NodePaint& edit_paint() noexcept { return paint_; }

  [[nodiscard]] const NodePaint& paint() const noexcept { return paint_; }
  [[nodiscard]] NodeContent content() const noexcept { return paint_.content; }
  [[nodiscard]] const QString& text() const noexcept { return paint_.text; }
  [[nodiscard]] const QtTextStyle& text_style() const noexcept { return paint_.style; }
  [[nodiscard]] const QImage& image() const noexcept { return paint_.image; }
  [[nodiscard]] uint8_t fit() const noexcept { return paint_.fit; }
  [[nodiscard]] const BoxVisual& visual() const noexcept { return paint_.visual; }
  [[nodiscard]] QRectF content_box() const noexcept { return paint_.content_box; }
  [[nodiscard]] std::span<const uint8_t> draw_list() const noexcept { return paint_.draw_list; }

  [[nodiscard]] const NodeSemantics& semantics() const noexcept override;
  void activate() override;

 protected:
  void paintEvent(QPaintEvent* event) override;
  void resizeEvent(QResizeEvent* event) override;
  void mousePressEvent(QMouseEvent* event) override;
  void mouseReleaseEvent(QMouseEvent* event) override;
  void mouseMoveEvent(QMouseEvent* event) override;
  void enterEvent(QEnterEvent* event) override;
  void leaveEvent(QEvent* event) override;
  void focusInEvent(QFocusEvent* event) override;
  void focusOutEvent(QFocusEvent* event) override;
  void keyPressEvent(QKeyEvent* event) override;

 private:
  void update_mask();

  QtNodeView* view_;
  NodePaint paint_;
  bool pressed_ = false;
};

/// Outer box shadows of a node, a sibling stacked right below it (a widget cannot paint outside
/// its own rect). Transparent for input.
class NveQtShadowWidget final : public QWidget {
 public:
  explicit NveQtShadowWidget(QWidget* parent);
  /// `box`: the node's rect in the parent's coordinates.
  void set_shadow(const BoxVisual& visual, const QRectF& box);
  [[nodiscard]] const BoxVisual& visual() const noexcept { return visual_; }

 protected:
  void paintEvent(QPaintEvent* event) override;

 private:
  BoxVisual visual_;
  QRectF box_;  // in parent coordinates
  ShadowCache cache_;  // the blurred shadows, re-rasterized only when they change
};

/// Single-line text input (IME-aware: composition is never clobbered or dispatched mid-preedit).
class NveQtLineEdit final : public QLineEdit {
 public:
  NveQtLineEdit(QtNodeView* view, QWidget* parent);
  void detach() noexcept { view_ = nullptr; }
  [[nodiscard]] bool composing() const noexcept { return composing_; }

 protected:
  void inputMethodEvent(QInputMethodEvent* event) override;
  void focusInEvent(QFocusEvent* event) override;
  void focusOutEvent(QFocusEvent* event) override;
  void enterEvent(QEnterEvent* event) override;
  void leaveEvent(QEvent* event) override;

 private:
  QtNodeView* view_;
  bool composing_ = false;
};

/// Multi-line text input.
class NveQtTextEdit final : public QPlainTextEdit {
 public:
  NveQtTextEdit(QtNodeView* view, QWidget* parent);
  void detach() noexcept { view_ = nullptr; }
  [[nodiscard]] bool composing() const noexcept { return composing_; }

 protected:
  void inputMethodEvent(QInputMethodEvent* event) override;
  void focusInEvent(QFocusEvent* event) override;
  void focusOutEvent(QFocusEvent* event) override;
  void enterEvent(QEnterEvent* event) override;
  void leaveEvent(QEvent* event) override;

 private:
  QtNodeView* view_;
  bool composing_ = false;
};

/// One retained node view: its widgets and every binding it owns (no side tables).
struct QtNodeView {
  NveQtWidget::Impl* host = nullptr;
  uint32_t id = 0u;  // packed `NodeId`
  NativeKind kind = NativeKind::Node;
  QWidget* widget = nullptr;          // the node's widget (owned)
  NveQtNodeWidget* frame = nullptr;   // painted frame (Node, Scroll); null for inputs
  QScrollArea* scroll = nullptr;      // Scroll
  QWidget* content = nullptr;         // where children are placed
  NveQtLineEdit* line = nullptr;      // LineEdit
  NveQtTextEdit* text_edit = nullptr; // TextEdit
  NveQtShadowWidget* shadow = nullptr;  // outer shadows (owned), null when none

  NveLayoutRect local{};
  int32_t z = 0;
  uint32_t order = 0u;  // index in the parent's scene children
  bool visible = true;
  NodeSemantics sem;
  std::string href;
  uint32_t event_mask = 0u;
  bool wants_press = false;
  bool hovered = false;
  BoxVisual visual;
  bool clips = false;   // `overflow` != visible (rounded frames mask their children)
  QColor text_color;    // inputs: the style sheet's text color

  // Scroll: programmatic targets (`requested_scroll_*`) applied once, after content extents.
  float requested_x = NAN;
  float requested_y = NAN;
  bool request_pending = false;
  uint8_t axes = NVE_SCROLL_AXES_VERTICAL;
  bool scrolling_programmatically = false;

  bool synced = false;  // updated at least once

  // Text input (controlled value).
  host::ControlledText text;
  uint32_t max_length = 0u;

  AppliedResources applied;
  uint64_t redraw_count = 0u;  // canvas repaints
};

class NveQtWidget::Impl final : public QtHost {
 public:
  Impl(NveQtWidget* owner, QtHostConfig config);
  ~Impl() override;

  /// `RetainedTree` adapter (NveQtWidgetReconciler.cpp).
  struct Adapter {
    using Handle = QtNodeView*;
    Impl* host = nullptr;
    uint32_t tag(const host::NodeData& node) const;
    Handle create(const host::NodeData& node);
    void update(Handle& view, const host::NodeData& node);
    /// Geometry-only change (`read_geometry` data): reposition and re-box, nothing decoded.
    void move(Handle& view, const host::NodeGeometry& node);
    void update_root(const host::NodeData& node);
    void destroy(uint32_t id, Handle& view);
    void set_children(const host::NodeGeometry& parent, Handle* parent_view, std::span<Handle* const> kids);
    /// Canvas draw list: a republish under the same id re-runs `update`.
    uint32_t module_resource(const host::NodeData& node) const { return host->canvas_.at(node.index).draw_list; }
  };

  [[nodiscard]] QtNodeView* find(uint32_t id) noexcept;
  void on_native_scroll(QtNodeView& view);
  void on_text_edited(QtNodeView& view);
  void on_submit(QtNodeView& view);

  NveQtWidget* owner_;
  host::SceneTables tables_;
  canvas::CanvasTable canvas_;
  host::RetainedTree<Adapter> tree_;
  Adapter adapter_{this};
  /// Scroll views whose content extent (or programmatic target) may have changed this sync:
  /// their children, a child's box, their own size, or their scroll settings.
  std::unordered_set<QtNodeView*> scroll_dirty_;
  std::unordered_set<QWidget*> restack_;
  NodeSemantics root_sem_;
  QColor root_background_{Qt::transparent};
  uint64_t reconcile_update_count_ = 0u;
  uint64_t reconcile_move_count_ = 0u;
  bool tearing_down_ = false;

 protected:
  void clear_views() override;
  size_t sync_views(const host::RuntimeSceneGuard& scene) override;
  bool tree_resource_unchanged(const host::SceneReader& scene, uint32_t id,
                               uint32_t applied) const noexcept override {
    return tree_.resource_unchanged(scene, id, applied);
  }
  void after_sync() override;

 private:
  friend struct Adapter;
  void place(QtNodeView& v, const host::NodeGeometry& node);
  void apply_box(QtNodeView& v, const host::NodeGeometry& node);
  void update_geometry(QtNodeView& v, const host::NodeData& node);
  void update_visual(QtNodeView& v, const host::NodeData& node);
  void update_content(QtNodeView& v, const host::NodeData& node);
  void update_text_input(QtNodeView& v, const host::NodeData& node);
  void update_scroll(QtNodeView& v, const host::NodeData& node);
  void update_interaction(QtNodeView& v, const host::NodeData& node);
  void apply_semantics(QtNodeView& v, const host::NodeData& node);
  void restack(QWidget* container);
  /// `container` (a node's content widget) is a scroll view's content: recompute its extent.
  void mark_scroll_extent(QWidget* container);
  void update_scroll_extents(QtNodeView& v);
};

}  // namespace nve::qt
