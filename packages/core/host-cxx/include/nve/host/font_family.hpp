// NVE host runtime: CSS `font-family` lists (`nve.typography.font_family`, the measurer's
// `NveRuntimeTextStyle::font_family`) split into family names.
//
// Toolkits differ: Pango splits a list on `,` without trimming, DirectWrite takes one name.
// Every host walks the authored list through `for_each_font_family`, so measurement and paint
// resolve the same font.
#pragma once

#include <string_view>

namespace nve::host {

/// Calls `visit(name)` for every non-empty entry of `list`, trimmed of ASCII spaces / tabs and
/// CSS quotes, in order. `visit` returns `false` to stop.
template <class Visit>
void for_each_font_family(std::string_view list, Visit&& visit) {
  const auto trimmed = [](char c) { return c == ' ' || c == '\t' || c == '"' || c == '\''; };
  while (!list.empty()) {
    const size_t comma = list.find(',');
    std::string_view name = list.substr(0u, comma);
    list = comma == std::string_view::npos ? std::string_view() : list.substr(comma + 1u);
    while (!name.empty() && trimmed(name.front())) name.remove_prefix(1u);
    while (!name.empty() && trimmed(name.back())) name.remove_suffix(1u);
    if (!name.empty() && !visit(name)) return;
  }
}

}  // namespace nve::host
