// NVE host runtime: CSS `font-family` lists (`nve.typography.font_family`, the measurer's
// `NveRuntimeTextStyle::font_family`) split into family names, and the CSS generic families.
//
// Toolkits differ: Pango splits a list on `,` without trimming, DirectWrite takes one name, GTK
// CSS needs each name quoted. Every host walks the authored list through `for_each_font_family`,
// so measurement and paint resolve the same font.
#pragma once

#include <nve/host/ascii.hpp>

#include <cstdint>
#include <string_view>

namespace nve::host {

/// Calls `visit(name, quoted)` for every non-empty entry of the CSS list `list`, in order: a
/// quoted entry (`"Foo, Bar"` / `'Foo'`) is its content verbatim (commas included, never a
/// generic keyword); an unquoted one is trimmed of ASCII spaces / tabs. `visit` returns `false`
/// to stop.
template <class Visit>
void for_each_font_family(std::string_view list, Visit&& visit) {
  const auto space = [](char c) { return c == ' ' || c == '\t'; };
  size_t i = 0u;
  while (i < list.size()) {
    while (i < list.size() && space(list[i])) ++i;
    if (i == list.size()) return;
    std::string_view name;
    bool quoted = false;
    if (list[i] == '"' || list[i] == '\'') {
      const char quote = list[i];
      const size_t close = list.find(quote, i + 1u);
      const size_t end = close == std::string_view::npos ? list.size() : close;
      name = list.substr(i + 1u, end - i - 1u);
      quoted = true;
      i = end;  // anything between the closing quote and the next comma is dropped
    }
    const size_t comma = list.find(',', i);
    if (!quoted) {
      name = list.substr(i, comma == std::string_view::npos ? std::string_view::npos : comma - i);
      while (!name.empty() && space(name.back())) name.remove_suffix(1u);
    }
    i = comma == std::string_view::npos ? list.size() : comma + 1u;
    if (!name.empty() && !visit(name, quoted)) return;
  }
}

/// A CSS generic font family (`font-family` keyword), mapped by each host to its platform default.
enum class GenericFamily : uint8_t { None, Serif, SansSerif, Monospace, Cursive, Fantasy, SystemUi };

/// The generic family an unquoted list entry names (ASCII case-insensitive, `ui-*` aliases
/// included), else `None`.
[[nodiscard]] constexpr GenericFamily generic_font_family(std::string_view name) noexcept {
  struct Entry {
    std::string_view css;
    GenericFamily generic;
  };
  constexpr Entry kGenerics[] = {
      {"serif", GenericFamily::Serif},         {"ui-serif", GenericFamily::Serif},
      {"sans-serif", GenericFamily::SansSerif}, {"ui-sans-serif", GenericFamily::SansSerif},
      {"monospace", GenericFamily::Monospace}, {"ui-monospace", GenericFamily::Monospace},
      {"cursive", GenericFamily::Cursive},     {"fantasy", GenericFamily::Fantasy},
      {"system-ui", GenericFamily::SystemUi},
  };
  for (const Entry& e : kGenerics) {
    if (ascii_iequals(e.css, name)) return e.generic;
  }
  return GenericFamily::None;
}

/// The canonical CSS keyword of `generic` (empty for `None`).
[[nodiscard]] constexpr std::string_view generic_font_keyword(GenericFamily generic) noexcept {
  switch (generic) {
    case GenericFamily::Serif: return "serif";
    case GenericFamily::SansSerif: return "sans-serif";
    case GenericFamily::Monospace: return "monospace";
    case GenericFamily::Cursive: return "cursive";
    case GenericFamily::Fantasy: return "fantasy";
    case GenericFamily::SystemUi: return "system-ui";
    case GenericFamily::None: break;
  }
  return {};
}

}  // namespace nve::host
