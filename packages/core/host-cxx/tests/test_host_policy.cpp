// Host policy tests (pure headers shared by every native shell): the link-scheme allowlist and
// local-image rule (`urls.hpp`), and the launch command line + app-state lifecycle
// (`launch_options.hpp`).
#include <clocale>
#include <cstdlib>
#include <string>
#include <vector>

#include "check.h"
#include "nve/host/core_resources.hpp"
#include "nve/host/launch_options.hpp"
#include "nve/host/urls.hpp"

namespace {

using namespace nve::host;

void set_env(const char* name, const char* value) {
#if defined(_WIN32)
  _putenv_s(name, value != nullptr ? value : "");
#else
  if (value != nullptr) {
    setenv(name, value, 1);
  } else {
    unsetenv(name);
  }
#endif
}

void test_openable_link() {
  for (const char* ok : {"https://example.com", "http://example.com/a?b=c#d", "HTTPS://EXAMPLE.COM/", "mailto:a@b.c",
                         "MailTo:someone@example.com?subject=hi", "https://[::1]:8080/x"}) {
    CHECK(openable_link(ok));
  }
  for (const char* bad : {"", "file:///C:/Windows/System32/calc.exe", "javascript:alert(1)", "C:\\Windows\\calc.exe",
                          "\\\\server\\share\\x", "ms-settings:privacy", "vbscript:x", "https:example.com",
                          "https:///evil", "https://\\\\server", "http//example.com", "mailto:", "1http://x",
                          "https://example.com/\nfile:x", "data:text/html,<b>x</b>", "ftp://example.com",
                          "shell:startup", "/usr/bin/open", "x-https://example.com"}) {
    CHECK(!openable_link(bad));
  }
  CHECK(!openable_link(std::string(9000u, 'a').insert(0, "https://")));
}

void test_local_image_path() {
  const auto path = [](const char* url) { return std::string(local_image_path(url).value_or("<none>")); };
  CHECK(path("/tmp/a.png") == "/tmp/a.png");
  CHECK(path("C:\\img\\a.png") == "C:\\img\\a.png");
  CHECK(path("C:/img/a.png") == "C:/img/a.png");
  CHECK(path("file:///tmp/a%20b.png") == "/tmp/a%20b.png");  // still percent-encoded
  CHECK(path("file:///C:/img/a.png") == "C:/img/a.png");
  CHECK(path("file:///a:b/img.png") == "/a:b/img.png");  // POSIX path with `:` is not a Windows drive
  CHECK(path("file:///1:/img.png") == "/1:/img.png");    // non-alpha drive is POSIX
  CHECK(path("file:///C:") == "<none>");                 // bare drive letter without slash is drive-relative
  CHECK(path("FILE://localhost/tmp/a.png") == "/tmp/a.png");
  CHECK(path("file:/tmp/a.png") == "/tmp/a.png");
  // Remote hosts and UNC paths would authenticate to the network (and block the UI thread).
  for (const char* bad : {"file://server/share/a.png", "file:////server/share/a.png", "\\\\server\\share\\a.png",
                          "//server/share/a.png", "file:///\\\\server\\share", "file://evil.example.com/x.png",
                          "https://example.com/a.png", "relative/a.png", "file://localhost", "C:relative.png",
                          "data:image/png;base64,AAAA", ""}) {
    CHECK(path(bad) == "<none>");
  }
  // Percent-encoded UNC / NT / device prefixes and control characters decode to non-local paths.
  for (const char* bad : {"file:///%5c%5cserver/share/a.png", "file:///%5C%5Cserver/share", "file:///%2f%2fserver/a",
                          "file:///%2F%2Fserver/a", "file:///%5c%3f%3f%5cUNC%5cserver%5cx", "file:/%5c%5c.%5cpipe%5cx",
                          "file://localhost/%5cserver", "file:///%5C\\server", "file:///tmp/a%00.png",
                          "file:///tmp/%0a.png", "file:///tmp/%7f.png", "file:///tmp/%zz.png", "file:///tmp/a%2",
                          "\\??\\C:\\a.png", "\\\\?\\C:\\a.png", "\\\\.\\pipe\\x", "/\\server\\share"}) {
    CHECK(path(bad) == "<none>");
  }
  CHECK(path("file:///tmp/%41%2f.png") == "/tmp/%41%2f.png");  // encoded but local
  CHECK(path("file:///C:/a%5cb.png") == "C:/a%5cb.png");
  CHECK(local_decoded_path("/tmp/a.png") && local_decoded_path("C:\\img\\a.png"));
  for (const char* bad : {"\\\\server\\share", "//server/share", "\\??\\C:\\x", "\\\\?\\C:\\x", "/\\x", "C:x",
                          "relative", "/tmp/\x01.png", ""}) {
    CHECK(!local_decoded_path(bad));
  }
}

/// Scheme and host decisions never follow the C locale (Turkish `I` lower-cases to dotless `ı`).
void test_policies_are_locale_independent() {
  for (const char* name : {"tr_TR.ISO8859-9", "tr_TR.ISO-8859-9", "tr_TR.UTF-8", "tr_TR"}) {
    if (std::setlocale(LC_ALL, name) != nullptr) break;
  }
  CHECK(openable_link("HTTPS://EXAMPLE.COM") && openable_link("MAILTO:a@b.c") && openable_link("hTtP://x"));
  CHECK(local_image_path("FILE://LOCALHOST/tmp/a.png").value_or("") == "/tmp/a.png");
  // Bytes >= 0x80 are never letters, whatever the locale says.
  CHECK(!openable_link("\xC4\xB1https://x") && !openable_link("h\xFDttp://x"));
  CHECK(!local_image_path("\xC9:\\x.png") && !local_image_path("file://LOCALHOS\xDD/x.png"));
  std::setlocale(LC_ALL, "C");
}

LaunchParse parse(std::vector<const char*> args, LaunchOptions& o, std::string* error = nullptr,
                  const LaunchExtraFlag& extra = {}) {
  args.insert(args.begin(), "shell");
  return parse_launch_args(static_cast<int>(args.size()), args.data(), o, error, extra);
}

void test_launch_args() {
  LaunchOptions o;
  std::string error;
  CHECK(parse({"--dylib", "app.so", "--state", "s.nvestate", "--schema", "3", "--width", "800", "--height", "600",
               "--reset-state"},
              o, &error) == LaunchParse::Run);
  CHECK(o.dylib_path == "app.so" && o.state_path == "s.nvestate" && o.schema_version == 3u && o.width == 800 &&
        o.height == 600 && o.reset_state && !o.no_state);
  LaunchOptions p;
  CHECK(parse({"--pair", "nve://h:1", "--pair-token", "t", "--no-state"}, p) == LaunchParse::Run);
  CHECK(p.pair_uri == "nve://h:1" && p.pair_token == "t" && p.no_state && p.pair_transport.empty());
  LaunchOptions pt;
  CHECK(parse({"--pair", "NVE://pair?host=127.0.0.1&quic=8091&wss=8092&transport=ws", "--pair-transport", "wt"},
              pt) == LaunchParse::Run);
  CHECK(pt.pair_transport == "webtransport");
  CHECK(pt.pair_uri == "nve://pair?host=127.0.0.1&quic=8091&wss=8092&transport=webtransport");
  LaunchOptions pt_auto;
  CHECK(parse({"--pair", "nve://pair?host=127.0.0.1&quic=8091&transport=wss", "--pair-transport", "auto"},
              pt_auto) == LaunchParse::Run);
  CHECK(pt_auto.pair_transport == "auto");
  CHECK(pt_auto.pair_uri == "nve://pair?host=127.0.0.1&quic=8091");
  LaunchOptions pt_scheme;
  CHECK(parse({"--pair", "wss://127.0.0.1:8092/nve-wire?token=t", "--pair-transport", "quic"}, pt_scheme) ==
        LaunchParse::Run);
  CHECK(pt_scheme.pair_uri == "quic://127.0.0.1:8092/nve-wire?token=t");
  LaunchOptions h;
  CHECK(parse({"--width", "10", "-h", "--bogus"}, h) == LaunchParse::Help);
  // Errors: unknown flag, missing / invalid / out-of-range values, exclusive sources, bad pair transport.
  for (const std::vector<const char*>& bad : std::vector<std::vector<const char*>>{
           {"--bogus"}, {"--dylib"}, {"--width", "0"}, {"--width", "12px"}, {"--height", "-3"}, {"--schema", "x"},
           {"--dylib", "a", "--pair", "nve://x"}, {"--pair-transport"}, {"--pair-transport", "gopher"},
           {"--pair", "ftp://127.0.0.1:21", "--pair-transport", "quic"}}) {
    LaunchOptions e;
    error.clear();
    CHECK(parse(bad, e, &error) == LaunchParse::Error && !error.empty());
  }
  // Host-specific flags.
  LaunchOptions x;
  std::string shot;
  const LaunchExtraFlag extra = [&shot](std::string_view flag, const char* value) {
    if (flag != "--screenshot" || value == nullptr) return 0;
    shot = value;
    return 2;
  };
  CHECK(parse({"--screenshot", "out.bmp", "--width", "5"}, x, nullptr, extra) == LaunchParse::Run);
  CHECK(shot == "out.bmp" && x.width == 5);
  CHECK(parse({"--screenshot"}, x, nullptr, extra) == LaunchParse::Error);
#if defined(_WIN32)
  // `wmain`: UTF-16 paths survive any code page; lone surrogates are rejected.
  LaunchOptions wo;
  const wchar_t* wargs[] = {L"shell", L"--dylib", L"C:\\Apps\\\u00e9t\u00e9\\\u65e5\u672c.dll"};
  CHECK(parse_launch_args(3, wargs, wo, &error) == LaunchParse::Run);
  CHECK(wo.dylib_path == "C:\\Apps\\\xC3\xA9t\xC3\xA9\\\xE6\x97\xA5\xE6\x9C\xAC.dll");
  const wchar_t bad_arg[] = {L'-', L'-', static_cast<wchar_t>(0xD800), 0};
  const wchar_t* bad[] = {L"shell", bad_arg};
  LaunchOptions wb;
  CHECK(parse_launch_args(2, bad, wb, &error) == LaunchParse::Error);
#endif
}

void test_launch_state() {
  set_env("NVE_STATE_DISABLE", nullptr);
  set_env("NVE_STATE_PATH", nullptr);
  set_env("NVE_RESET_STATE", nullptr);
  // Default: a per-app cache path; restore after the requested app loaded.
  LaunchOptions o;
  resolve_launch_state(o, "builtin-app");
  CHECK(!o.no_state && o.state_path.find("builtin-app") != std::string::npos);
  CHECK(state_action_after_load(o, true) == StateAction::Restore);
  // A failed --dylib that fell back to the built-in app: nothing restored, cleared, or saved.
  LaunchOptions f;
  f.dylib_path = "/nope/app.so";
  f.reset_state = true;
  resolve_launch_state(f, "builtin-app");
  CHECK(!f.no_state && f.state_path.find("app") != std::string::npos);
  CHECK(state_action_after_load(f, false) == StateAction::None && f.no_state);
  // --reset-state clears only once the requested app runs.
  LaunchOptions r;
  r.reset_state = true;
  resolve_launch_state(r, "builtin-app");
  CHECK(state_action_after_load(r, true) == StateAction::Clear);
  // Env with --state: NVE_STATE_DISABLE wins; NVE_RESET_STATE resets; NVE_STATE_PATH yields to --state.
  set_env("NVE_STATE_DISABLE", "1");
  LaunchOptions d;
  d.state_path = "explicit.nvestate";
  resolve_launch_state(d, "builtin-app");
  CHECK(d.no_state && state_action_after_load(d, true) == StateAction::None);
  set_env("NVE_STATE_DISABLE", nullptr);
  set_env("NVE_RESET_STATE", "1");
  set_env("NVE_STATE_PATH", "env.nvestate");
  LaunchOptions e;
  e.state_path = "explicit.nvestate";
  resolve_launch_state(e, "builtin-app");
  CHECK(e.state_path == "explicit.nvestate" && e.reset_state && state_action_after_load(e, true) == StateAction::Clear);
  LaunchOptions v;
  resolve_launch_state(v, "builtin-app");
  CHECK(v.state_path == "env.nvestate");
  // Paired apps keep their state on the server.
  LaunchOptions p;
  p.pair_uri = "nve://h:1";
  p.state_path = "explicit.nvestate";
  resolve_launch_state(p, "builtin-app");
  CHECK(p.no_state && state_action_after_load(p, true) == StateAction::None);
  set_env("NVE_RESET_STATE", nullptr);
  set_env("NVE_STATE_PATH", nullptr);
}

void test_pair_transport_and_aria_relations() {
  CHECK(canonical_pair_transport("auto").value_or("") == "auto");
  CHECK(canonical_pair_transport("HAPPY-EYEBALLS").value_or("") == "auto");
  CHECK(canonical_pair_transport("udp").value_or("") == "quic");
  CHECK(canonical_pair_transport("h3").value_or("") == "webtransport");
  CHECK(canonical_pair_transport("TLS").value_or("") == "wss");
  CHECK(canonical_pair_transport("tcp").value_or("") == "ws");
  CHECK(!canonical_pair_transport("sctp").has_value());
  CHECK(!canonical_pair_transport("quic\n").has_value());

  CHECK(apply_pair_transport("127.0.0.1:8091", "wss").value_or("") == "wss://127.0.0.1:8091");
  CHECK(apply_pair_transport("[::1]:8091", "wt").value_or("") == "wt://[::1]:8091");
  CHECK(apply_pair_transport("quic://127.0.0.1:8091/nve-wire", "auto").value_or("") == "127.0.0.1:8091/nve-wire");
  CHECK(apply_pair_transport("NVE://pair?host=h&quic=1", "").value_or("") == "nve://pair?host=h&quic=1");
  CHECK(!apply_pair_transport("http://127.0.0.1:80", "ws").has_value());
  CHECK(!apply_pair_transport("nve://pair?host=h\x07", "quic").has_value());
  CHECK(!apply_pair_transport("", "quic").has_value());

  // `AriaRelations` resource kind matches generated `nve.res.aria_relations` hash.
  CHECK(kAriaRelationsKind.value == 0x099DBC073D018263ULL);
  const AriaRelations src{"panel-1", "lbl-a lbl-b", "desc-1", "opt-2"};
  const std::vector<uint8_t> encoded = encode_aria_relations(src);
  CHECK((encoded.size() & 3u) == 0u);
  const std::optional<AriaRelations> decoded = decode_aria_relations(encoded);
  CHECK(decoded.has_value() && *decoded == src && !decoded->empty());

  const std::optional<AriaRelations> empty = decode_aria_relations(encode_aria_relations(AriaRelations{}));
  CHECK(empty.has_value() && empty->empty());

  // Reject unaligned bytes, non-zero padding, truncated payload, trailing bytes, and bad UTF-8.
  CHECK(!decode_aria_relations(std::span<const uint8_t>(encoded.data(), encoded.size() - 1u)).has_value());
  std::vector<uint8_t> bad_pad = encode_aria_relations(AriaRelations{"a", "", "", ""});
  CHECK(bad_pad.size() == 20u);
  bad_pad.back() = 1u;
  CHECK(!decode_aria_relations(bad_pad).has_value());
  std::vector<uint8_t> trailing = encoded;
  trailing.insert(trailing.end(), 4u, 0u);
  CHECK(!decode_aria_relations(trailing).has_value());
  std::vector<uint8_t> bad_utf8 = encode_aria_relations(AriaRelations{"\xFF", "", "", ""});
  CHECK(!decode_aria_relations(bad_utf8).has_value());
}

}  // namespace

int main() {
  test_openable_link();
  test_local_image_path();
  test_policies_are_locale_independent();
  test_launch_args();
  test_launch_state();
  test_pair_transport_and_aria_relations();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_policy_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_policy_test: ok\n");
  return 0;
}
