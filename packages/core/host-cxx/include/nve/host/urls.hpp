// NVE host runtime: URL policies every native host applies before handing an app-supplied URL
// to the platform.
//
//   openable_link(href)     may a link press open `href` with the system handler? Only
//                           `http://`, `https://`, and `mailto:` (scheme parsed per RFC 3986,
//                           case-insensitive); never `file:`, `javascript:`, custom protocols,
//                           shell paths, or anything with control characters.
//   local_image_path(url)   the local filesystem path an image URL names (absolute path or
//                           `file:` URL whose host is empty or `localhost`), else `nullopt`:
//                           UNC paths and remote `file://host/...` URLs would make the OS
//                           authenticate to that host (NTLM credential leak) and block the UI
//                           thread on the network. The path is still percent-encoded for
//                           `file:` URLs (decode with the platform: `PathCreateFromUrlW`,
//                           `g_filename_from_uri`).
#pragma once

#include <cctype>
#include <optional>
#include <string_view>

namespace nve::host {

namespace url_detail {

[[nodiscard]] inline bool iequals(std::string_view a, std::string_view b) noexcept {
  if (a.size() != b.size()) return false;
  for (size_t i = 0; i < a.size(); ++i) {
    if (std::tolower(static_cast<unsigned char>(a[i])) != std::tolower(static_cast<unsigned char>(b[i]))) return false;
  }
  return true;
}

[[nodiscard]] inline bool has_control_chars(std::string_view s) noexcept {
  for (const char c : s) {
    const auto u = static_cast<unsigned char>(c);
    if (u < 0x20u || u == 0x7Fu) return true;
  }
  return false;
}

/// RFC 3986 scheme (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )` before the first `:`), or empty.
[[nodiscard]] inline std::string_view scheme(std::string_view url) noexcept {
  const size_t colon = url.find(':');
  if (colon == std::string_view::npos || colon == 0u) return {};
  if (!std::isalpha(static_cast<unsigned char>(url[0]))) return {};
  for (size_t i = 1; i < colon; ++i) {
    const auto c = static_cast<unsigned char>(url[i]);
    if (!std::isalnum(c) && c != '+' && c != '-' && c != '.') return {};
  }
  return url.substr(0u, colon);
}

/// A rooted local path: POSIX `/x` (not `//`), or a Windows drive path `C:\x` / `C:/x`.
[[nodiscard]] inline bool rooted_local_path(std::string_view p) noexcept {
  if (p.size() >= 3u && std::isalpha(static_cast<unsigned char>(p[0])) && p[1] == ':' && (p[2] == '\\' || p[2] == '/')) {
    return true;
  }
  return !p.empty() && p[0] == '/' && (p.size() == 1u || (p[1] != '/' && p[1] != '\\'));
}

}  // namespace url_detail

[[nodiscard]] inline bool openable_link(std::string_view href) noexcept {
  if (href.empty() || href.size() > 8192u || url_detail::has_control_chars(href)) return false;
  const std::string_view s = url_detail::scheme(href);
  const std::string_view rest = href.substr(s.size() + 1u);
  if (url_detail::iequals(s, "http") || url_detail::iequals(s, "https")) {
    // `//authority` with a non-empty host.
    return rest.size() > 2u && rest[0] == '/' && rest[1] == '/' && rest[2] != '/' && rest[2] != '\\';
  }
  return url_detail::iequals(s, "mailto") && !rest.empty();
}

[[nodiscard]] inline std::optional<std::string_view> local_image_path(std::string_view url) noexcept {
  if (url.empty() || url_detail::has_control_chars(url)) return std::nullopt;
  const std::string_view s = url_detail::scheme(url);
  if (url_detail::iequals(s, "file")) {
    std::string_view rest = url.substr(5u);
    if (rest.size() >= 2u && rest[0] == '/' && rest[1] == '/') {  // `file://host/path`
      rest.remove_prefix(2u);
      const size_t slash = rest.find('/');
      const std::string_view host = rest.substr(0u, slash);
      if (!host.empty() && !url_detail::iequals(host, "localhost")) return std::nullopt;
      if (slash == std::string_view::npos) return std::nullopt;
      rest.remove_prefix(slash);
    }
    // `/C:/x` (Windows drive in URL form) or a rooted POSIX path; `//` would be UNC again.
    const std::string_view path = rest.size() >= 4u && rest[0] == '/' && rest[2] == ':' ? rest.substr(1u) : rest;
    return url_detail::rooted_local_path(path) ? std::optional<std::string_view>(path) : std::nullopt;
  }
  // A bare path ("C:" scans as a one-letter scheme).
  if ((s.empty() || s.size() == 1u) && url_detail::rooted_local_path(url)) return url;
  return std::nullopt;
}

}  // namespace nve::host
