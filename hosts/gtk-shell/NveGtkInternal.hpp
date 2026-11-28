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

  // Press / link activation / keyboard & focus.
  GtkGesture* click = nullptr;
  bool wants_press = false;
  bool wants_key_down = false;
  bool wants_key_up = false;
  bool focusable = false;
  std::optional<int16_t> tab_index;
  std::string dom_id;
  uint32_t aria_relations_res = 0u;
  std::unordered_map<int, std::vector<GtkWidget*>> applied_relations;

  // Scroll.
  gulong h_handler = 0u;
  gulong v_handler = 0u;
  float requested_x = NAN;
  float requested_y = NAN;
  bool scrolling_programmatically = false;

  // Text input (`GtkEntry` / `GtkTextView` buffer) & IME preedit.
  gulong changed_handler = 0u;
  gulong activate_handler = 0u;
  gulong realize_handler = 0u;
  GtkEventController* focus = nullptr;
  GtkIMContext* im_context = nullptr;
  host::ControlledText text;
  std::string preedit;
  bool composing = false;
  bool pending_change_after_compose = false;
  bool preedit_ended_awaiting_commit = false;
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

struct MappedKey {
  std::string key;
  std::string code;
  uint32_t flags = 0u;
  uint32_t key_code = 0u;
};

[[nodiscard]] WidgetKind widget_kind(const host::NodeData& node, bool has_canvas) noexcept;
/// Construct-only in GTK4: part of the view tag, so a role change recreates the widget.
[[nodiscard]] GtkAccessibleRole accessible_role(const host::NodeData& node) noexcept;
[[nodiscard]] GtkNodeSemantics node_semantics(const host::NodeData& node,
                                              std::optional<host::AriaRelations> relations = std::nullopt);
void apply_accessibility(GtkWidget* widget, const GtkNodeSemantics& sem);
void set_accessible_relation(GtkAccessible* acc, GtkAccessibleRelation rel, const std::vector<GtkWidget*>& targets);
[[nodiscard]] std::vector<std::string_view> split_idrefs(std::string_view raw);
[[nodiscard]] MappedKey map_gdk_key(guint keyval, guint keycode, GdkModifierType state);
[[nodiscard]] GtkIMContext* find_widget_im_context(GtkWidget* widget, WidgetKind kind) noexcept;
[[nodiscard]] std::string build_css_rule(const std::string& selector, const host::NodeData& node,
                                         WidgetKind kind, std::span<const host::BoxShadow> shadows);
[[nodiscard]] std::string css_rgba(uint32_t rgba);
/// An authored `font-family` list made safe for a CSS string and Pango (control bytes, quotes,
/// and CSS block punctuation dropped; bounded length). The painter's CSS and the measurer's
/// Pango description use the same string, so both resolve the same font.
[[nodiscard]] std::string font_family_list(std::string_view authored);
/// The same list as a CSS `font-family` value: each family a quoted CSS string, unquoted
/// generics (`monospace`, ASCII case-insensitive) as keywords.
[[nodiscard]] std::string font_family_css(std::string_view authored);
/// Pango's wrap mode for a resolved `TextWrap` (`None` keeps Pango's default; callers unset the width).
[[nodiscard]] PangoWrapMode pango_wrap(host::TextWrap wrap) noexcept;
[[nodiscard]] std::string css_float(double value);
[[nodiscard]] const char* cursor_name(uint8_t cursor) noexcept;
void load_css(GtkCssProvider* provider, const std::string& css);
void detach_from_parent(GtkWidget* widget);
void unparent_children(GtkWidget* container);
[[nodiscard]] int px(float v) noexcept;

}  // namespace gtk_internal

}  // namespace nve::gtk
