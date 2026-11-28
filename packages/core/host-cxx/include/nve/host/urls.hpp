// NVE host runtime: URL policies every native host applies before handing an app-supplied URL
// to the platform.
//
//   openable_link(href)     may a link press open `href` with the system handler? Only
//                           `http://`, `https://`, and `mailto:` (scheme parsed per RFC 3986,
//                           ASCII case-insensitive); never `file:`, `javascript:`, custom
//                           protocols, shell paths, or anything with control characters.
//   local_image_path(url)   the local filesystem path an image URL names (absolute path or
//                           `file:` URL whose host is empty or `localhost`), else `nullopt`:
//                           UNC / device paths (`\\host`, `//host`, `\??\`, `\\?\`, `\\.\`) and
//                           remote `file://host/...` URLs would make the OS authenticate to that
//                           host (NTLM credential leak) and block the UI thread on the network,
//                           also when percent-encoded (`/%5c%5chost`). The path is still
//                           percent-encoded for `file:` URLs (decode with the platform:
//                           `PathCreateFromUrlW`, `g_filename_from_uri`) and hosts re-verify the
//                           decoded path is local (`local_decoded_path`).
//
// Every class test is ASCII-only (`ascii.hpp`): the C locale never changes a decision.
#pragma once

#include <nve/host/ascii.hpp>

#include <optional>
#include <string_view>

namespace nve::host {

namespace url_detail {

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
  if (colon == std::string_view::npos || colon == 0u || !ascii_alpha(url[0])) return {};
  for (size_t i = 1; i < colon; ++i) {
    const char c = url[i];
    if (!ascii_alnum(c) && c != '+' && c != '-' && c != '.') return {};
  }
  return url.substr(0u, colon);
}

[[nodiscard]] constexpr bool is_slash(char c) noexcept { return c == '/' || c == '\\'; }

/// A rooted local path: POSIX `/x` (not `//` / `/\`), or a Windows drive path `C:\x` / `C:/x`.
/// UNC (`\\host`, `//host`) and NT / device namespaces (`\??\`, `\\?\`, `\\.\`) never qualify:
/// they start with `\`.
[[nodiscard]] inline bool rooted_local_path(std::string_view p) noexcept {
  if (p.size() >= 3u && ascii_alpha(p[0]) && p[1] == ':' && is_slash(p[2])) return true;
  return !p.empty() && p[0] == '/' && (p.size() == 1u || !is_slash(p[1]));
}

/// `true` when percent-decoding `path` yields no control character (`%00`..`%1F`, `%7F`) and a
/// still-rooted local path (`/%5c%5chost/share` decodes to the UNC `/\\host`).
[[nodiscard]] inline bool decodes_local(std::string_view path) noexcept {
  char head[4]{};
  size_t n = 0u;
  for (size_t i = 0; i < path.size(); ++i) {
    char c = path[i];
    if (c == '%') {
      const int hi = i + 2u < path.size() ? ascii_hex(path[i + 1u]) : -1;
      const int lo = hi >= 0 ? ascii_hex(path[i + 2u]) : -1;
      if (lo < 0) return false;  // a malformed escape: platforms decode those differently
      c = static_cast<char>(hi * 16 + lo);
      if (static_cast<unsigned char>(c) < 0x20u || c == 0x7F) return false;
      i += 2u;
    }
    if (n < sizeof head) head[n++] = c;
  }
  return rooted_local_path(std::string_view(head, n));
}

}  // namespace url_detail

[[nodiscard]] inline bool openable_link(std::string_view href) noexcept {
  if (href.empty() || href.size() > 8192u || url_detail::has_control_chars(href)) return false;
  const std::string_view s = url_detail::scheme(href);
  if (s.empty()) return false;
  const std::string_view rest = href.substr(s.size() + 1u);
  if (ascii_iequals(s, "http") || ascii_iequals(s, "https")) {
    // `//authority` with a non-empty host.
    return rest.size() > 2u && rest[0] == '/' && rest[1] == '/' && !url_detail::is_slash(rest[2]);
  }
  return ascii_iequals(s, "mailto") && !rest.empty();
}

[[nodiscard]] inline std::optional<std::string_view> local_image_path(std::string_view url) noexcept {
  if (url.empty() || url_detail::has_control_chars(url)) return std::nullopt;
  const std::string_view s = url_detail::scheme(url);
  if (ascii_iequals(s, "file")) {
    std::string_view rest = url.substr(5u);
    if (rest.size() >= 2u && rest[0] == '/' && rest[1] == '/') {  // `file://host/path`
      rest.remove_prefix(2u);
      const size_t slash = rest.find('/');
      const std::string_view host = rest.substr(0u, slash);
      if (!host.empty() && !ascii_iequals(host, "localhost")) return std::nullopt;
      if (slash == std::string_view::npos) return std::nullopt;
      rest.remove_prefix(slash);
    }
    // `/C:/x` (Windows drive in URL form) or a rooted POSIX path; `//` would be UNC again.
    // Tighten to an ASCII drive letter followed by `:` and end-of-path or slash so POSIX paths
    // with colons (e.g. `file:///a:b/img.png`) keep their leading `/`.
    const bool win_drive = rest.size() >= 3u && rest[0] == '/' && ascii_alpha(rest[1]) && rest[2] == ':' &&
                           (rest.size() == 3u || url_detail::is_slash(rest[3]));
    const std::string_view path = win_drive ? rest.substr(1u) : rest;
    return url_detail::rooted_local_path(path) && url_detail::decodes_local(path) ? std::optional(path)
                                                                                  : std::nullopt;
  }
  // A bare path ("C:" scans as a one-letter scheme); never percent-decoded by the platform.
  if ((s.empty() || s.size() == 1u) && url_detail::rooted_local_path(url)) return url;
  return std::nullopt;
}

/// Re-verification of a path the platform decoded from a `local_image_path` URL: still rooted and
/// local (no UNC / NT / device prefix), no control characters.
[[nodiscard]] inline bool local_decoded_path(std::string_view path) noexcept {
  return !url_detail::has_control_chars(path) && url_detail::rooted_local_path(path);
}

}  // namespace nve::host
