#include "NveGtkInternal.hpp"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <string>
#include <utility>
#include <vector>

namespace nve {

namespace {

bool read_raw_string(const RawPropMap& props, const char* key, std::string& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    out = *s;
    return true;
  }
  return false;
}

bool read_raw_value_as_string(const RawPropMap& props, const char* key, std::string& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    out = *s;
    return true;
  }
  if (const auto* i = std::get_if<int64_t>(&it->second)) {
    out = std::to_string(*i);
    return true;
  }
  if (const auto* d = std::get_if<double>(&it->second)) {
    if (std::isfinite(*d)) {
      out = std::to_string(*d);
      return true;
    }
    return false;
  }
  if (const auto* b = std::get_if<bool>(&it->second)) {
    out = *b ? "true" : "false";
    return true;
  }
  return false;
}

bool read_raw_bool(const RawPropMap& props, const char* key, bool& out) {
  const auto it = props.find(key);
  if (it == props.end()) return false;
  if (const auto* b = std::get_if<bool>(&it->second)) {
    out = *b;
    return true;
  }
  if (const auto* i = std::get_if<int64_t>(&it->second)) {
    out = (*i != 0);
    return true;
  }
  if (const auto* s = std::get_if<std::string>(&it->second)) {
    const std::string lower = gtk_internal::ascii_lower(*s);
    if (lower == "true" || lower == "1" || lower == "yes") {
      out = true;
      return true;
    }
    if (lower == "false" || lower == "0" || lower == "no") {
      out = false;
      return true;
    }
  }
  return false;
}

bool parse_a11y_role(std::string_view raw, GtkA11yRole& out) {
  const std::string lower = gtk_internal::ascii_lower(raw);
  if (lower == "button") {
    out = GtkA11yRole::Button;
    return true;
  }
  if (lower == "text" || lower == "statictext" || lower == "label") {
    out = GtkA11yRole::Text;
    return true;
  }
  if (lower == "header" || lower == "heading") {
    out = GtkA11yRole::Header;
    return true;
  }
  if (lower == "textbox" || lower == "textinput" || lower == "search" || lower == "searchbox") {
    out = GtkA11yRole::TextBox;
    return true;
  }
  if (lower == "scrollarea" || lower == "scrollview") {
    out = GtkA11yRole::ScrollArea;
    return true;
  }
  if (lower == "image" || lower == "img") {
    out = GtkA11yRole::Image;
    return true;
  }
  if (lower == "list") {
    out = GtkA11yRole::List;
    return true;
  }
  if (lower == "listitem") {
    out = GtkA11yRole::ListItem;
    return true;
  }
  if (lower == "checkbox" || lower == "switch") {
    out = GtkA11yRole::CheckBox;
    return true;
  }
  if (lower == "link") {
    out = GtkA11yRole::Link;
    return true;
  }
  if (lower == "group" || lower == "region") {
    out = GtkA11yRole::Group;
    return true;
  }
  if (lower == "none" || lower == "presentation") {
    out = GtkA11yRole::None;
    return true;
  }
  return false;
}

GtkA11yRole infer_default_role(const NveRenderNode& node) noexcept {
  if (node.kind == NVE_NODE_KIND_PRESSABLE || (node.flags & NVE_NODE_INTERACTIVE) != 0u) {
    return GtkA11yRole::Button;
  }
  switch (node.kind) {
    case NVE_NODE_KIND_TEXT:
      return GtkA11yRole::Text;
    case NVE_NODE_KIND_TEXT_INPUT:
      return GtkA11yRole::TextBox;
    case NVE_NODE_KIND_SCROLL_VIEW:
      return GtkA11yRole::ScrollArea;
    case NVE_NODE_KIND_IMAGE:
      return GtkA11yRole::Image;
    default:
      return GtkA11yRole::Group;
  }
}

