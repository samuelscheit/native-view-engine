// C++20 end-to-end link-and-run test against the real compiled `libnve_runtime` static library.
//
// Exercises header include-order safety (`api.h` before `runtime_session.hpp`), the single
// exported `nve_runtime_get_interface` entry point, RAII `RuntimeSession`, real binary `NVE2`
// dynamic table + text + hierarchy submission, zero-copy `RuntimeSceneGuard` / `RuntimeSceneReader`
// + `TableReader` + post-release/move/self-move empty-span safety, `NVE_RUNTIME_BUSY` exclusion,
// owner-thread `hit_test` under active reader lock, `set_viewport` + `tick`, `get_metrics`, live
// `set_wake_callback` firing (`g_wake_count > 0`), exact pairing URI/pin validation, live loopback
// WebSocket pairing roundtrip, `last_error`, move semantics, and clean RAII `destroy` under ASan/UBSan.

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <span>
#include <string>
#include <string_view>
#include <thread>
#include <utility>
#include <vector>

#if !defined(_WIN32)
#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

#include "check.h"
#include "nve/host/nve2_builder.hpp"
// Include `api.h` BEFORE `runtime_session.hpp` to verify `NVE_RUNTIME_API` and
// `NVE_STATIC_ASSERT` never clash with `NVE_API` regardless of include order.
#include "nve/host/api.h"
#include "nve/host/runtime_session.hpp"

