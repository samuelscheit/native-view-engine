// NVE host runtime: UTF-8 <-> UTF-16 conversion (strict both ways, plus a lossy U+FFFD UTF-8
// decoder), for hosts whose strings are UTF-16 (JNI's `GetStringRegion` / `NewString`, which,
// unlike the `*UTF` calls, are not Modified UTF-8).
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

/// `s` as UTF-16, each ill-formed subsequence replaced by one U+FFFD. Replacement follows the
/// Unicode "maximal subpart" practice (W3C/WHATWG `TextDecoder`, Rust `from_utf8_lossy`): a
/// truncated but otherwise valid prefix is one U+FFFD; any other invalid byte is one each.
[[nodiscard]] inline std::u16string utf8_to_utf16_lossy(std::string_view s) {
  const auto* p = reinterpret_cast<const uint8_t*>(s.data());
  const size_t n = s.size();
  std::u16string out;
  out.reserve(n);
  for (size_t i = 0; i < n;) {
    const uint8_t b = p[i++];
    if (b < 0x80u) {
      out.push_back(static_cast<char16_t>(b));
      continue;
    }
    // The lead byte fixes the sequence length and the valid range of the first continuation
    // byte (which rules out overlongs, surrogates, and code points above U+10FFFF).
    size_t need = 0;
    uint32_t c = 0;
    uint8_t lo = 0x80u, hi = 0xBFu;
    if (b >= 0xC2u && b <= 0xDFu) {
      need = 1, c = b & 0x1Fu;
    } else if (b >= 0xE0u && b <= 0xEFu) {
      need = 2, c = b & 0x0Fu;
      if (b == 0xE0u) lo = 0xA0u;
      if (b == 0xEDu) hi = 0x9Fu;
    } else if (b >= 0xF0u && b <= 0xF4u) {
      need = 3, c = b & 0x07u;
      if (b == 0xF0u) lo = 0x90u;
      if (b == 0xF4u) hi = 0x8Fu;
    }
    size_t got = 0;
    while (got < need && i < n && p[i] >= lo && p[i] <= hi) {
      c = (c << 6) | (p[i++] & 0x3Fu);
      lo = 0x80u, hi = 0xBFu;
      ++got;
    }
    if (need == 0 || got < need) {
      out.push_back(u'\uFFFD');
    } else if (c >= 0x10000u) {
      c -= 0x10000u;
      out.push_back(static_cast<char16_t>(0xD800u + (c >> 10)));
      out.push_back(static_cast<char16_t>(0xDC00u + (c & 0x3FFu)));
    } else {
      out.push_back(static_cast<char16_t>(c));
    }
  }
  return out;
}

/// `s` as UTF-16, or `std::nullopt` when it is not well-formed UTF-8.
[[nodiscard]] inline std::optional<std::u16string> utf8_to_utf16(std::string_view s) {
  if (nve_utf8_valid(reinterpret_cast<const uint8_t*>(s.data()), s.size()) == 0) return std::nullopt;
  return utf8_to_utf16_lossy(s);  // identical to a strict decode on well-formed input
}

}  // namespace nve::host
