#include "NveGtkInternal.hpp"

#include <nve/host/font_family.hpp>

#include <algorithm>
#include <cmath>
#include <string>

namespace nve::gtk {

bool operator==(const GtkNodeSemantics& a, const GtkNodeSemantics& b) noexcept {
  const auto same_f = [](float x, float y) { return (std::isnan(x) && std::isnan(y)) || x == y; };
  return a.role == b.role && a.label == b.label && a.hint == b.hint && a.value_text == b.value_text &&
         a.placeholder == b.placeholder && a.level == b.level && a.checked == b.checked &&
         a.selected == b.selected && a.expanded == b.expanded && a.pressed == b.pressed &&
         same_f(a.value_now, b.value_now) && same_f(a.value_min, b.value_min) && same_f(a.value_max, b.value_max) &&
         a.disabled == b.disabled && a.hidden == b.hidden && a.secure == b.secure && a.multiline == b.multiline &&
         a.read_only == b.read_only;
}

namespace {

/// Longest `font-family` name emitted into CSS (real family names are far shorter).
constexpr size_t kMaxFontFamily = 256u;

GtkAccessibleRole aria_to_gtk(uint8_t role) noexcept {
  switch (role) {
    case NVE_ARIA_ROLE_BUTTON: return GTK_ACCESSIBLE_ROLE_BUTTON;
    case NVE_ARIA_ROLE_LINK: return GTK_ACCESSIBLE_ROLE_LINK;
    case NVE_ARIA_ROLE_HEADING: return GTK_ACCESSIBLE_ROLE_HEADING;
    case NVE_ARIA_ROLE_TEXT: return GTK_ACCESSIBLE_ROLE_LABEL;
    case NVE_ARIA_ROLE_IMAGE: return GTK_ACCESSIBLE_ROLE_IMG;
    case NVE_ARIA_ROLE_TEXT_BOX: return GTK_ACCESSIBLE_ROLE_TEXT_BOX;
    case NVE_ARIA_ROLE_SEARCH_BOX: return GTK_ACCESSIBLE_ROLE_SEARCH_BOX;
    case NVE_ARIA_ROLE_CHECKBOX: return GTK_ACCESSIBLE_ROLE_CHECKBOX;
    case NVE_ARIA_ROLE_SWITCH: return GTK_ACCESSIBLE_ROLE_SWITCH;
    case NVE_ARIA_ROLE_RADIO: return GTK_ACCESSIBLE_ROLE_RADIO;
    case NVE_ARIA_ROLE_SLIDER: return GTK_ACCESSIBLE_ROLE_SLIDER;
    case NVE_ARIA_ROLE_TAB: return GTK_ACCESSIBLE_ROLE_TAB;
    case NVE_ARIA_ROLE_TAB_LIST: return GTK_ACCESSIBLE_ROLE_TAB_LIST;
    case NVE_ARIA_ROLE_TAB_PANEL: return GTK_ACCESSIBLE_ROLE_TAB_PANEL;
    case NVE_ARIA_ROLE_DIALOG: return GTK_ACCESSIBLE_ROLE_DIALOG;
    case NVE_ARIA_ROLE_ALERT_DIALOG: return GTK_ACCESSIBLE_ROLE_ALERT_DIALOG;
    case NVE_ARIA_ROLE_ALERT: return GTK_ACCESSIBLE_ROLE_ALERT;
    case NVE_ARIA_ROLE_STATUS: return GTK_ACCESSIBLE_ROLE_STATUS;
    case NVE_ARIA_ROLE_TOOLTIP: return GTK_ACCESSIBLE_ROLE_TOOLTIP;
    case NVE_ARIA_ROLE_NAVIGATION: return GTK_ACCESSIBLE_ROLE_NAVIGATION;
    case NVE_ARIA_ROLE_MAIN: return GTK_ACCESSIBLE_ROLE_MAIN;
#if GTK_CHECK_VERSION(4, 14, 0)
    case NVE_ARIA_ROLE_ARTICLE: return GTK_ACCESSIBLE_ROLE_ARTICLE;
#endif
    case NVE_ARIA_ROLE_COMPLEMENTARY:
    case NVE_ARIA_ROLE_CONTENT_INFO: return GTK_ACCESSIBLE_ROLE_LANDMARK;  // no dedicated GTK roles
    case NVE_ARIA_ROLE_BANNER: return GTK_ACCESSIBLE_ROLE_BANNER;
    case NVE_ARIA_ROLE_FORM: return GTK_ACCESSIBLE_ROLE_FORM;
    case NVE_ARIA_ROLE_REGION: return GTK_ACCESSIBLE_ROLE_REGION;
    case NVE_ARIA_ROLE_GROUP: return GTK_ACCESSIBLE_ROLE_GROUP;
    case NVE_ARIA_ROLE_LIST: return GTK_ACCESSIBLE_ROLE_LIST;
    case NVE_ARIA_ROLE_LIST_ITEM: return GTK_ACCESSIBLE_ROLE_LIST_ITEM;
    case NVE_ARIA_ROLE_LIST_BOX: return GTK_ACCESSIBLE_ROLE_LIST_BOX;
    case NVE_ARIA_ROLE_OPTION: return GTK_ACCESSIBLE_ROLE_OPTION;
    case NVE_ARIA_ROLE_MENU: return GTK_ACCESSIBLE_ROLE_MENU;
    case NVE_ARIA_ROLE_MENU_ITEM: return GTK_ACCESSIBLE_ROLE_MENU_ITEM;
    case NVE_ARIA_ROLE_SEPARATOR: return GTK_ACCESSIBLE_ROLE_SEPARATOR;
    case NVE_ARIA_ROLE_PRESENTATION: return GTK_ACCESSIBLE_ROLE_PRESENTATION;
    case NVE_ARIA_ROLE_TABLE: return GTK_ACCESSIBLE_ROLE_TABLE;
    case NVE_ARIA_ROLE_ROW: return GTK_ACCESSIBLE_ROLE_ROW;
    case NVE_ARIA_ROLE_CELL: return GTK_ACCESSIBLE_ROLE_CELL;
    case NVE_ARIA_ROLE_COLUMN_HEADER: return GTK_ACCESSIBLE_ROLE_COLUMN_HEADER;
    case NVE_ARIA_ROLE_ROW_HEADER: return GTK_ACCESSIBLE_ROLE_ROW_HEADER;
    case NVE_ARIA_ROLE_PROGRESS_BAR: return GTK_ACCESSIBLE_ROLE_PROGRESS_BAR;
    case NVE_ARIA_ROLE_COMBO_BOX: return GTK_ACCESSIBLE_ROLE_COMBO_BOX;
    case NVE_ARIA_ROLE_GRID: return GTK_ACCESSIBLE_ROLE_GRID;
    case NVE_ARIA_ROLE_GRID_CELL: return GTK_ACCESSIBLE_ROLE_GRID_CELL;
    default: return GTK_ACCESSIBLE_ROLE_GENERIC;  // scroll area, code, code block: no GTK role
  }
}

}  // namespace

/// Explicit `nve.accessibility.role`, else the node kind's native role (`host::aria_role`).
GtkAccessibleRole gtk_internal::accessible_role(const host::NodeData& node) noexcept {
  return aria_to_gtk(host::aria_role(node));
}

namespace {

GtkAccessibleTristate tristate(uint8_t v) noexcept {
  if (v == NVE_ARIA_TRI_STATE_TRUE) return GTK_ACCESSIBLE_TRISTATE_TRUE;
  if (v == NVE_ARIA_TRI_STATE_MIXED) return GTK_ACCESSIBLE_TRISTATE_MIXED;
  return GTK_ACCESSIBLE_TRISTATE_FALSE;
}

void set_or_reset_string(GtkAccessible* acc, GtkAccessibleProperty prop, const std::string& v) {
  if (!v.empty()) {
    gtk_accessible_update_property(acc, prop, v.c_str(), -1);
  } else {
    gtk_accessible_reset_property(acc, prop);
  }
}

void set_or_reset_double(GtkAccessible* acc, GtkAccessibleProperty prop, float v) {
  if (std::isfinite(v)) {
    gtk_accessible_update_property(acc, prop, static_cast<double>(v), -1);
  } else {
    gtk_accessible_reset_property(acc, prop);
  }
}

}  // namespace

namespace gtk_internal {

WidgetKind widget_kind(const host::NodeData& node, bool has_canvas) noexcept {
  if (has_canvas) return WidgetKind::Canvas;
  switch (node.kind) {
    case host::NodeKind::Text: return WidgetKind::Label;
    case host::NodeKind::Image: return WidgetKind::Picture;
    case host::NodeKind::TextInput: return node.input.multiline ? WidgetKind::TextView : WidgetKind::Entry;
    case host::NodeKind::Scroll: return WidgetKind::Scrolled;
    case host::NodeKind::View:
    case host::NodeKind::Link: break;
  }
  return node.scrollable() ? WidgetKind::Scrolled : WidgetKind::Fixed;
}

GtkNodeSemantics node_semantics(const host::NodeData& node) {
  GtkNodeSemantics s;
  s.role = accessible_role(node);
  const host::AccessibilityData& a = node.a11y;
  s.label.assign(host::accessible_label(node));
  s.hint.assign(a.hint);
  s.value_text.assign(a.value_text);
  s.placeholder.assign(node.input.placeholder);
  s.level = a.level;
  s.checked = a.checked;
  s.selected = a.selected;
  s.expanded = a.expanded;
  s.pressed = a.pressed;
  s.value_now = a.value_now;
  s.value_min = a.value_min;
  s.value_max = a.value_max;
  s.disabled = node.interaction.disabled;
  s.hidden = a.hidden;
  s.secure = node.input.secure || node.input.input_kind == NVE_INPUT_KIND_PASSWORD;
  s.multiline = node.input.multiline;
  s.read_only = node.input.read_only;
  return s;
}

void apply_accessibility(GtkWidget* widget, const GtkNodeSemantics& s) {
  GtkAccessible* acc = GTK_ACCESSIBLE(widget);
  set_or_reset_string(acc, GTK_ACCESSIBLE_PROPERTY_LABEL, s.label);
  set_or_reset_string(acc, GTK_ACCESSIBLE_PROPERTY_DESCRIPTION, s.hint);
  set_or_reset_string(acc, GTK_ACCESSIBLE_PROPERTY_VALUE_TEXT, s.value_text);
  set_or_reset_string(acc, GTK_ACCESSIBLE_PROPERTY_PLACEHOLDER, s.placeholder);
  set_or_reset_double(acc, GTK_ACCESSIBLE_PROPERTY_VALUE_NOW, s.value_now);
  set_or_reset_double(acc, GTK_ACCESSIBLE_PROPERTY_VALUE_MIN, s.value_min);
  set_or_reset_double(acc, GTK_ACCESSIBLE_PROPERTY_VALUE_MAX, s.value_max);
  if (s.level != 0u) {
    gtk_accessible_update_property(acc, GTK_ACCESSIBLE_PROPERTY_LEVEL, static_cast<int>(s.level), -1);
  } else {
    gtk_accessible_reset_property(acc, GTK_ACCESSIBLE_PROPERTY_LEVEL);
  }
  gtk_accessible_update_property(acc, GTK_ACCESSIBLE_PROPERTY_MULTI_LINE, s.multiline ? TRUE : FALSE,
                                 GTK_ACCESSIBLE_PROPERTY_READ_ONLY, s.read_only ? TRUE : FALSE, -1);
  gtk_accessible_update_state(acc, GTK_ACCESSIBLE_STATE_DISABLED, s.disabled ? TRUE : FALSE,
                              GTK_ACCESSIBLE_STATE_HIDDEN, s.hidden ? TRUE : FALSE, -1);
  if (s.checked != NVE_ARIA_TRI_STATE_UNSET) {
    gtk_accessible_update_state(acc, GTK_ACCESSIBLE_STATE_CHECKED, tristate(s.checked), -1);
  } else {
    gtk_accessible_reset_state(acc, GTK_ACCESSIBLE_STATE_CHECKED);
  }
  if (s.pressed != NVE_ARIA_TRI_STATE_UNSET) {
    gtk_accessible_update_state(acc, GTK_ACCESSIBLE_STATE_PRESSED, tristate(s.pressed), -1);
  } else {
    gtk_accessible_reset_state(acc, GTK_ACCESSIBLE_STATE_PRESSED);
  }
  if (s.selected != NVE_ARIA_TRI_STATE_UNSET) {
    gtk_accessible_update_state(acc, GTK_ACCESSIBLE_STATE_SELECTED,
                                s.selected == NVE_ARIA_TRI_STATE_TRUE ? TRUE : FALSE, -1);
  } else {
    gtk_accessible_reset_state(acc, GTK_ACCESSIBLE_STATE_SELECTED);
  }
  if (s.expanded != NVE_ARIA_TRI_STATE_UNSET) {
    gtk_accessible_update_state(acc, GTK_ACCESSIBLE_STATE_EXPANDED,
                                s.expanded == NVE_ARIA_TRI_STATE_TRUE ? TRUE : FALSE, -1);
  } else {
    gtk_accessible_reset_state(acc, GTK_ACCESSIBLE_STATE_EXPANDED);
  }
  gtk_widget_set_sensitive(widget, s.disabled ? FALSE : TRUE);
}

std::string css_float(double value) {
  if (!std::isfinite(value)) return "0";
  char buf[G_ASCII_DTOSTR_BUF_SIZE];
  g_ascii_formatd(buf, sizeof(buf), "%.4f", value);
  std::string out(buf);
  if (out.find('.') != std::string::npos) {
    while (!out.empty() && out.back() == '0') out.pop_back();
    if (!out.empty() && out.back() == '.') out.pop_back();
  }
  return (out.empty() || out == "-0") ? std::string("0") : out;
}

std::string font_family_list(std::string_view authored) {
  // Pango splits a family on `,` but does not trim (`"A, monospace"` misses `monospace` on the
  // CoreText font map), so the entries are joined back with a bare `,`.
  std::string out;
  host::for_each_font_family(authored.substr(0u, kMaxFontFamily), [&out](std::string_view name) {
    std::string entry(name);
    // Inside a CSS string: drop anything that could end it or the block, and control bytes.
    std::erase_if(entry, [](char c) {
      const auto u = static_cast<unsigned char>(c);
      return u < 0x20u || u == 0x7Fu || c == '"' || c == '\\' || c == '{' || c == '}' || c == ';';
    });
    if (!entry.empty()) out += (out.empty() ? "" : ",") + entry;
    return true;
  });
  return out;
}

std::string css_rgba(uint32_t rgba) {
  const auto ch = [rgba](uint32_t shift) { return std::to_string((rgba >> shift) & 0xFFu); };
  return "rgba(" + ch(0u) + ", " + ch(8u) + ", " + ch(16u) + ", " +
         css_float(static_cast<double>((rgba >> 24u) & 0xFFu) / 255.0) + ")";
}

const char* cursor_name(uint8_t cursor) noexcept {
  switch (cursor) {
    case NVE_CURSOR_KIND_DEFAULT: return "default";
    case NVE_CURSOR_KIND_POINTER: return "pointer";
    case NVE_CURSOR_KIND_TEXT: return "text";
    case NVE_CURSOR_KIND_GRAB: return "grab";
    case NVE_CURSOR_KIND_GRABBING: return "grabbing";
    case NVE_CURSOR_KIND_NOT_ALLOWED: return "not-allowed";
    case NVE_CURSOR_KIND_COL_RESIZE: return "col-resize";
    case NVE_CURSOR_KIND_ROW_RESIZE: return "row-resize";
    case NVE_CURSOR_KIND_MOVE: return "move";
    case NVE_CURSOR_KIND_CROSSHAIR: return "crosshair";
    case NVE_CURSOR_KIND_PROGRESS: return "progress";
    case NVE_CURSOR_KIND_WAIT: return "wait";
    default: return nullptr;  // auto: inherit
  }
}

std::string build_css_rule(const std::string& selector, const host::NodeData& node, WidgetKind kind,
                           std::span<const host::BoxShadow> shadows) {
  const host::VisualStyle& v = node.visual;
  std::string d;
  d.reserve(256u);
  d += "  background-color: " + (v.background != 0u ? css_rgba(v.background) : std::string("transparent")) + ";\n";
  d += "  background-image: none;\n";
  if (std::isfinite(v.border_radius) && v.border_radius > 0.0f) {
    d += "  border-radius: " + css_float(v.border_radius) + "px;\n";
  }
  if (std::isfinite(v.border_width) && v.border_width > 0.0f && v.border_color != 0u) {
    d += "  border: " + css_float(v.border_width) + "px solid " + css_rgba(v.border_color) + ";\n";
  } else if (kind != WidgetKind::Entry && kind != WidgetKind::TextView) {
    d += "  border: 0px none transparent;\n";
  }
  if (!shadows.empty()) {
    d += "  box-shadow: ";
    for (size_t i = 0; i < shadows.size(); ++i) {
      const host::BoxShadow& s = shadows[i];
      if (i != 0u) d += ", ";
      if (s.inset) d += "inset ";
      d += css_float(s.offset_x) + "px " + css_float(s.offset_y) + "px " + css_float(s.blur_radius) + "px " +
           css_float(s.spread_radius) + "px " + css_rgba(s.color);
    }
    d += ";\n";
  }
  // Inherited typography applies to every text-bearing widget.
  if (kind == WidgetKind::Label || kind == WidgetKind::Entry || kind == WidgetKind::TextView) {
    const host::TextStyle& t = node.text_style;
    d += "  color: " + css_rgba(t.color) + ";\n";
    d += "  font-size: " + css_float(t.font_size) + "px;\n";
    d += "  font-weight: " + std::to_string(std::clamp<unsigned>(t.font_weight, 1u, 1000u)) + ";\n";
    d += t.font_style == NVE_FONT_STYLE_ITALIC    ? "  font-style: italic;\n"
         : t.font_style == NVE_FONT_STYLE_OBLIQUE ? "  font-style: oblique;\n"
                                                  : "  font-style: normal;\n";
    if (const std::string family = font_family_list(t.font_family); !family.empty()) {
      d += "  font-family: \"" + family + "\";\n";
    }
    if (t.letter_spacing != 0.0f && std::isfinite(t.letter_spacing)) {
      d += "  letter-spacing: " + css_float(t.letter_spacing) + "px;\n";
    }
    // Absolute: GTK scales a unit-less `line-height` by the font's natural line height.
    if (const double lh = host::line_height_points(t.line_height.unit, t.line_height.value, t.font_size); lh > 0.0) {
      d += "  line-height: " + css_float(lh) + "px;\n";
    }
    std::string deco;
    if ((t.text_decoration & NVE_TEXT_DECORATION_UNDERLINE) != 0u) deco += " underline";
    if ((t.text_decoration & NVE_TEXT_DECORATION_OVERLINE) != 0u) deco += " overline";
    if ((t.text_decoration & NVE_TEXT_DECORATION_LINE_THROUGH) != 0u) deco += " line-through";
    d += "  text-decoration-line:" + (deco.empty() ? std::string(" none") : deco) + ";\n";
  }
  d += "  min-width: 0px;\n  min-height: 0px;\n";
  return "." + selector + " {\n" + d + "}\n";
}

void load_css(GtkCssProvider* provider, const std::string& css) {
#if GTK_CHECK_VERSION(4, 12, 0)
  gtk_css_provider_load_from_string(provider, css.c_str());
#else
  gtk_css_provider_load_from_data(provider, css.data(), static_cast<gssize>(css.size()));
#endif
}

void detach_from_parent(GtkWidget* widget) {
  GtkWidget* parent = widget != nullptr ? gtk_widget_get_parent(widget) : nullptr;
  if (parent == nullptr) return;
  if (GTK_IS_FIXED(parent)) {
    gtk_fixed_remove(GTK_FIXED(parent), widget);
  } else {
    gtk_widget_unparent(widget);
  }
}

void unparent_children(GtkWidget* container) {
  if (container == nullptr || !GTK_IS_FIXED(container)) return;
  while (GtkWidget* child = gtk_widget_get_first_child(container)) gtk_fixed_remove(GTK_FIXED(container), child);
}

int px(float v) noexcept {
  // GTK sizes are `int`: clamp before rounding (`lround` of a huge finite float is UB-adjacent).
  constexpr float kMax = 1.0e7f;
  return std::isfinite(v) && v > 0.0f ? static_cast<int>(std::lround(std::min(v, kMax))) : 0;
}

}  // namespace gtk_internal

// --- NveGtkHost: semantics & CSS -----------------------------------------------------------

std::optional<GtkNodeSemantics> NveGtkHost::accessibility_info_for_node_id(uint32_t node_id) const {
  auto& tree = const_cast<host::RetainedTree<Adapter>&>(tree_);
  GtkNodeView* const* view = tree.find(node_id);
  if (view == nullptr || *view == nullptr) return std::nullopt;
  return (*view)->sem;
}

void NveGtkHost::apply_semantics(GtkNodeView& view, const host::NodeData& node) {
  GtkNodeSemantics sem = gtk_internal::node_semantics(node);
  if (sem == view.sem && view.synced) return;
  view.sem = std::move(sem);
  gtk_internal::apply_accessibility(view.widget, view.sem);
}

void NveGtkHost::apply_css(GtkNodeView& view, const host::NodeData& node) {
  std::vector<host::BoxShadow> shadows;
  if (auto decoded = host::decode_box_shadows(scene_resource(node.visual.box_shadows, host::kBoxShadowsKind))) {
    shadows = std::move(*decoded);
  }
  set_css_rule(view.id, gtk_internal::build_css_rule(scoped_class(view.id), node, view.kind, shadows));
}

std::string NveGtkHost::scoped_class(uint32_t node_id) const {
  return "nve-host-" + std::to_string(host_instance_id_) + "-node-" + std::to_string(node_id);
}

void NveGtkHost::set_css_rule(uint32_t node_id, std::string rule) {
  auto [it, inserted] = css_rules_.try_emplace(node_id);
  if (!inserted && it->second == rule) return;
  it->second = std::move(rule);
  css_dirty_ = true;
}

void NveGtkHost::flush_css() {
  if (!css_dirty_) return;
  std::vector<uint32_t> ids;
  ids.reserve(css_rules_.size());
  for (const auto& [id, rule] : css_rules_) ids.push_back(id);
  std::sort(ids.begin(), ids.end());
  std::string css;
  for (const uint32_t id : ids) css += css_rules_[id];
  active_css_ = std::move(css);
  gtk_internal::load_css(css_provider_, active_css_);
  css_dirty_ = false;
}

}  // namespace nve::gtk
