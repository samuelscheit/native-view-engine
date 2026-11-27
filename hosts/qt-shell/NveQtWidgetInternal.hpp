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
/// What a node frame paints between its background and its border.
enum class NodeContent : uint8_t { None, Text, Image, Canvas };

inline constexpr const char* kNodeIdProperty = "nveNodeId";

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
  void set_text(const QString& text, const QtTextStyle& style);
  void set_image(const QImage& image, uint8_t fit);
  /// `true` when the bytes changed (and a repaint was queued).
  bool set_draw_list(std::span<const uint8_t> bytes);
  void clear_content();

  [[nodiscard]] NodeContent content() const noexcept { return content_; }
  [[nodiscard]] const QString& text() const noexcept { return text_; }
  [[nodiscard]] const QtTextStyle& text_style() const noexcept { return style_; }
  [[nodiscard]] const QImage& image() const noexcept { return image_; }
  [[nodiscard]] uint8_t fit() const noexcept { return fit_; }
  [[nodiscard]] const BoxVisual& visual() const noexcept { return visual_; }
  [[nodiscard]] QRectF content_box() const noexcept { return content_box_; }
  [[nodiscard]] std::span<const uint8_t> draw_list() const noexcept { return draw_list_; }

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
  BoxVisual visual_;
  QRectF content_box_;
  bool clip_rounded_ = false;
  NodeContent content_ = NodeContent::None;
  QString text_;
  QtTextStyle style_;
  QImage image_;
  uint8_t fit_ = NVE_IMAGE_FIT_COVER;
  std::vector<uint8_t> draw_list_;  // painted later, after the scene is released
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

  // Image / canvas.
  uint32_t image_src = 0u;
  uint8_t image_fit = NVE_IMAGE_FIT_COVER;
  uint32_t image_tint = 0u;  // 0 = untinted
  uint32_t canvas_res = 0u;
  uint32_t canvas_generation = 0u;
  uint64_t redraw_count = 0u;
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
    void update_root(const host::NodeData& node);
    void destroy(uint32_t id, Handle& view);
    void set_children(const host::NodeData& parent, Handle* parent_view, std::span<Handle* const> kids);
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
  std::unordered_set<QtNodeView*> scroll_views_;
  std::unordered_set<QWidget*> restack_;
  NodeSemantics root_sem_;
  QColor root_background_{Qt::transparent};
  uint64_t reconcile_update_count_ = 0u;
  bool tearing_down_ = false;

 protected:
  void clear_views() override;
  size_t sync_views(const host::RuntimeSceneGuard& scene) override;
  void after_sync() override;

 private:
  friend struct Adapter;
  void update_geometry(QtNodeView& v, const host::NodeData& node);
  void update_visual(QtNodeView& v, const host::NodeData& node);
  void update_content(QtNodeView& v, const host::NodeData& node);
  void update_text_input(QtNodeView& v, const host::NodeData& node);
  void update_scroll(QtNodeView& v, const host::NodeData& node);
  void update_interaction(QtNodeView& v, const host::NodeData& node);
  void apply_semantics(QtNodeView& v, const host::NodeData& node);
  void restack(QWidget* container);
  void update_scroll_extents();
};

}  // namespace nve::qt
