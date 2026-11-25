#pragma once

#include "NveGtkHost.hpp"

#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace nve::gtk_internal {

std::string ascii_lower(std::string_view sv);

std::unordered_map<uint32_t, RawPropMap> extract_raw_props_by_node(
    const std::vector<RenderedNodeSnapshot>& snapshots);

std::string extract_image_resize_mode(const RawPropMap* raw_props);

GtkNodeSemantics extract_node_semantics(
    const NveRenderNode& node,
    std::string_view text,
    const RawPropMap* raw_props);

std::string format_css_float(double value);
std::string format_css_rgba(uint32_t rgba);
bool is_leaf_text_node(const NveRenderNode& node) noexcept;
std::string build_node_css_rule(
    const std::string& selector_class,
    const NveRenderNode& node);
void load_css_provider_string(GtkCssProvider* provider, const std::string& css);

void unparent_all_fixed_children(GtkWidget* fixed_widget);
void detach_from_parent(GtkWidget* widget);

} // namespace nve::gtk_internal
