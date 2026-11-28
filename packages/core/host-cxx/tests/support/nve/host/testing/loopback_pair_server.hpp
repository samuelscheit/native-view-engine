// Test support shared by the native host suites: a minimal in-process `nve dev` pairing server.
//
// It speaks just enough of the `ws://` transport `pair_connect` accepts: the RFC 6455 handshake,
// the client's Hello, a `HelloAck` (Control channel), then every `NVE2` frame queued with
// `send_keyframe` as a `SceneKeyframe` (UiScene channel), in order. One client, loopback only.
//
//   LoopbackPairServer server;                       // listening on 127.0.0.1:<ephemeral>
//   server.send_keyframe(writer.finish(1));          // sent once the client has paired
//   host.connect_paired(server.uri(), "token", &e);  // runtime wakes as frames arrive
#pragma once

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <deque>
#include <mutex>
#include <span>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

#if defined(_WIN32)
#include <winsock2.h>
#include <ws2tcpip.h>
#else
#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>
#endif

namespace nve::host::testing {

namespace pair_detail {

#if defined(_WIN32)
using Socket = SOCKET;
inline constexpr Socket kNoSocket = INVALID_SOCKET;
inline void close_socket(Socket s) { ::closesocket(s); }
#else
using Socket = int;
inline constexpr Socket kNoSocket = -1;
inline void close_socket(Socket s) { ::close(s); }
#endif

inline void recv_timeout(Socket s, int ms) {
#if defined(_WIN32)
  const DWORD v = static_cast<DWORD>(ms);
  (void)::setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, reinterpret_cast<const char*>(&v), sizeof(v));
#else
  timeval tv{};
  tv.tv_sec = ms / 1000;
  tv.tv_usec = (ms % 1000) * 1000;
  (void)::setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
#endif
}

inline uint32_t rotl32(uint32_t x, unsigned n) { return (x << n) | (x >> (32u - n)); }

inline std::array<uint8_t, 20> sha1(std::string_view msg) {
  std::vector<uint8_t> data(msg.begin(), msg.end());
  const uint64_t bit_len = static_cast<uint64_t>(data.size()) * 8u;
  data.push_back(0x80u);
  while ((data.size() % 64u) != 56u) data.push_back(0u);
  for (int i = 7; i >= 0; --i) data.push_back(static_cast<uint8_t>(bit_len >> (static_cast<unsigned>(i) * 8u)));
  uint32_t h[5] = {0x67452301u, 0xEFCDAB89u, 0x98BADCFEu, 0x10325476u, 0xC3D2E1F0u};
  for (size_t chunk = 0; chunk < data.size(); chunk += 64u) {
    uint32_t w[80]{};
    for (size_t i = 0; i < 16u; ++i) {
      w[i] = (uint32_t{data[chunk + i * 4u]} << 24u) | (uint32_t{data[chunk + i * 4u + 1u]} << 16u) |
             (uint32_t{data[chunk + i * 4u + 2u]} << 8u) | uint32_t{data[chunk + i * 4u + 3u]};
    }
    for (size_t i = 16u; i < 80u; ++i) w[i] = rotl32(w[i - 3u] ^ w[i - 8u] ^ w[i - 14u] ^ w[i - 16u], 1u);
    uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4];
    for (size_t i = 0; i < 80u; ++i) {
      const uint32_t f = i < 20u ? (b & c) | ((~b) & d) : i < 40u ? b ^ c ^ d : i < 60u ? (b & c) | (b & d) | (c & d) : b ^ c ^ d;
      const uint32_t k = i < 20u ? 0x5A827999u : i < 40u ? 0x6ED9EBA1u : i < 60u ? 0x8F1BBCDCu : 0xCA62C1D6u;
      const uint32_t t = rotl32(a, 5u) + f + e + k + w[i];
      e = d;
      d = c;
      c = rotl32(b, 30u);
      b = a;
      a = t;
    }
    h[0] += a;
    h[1] += b;
    h[2] += c;
    h[3] += d;
    h[4] += e;
  }
  std::array<uint8_t, 20> out{};
  for (size_t i = 0; i < 20u; ++i) out[i] = static_cast<uint8_t>(h[i / 4u] >> (24u - 8u * (i % 4u)));
  return out;
}

inline std::string base64(std::span<const uint8_t> in) {
  static constexpr char kTab[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  std::string out;
  for (size_t i = 0; i < in.size(); i += 3u) {
    const uint32_t v = (uint32_t{in[i]} << 16u) | (i + 1u < in.size() ? uint32_t{in[i + 1u]} << 8u : 0u) |
                       (i + 2u < in.size() ? uint32_t{in[i + 2u]} : 0u);
    out.push_back(kTab[(v >> 18u) & 63u]);
    out.push_back(kTab[(v >> 12u) & 63u]);
    out.push_back(i + 1u < in.size() ? kTab[(v >> 6u) & 63u] : '=');
    out.push_back(i + 2u < in.size() ? kTab[v & 63u] : '=');
  }
  return out;
}

inline void push_u32(std::vector<uint8_t>& out, uint32_t v) {
  for (unsigned s = 0u; s < 32u; s += 8u) out.push_back(static_cast<uint8_t>(v >> s));
}

/// `nve-wire-2` frame: "NW", version 2, channel, kind, flags, reserved u16, seq u32, len u32.
inline std::vector<uint8_t> wire_frame(uint8_t channel, uint8_t kind, uint32_t seq, std::span<const uint8_t> payload) {
  std::vector<uint8_t> out{'N', 'W', 2u, channel, kind, 0u, 0u, 0u};
  push_u32(out, seq);
  push_u32(out, static_cast<uint32_t>(payload.size()));
  out.insert(out.end(), payload.begin(), payload.end());
  return out;
}

inline void send_all(Socket s, std::span<const uint8_t> bytes) {
  size_t off = 0u;
  while (off < bytes.size()) {
#if defined(_WIN32)
    const int n = ::send(s, reinterpret_cast<const char*>(bytes.data() + off), static_cast<int>(bytes.size() - off), 0);
#else
    const ssize_t n = ::send(s, bytes.data() + off, bytes.size() - off, 0);
#endif
    if (n <= 0) return;
    off += static_cast<size_t>(n);
  }
}

/// Unmasked server -> client binary message.
inline void send_ws_binary(Socket s, std::span<const uint8_t> payload) {
  std::vector<uint8_t> msg{0x82u};  // FIN | binary
  if (payload.size() < 126u) {
    msg.push_back(static_cast<uint8_t>(payload.size()));
  } else if (payload.size() <= 0xFFFFu) {
    msg.push_back(126u);
    msg.push_back(static_cast<uint8_t>(payload.size() >> 8u));
    msg.push_back(static_cast<uint8_t>(payload.size()));
  } else {
    msg.push_back(127u);
    for (int i = 7; i >= 0; --i) msg.push_back(static_cast<uint8_t>(uint64_t{payload.size()} >> (8u * static_cast<unsigned>(i))));
  }
  msg.insert(msg.end(), payload.begin(), payload.end());
  send_all(s, msg);
}

}  // namespace pair_detail

