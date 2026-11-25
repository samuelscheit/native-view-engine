#pragma once

#include "NveQtCommon.hpp"
#include "NveQtWidget.hpp"

#include <QColor>
#include <QLineEdit>
#include <QPixmap>
#include <QString>
#include <QWidget>

#include <cstdint>
#include <functional>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

class QFocusEvent;
class QInputMethodEvent;
class QMouseEvent;
class QPaintEvent;
class QScrollArea;

namespace nve::qt {

inline constexpr const char* kNodeIdProperty = "nveNodeId";
inline constexpr const char* kA11yRoleProperty = "nveAccessibleRole";
inline constexpr const char* kA11yValueProperty = "nveAccessibleValue";
inline constexpr const char* kImageSrcProperty = "nveImageSrc";
inline constexpr const char* kImageDecodedProperty = "nveImageDecoded";
inline constexpr const char* kImageSizeProperty = "nveImageSize";
inline constexpr const char* kImageResizeModeProperty = "nveImageResizeMode";

void ensureAccessibleFactoryInstalled();
void applyWidgetAccessibility(QWidget* widget, const NodeSemantics& sem);

inline float opacityFromU8(uint8_t opacity_u8) noexcept {
  return static_cast<float>(opacity_u8) / 255.0f;
}

void applyWidgetOpacity(QWidget* widget, float opacity);

/// IME-safe `QLineEdit` subclass supporting `Qt::WA_InputMethodEnabled`, preedit vs commit
/// disambiguation without dropping multi-byte CJK/accented characters, `placeholder`,
/// `secureTextEntry`, focus/blur, and Return-key submit.
class NveQtLineEdit final : public QLineEdit {
 public:
  using TextChangeHandler = std::function<void(uint32_t, std::string_view)>;
  using SubmitHandler = std::function<void(uint32_t, std::string_view)>;
  using FocusHandler = std::function<void(uint32_t, bool)>;

  NveQtLineEdit(
      uint32_t node_id,
      TextChangeHandler on_text_change,
      SubmitHandler on_submit,
      FocusHandler on_focus,
      QWidget* parent = nullptr);

  void applySemantics(const NodeSemantics& sem);
  void applySceneText(const QString& next_text);

 protected:
  void inputMethodEvent(QInputMethodEvent* event) override;
  void focusInEvent(QFocusEvent* event) override;
  void focusOutEvent(QFocusEvent* event) override;

 private:
  void dispatchCurrentTextIfChanged();

  uint32_t node_id_{0};
  TextChangeHandler on_text_change_;
  SubmitHandler on_submit_;
  FocusHandler on_focus_;
  QString last_dispatched_text_;
  QString last_scene_text_;
  bool has_synced_scene_{false};
  bool composing_ime_{false};
  bool updating_programmatically_{false};
};

/// Lightweight custom-painted container widget for `NVE_NODE_KIND_VIEW`,
/// `NVE_NODE_KIND_PRESSABLE`, and wrapper `NVE_NODE_KIND_TEXT` nodes.
class NveQtBoxWidget final : public QWidget {
 public:
  using PressHandler = std::function<void(uint32_t, float, float)>;

  explicit NveQtBoxWidget(uint32_t node_id, PressHandler on_press, QWidget* parent = nullptr);

  void applyStyle(const NveRenderNode& node, bool interactive);
  void setInteractive(bool interactive);

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
};

/// Native `QWidget` for `NVE_NODE_KIND_IMAGE` rendering cached `QPixmap`s from `NveQtImageCache`
/// with all 4 React Native `resizeMode`s (`cover`, `contain`, `stretch`, `center`) and rounded clip.
class NveQtImageWidget final : public QWidget {
 public:
  using PressHandler = std::function<void(uint32_t, float, float)>;

  explicit NveQtImageWidget(uint32_t node_id, PressHandler on_press, QWidget* parent = nullptr);

  void applyNode(
      const NveRenderNode& node,
      std::string_view src,
      ImageResizeMode resize_mode,
      bool interactive);
  void setResizeMode(ImageResizeMode resize_mode);
  void setInteractive(bool interactive);

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
  std::string image_src_;
  ImageResizeMode resize_mode_{ImageResizeMode::Cover};
  QPixmap pixmap_;
};

class NveQtWidget::Impl {
 public:
  explicit Impl(NveQtWidget* owner);
  ~Impl();

  NveSurfaceConfig currentSurfaceConfig(uint32_t surface_id) const;
  void refreshRawPropsFromView(const NveSceneView& view);
  void syncViews(bool check_raw_props = false);
  void clearViews();

  const NodeSemantics* findSemantics(uint32_t node_id) const noexcept;
  ImageResizeMode findResizeMode(uint32_t node_id) const noexcept;

  void handlePressFromWidget(uint32_t node_id, float x, float y);
  void handleScrollFromWidget(uint32_t node_id, QScrollArea* scroll);

  NveDirectEvent makeBaseEvent(uint32_t node_id, nve::EventKind kind, uint8_t payload_tag) const;
  uint32_t dispatchSingleDirectEvent(const NveDirectEvent& ev);
  uint32_t dispatchPress(uint32_t node_id, float x, float y);
  uint32_t dispatchScroll(
      uint32_t node_id,
      float offset_x,
      float offset_y,
      float content_width,
      float content_height);
  uint32_t dispatchTextChange(uint32_t node_id, std::string_view text);
  uint32_t dispatchSubmit(uint32_t node_id, std::string_view text);
  uint32_t dispatchFocus(uint32_t node_id, bool focused);

  void syncChildrenImpl(
      uint32_t parent_id,
      QWidget** parent_view,
      const std::vector<QWidget*>& resolved_children);

  struct QtViewAdapter {
    Impl* self{nullptr};

    void update_root(const NveSceneView& view, const NveRenderNode& node);
    QWidget* create_view(const NveSceneView& view, const NveRenderNode& node);
    void update_view(const NveSceneView& view, QWidget*& widget, const NveRenderNode& node);
    void destroy_view(uint32_t node_id, QWidget*& view);

    template <typename LookupFn>
    void sync_children(
        uint32_t parent_id,
        QWidget** parent_view,
        std::span<const uint32_t> child_ids,
        LookupFn&& lookup_fn) {
      std::vector<QWidget*> resolved;
      resolved.reserve(child_ids.size());
      for (uint32_t child_id : child_ids) {
        QWidget** child_slot = lookup_fn(child_id);
        if (child_slot != nullptr && *child_slot != nullptr) {
          resolved.push_back(*child_slot);
        }
      }
      self->syncChildrenImpl(parent_id, parent_view, resolved);
    }
  };

  NveQtWidget* owner_{nullptr};
  nve::NativeHostSession session_;
  QtViewAdapter adapter_;
  nve::IncrementalSceneReconciler<QWidget*, QtViewAdapter> reconciler_;
  std::unordered_map<uint32_t, NodeSemantics> node_semantics_;
  std::unordered_map<uint32_t, ImageResizeMode> node_resize_modes_;
  std::unordered_map<uint32_t, bool> node_interactive_;
  QColor root_bg_color_{0, 0, 0, 0};
  uint32_t surface_id_{1};
  uint64_t reconcile_update_count_{0};
  bool raw_props_initialized_{false};
  bool in_sync_views_{false};
  bool destroying_{false};
};

} // namespace nve::qt