GtkAccessibleRole to_gtk_accessible_role(GtkA11yRole role) noexcept {
  switch (role) {
    case GtkA11yRole::Button:
      return GTK_ACCESSIBLE_ROLE_BUTTON;
    case GtkA11yRole::Text:
      return GTK_ACCESSIBLE_ROLE_LABEL;
    case GtkA11yRole::Header:
      return GTK_ACCESSIBLE_ROLE_HEADING;
    case GtkA11yRole::TextBox:
      return GTK_ACCESSIBLE_ROLE_TEXT_BOX;
    case GtkA11yRole::ScrollArea:
      return GTK_ACCESSIBLE_ROLE_GENERIC;
    case GtkA11yRole::Image:
      return GTK_ACCESSIBLE_ROLE_IMG;
    case GtkA11yRole::List:
      return GTK_ACCESSIBLE_ROLE_LIST;
    case GtkA11yRole::ListItem:
      return GTK_ACCESSIBLE_ROLE_LIST_ITEM;
    case GtkA11yRole::CheckBox:
      return GTK_ACCESSIBLE_ROLE_CHECKBOX;
    case GtkA11yRole::Link:
      return GTK_ACCESSIBLE_ROLE_LINK;
    case GtkA11yRole::Group:
      return GTK_ACCESSIBLE_ROLE_GROUP;
    case GtkA11yRole::None:
      return GTK_ACCESSIBLE_ROLE_NONE;
  }
  return GTK_ACCESSIBLE_ROLE_GROUP;
}

} // namespace

