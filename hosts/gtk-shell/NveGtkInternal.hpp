#pragma once

#include "NveGtkHost.hpp"

#include <nve/host/controlled_text.hpp>

#include <cmath>
#include <string>
#include <string_view>
#include <vector>

namespace nve::gtk {

/// `g_object_get_data` key of the `GtkNodeView` behind each retained widget.
inline constexpr const char* kViewKey = "nve-gtk-view";

/// Widget family realized for a node; part of the `RetainedTree` tag, so a change recreates it.
enum class WidgetKind : uint32_t { Fixed, Label, Picture, Entry, TextView, Scrolled, Canvas };

/// One retained GTK view: its widget plus every signal binding it owns (no side tables).
struct GtkNodeView {
  NveGtkHost* host = nullptr;
  uint32_t id = 0u;  // packed `NodeId`
  WidgetKind kind = WidgetKind::Fixed;
  GtkWidget* widget = nullptr;   // owned (ref-sunk)
  GtkWidget* content = nullptr;  // where children are placed (`GtkFixed`), null for leaves
  NveLayoutRect local{};
  GtkNodeSemantics sem;
  std::string href;

  // Press / link activation.
  GtkGesture* click = nullptr;
  bool wants_press = false;

  // Scroll.
  gulong h_handler = 0u;
  gulong v_handler = 0u;
  float requested_x = NAN;
  float requested_y = NAN;
  bool scrolling_programmatically = false;

  // Text input (`GtkEntry` / `GtkTextView` buffer).
  gulong changed_handler = 0u;
  gulong activate_handler = 0u;
  GtkEventController* focus = nullptr;
  host::ControlledText text;
  bool synced = false;

  // Image / canvas: the resource ids applied (kept per `RetainedTree::resource_unchanged`).
  uint32_t image_src = 0u;
  uint32_t canvas_res = 0u;
  std::vector<uint8_t> draw_list;
  bool extents_dirty = false;  // queued in `NveGtkHost::dirty_scroll_extents_`
  uint64_t redraw_count = 0u;

  [[nodiscard]] float content_width() const noexcept;
  [[nodiscard]] float content_height() const noexcept;
};

namespace gtk_internal {

[[nodiscard]] WidgetKind widget_kind(const host::NodeData& node, bool has_canvas) noexcept;
/// Construct-only in GTK4: part of the view tag, so a role change recreates the widget.
[[nodiscard]] GtkAccessibleRole accessible_role(const host::NodeData& node) noexcept;
[[nodiscard]] GtkNodeSemantics node_semantics(const host::NodeData& node);
void apply_accessibility(GtkWidget* widget, const GtkNodeSemantics& sem);
[[nodiscard]] std::string build_css_rule(const std::string& selector, const host::NodeData& node,
                                         WidgetKind kind, std::span<const host::BoxShadow> shadows);
[[nodiscard]] std::string css_rgba(uint32_t rgba);
/// An authored `font-family` list made safe for a CSS string and Pango (control bytes, quotes,
/// and CSS block punctuation dropped; bounded length). The painter's CSS and the measurer's
/// Pango description use the same string, so both resolve the same font.
[[nodiscard]] std::string font_family_list(std::string_view authored);
[[nodiscard]] std::string css_float(double value);
[[nodiscard]] const char* cursor_name(uint8_t cursor) noexcept;
void load_css(GtkCssProvider* provider, const std::string& css);
void detach_from_parent(GtkWidget* widget);
void unparent_children(GtkWidget* container);
[[nodiscard]] int px(float v) noexcept;

}  // namespace gtk_internal

}  // namespace nve::gtk