namespace {

using nve::host::field;
using nve::host::Key64;
using nve::host::RuntimeSceneGuard;
using nve::host::RuntimeSceneReader;
using nve::host::RuntimeSession;
using nve::host::SceneError;
using nve::host::TableReader;


void push_u32(std::vector<uint8_t>& out, uint32_t v) {
  for (unsigned shift = 0u; shift < 32u; shift += 8u) out.push_back(static_cast<uint8_t>(v >> shift));
}

/// Builds a valid binary `NVE2` frame declaring dynamic sparse table `"cxx.demo.badge"` (3 words:
/// word 0 = u32 badge_count, words 1..2 = StrSpan label), creating ROOT(1) + child(2), patching
/// badge_count = `badge_value`, setting UTF-8 label = `label`, and attaching child(2) under ROOT(1).
std::vector<uint8_t> build_demo_nve2(uint32_t badge_value, std::string_view label) {
  constexpr uint8_t kWireSlot = 40u;
  constexpr std::array<uint32_t, 3> kDefaults{0u, 0u, 0u};
  constexpr std::array<uint8_t, 3> kEffects{1u, 1u, 1u};
  const std::array<uint32_t, 3> row{badge_value, 0u, 0u};
  return nve::host::Nve2FrameBuilder()
      .declare({Key64::of("cxx.demo.badge"), Key64::of("cxx.demo.badge.schema.v1"), kWireSlot, true /* sparse */,
                2u /* PUBLISH_TO_SCENE */, kDefaults, kEffects, uint32_t{1} << 1u /* words 1..2 = StrSpan label */,
                0u, "cxx.demo.badge"})
      .create_nodes({1u, 2u})
      .patch(kWireSlot, 2u, 1u, row)
      .set_string(kWireSlot, 2u, 1u, label)
      .set_children(1u, {2u})
      .finish();
}

#if !defined(_WIN32) && defined(NVE_RUNTIME_TEST_HAS_WIRE)
// Minimal RFC 6455 loopback WebSocket responder for live `pair_connect` + `WakeFn` verification.
uint32_t rotl32(uint32_t x, unsigned n) { return (x << n) | (x >> (32u - n)); }

std::array<uint8_t, 20> sha1_digest(std::string_view msg) {
  std::vector<uint8_t> data(msg.begin(), msg.end());
  const uint64_t bit_len = static_cast<uint64_t>(data.size()) * 8u;
  data.push_back(0x80u);
  while ((data.size() % 64u) != 56u) data.push_back(0u);
  for (int i = 7; i >= 0; --i) {
    data.push_back(static_cast<uint8_t>((bit_len >> (static_cast<unsigned>(i) * 8u)) & 0xFFu));
  }
  uint32_t h0 = 0x67452301u, h1 = 0xEFCDAB89u, h2 = 0x98BADCFEu, h3 = 0x10325476u, h4 = 0xC3D2E1F0u;
  for (size_t chunk = 0; chunk < data.size(); chunk += 64u) {
    uint32_t w[80]{};
    for (size_t i = 0; i < 16u; ++i) {
      w[i] = (static_cast<uint32_t>(data[chunk + i * 4u]) << 24u) |
             (static_cast<uint32_t>(data[chunk + i * 4u + 1u]) << 16u) |
             (static_cast<uint32_t>(data[chunk + i * 4u + 2u]) << 8u) |
             static_cast<uint32_t>(data[chunk + i * 4u + 3u]);
    }
    for (size_t i = 16u; i < 80u; ++i) w[i] = rotl32(w[i - 3u] ^ w[i - 8u] ^ w[i - 14u] ^ w[i - 16u], 1u);
    uint32_t a = h0, b = h1, c = h2, d = h3, e = h4;
    for (size_t i = 0; i < 80u; ++i) {
      uint32_t f = 0u, k = 0u;
      if (i < 20u) {
        f = (b & c) | ((~b) & d);
        k = 0x5A827999u;
      } else if (i < 40u) {
        f = b ^ c ^ d;
        k = 0x6ED9EBA1u;
      } else if (i < 60u) {
        f = (b & c) | (b & d) | (c & d);
        k = 0x8F1BBCDCu;
      } else {
        f = b ^ c ^ d;
        k = 0xCA62C1D6u;
      }
      const uint32_t tmp = rotl32(a, 5u) + f + e + k + w[i];
      e = d;
      d = c;
      c = rotl32(b, 30u);
      b = a;
      a = tmp;
    }
    h0 += a;
    h1 += b;
    h2 += c;
    h3 += d;
    h4 += e;
  }
  std::array<uint8_t, 20> out{};
  const uint32_t hs[5] = {h0, h1, h2, h3, h4};
  for (size_t i = 0; i < 5u; ++i) {
    out[i * 4u + 0u] = static_cast<uint8_t>((hs[i] >> 24u) & 0xFFu);
    out[i * 4u + 1u] = static_cast<uint8_t>((hs[i] >> 16u) & 0xFFu);
    out[i * 4u + 2u] = static_cast<uint8_t>((hs[i] >> 8u) & 0xFFu);
    out[i * 4u + 3u] = static_cast<uint8_t>(hs[i] & 0xFFu);
  }
  return out;
}

std::string base64_encode(std::span<const uint8_t> in) {
  static constexpr char kTab[] =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  std::string out;
  for (size_t i = 0; i < in.size(); i += 3u) {
    const uint32_t b0 = in[i];
    const uint32_t b1 = (i + 1u < in.size()) ? in[i + 1u] : 0u;
    const uint32_t b2 = (i + 2u < in.size()) ? in[i + 2u] : 0u;
    const uint32_t v = (b0 << 16u) | (b1 << 8u) | b2;
    out.push_back(kTab[(v >> 18u) & 63u]);
    out.push_back(kTab[(v >> 12u) & 63u]);
    out.push_back((i + 1u < in.size()) ? kTab[(v >> 6u) & 63u] : '=');
    out.push_back((i + 2u < in.size()) ? kTab[v & 63u] : '=');
  }
  return out;
}

std::vector<uint8_t> build_wire_frame(uint8_t channel, uint8_t kind, uint32_t seq,
                                      std::span<const uint8_t> payload) {
  std::vector<uint8_t> out;
  out.reserve(16u + payload.size());
  out.push_back(static_cast<uint8_t>('N'));
  out.push_back(static_cast<uint8_t>('W'));
  out.push_back(2u);  // NVE_WIRE_VERSION
  out.push_back(channel);
  out.push_back(kind);
  out.push_back(0u);  // flags
  out.push_back(0u);  // reserved lo
  out.push_back(0u);  // reserved hi
  push_u32(out, seq);
  push_u32(out, static_cast<uint32_t>(payload.size()));
  out.insert(out.end(), payload.begin(), payload.end());
  return out;
}

void write_all_fd(int fd, const void* buf, size_t len) {
  const auto* p = static_cast<const uint8_t*>(buf);
  size_t off = 0u;
  while (off < len) {
    const ssize_t n = ::send(fd, p + off, len - off, 0);
    if (n <= 0) break;
    off += static_cast<size_t>(n);
  }
}

void send_ws_binary(int fd, std::span<const uint8_t> payload) {
  std::vector<uint8_t> hdr;
  hdr.push_back(0x82u);  // FIN | binary opcode
  if (payload.size() < 126u) {
    hdr.push_back(static_cast<uint8_t>(payload.size()));
  } else {
    hdr.push_back(126u);
    hdr.push_back(static_cast<uint8_t>((payload.size() >> 8u) & 0xFFu));
    hdr.push_back(static_cast<uint8_t>(payload.size() & 0xFFu));
  }
  write_all_fd(fd, hdr.data(), hdr.size());
  write_all_fd(fd, payload.data(), payload.size());
}
#endif

std::atomic<int> g_wake_count{0};

extern "C" void test_wake_cb(void* ctx) {
  auto* counter = static_cast<std::atomic<int>*>(ctx);
  counter->fetch_add(1, std::memory_order_release);
}

void test_live_runtime_session() {
  // ABI version mismatch returns NULL; matching version returns a complete vtable.
  CHECK(nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION + 1u) == nullptr);
  const NveRuntimeApi* api = nve_runtime_get_interface(NVE_RUNTIME_ABI_VERSION);
  CHECK(api != nullptr);
  CHECK(RuntimeSession::validate_api(api));