namespace gtk_internal {

std::string ascii_lower(std::string_view sv) {
  std::string out;
  out.reserve(sv.size());
  for (unsigned char ch : sv) {
    out.push_back(static_cast<char>(std::tolower(ch)));
  }
  return out;
}

std::unordered_map<uint32_t, RawPropMap> extract_raw_props_by_node(
    const std::vector<RenderedNodeSnapshot>& snapshots) {
  std::unordered_map<uint32_t, RawPropMap> result;
  for (const auto& snap : snapshots) {
    if (!snap.raw_props.empty()) {
      result.emplace(snap.node_id, snap.raw_props);
    }
  }
  return result;
}

std::string extract_image_resize_mode(const RawPropMap* raw_props) {
  if (raw_props == nullptr) {
    return "cover";
  }
  std::string mode;
  if (read_raw_string(*raw_props, "resizeMode", mode) ||
      read_raw_string(*raw_props, "resize_mode", mode)) {
    return ascii_lower(mode);
  }
  return "cover";
}

GtkNodeSemantics extract_node_semantics(
    const NveRenderNode& node,
    std::string_view text,
    const RawPropMap* raw_props) {
  GtkNodeSemantics sem{};
  sem.role = infer_default_role(node);

  if (raw_props != nullptr) {
    std::string role_str;
    if (read_raw_string(*raw_props, "accessibilityRole", role_str) ||
        read_raw_string(*raw_props, "role", role_str)) {
      GtkA11yRole parsed = sem.role;
      if (parse_a11y_role(role_str, parsed)) {
        sem.role = parsed;
        sem.explicit_role = true;
      }
    }

    if (read_raw_string(*raw_props, "accessibilityLabel", sem.label) ||
        read_raw_string(*raw_props, "aria-label", sem.label)) {
      sem.has_explicit_label = !sem.label.empty();
    }

    (void)read_raw_string(*raw_props, "accessibilityHint", sem.hint);

    if (read_raw_value_as_string(*raw_props, "accessibilityValue", sem.value)) {
      sem.has_explicit_value = true;
    }

    if (!read_raw_bool(*raw_props, "accessibilityDisabled", sem.disabled)) {
      (void)read_raw_bool(*raw_props, "disabled", sem.disabled);
    }

    if (!read_raw_string(*raw_props, "testID", sem.test_id)) {
      (void)read_raw_string(*raw_props, "data-testid", sem.test_id);
    }

    (void)read_raw_string(*raw_props, "placeholder", sem.placeholder);
    (void)read_raw_bool(*raw_props, "secureTextEntry", sem.secure_text_entry);
    (void)read_raw_bool(*raw_props, "multiline", sem.multiline);
  }

  if (!sem.has_explicit_label && (node.flags & NVE_NODE_LEAF_TEXT) != 0u && !text.empty()) {
    sem.label.assign(text.data(), text.size());
  }

  sem.gtk_role = to_gtk_accessible_role(sem.role);
  return sem;
}

std::string format_css_float(double value) {
  if (!std::isfinite(value)) {
    return "0";
  }
  char buf[G_ASCII_DTOSTR_BUF_SIZE];
  g_ascii_formatd(buf, sizeof(buf), "%.4f", value);
  std::string out(buf);
  if (out.find('.') != std::string::npos) {
    while (!out.empty() && out.back() == '0') {
      out.pop_back();
    }
    if (!out.empty() && out.back() == '.') {
      out.pop_back();
    }
  }
  if (out == "-0" || out.empty()) {
    return "0";
  }
  return out;
}

std::string format_css_rgba(uint32_t rgba) {
  const RgbaColor c = unpack_rgba(rgba);
  return "rgba(" + std::to_string(static_cast<unsigned>(c.r8)) + ", " +
         std::to_string(static_cast<unsigned>(c.g8)) + ", " +
         std::to_string(static_cast<unsigned>(c.b8)) + ", " +
         format_css_float(static_cast<double>(c.a)) + ")";
}

bool is_leaf_text_node(const NveRenderNode& node) noexcept {
  return (node.flags & NVE_NODE_LEAF_TEXT) != 0u;
}

std::string build_node_css_rule(
    const std::string& selector_class,
    const NveRenderNode& node) {
  std::string decls;
  decls.reserve(192);

  if (node.bg_rgba != 0u) {
    decls += "  background-color: " + format_css_rgba(node.bg_rgba) + ";\n";
  } else if (is_leaf_text_node(node)) {
    decls += "  background-color: transparent;\n";
  }

  if (std::isfinite(node.border_radius) && node.border_radius > 0.0f) {
    decls += "  border-radius: " + format_css_float(static_cast<double>(node.border_radius)) +
             "px;\n";
  }

  if (std::isfinite(node.border_width) && node.border_width > 0.0f &&
      node.border_rgba != 0u) {
    decls += "  border: " + format_css_float(static_cast<double>(node.border_width)) +
             "px solid " + format_css_rgba(node.border_rgba) + ";\n";
  } else if (node.kind != NVE_NODE_KIND_TEXT_INPUT) {
    decls += "  border: 0px none transparent;\n";
  }

  if (node.text_rgba != 0u) {
    decls += "  color: " + format_css_rgba(node.text_rgba) + ";\n";
  }

  if (std::isfinite(node.font_size) && node.font_size > 0.0f) {
    decls += "  font-size: " + format_css_float(static_cast<double>(node.font_size)) + "px;\n";
  }

  decls += "  min-width: 0px;\n  min-height: 0px;\n";

  return "." + selector_class + " {\n" + decls + "}\n";
}

void load_css_provider_string(GtkCssProvider* provider, const std::string& css) {
  if (provider == nullptr) {
    return;
  }
#if GTK_CHECK_VERSION(4, 12, 0)
  gtk_css_provider_load_from_string(provider, css.c_str());
#else
  gtk_css_provider_load_from_data(provider, css.data(), static_cast<gssize>(css.size()));
#endif
}

} // namespace gtk_internal

// -----------------------------------------------------------------------------
// NveGtkHost accessibility, CSS styling, and GtkEntry / IME helpers
// -----------------------------------------------------------------------------

std::optional<GtkNodeSemantics> NveGtkHost::accessibility_info_for_node_id(uint32_t node_id) const {
  if (const GtkNodeSemantics* sem = find_semantics(node_id)) {
    return *sem;
  }
  return std::nullopt;
}