class LoopbackPairServer {
 public:
  LoopbackPairServer() {
#if defined(_WIN32)
    WSADATA wsa{};
    wsa_ = ::WSAStartup(MAKEWORD(2, 2), &wsa) == 0;
#endif
    listen_ = ::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (listen_ == pair_detail::kNoSocket) return;
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = 0u;
    socklen_t len = sizeof(addr);
    if (::bind(listen_, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) != 0 ||
        ::getsockname(listen_, reinterpret_cast<sockaddr*>(&addr), &len) != 0 || ::listen(listen_, 1) != 0) {
      return;
    }
    port_ = ntohs(addr.sin_port);
    thread_ = std::thread([this] { serve(); });
  }

  ~LoopbackPairServer() {
    stop_.store(true, std::memory_order_release);
    if (listen_ != pair_detail::kNoSocket) {
#if defined(_WIN32)
      (void)::shutdown(listen_, SD_BOTH);
#else
      (void)::shutdown(listen_, SHUT_RDWR);
#endif
      pair_detail::close_socket(listen_);  // unblocks `accept`
    }
    if (thread_.joinable()) thread_.join();
#if defined(_WIN32)
    if (wsa_) ::WSACleanup();
#endif
  }
  LoopbackPairServer(const LoopbackPairServer&) = delete;
  LoopbackPairServer& operator=(const LoopbackPairServer&) = delete;

  [[nodiscard]] bool listening() const noexcept { return port_ != 0u; }
  [[nodiscard]] std::string uri() const { return "ws://127.0.0.1:" + std::to_string(port_) + "/nve-wire"; }
  /// True once the client completed the handshake and Hello / HelloAck.
  [[nodiscard]] bool paired() const noexcept { return paired_.load(std::memory_order_acquire); }

  /// Queues an `NVE2` frame, sent as the next `SceneKeyframe` once the client has paired.
  void send_keyframe(std::vector<uint8_t> nve2) {
    const std::lock_guard<std::mutex> lock(mu_);
    queue_.push_back(std::move(nve2));
  }

 private:
  void serve() {
    using namespace pair_detail;
    const Socket c = ::accept(listen_, nullptr, nullptr);
    if (c == kNoSocket) return;
    recv_timeout(c, 2000);
    std::string req;
    char buf[2048];
    while (req.find("\r\n\r\n") == std::string::npos) {
      const auto n = ::recv(c, buf, sizeof(buf), 0);
      if (n <= 0) break;
      req.append(buf, static_cast<size_t>(n));
    }
    std::string key;
    if (const size_t at = req.find("Sec-WebSocket-Key:"); at != std::string::npos) {
      size_t s = at + 18u;
      while (s < req.size() && req[s] == ' ') ++s;
      key = req.substr(s, req.find("\r\n", s) - s);
    }
    const auto digest = sha1(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    const std::string resp = "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                             "Sec-WebSocket-Accept: " + base64(digest) + "\r\n\r\n";
    send_all(c, std::span<const uint8_t>(reinterpret_cast<const uint8_t*>(resp.data()), resp.size()));
    (void)::recv(c, buf, sizeof(buf), 0);  // the client's Hello (best effort)
    std::vector<uint8_t> ack;  // HelloAck: surface id 1, schema 2, caps 0
    push_u32(ack, 1u);
    ack.insert(ack.end(), {2u, 0u, 0u, 0u});
    push_u32(ack, 0u);
    uint32_t seq = 1u;
    send_ws_binary(c, wire_frame(0u, 2u, seq++, ack));
    paired_.store(true, std::memory_order_release);
    while (!stop_.load(std::memory_order_acquire)) {
      std::deque<std::vector<uint8_t>> out;
      {
        const std::lock_guard<std::mutex> lock(mu_);
        out.swap(queue_);
      }
      for (const auto& frame : out) send_ws_binary(c, wire_frame(1u, 4u, seq++, frame));
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
    close_socket(c);
  }

  pair_detail::Socket listen_ = pair_detail::kNoSocket;
  uint16_t port_ = 0u;
#if defined(_WIN32)
  bool wsa_ = false;
#endif
  std::thread thread_;
  std::mutex mu_;
  std::deque<std::vector<uint8_t>> queue_;
  std::atomic<bool> stop_{false};
  std::atomic<bool> paired_{false};
};

}  // namespace nve::host::testing
