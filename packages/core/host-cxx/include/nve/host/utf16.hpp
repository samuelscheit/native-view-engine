// NVE host runtime: strict UTF-8 <-> UTF-16 conversion, for hosts whose strings are UTF-16 (JNI's
// `GetStringRegion` / `NewString`, which, unlike the `*UTF` calls, are not Modified UTF-8).
#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

#include "utf8.h"

namespace nve::host {

/// `s` as UTF-8, or `std::nullopt` when it holds a lone surrogate (not a Unicode string).
[[nodiscard]] inline std::optional<std::string> utf16_to_utf8(std::u16string_view s) {
  std::string out;
  out.reserve(s.size());
  for (size_t i = 0; i < s.size(); ++i) {
    uint32_t c = s[i];
    if (c >= 0xD800u && c <= 0xDBFFu) {
      if (i + 1 >= s.size() || s[i + 1] < 0xDC00u || s[i + 1] > 0xDFFFu) return std::nullopt;
      c = 0x10000u + ((c - 0xD800u) << 10) + (static_cast<uint32_t>(s[++i]) - 0xDC00u);
    } else if (c >= 0xDC00u && c <= 0xDFFFu) {
      return std::nullopt;
    }
    if (c < 0x80u) {
      out.push_back(static_cast<char>(c));
    } else if (c < 0x800u) {
      out.push_back(static_cast<char>(0xC0u | (c >> 6)));
      out.push_back(static_cast<char>(0x80u | (c & 0x3Fu)));
    } else if (c < 0x10000u) {
      out.push_back(static_cast<char>(0xE0u | (c >> 12)));
      out.push_back(static_cast<char>(0x80u | ((c >> 6) & 0x3Fu)));
      out.push_back(static_cast<char>(0x80u | (c & 0x3Fu)));
    } else {
      out.push_back(static_cast<char>(0xF0u | (c >> 18)));
      out.push_back(static_cast<char>(0x80u | ((c >> 12) & 0x3Fu)));
      out.push_back(static_cast<char>(0x80u | ((c >> 6) & 0x3Fu)));
      out.push_back(static_cast<char>(0x80u | (c & 0x3Fu)));
    }
  }
  return out;
}

/// `s` as UTF-16, or `std::nullopt` when it is not well-formed UTF-8.
[[nodiscard]] inline std::optional<std::u16string> utf8_to_utf16(std::string_view s) {
  const auto* p = reinterpret_cast<const uint8_t*>(s.data());
  if (nve_utf8_valid(p, s.size()) == 0) return std::nullopt;
  std::u16string out;
  out.reserve(s.size());
  for (size_t i = 0; i < s.size();) {
    const uint8_t b = p[i];
    // Well-formed input: the lead byte gives the length, continuation bytes carry 6 bits each.
    const size_t len = b < 0x80u ? 1u : b < 0xE0u ? 2u : b < 0xF0u ? 3u : 4u;
    uint32_t c = len == 1u ? b : len == 2u ? (b & 0x1Fu) : len == 3u ? (b & 0x0Fu) : (b & 0x07u);
    for (size_t k = 1; k < len; ++k) c = (c << 6) | (p[i + k] & 0x3Fu);
    i += len;
    if (c >= 0x10000u) {
      c -= 0x10000u;
      out.push_back(static_cast<char16_t>(0xD800u + (c >> 10)));
      out.push_back(static_cast<char16_t>(0xDC00u + (c & 0x3FFu)));
    } else {
      out.push_back(static_cast<char16_t>(c));
    }
  }
  return out;
}

}  // namespace nve::host