const GtkNodeSemantics* NveGtkHost::find_semantics(uint32_t node_id) const noexcept {
  const auto it = node_semantics_.find(node_id);
  return (it != node_semantics_.end()) ? &it->second : nullptr;
}

void NveGtkHost::apply_accessibility_to_widget(GtkWidget* widget, const GtkNodeSemantics& sem) {
  if (widget == nullptr) {
    return;
  }
  GtkAccessible* acc = GTK_ACCESSIBLE(widget);
  if (!sem.label.empty()) {
    gtk_accessible_update_property(acc, GTK_ACCESSIBLE_PROPERTY_LABEL, sem.label.c_str(), -1);
  } else {
    gtk_accessible_reset_property(acc, GTK_ACCESSIBLE_PROPERTY_LABEL);
  }

  if (!sem.hint.empty()) {
    gtk_accessible_update_property(
        acc, GTK_ACCESSIBLE_PROPERTY_DESCRIPTION, sem.hint.c_str(), -1);
  } else {
    gtk_accessible_reset_property(acc, GTK_ACCESSIBLE_PROPERTY_DESCRIPTION);
  }

  if (sem.has_explicit_value) {
    gtk_accessible_update_property(
        acc, GTK_ACCESSIBLE_PROPERTY_VALUE_TEXT, sem.value.c_str(), -1);
  } else {
    gtk_accessible_reset_property(acc, GTK_ACCESSIBLE_PROPERTY_VALUE_TEXT);
  }

  gtk_accessible_update_state(acc, GTK_ACCESSIBLE_STATE_DISABLED, sem.disabled ? TRUE : FALSE, -1);
  gtk_widget_set_sensitive(widget, sem.disabled ? FALSE : TRUE);

  if (!sem.test_id.empty()) {
    gtk_widget_set_name(widget, sem.test_id.c_str());
  }
}

NveGtkHost::EntryBinding* NveGtkHost::ensure_entry_binding(uint32_t node_id, GtkWidget* widget) {
  if (widget == nullptr || !GTK_IS_ENTRY(widget)) {
    return nullptr;
  }
  const auto it = entry_bindings_.find(node_id);
  if (it != entry_bindings_.end() && it->second != nullptr && it->second->widget == widget) {
    return it->second.get();
  }
  remove_entry_binding(node_id, widget);

  auto binding = std::make_unique<EntryBinding>();
  binding->host = this;
  binding->node_id = node_id;
  binding->widget = widget;
  binding->changed_handler = g_signal_connect(
      GTK_EDITABLE(widget),
      "changed",
      gtk_detail::to_gcallback(&NveGtkHost::on_entry_changed),
      binding.get());
  binding->activate_handler = g_signal_connect(
      GTK_ENTRY(widget),
      "activate",
      gtk_detail::to_gcallback(&NveGtkHost::on_entry_activate),
      binding.get());

  GtkEventController* focus_ctrl = gtk_event_controller_focus_new();
  g_signal_connect(
      focus_ctrl,
      "enter",
      gtk_detail::to_gcallback(&NveGtkHost::on_entry_focus_enter),
      binding.get());
  g_signal_connect(
      focus_ctrl,
      "leave",
      gtk_detail::to_gcallback(&NveGtkHost::on_entry_focus_leave),
      binding.get());
  gtk_widget_add_controller(widget, focus_ctrl);
  binding->focus_controller = focus_ctrl;

  EntryBinding* raw = binding.get();
  entry_bindings_[node_id] = std::move(binding);
  return raw;
}

