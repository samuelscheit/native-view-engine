// Portable text / selection offset helpers shared by Win32 UIA actions and portable parity tests.
#pragma once

#include <algorithm>
#include <charconv>
#include <cmath>
#include <cstddef>
#include <string>
#include <string_view>

namespace nve::win {

/// Locale-independent decimal formatting for UIA `SetNumericValue`; returns empty for NaN/Inf.
[[nodiscard]] inline std::string format_numeric_value(double v) {
  if (!std::isfinite(v)) return {};
  char buf[64]{};
  const auto [ptr, ec] = std::to_chars(buf, buf + sizeof(buf), v);
  return ec == std::errc{} ? std::string(buf, static_cast<size_t>(ptr - buf)) : std::string{};
}

/// Strips `\r` and `\n` for single-line `TextInput` paste / UIA `SetValue` / `ReplaceSelectedText`.
[[nodiscard]] inline std::string sanitize_single_line(std::string_view raw) {
  std::string out;
  out.reserve(raw.size());
  for (const char c : raw) {
    if (c != '\r' && c != '\n') out.push_back(c);
  }
  return out;
}

/// Converts a Unicode scalar-value index in UTF-8 `utf8` into a UTF-16 code-unit offset.
[[nodiscard]] inline size_t utf16_offset_for_char_index(std::string_view utf8, size_t target_chars) noexcept {
  size_t i = 0u, chars = 0u, u16 = 0u;
  while (i < utf8.size() && chars < target_chars) {
    const auto lead = static_cast<unsigned char>(utf8[i]);
    const size_t step = (lead & 0x80u) == 0u ? 1u : (lead & 0xE0u) == 0xC0u ? 2u : (lead & 0xF0u) == 0xE0u ? 3u : 4u;
    u16 += step == 4u ? 2u : 1u;
    i += std::min(step, utf8.size() - i);
    ++chars;
  }
  return u16;
}

/// Maps a UTF-16 offset in `\n`-normalized `lf_utf8` to the UTF-16 offset in a Win32 multiline
/// `EDIT` control whose bare `\n` line breaks are expanded to `\r\n` (`to_control_text`).
[[nodiscard]] inline size_t crlf_utf16_offset_from_lf(std::string_view lf_utf8, size_t lf_u16) noexcept {
  size_t i = 0u, cur_lf = 0u, cur_crlf = 0u;
  while (i < lf_utf8.size() && cur_lf < lf_u16) {
    const auto lead = static_cast<unsigned char>(lf_utf8[i]);
    if (lead == '\n' && (i == 0u || lf_utf8[i - 1u] != '\r')) {
      cur_lf += 1u;
      cur_crlf += 2u;
      ++i;
      continue;
    }
    const size_t step = (lead & 0x80u) == 0u ? 1u : (lead & 0xE0u) == 0xC0u ? 2u : (lead & 0xF0u) == 0xE0u ? 3u : 4u;
    const size_t units = step == 4u ? 2u : 1u;
    const size_t take = std::min(units, lf_u16 - cur_lf);
    cur_lf += take;
    cur_crlf += take;
    i += std::min(step, lf_utf8.size() - i);
  }
  return cur_crlf;
}

/// Inverse of `crlf_utf16_offset_from_lf`: maps a Win32 multiline `EDIT` `EM_GETSEL` UTF-16 offset
/// back to the UTF-16 offset in `\n`-normalized `lf_utf8` (an index between `\r` and `\n` snaps
/// cleanly before `\n`).
[[nodiscard]] inline size_t lf_utf16_offset_from_crlf(std::string_view lf_utf8, size_t crlf_u16) noexcept {
  size_t i = 0u, cur_lf = 0u, cur_crlf = 0u;
  while (i < lf_utf8.size() && cur_crlf < crlf_u16) {
    const auto lead = static_cast<unsigned char>(lf_utf8[i]);
    if (lead == '\n' && (i == 0u || lf_utf8[i - 1u] != '\r')) {
      if (cur_crlf + 2u > crlf_u16) break;
      cur_crlf += 2u;
      cur_lf += 1u;
      ++i;
      continue;
    }
    const size_t step = (lead & 0x80u) == 0u ? 1u : (lead & 0xE0u) == 0xC0u ? 2u : (lead & 0xF0u) == 0xE0u ? 3u : 4u;
    const size_t units = step == 4u ? 2u : 1u;
    const size_t take = std::min(units, crlf_u16 - cur_crlf);
    cur_crlf += take;
    cur_lf += take;
    i += std::min(step, lf_utf8.size() - i);
  }
  return cur_lf;
}

}  // namespace nve::win
