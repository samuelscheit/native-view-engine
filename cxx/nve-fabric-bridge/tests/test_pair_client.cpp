#include "nve_pair_client.hpp"

#include <atomic>
#include <chrono>
#include <cstring>
#include <iostream>
#include <mutex>
#include <thread>
#include <vector>

#if defined(_WIN32)
#include <winsock2.h>
#include <ws2tcpip.h>
using TestSock = SOCKET;
constexpr TestSock kBadSock = INVALID_SOCKET;
#else
#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>
using TestSock = int;
constexpr TestSock kBadSock = -1;
#endif

namespace {

int g_failures = 0;

#define CHECK(expr)                                                                     \
  do {                                                                                  \
    if (!(expr)) {                                                                      \
      std::cerr << "FAIL [" << __FILE__ << ":" << __LINE__ << "]: " #expr << "\n";      \
      ++g_failures;                                                                     \
    }                                                                                   \
  } while (0)

void close_sock(TestSock fd) {
  if (fd == kBadSock) return;
#if defined(_WIN32)
  ::closesocket(fd);
#else
  ::close(fd);
#endif
}

bool send_all_sock(TestSock fd, std::span<const uint8_t> bytes) {
  size_t off = 0;
  while (off < bytes.size()) {
#if defined(_WIN32)
    const int n = ::send(fd, reinterpret_cast<const char*>(bytes.data() + off),
                         static_cast<int>(bytes.size() - off), 0);
#else
    const ssize_t n = ::send(fd, bytes.data() + off, bytes.size() - off, 0);
#endif
    if (n <= 0) return false;
    off += static_cast<size_t>(n);
  }
  return true;
}

bool recv_exact_sock(TestSock fd, uint8_t* out, size_t len) {
  size_t off = 0;
  while (off < len) {
#if defined(_WIN32)
    const int n = ::recv(fd, reinterpret_cast<char*>(out + off), static_cast<int>(len - off), 0);
#else
    const ssize_t n = ::recv(fd, out + off, len - off, 0);
#endif
    if (n <= 0) return false;
    off += static_cast<size_t>(n);
  }
  return true;
}

bool read_frame_sock(TestSock fd, std::vector<uint8_t>& buf, nve::WireFrameView& out) {
  std::array<uint8_t, nve::kWireHeaderSize> hdr_bytes{};
  if (!recv_exact_sock(fd, hdr_bytes.data(), hdr_bytes.size())) return false;
  nve::WireHeader hdr{};
  if (!nve::wire_v2::decode_header(hdr_bytes, hdr)) return false;
  buf.resize(hdr.total_wire_len());
  std::memcpy(buf.data(), hdr_bytes.data(), nve::kWireHeaderSize);
  if (buf.size() > nve::kWireHeaderSize) {
    if (!recv_exact_sock(fd, buf.data() + nve::kWireHeaderSize, buf.size() - nve::kWireHeaderSize)) {
      return false;
    }
  }
  return nve::wire_v2::decode_frame(buf, out);
}

void test_wire_v2_codec_and_guards() {
  const std::vector<uint8_t> payload{'d', 'e', 'l', 't', 'a'};
  auto encoded = nve::wire_v2::encode_frame(
      nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 42u, payload);
  CHECK(encoded.size() == nve::kWireHeaderSize + payload.size() + nve::kWireTrailerSize);

  nve::WireFrameView view{};
  CHECK(nve::wire_v2::decode_frame(encoded, view));
  CHECK(view.header.channel == nve::WireChannel::UiScene);
  CHECK(view.header.kind == nve::WireKind::CommandDelta);
  CHECK(view.header.sequence == 42u);
  CHECK(view.payload.size() == payload.size());

  // Flip 1 bit -> XXH64 checksum rejects.
  encoded[nve::kWireHeaderSize] ^= 0x01u;
  CHECK(!nve::wire_v2::decode_frame(encoded, view));

  // Oversized payload_len rejected BEFORE allocation.
  std::array<uint8_t, nve::kWireHeaderSize> oom{};
  oom[0] = 'N'; oom[1] = 'W'; oom[2] = 2u; oom[3] = 1u; oom[4] = 5u;
  oom[12] = 0xFFu; oom[13] = 0xFFu; oom[14] = 0xFFu; oom[15] = 0x0Fu;
  nve::WireHeader hdr{};
  CHECK(!nve::wire_v2::decode_header(oom, hdr, 1024u));

  // URI parser tests.
  nve::PairClientConfig cfg{};
  std::string err;
  CHECK(nve::parse_pair_endpoint(
      "nve://pair?host=127.0.0.1&quic=8091&wss=8092&token=nve_abc&cert_hash=deadbeef", "", cfg, &err));
  CHECK(cfg.host == "127.0.0.1");
  CHECK(cfg.port == 8092u);
  CHECK(cfg.pairing_token == "nve_abc");
  CHECK(cfg.cert_hash_hex == "deadbeef");
  CHECK(nve::parse_pair_endpoint("quic://127.0.0.1:9001", "", cfg, &err));
  CHECK(cfg.transport == nve::PairTransport::Quic && cfg.port == 9001u);

  // Constant-time compare.
  CHECK(nve::wire_v2::constant_time_eq("secret-token", "secret-token"));
  CHECK(!nve::wire_v2::constant_time_eq("secret-token", "secret-tokeN"));
  CHECK(!nve::wire_v2::constant_time_eq("short", "longer"));
}

void test_live_loopback_handshake_command_delta_and_event_batch() {
  TestSock listen_fd = ::socket(AF_INET, SOCK_STREAM, 0);
  CHECK(listen_fd != kBadSock);
  int reuse = 1;
  (void)::setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, reinterpret_cast<const char*>(&reuse), sizeof(reuse));
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = 0;
  CHECK(::bind(listen_fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
  socklen_t alen = sizeof(addr);
  CHECK(::getsockname(listen_fd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
  const uint16_t port = ntohs(addr.sin_port);
  CHECK(::listen(listen_fd, 4) == 0);

  const std::string expected_token = "nve_cxx_bearer_secret";
  std::atomic<bool> saw_event{false};

  std::thread srv([&]() {
    // 1. First connection: bad token -> Reject
    TestSock c1 = ::accept(listen_fd, nullptr, nullptr);
    if (c1 != kBadSock) {
      std::vector<uint8_t> buf;
      nve::WireFrameView v{};
      if (read_frame_sock(c1, buf, v) && v.header.kind == nve::WireKind::Hello) {
        auto rej = nve::wire_v2::encode_frame(
            nve::WireChannel::Control,
            nve::WireKind::Reject,
            1u,
            nve::wire_v2::encode_reject(nve::WireRejectReason::Unauthorized, "bad bearer token"));
        (void)send_all_sock(c1, rej);
      }
      close_sock(c1);
    }

    // 2. Second connection: valid token -> HelloAck + CommandDelta -> read EventBatch
    TestSock c2 = ::accept(listen_fd, nullptr, nullptr);
    if (c2 != kBadSock) {
      std::vector<uint8_t> buf;
      nve::WireFrameView v{};
      if (read_frame_sock(c2, buf, v) && v.header.kind == nve::WireKind::Hello) {
        nve::WireHello h{};
        CHECK(nve::wire_v2::decode_hello(v.payload, h));
        CHECK(nve::wire_v2::constant_time_eq(h.bearer_token, expected_token));
        const auto ack = nve::wire_v2::encode_hello_ack({77u, 2u, 0x0Fu});
        CHECK(send_all_sock(
            c2, nve::wire_v2::encode_frame(nve::WireChannel::Control, nve::WireKind::HelloAck, 1u, ack)));
        const std::vector<uint8_t> delta{'N', 'V', 'E', '1', 1, 2, 3, 4};
        CHECK(send_all_sock(
            c2, nve::wire_v2::encode_frame(nve::WireChannel::UiScene, nve::WireKind::CommandDelta, 5u, delta)));
        while (read_frame_sock(c2, buf, v)) {
          if (v.header.kind == nve::WireKind::EventBatch) {
            saw_event.store(true);
            break;
          }
          if (v.header.kind == nve::WireKind::Bye) break;
        }
      }
      close_sock(c2);
    }
    close_sock(listen_fd);
  });

  // Unauthorized token rejected cleanly.
  nve::PairClient bad_client;
  nve::PairClientConfig bad_cfg{};
  bad_cfg.host = "127.0.0.1";
  bad_cfg.port = port;
  bad_cfg.pairing_token = "wrong-token";
  std::string err;
  CHECK(!bad_client.connect(bad_cfg, {}, &err));
  CHECK(err.find("rejected") != std::string::npos);

  // Authorized token succeeds & round-trips CommandDelta + EventBatch.
  nve::PairClient client;
  nve::PairClientConfig ok_cfg{};
  ok_cfg.host = "127.0.0.1";
  ok_cfg.port = port;
  ok_cfg.pairing_token = expected_token;
  std::atomic<uint32_t> wakes{0};
  CHECK(client.connect(ok_cfg, [&]() { wakes.fetch_add(1u); }, &err));
  CHECK(client.is_connected());
  CHECK(client.session_id() == 77u);

  std::vector<uint8_t> frame;
  uint32_t seq = 0;
  for (int i = 0; i < 100 && !client.pop_frame(frame, &seq); ++i) {
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
  }
  CHECK(seq == 5u);
  CHECK(frame.size() == 8u);
  CHECK(wakes.load() >= 1u);

  const std::vector<uint8_t> ev{'N', 'V', 'E', 'V', 0, 0};
  CHECK(client.send_events(ev, &err));
  for (int i = 0; i < 100 && !saw_event.load(); ++i) {
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
  }
  CHECK(saw_event.load());

  client.disconnect();
  srv.join();
}

} // namespace

int main() {
  test_wire_v2_codec_and_guards();
  test_live_loopback_handshake_command_delta_and_event_batch();
  if (g_failures > 0) {
    std::cerr << g_failures << " pair_client_test failure(s)\n";
    return 1;
  }
  std::cout << "pair_client_test: ALL PASSED (NveWireFrame v2)\n";
  return 0;
}