void NveGtkHost::remove_entry_binding(uint32_t node_id, GtkWidget* widget) {
  const auto it = entry_bindings_.find(node_id);
  if (it == entry_bindings_.end()) {
    return;
  }
  if (it->second != nullptr) {
    GtkWidget* target = (widget != nullptr) ? widget : it->second->widget;
    if (target != nullptr) {
      if (it->second->changed_handler != 0u &&
          g_signal_handler_is_connected(target, it->second->changed_handler)) {
        g_signal_handler_disconnect(target, it->second->changed_handler);
      }
      if (it->second->activate_handler != 0u &&
          g_signal_handler_is_connected(target, it->second->activate_handler)) {
        g_signal_handler_disconnect(target, it->second->activate_handler);
      }
      if (it->second->focus_controller != nullptr) {
        gtk_widget_remove_controller(target, it->second->focus_controller);
      }
    }
  }
  entry_bindings_.erase(it);
}

std::string NveGtkHost::scoped_node_class(uint32_t node_id) const {
  return "nve-host-" + std::to_string(host_instance_id_) + "-node-" + std::to_string(node_id);
}

std::string NveGtkHost::scoped_root_class() const {
  return "nve-host-" + std::to_string(host_instance_id_) + "-root";
}

void NveGtkHost::set_node_css(uint32_t node_id, std::string rule) {
  auto it = node_css_.find(node_id);
  if (it != node_css_.end() && it->second == rule) {
    return;
  }
  node_css_[node_id] = std::move(rule);
  css_dirty_ = true;
}

void NveGtkHost::erase_node_css(uint32_t node_id) {
  if (node_css_.erase(node_id) > 0u) {
    css_dirty_ = true;
  }
}

void NveGtkHost::flush_css_if_dirty() {
  if (!css_dirty_) {
    return;
  }
  std::vector<uint32_t> sorted_ids;
  sorted_ids.reserve(node_css_.size());
  for (const auto& [nid, _] : node_css_) {
    sorted_ids.push_back(nid);
  }
  std::sort(sorted_ids.begin(), sorted_ids.end());

  std::string combined;
  combined.reserve(root_css_.size() + sorted_ids.size() * 160u);
  combined += root_css_;
  for (uint32_t nid : sorted_ids) {
    combined += node_css_[nid];
  }

  active_css_ = std::move(combined);
  gtk_internal::load_css_provider_string(css_provider_, active_css_);
  css_dirty_ = false;
}

void NveGtkHost::on_entry_changed(GtkEditable* editable, gpointer user_data) {
  auto* binding = static_cast<EntryBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr || binding->updating_programmatically) {
    return;
  }
  const char* raw = gtk_editable_get_text(editable);
  const std::string current = (raw != nullptr) ? std::string(raw) : std::string{};
  if (current == binding->last_dispatched_text) {
    return;
  }
  binding->last_dispatched_text = current;
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  binding->user_editing = true;
  (void)host->dispatch_text_change(node_id, current);
  const auto it = host->entry_bindings_.find(node_id);
  if (it != host->entry_bindings_.end() && it->second != nullptr) {
    it->second->user_editing = false;
  }
}

void NveGtkHost::on_entry_activate(GtkEntry* entry, gpointer user_data) {
  auto* binding = static_cast<EntryBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr) {
    return;
  }
  const char* raw = gtk_editable_get_text(GTK_EDITABLE(entry));
  const std::string current = (raw != nullptr) ? std::string(raw) : std::string{};
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  (void)host->dispatch_submit(node_id, current);
}

void NveGtkHost::on_entry_focus_enter(GtkEventControllerFocus* controller, gpointer user_data) {
  (void)controller;
  auto* binding = static_cast<EntryBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr) {
    return;
  }
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  (void)host->dispatch_focus(node_id, true);
}

void NveGtkHost::on_entry_focus_leave(GtkEventControllerFocus* controller, gpointer user_data) {
  (void)controller;
  auto* binding = static_cast<EntryBinding*>(user_data);
  if (binding == nullptr || binding->host == nullptr) {
    return;
  }
  NveGtkHost* host = binding->host;
  const uint32_t node_id = binding->node_id;
  (void)host->dispatch_focus(node_id, false);
}

} // namespace nve