  const NveRuntimeConfig cfg = RuntimeSession::default_config(1u, 400.0f, 300.0f, 1.0f);
  std::optional<RuntimeSession> opt_session = RuntimeSession::create(api, cfg);
  CHECK(opt_session.has_value());

  // Exercise move semantics cleanly.
  RuntimeSession session = std::move(*opt_session);
  CHECK(session.valid());
  CHECK(!opt_session->valid());

  // Install and clear wake callback cleanly.
  g_wake_count.store(0, std::memory_order_release);
  CHECK(session.set_wake_callback(test_wake_cb, &g_wake_count) == NVE_RUNTIME_OK);
  CHECK(session.clear_wake_callback() == NVE_RUNTIME_OK);

  // Submit real binary NVE2 frame with dynamic table, node hierarchy, and UTF-8 string.
  const std::vector<uint8_t> frame = build_demo_nve2(42u, "Hello C++20 Runtime!");
  CHECK(session.submit_nve2(frame) == NVE_RUNTIME_OK);

  // Acquire zero-copy scene view under RAII guard + verify move/release empty-span safety.
  {
    NveRuntimeStatus st = NVE_RUNTIME_ERROR;
    SceneError serr = SceneError::NullView;
    std::optional<RuntimeSceneGuard> opt_guard = session.acquire_scene(&st, &serr);
    CHECK(opt_guard.has_value());
    CHECK(st == NVE_RUNTIME_OK);
    CHECK(serr == SceneError::None);

    // Move-construct guard; moved-from guard must clear raw_ & reader_ spans immediately.
    RuntimeSceneGuard guard = std::move(*opt_guard);
    CHECK(!opt_guard->active());
    CHECK(opt_guard->reader().rows().empty() && opt_guard->reader().rects().empty());
    CHECK(opt_guard->raw().rows == nullptr && opt_guard->raw().node_capacity == 0u);

    // Self-move assignment must preserve live guard cleanly.
    RuntimeSceneGuard* self_ptr = &guard;
    guard = std::move(*self_ptr);
    CHECK(guard.active());

    const RuntimeSceneReader& scene = guard.reader();
    CHECK(scene.frame_id() == 1u);
    CHECK(scene.node_capacity() >= 3u);
    CHECK(scene.is_live(1u));
    CHECK(scene.is_live(2u));

    const NveLayoutRect* root_rect = scene.rect(1u);
    CHECK(root_rect != nullptr);
    CHECK(root_rect->width == 400.0f);
    CHECK(root_rect->height == 300.0f);

    const auto root_kids = scene.node_children(1u);
    CHECK(root_kids.has_value() && root_kids->size() == 1u && (*root_kids)[0] == 2u);

    const std::optional<TableReader> badge = scene.find_table(Key64::of("cxx.demo.badge"));
    CHECK(badge.has_value() && badge->is_sparse() && badge->row_count() == 1u);
    if (badge.has_value()) {
      const std::optional<uint32_t> row = badge->row_of(2u);
      CHECK(row.has_value());
      if (row.has_value()) {
        CHECK(badge->u32(*row, field(0, 0, 32, NVE_FIELD_U32)) == 42u);
        const auto str_span = badge->str(*row, field(1, 0, 32, NVE_FIELD_STR));
        CHECK(str_span.has_value());
        if (str_span.has_value()) {
          CHECK(scene.text(str_span->offset, str_span->len) == "Hello C++20 Runtime!");
        }
      }
    }

    // While reader guard is held: mutating `tick` MUST return `NVE_RUNTIME_BUSY`,
    // while owner-thread `hit_test` MUST succeed (0 = no Interaction table subscribed).
    bool busy_changed = false;
    CHECK(session.tick(0.016f, &busy_changed) == NVE_RUNTIME_BUSY);
    uint32_t hit_id = 999u;
    CHECK(session.hit_test(20.0f, 20.0f, &hit_id) == NVE_RUNTIME_OK);
    CHECK(hit_id == 0u);

    // Explicit release() clears spans and releases the reader lock immediately.
    guard.release();
    CHECK(!guard.active());
    CHECK(guard.reader().rows().empty() && guard.reader().rects().empty());
    CHECK(guard.raw().rows == nullptr && guard.raw().node_capacity == 0u);
  }

