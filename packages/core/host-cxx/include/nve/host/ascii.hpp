// NVE host runtime: locale-independent ASCII character classes for protocol text (URL schemes,
// CSS keywords). `<cctype>` follows the C locale, which a host process may change (`setlocale`,
// toolkit initialization), so security and parsing decisions never use it.
#pragma once

#include <cstddef>
#include <string_view>

namespace nve::host {

[[nodiscard]] constexpr bool ascii_alpha(char c) noexcept { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'); }
[[nodiscard]] constexpr bool ascii_digit(char c) noexcept { return c >= '0' && c <= '9'; }
[[nodiscard]] constexpr bool ascii_alnum(char c) noexcept { return ascii_alpha(c) || ascii_digit(c); }
[[nodiscard]] constexpr char ascii_lower(char c) noexcept { return c >= 'A' && c <= 'Z' ? static_cast<char>(c - 'A' + 'a') : c; }

/// ASCII case-insensitive equality (bytes >= 0x80 compare exactly).
[[nodiscard]] constexpr bool ascii_iequals(std::string_view a, std::string_view b) noexcept {
  if (a.size() != b.size()) return false;
  for (size_t i = 0; i < a.size(); ++i) {
    if (ascii_lower(a[i]) != ascii_lower(b[i])) return false;
  }
  return true;
}

/// Value of hex digit `c`, or -1.
[[nodiscard]] constexpr int ascii_hex(char c) noexcept {
  return ascii_digit(c) ? c - '0' : (c >= 'a' && c <= 'f') ? c - 'a' + 10 : (c >= 'A' && c <= 'F') ? c - 'A' + 10 : -1;
}

}  // namespace nve::host