  // After releasing guard: `set_viewport` + `tick` advances `frame_id` and updates ROOT rect.
  CHECK(session.set_viewport(640.0f, 480.0f, 2.0f) == NVE_RUNTIME_OK);
  bool changed = false;
  CHECK(session.tick(0.016f, &changed) == NVE_RUNTIME_OK);
  CHECK(changed);

  NveRuntimeMetrics metrics{};
  CHECK(session.metrics(&metrics) == NVE_RUNTIME_OK);
  CHECK(metrics.frame_id == 2u);
  CHECK(metrics.live_nodes == 2u);
  CHECK(metrics.pair_connected == 0u);

  {
    auto guard2 = session.acquire_scene();
    CHECK(guard2.has_value());
    const NveLayoutRect* r1 = (*guard2)->rect(1u);
    CHECK(r1 != nullptr && r1->width == 640.0f && r1->height == 480.0f);
  }

  // Exact pairing URI + TLS cert-pin validation + last_error roundtrip.
#if defined(NVE_RUNTIME_TEST_HAS_WIRE)
  CHECK(session.pair_connect("", "") == NVE_RUNTIME_INVALID_ARG);
  CHECK(!session.last_error().empty());
  CHECK(session.pair_connect("not-a-url", "tok") == NVE_RUNTIME_INVALID_ARG);
  CHECK(session.pair_connect("wss://127.0.0.1:8092/nve-wire", "tok") == NVE_RUNTIME_INVALID_ARG);
  CHECK(!session.pair_is_connected());
  CHECK(session.pair_disconnect() == NVE_RUNTIME_OK);
#else
  CHECK(session.pair_connect("", "") == NVE_RUNTIME_UNSUPPORTED);
  CHECK(!session.last_error().empty());
  CHECK(!session.pair_is_connected());
  CHECK(session.pair_disconnect() == NVE_RUNTIME_UNSUPPORTED);
#endif

#if !defined(_WIN32) && defined(NVE_RUNTIME_TEST_HAS_WIRE)
  // Live loopback WebSocket pairing roundtrip verifying `g_wake_count > 0` + remote scene sync!
  {
    const int srv_fd = ::socket(AF_INET, SOCK_STREAM, 0);
    CHECK(srv_fd >= 0);
    int opt = 1;
    (void)::setsockopt(srv_fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));
    timeval tv{};
    tv.tv_sec = 2;
    tv.tv_usec = 0;
    (void)::setsockopt(srv_fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = 0u;
    CHECK(::bind(srv_fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
    socklen_t alen = sizeof(addr);
    CHECK(::getsockname(srv_fd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
    CHECK(::listen(srv_fd, 2) == 0);
    const uint16_t port = ntohs(addr.sin_port);

    std::atomic<bool> stop_srv{false};
    std::thread srv_thread([srv_fd, &stop_srv]() {
      const int cfd = ::accept(srv_fd, nullptr, nullptr);
      if (cfd < 0) return;
      timeval ctv{};
      ctv.tv_sec = 2;
      ctv.tv_usec = 0;
      (void)::setsockopt(cfd, SOL_SOCKET, SO_RCVTIMEO, &ctv, sizeof(ctv));
      std::string req;
      char buf[1024];
      while (req.find("\r\n\r\n") == std::string::npos) {
        const ssize_t n = ::recv(cfd, buf, sizeof(buf), 0);
        if (n <= 0) break;
        req.append(buf, static_cast<size_t>(n));
      }
      const std::string key_hdr = "Sec-WebSocket-Key:";
      std::string ws_key;
      const size_t pos = req.find(key_hdr);
      if (pos != std::string::npos) {
        size_t s = pos + key_hdr.size();
        while (s < req.size() && req[s] == ' ') ++s;
        const size_t e = req.find("\r\n", s);
        if (e != std::string::npos) ws_key = req.substr(s, e - s);
      }
      const auto digest = sha1_digest(ws_key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
      const std::string accept_key = base64_encode(digest);
      const std::string resp =
          "HTTP/1.1 101 Switching Protocols\r\n"
          "Upgrade: websocket\r\n"
          "Connection: Upgrade\r\n"
          "Sec-WebSocket-Accept: " + accept_key + "\r\n\r\n";
      write_all_fd(cfd, resp.data(), resp.size());

      // Read client Hello binary WS frame (best effort; non-empty).
      (void)::recv(cfd, buf, sizeof(buf), 0);

      // Send HelloAck (channel=0 Control, kind=2 HelloAck, 12B payload: sid=1, schema=2, caps=0).
      std::vector<uint8_t> ack_payload;
      push_u32(ack_payload, 1u);
      ack_payload.push_back(2u);
      ack_payload.push_back(0u);
      ack_payload.push_back(0u);
      ack_payload.push_back(0u);
      push_u32(ack_payload, 0u);
      const auto ack_wire = build_wire_frame(0u, 2u, 1u, ack_payload);
      send_ws_binary(cfd, ack_wire);

      // Send SceneKeyframe (channel=1 UiScene, kind=4 SceneKeyframe) carrying NVE2 scene.
      const auto nve2 = build_demo_nve2(77u, "Paired C++20!");
      const auto kf_wire = build_wire_frame(1u, 4u, 2u, nve2);
      send_ws_binary(cfd, kf_wire);

      while (!stop_srv.load(std::memory_order_acquire)) {
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
      }
      ::close(cfd);
    });

    g_wake_count.store(0, std::memory_order_release);
    CHECK(session.set_wake_callback(test_wake_cb, &g_wake_count) == NVE_RUNTIME_OK);
    const std::string uri = "ws://127.0.0.1:" + std::to_string(port) + "/nve-wire";
    CHECK(session.pair_connect(uri, "cxx-live-token") == NVE_RUNTIME_OK);

    bool paired_synced = false;
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    while (std::chrono::steady_clock::now() < deadline) {
      bool tick_changed = false;
      CHECK(session.tick(0.016f, &tick_changed) == NVE_RUNTIME_OK);
      if (tick_changed) {
        auto pg = session.acquire_scene();
        if (pg.has_value()) {
          const auto badge = (*pg)->find_table(Key64::of("cxx.demo.badge"));
          if (badge.has_value()) {
            const auto row = badge->row_of(2u);
            if (row.has_value() && badge->u32(*row, field(0, 0, 32, NVE_FIELD_U32)) == 77u) {
              const auto s = badge->str(*row, field(1, 0, 32, NVE_FIELD_STR));
              CHECK(s.has_value() && (*pg)->text(s->offset, s->len) == "Paired C++20!");
              paired_synced = true;
              break;
            }
          }
        }
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    CHECK(paired_synced);
    CHECK(g_wake_count.load(std::memory_order_acquire) > 0);
    CHECK(session.pair_is_connected());

    stop_srv.store(true, std::memory_order_release);
    CHECK(session.pair_disconnect() == NVE_RUNTIME_OK);
    CHECK(!session.pair_is_connected());
    (void)::shutdown(srv_fd, SHUT_RDWR);
    srv_thread.join();
    ::close(srv_fd);
  }
#endif

  // Explicit reset destroys cleanly; second reset is a no-op.
  session.reset();
  CHECK(!session.valid());
  session.reset();
}

}  // namespace

int main() {
  test_live_runtime_session();
  if (g_failures != 0) {
    fprintf(stderr, "nve_host_runtime_session_test: %d failure(s)\n", g_failures);
    return 1;
  }
  printf("nve_host_runtime_session_test: ok\n");
  return 0;
}
