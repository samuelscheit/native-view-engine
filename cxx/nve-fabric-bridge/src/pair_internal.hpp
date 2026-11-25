#pragma once

#if defined(NVE_PAIR_NEED_SOCKETS)
#if defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#else
#include <arpa/inet.h>
#include <cerrno>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>
#endif
#endif

#include "nve_pair_client.hpp"

#include <algorithm>
#include <array>
#include <atomic>
#include <cctype>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace nve::pair_detail {

inline constexpr size_t kNveCmdHeaderSize = 24u;
inline constexpr uint8_t kCmdDeltaCopyRun = 1u;
inline constexpr uint8_t kCmdDeltaLiteralOp = 2u;
inline constexpr uint8_t kCmdDeltaXorOp = 3u;
inline constexpr size_t kMaxInflightFragMessages = 16u;
inline constexpr size_t kMaxWsUpgradeHeaderBytes = 8u * 1024u;
inline constexpr size_t kMaxPendingFrames = 256u;
inline constexpr size_t kMinEncodedEventBytes = 18u;
inline constexpr std::string_view kWsGuid = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

inline uint16_t read_u16_le(const uint8_t* p) noexcept {
  return static_cast<uint16_t>(p[0]) | (static_cast<uint16_t>(p[1]) << 8u);
}

inline uint32_t read_u32_le(const uint8_t* p) noexcept {
  return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8u) |
         (static_cast<uint32_t>(p[2]) << 16u) | (static_cast<uint32_t>(p[3]) << 24u);
}

inline uint64_t read_u64_le(const uint8_t* p) noexcept {
  return static_cast<uint64_t>(read_u32_le(p)) |
         (static_cast<uint64_t>(read_u32_le(p + 4)) << 32u);
}

inline uint16_t read_u16_be(const uint8_t* p) noexcept {
  return static_cast<uint16_t>((static_cast<uint16_t>(p[0]) << 8u) | static_cast<uint16_t>(p[1]));
}

inline uint32_t read_u32_be(const uint8_t* p) noexcept {
  return (static_cast<uint32_t>(p[0]) << 24u) | (static_cast<uint32_t>(p[1]) << 16u) |
         (static_cast<uint32_t>(p[2]) << 8u) | static_cast<uint32_t>(p[3]);
}

inline uint64_t read_u64_be(const uint8_t* p) noexcept {
  return (static_cast<uint64_t>(read_u32_be(p)) << 32u) |
         static_cast<uint64_t>(read_u32_be(p + 4));
}

inline void write_u16_le(uint8_t* p, uint16_t v) noexcept {
  p[0] = static_cast<uint8_t>(v & 0xFFu);
  p[1] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
}

inline void write_u32_le(uint8_t* p, uint32_t v) noexcept {
  p[0] = static_cast<uint8_t>(v & 0xFFu);
  p[1] = static_cast<uint8_t>((v >> 8u) & 0xFFu);
  p[2] = static_cast<uint8_t>((v >> 16u) & 0xFFu);
  p[3] = static_cast<uint8_t>((v >> 24u) & 0xFFu);
}

inline void write_u64_le(uint8_t* p, uint64_t v) noexcept {
  write_u32_le(p, static_cast<uint32_t>(v & 0xFFFFFFFFu));
  write_u32_le(p + 4, static_cast<uint32_t>((v >> 32u) & 0xFFFFFFFFu));
}

inline void push_u16_le(std::vector<uint8_t>& out, uint16_t v) {
  uint8_t b[2];
  write_u16_le(b, v);
  out.insert(out.end(), b, b + 2);
}

inline void push_u32_le(std::vector<uint8_t>& out, uint32_t v) {
  uint8_t b[4];
  write_u32_le(b, v);
  out.insert(out.end(), b, b + 4);
}

inline void push_u64_le(std::vector<uint8_t>& out, uint64_t v) {
  uint8_t b[8];
  write_u64_le(b, v);
  out.insert(out.end(), b, b + 8);
}

inline void push_u16_be(std::vector<uint8_t>& out, uint16_t v) {
  out.push_back(static_cast<uint8_t>((v >> 8u) & 0xFFu));
  out.push_back(static_cast<uint8_t>(v & 0xFFu));
}

inline void push_u64_be(std::vector<uint8_t>& out, uint64_t v) {
  for (int shift = 56; shift >= 0; shift -= 8) {
    out.push_back(static_cast<uint8_t>((v >> static_cast<unsigned>(shift)) & 0xFFu));
  }
}

inline uint32_t rotl32(uint32_t x, unsigned n) noexcept {
  return (x << n) | (x >> (32u - n));
}

inline void write_varint(uint64_t v, std::vector<uint8_t>& out) {
  for (;;) {
    uint8_t byte = static_cast<uint8_t>(v & 0x7Fu);
    v >>= 7u;
    if (v != 0u) {
      out.push_back(static_cast<uint8_t>(byte | 0x80u));
    } else {
      out.push_back(byte);
      break;
    }
  }
}

inline bool read_varint(std::span<const uint8_t> bytes, size_t& pos, uint64_t& out) noexcept {
  uint32_t shift = 0u;
  uint64_t result = 0u;
  for (int i = 0; i < 10; ++i) {
    if (pos >= bytes.size()) return false;
    const uint8_t b = bytes[pos++];
    const uint64_t low = static_cast<uint64_t>(b & 0x7Fu);
    if (shift == 63u && low > 1u) return false;
    result |= (low << shift);
    if ((b & 0x80u) == 0u) {
      out = result;
      return true;
    }
    shift += 7u;
  }
  return false;
}

inline bool valid_transport_msg_kind(uint8_t v, TransportMsgKind& out) noexcept {
  if (v >= 1u && v <= 11u) {
    out = static_cast<TransportMsgKind>(v);
    return true;
  }
  return false;
}

inline bool valid_reject_reason(uint8_t v, PairRejectReason& out) noexcept {
  if (v >= 1u && v <= 4u) {
    out = static_cast<PairRejectReason>(v);
    return true;
  }
  return false;
}

inline bool valid_delta_packet_kind(uint8_t v, DeltaPacketKind& out) noexcept {
  if (v >= 1u && v <= 5u) {
    out = static_cast<DeltaPacketKind>(v);
    return true;
  }
  return false;
}

inline bool valid_ws_opcode(uint8_t v, WsOpcode& out) noexcept {
  if (v == 0x0u || v == 0x1u || v == 0x2u || v == 0x8u || v == 0x9u || v == 0xAu) {
    out = static_cast<WsOpcode>(v);
    return true;
  }
  return false;
}

inline bool ascii_iequals(std::string_view a, std::string_view b) noexcept {
  if (a.size() != b.size()) return false;
  for (size_t i = 0u; i < a.size(); ++i) {
    if (std::tolower(static_cast<unsigned char>(a[i])) !=
        std::tolower(static_cast<unsigned char>(b[i]))) {
      return false;
    }
  }
  return true;
}

inline std::string_view trim_ascii(std::string_view sv) noexcept {
  auto is_ws = [](char c) noexcept { return c == ' ' || c == '\t' || c == '\r' || c == '\n'; };
  while (!sv.empty() && is_ws(sv.front())) sv.remove_prefix(1u);
  while (!sv.empty() && is_ws(sv.back())) sv.remove_suffix(1u);
  return sv;
}

inline bool validate_input_events_bytes(
    std::span<const uint8_t> payload,
    size_t max_bytes) noexcept {
  if (payload.size() > max_bytes || payload.size() < 4u) return false;
  const uint32_t count = read_u32_le(payload.data());
  if (count == 0u) return payload.size() == 4u;
  return static_cast<size_t>(count) <= (payload.size() - 4u) / kMinEncodedEventBytes;
}

struct WsFrameHeaderInfo {
  bool fin{true};
  WsOpcode opcode{WsOpcode::Binary};
  bool masked{false};
  size_t payload_offset{0u};
  size_t payload_len{0u};
  std::array<uint8_t, 4> mask_key{};
};

uint64_t next_nonce(uint64_t seed_hint) noexcept;

void encode_xor_rle_delta_into(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next,
    std::vector<uint8_t>& out);

bool parse_ws_frame_header(
    std::span<const uint8_t> bytes,
    size_t max_payload_len,
    WsFrameHeaderInfo& out) noexcept;

bool verify_hello_ack_envelope(
    const PairClientConfig& config,
    const TransportEnvelopeView& env,
    uint64_t client_nonce,
    uint32_t& out_sid,
    size_t& out_window,
    std::string* out_error);

std::string build_ws_upgrade_request(
    const PairClientConfig& config,
    std::string& out_expected_accept);

bool verify_ws_upgrade_response(
    std::string_view resp,
    std::string_view expected_accept,
    std::string* out_error);

#if defined(NVE_PAIR_NEED_SOCKETS)
#if defined(_WIN32)
using SocketHandle = SOCKET;
inline constexpr SocketHandle kInvalidSocket = INVALID_SOCKET;
using PollFd = WSAPOLLFD;
using PollCount = ULONG;
using SockLen = int;

class WinsockRuntime {
 public:
  static bool ensure_initialized() noexcept {
    static WinsockRuntime instance;
    return instance.ok_;
  }
 private:
  WinsockRuntime() noexcept {
    WSADATA data{};
    ok_ = (::WSAStartup(MAKEWORD(2, 2), &data) == 0);
  }
  ~WinsockRuntime() noexcept {
    if (ok_) ::WSACleanup();
  }
  bool ok_{false};
};

inline bool is_valid_socket(SocketHandle s) noexcept { return s != INVALID_SOCKET; }
inline int last_socket_error() noexcept { return ::WSAGetLastError(); }
inline bool is_would_block(int err) noexcept { return err == WSAEWOULDBLOCK; }
inline bool is_in_progress(int err) noexcept {
  return err == WSAEWOULDBLOCK || err == WSAEINPROGRESS;
}
inline bool is_interrupted(int err) noexcept { return err == WSAEINTR; }
inline void close_socket(SocketHandle s) noexcept {
  if (is_valid_socket(s)) ::closesocket(s);
}
inline void shutdown_socket(SocketHandle s) noexcept {
  if (is_valid_socket(s)) ::shutdown(s, SD_BOTH);
}
inline void disable_sigpipe(SocketHandle) noexcept {}
inline bool set_nonblocking(SocketHandle s, bool nonblocking) noexcept {
  u_long mode = nonblocking ? 1ul : 0ul;
  return ::ioctlsocket(s, FIONBIO, &mode) == 0;
}
inline void set_tcp_nodelay(SocketHandle s) noexcept {
  const int one = 1;
  (void)::setsockopt(s, IPPROTO_TCP, TCP_NODELAY, reinterpret_cast<const char*>(&one), sizeof(one));
}
inline int get_so_error(SocketHandle s, int& out_err) noexcept {
  SockLen len = static_cast<SockLen>(sizeof(out_err));
  return ::getsockopt(s, SOL_SOCKET, SO_ERROR, reinterpret_cast<char*>(&out_err), &len);
}
inline int socket_poll(PollFd* pfds, PollCount count, int timeout_ms) noexcept {
  return ::WSAPoll(pfds, count, timeout_ms);
}
inline int clamp_win_len(size_t len) noexcept {
  return static_cast<int>(
      std::min<size_t>(len, static_cast<size_t>(std::numeric_limits<int>::max())));
}
inline ptrdiff_t socket_send(SocketHandle s, const uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(
      ::send(s, reinterpret_cast<const char*>(data), clamp_win_len(len), 0));
}
inline ptrdiff_t socket_recv(SocketHandle s, uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(::recv(s, reinterpret_cast<char*>(data), clamp_win_len(len), 0));
}
inline ptrdiff_t socket_sendto(
    SocketHandle s, const uint8_t* data, size_t len, const sockaddr* addr, SockLen alen) noexcept {
  return static_cast<ptrdiff_t>(
      ::sendto(s, reinterpret_cast<const char*>(data), clamp_win_len(len), 0, addr, alen));
}
inline ptrdiff_t socket_recvfrom(SocketHandle s, uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(
      ::recvfrom(s, reinterpret_cast<char*>(data), clamp_win_len(len), 0, nullptr, nullptr));
}
inline std::string gai_error_string(int code) {
  const char* msg = ::gai_strerrorA(code);
  return msg != nullptr ? std::string(msg) : std::to_string(code);
}
inline bool create_wake_pair(SocketHandle wake_pair[2]) noexcept {
  wake_pair[0] = kInvalidSocket;
  wake_pair[1] = kInvalidSocket;
  SocketHandle listener = ::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
  if (!is_valid_socket(listener)) return false;
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = 0;
  SockLen alen = static_cast<SockLen>(sizeof(addr));
  if (::bind(listener, reinterpret_cast<const sockaddr*>(&addr), alen) != 0 ||
      ::getsockname(listener, reinterpret_cast<sockaddr*>(&addr), &alen) != 0 ||
      ::listen(listener, 1) != 0) {
    close_socket(listener);
    return false;
  }
  SocketHandle tx = ::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
  if (!is_valid_socket(tx) || ::connect(tx, reinterpret_cast<const sockaddr*>(&addr), alen) != 0) {
    close_socket(tx);
    close_socket(listener);
    return false;
  }
  SocketHandle rx = ::accept(listener, nullptr, nullptr);
  close_socket(listener);
  if (!is_valid_socket(rx)) {
    close_socket(tx);
    return false;
  }
  set_tcp_nodelay(rx);
  set_tcp_nodelay(tx);
  (void)set_nonblocking(rx, true);
  (void)set_nonblocking(tx, true);
  wake_pair[0] = rx;
  wake_pair[1] = tx;
  return true;
}
inline void signal_wake_pair(SocketHandle wake_tx) noexcept {
  if (is_valid_socket(wake_tx)) {
    const char b = 1;
    (void)::send(wake_tx, &b, 1, 0);
  }
}
inline void close_wake_handle(SocketHandle& h) noexcept {
  if (is_valid_socket(h)) {
    ::closesocket(h);
    h = kInvalidSocket;
  }
}
#else
using SocketHandle = int;
inline constexpr SocketHandle kInvalidSocket = -1;
using PollFd = pollfd;
using PollCount = nfds_t;
using SockLen = socklen_t;

inline bool is_valid_socket(SocketHandle s) noexcept { return s >= 0; }
inline int last_socket_error() noexcept { return errno; }
inline bool is_would_block(int err) noexcept { return err == EAGAIN || err == EWOULDBLOCK; }
inline bool is_in_progress(int err) noexcept { return err == EINPROGRESS; }
inline bool is_interrupted(int err) noexcept { return err == EINTR; }
inline void close_socket(SocketHandle s) noexcept {
  if (is_valid_socket(s)) ::close(s);
}
inline void shutdown_socket(SocketHandle s) noexcept {
  if (is_valid_socket(s)) ::shutdown(s, SHUT_RDWR);
}
inline void disable_sigpipe(SocketHandle fd) noexcept {
#if defined(SO_NOSIGPIPE)
  const int one = 1;
  (void)::setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof(one));
#else
  (void)fd;
#endif
}
inline bool set_nonblocking(SocketHandle fd, bool nonblocking) noexcept {
  const int flags = ::fcntl(fd, F_GETFL, 0);
  if (flags < 0) return false;
  return ::fcntl(fd, F_SETFL, nonblocking ? (flags | O_NONBLOCK) : (flags & ~O_NONBLOCK)) == 0;
}
inline void set_tcp_nodelay(SocketHandle fd) noexcept {
  const int one = 1;
  (void)::setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
}
inline int get_so_error(SocketHandle fd, int& out_err) noexcept {
  SockLen len = sizeof(out_err);
  return ::getsockopt(fd, SOL_SOCKET, SO_ERROR, &out_err, &len);
}
inline int socket_poll(PollFd* pfds, PollCount count, int timeout_ms) noexcept {
  return ::poll(pfds, count, timeout_ms);
}
inline ptrdiff_t socket_send(SocketHandle fd, const uint8_t* data, size_t len) noexcept {
#if defined(MSG_NOSIGNAL)
  const int flags = MSG_NOSIGNAL;
#else
  const int flags = 0;
#endif
  return static_cast<ptrdiff_t>(::send(fd, data, len, flags));
}
inline ptrdiff_t socket_recv(SocketHandle fd, uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(::recv(fd, data, len, 0));
}
inline ptrdiff_t socket_sendto(
    SocketHandle fd, const uint8_t* data, size_t len, const sockaddr* addr, SockLen alen) noexcept {
  return static_cast<ptrdiff_t>(::sendto(fd, data, len, 0, addr, alen));
}
inline ptrdiff_t socket_recvfrom(SocketHandle fd, uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(::recvfrom(fd, data, len, 0, nullptr, nullptr));
}
inline std::string gai_error_string(int code) {
  const char* msg = ::gai_strerror(code);
  return msg != nullptr ? std::string(msg) : std::to_string(code);
}
inline bool create_wake_pair(SocketHandle wake_pair[2]) noexcept {
  wake_pair[0] = kInvalidSocket;
  wake_pair[1] = kInvalidSocket;
  if (::pipe(wake_pair) != 0) return false;
  (void)set_nonblocking(wake_pair[0], true);
  (void)set_nonblocking(wake_pair[1], true);
  return true;
}
inline void signal_wake_pair(SocketHandle wake_tx) noexcept {
  if (is_valid_socket(wake_tx)) {
    const uint8_t b = 1u;
    const ssize_t ignored = ::write(wake_tx, &b, 1u);
    (void)ignored;
  }
}
inline void close_wake_handle(SocketHandle& h) noexcept {
  if (is_valid_socket(h)) {
    ::close(h);
    h = kInvalidSocket;
  }
}
#endif

inline int compute_wait_timeout_ms(
    std::chrono::steady_clock::time_point now,
    std::chrono::steady_clock::time_point deadline) noexcept {
  if (deadline <= now) return 0;
  const int64_t rem_us =
      std::chrono::duration_cast<std::chrono::microseconds>(deadline - now).count();
  const int64_t rem_ms = (rem_us + 999) / 1000;
  return static_cast<int>(std::clamp<int64_t>(rem_ms, 1, 60000));
}

inline int wait_readable_fd(
    SocketHandle fd,
    SocketHandle wake_rx,
    const std::atomic<bool>& shutdown_flag,
    int timeout_ms) noexcept {
  PollFd pfds[2]{};
  pfds[0].fd = fd;
  pfds[0].events = POLLIN;
  PollCount nfds = 1u;
  if (is_valid_socket(wake_rx)) {
    pfds[1].fd = wake_rx;
    pfds[1].events = POLLIN;
    nfds = 2u;
  }
  for (;;) {
    if (shutdown_flag.load(std::memory_order_acquire)) return -1;
    const int rc = socket_poll(pfds, nfds, timeout_ms);
    if (rc > 0) {
      if (nfds == 2u && (pfds[1].revents & (POLLIN | POLLHUP | POLLERR)) != 0) return -1;
      return ((pfds[0].revents & (POLLIN | POLLHUP | POLLERR)) != 0) ? 1 : 0;
    }
    if (rc == 0) return 0;
    if (is_interrupted(last_socket_error())) continue;
    return -1;
  }
}

inline bool write_all_stream_fd(
    SocketHandle fd,
    const std::atomic<bool>& shutdown_flag,
    const uint8_t* data,
    size_t len) noexcept {
  using Clock = std::chrono::steady_clock;
  size_t sent = 0u;
  const auto deadline = Clock::now() + std::chrono::milliseconds(2000);
  while (sent < len) {
    if (shutdown_flag.load(std::memory_order_acquire)) return false;
    const ptrdiff_t n = socket_send(fd, data + sent, len - sent);
    if (n > 0) {
      sent += static_cast<size_t>(n);
      continue;
    }
    const int err = last_socket_error();
    if (n < 0 && is_interrupted(err)) continue;
    if (n < 0 && is_would_block(err)) {
      const auto now = Clock::now();
      if (now >= deadline) return false;
      PollFd pfd{};
      pfd.fd = fd;
      pfd.events = POLLOUT;
      const int rc = socket_poll(&pfd, 1u, compute_wait_timeout_ms(now, deadline));
      if (rc < 0 && !is_interrupted(last_socket_error())) return false;
      continue;
    }
    return false;
  }
  return true;
}

inline int read_exact_stream_fd(
    SocketHandle fd,
    SocketHandle wake_rx,
    const std::atomic<bool>& shutdown_flag,
    uint8_t* buf,
    size_t len,
    bool allow_idle_on_zero,
    int idle_wait_ms,
    int stall_timeout_ms) noexcept {
  using Clock = std::chrono::steady_clock;
  size_t offset = 0u;
  std::optional<Clock::time_point> started;
  while (offset < len) {
    if (shutdown_flag.load(std::memory_order_acquire)) return -1;
    const ptrdiff_t n = socket_recv(fd, buf + offset, len - offset);
    if (n > 0) {
      offset += static_cast<size_t>(n);
      if (!started.has_value()) started = Clock::now();
      continue;
    }
    if (n == 0) return -1;
    const int err = last_socket_error();
    if (is_interrupted(err)) continue;
    if (is_would_block(err)) {
      if (offset == 0u && allow_idle_on_zero) {
        const int ready = wait_readable_fd(fd, wake_rx, shutdown_flag, idle_wait_ms);
        if (ready <= 0) return ready;
        continue;
      }
      const auto now = Clock::now();
      if (!started.has_value()) started = now;
      const auto stall_deadline = *started + std::chrono::milliseconds(stall_timeout_ms);
      if (now >= stall_deadline) return -1;
      if (wait_readable_fd(fd, wake_rx, shutdown_flag, compute_wait_timeout_ms(now, stall_deadline)) <
          0) {
        return -1;
      }
      continue;
    }
    return -1;
  }
  return 1;
}

inline bool dial_pair_endpoint(
    const PairClientConfig& config,
    SocketHandle& out_fd,
    sockaddr_storage& out_addr,
    SockLen& out_len,
    std::string* out_error) {
  addrinfo hints{};
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = (config.transport == PairTransport::Udp) ? SOCK_DGRAM : SOCK_STREAM;
  const std::string port_str = std::to_string(config.port);
  addrinfo* res = nullptr;
  const int gai = ::getaddrinfo(config.host.c_str(), port_str.c_str(), &hints, &res);
  if (gai != 0 || res == nullptr) {
    if (out_error != nullptr) *out_error = "getaddrinfo failed: " + gai_error_string(gai);
    return false;
  }
  SocketHandle fd = kInvalidSocket;
  for (addrinfo* ai = res; ai != nullptr; ai = ai->ai_next) {
    fd = ::socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
    if (!is_valid_socket(fd)) continue;
    disable_sigpipe(fd);
    (void)set_nonblocking(fd, true);
    if (config.transport == PairTransport::Udp) {
      std::memcpy(&out_addr, ai->ai_addr, ai->ai_addrlen);
      out_len = static_cast<SockLen>(ai->ai_addrlen);
      break;
    }
    set_tcp_nodelay(fd);
    if (::connect(fd, ai->ai_addr, static_cast<SockLen>(ai->ai_addrlen)) == 0) {
      std::memcpy(&out_addr, ai->ai_addr, ai->ai_addrlen);
      out_len = static_cast<SockLen>(ai->ai_addrlen);
      break;
    }
    if (is_in_progress(last_socket_error())) {
      PollFd pfd{};
      pfd.fd = fd;
      pfd.events = POLLOUT;
      int so_err = 0;
      if (socket_poll(&pfd, 1u, static_cast<int>(config.handshake_timeout_ms)) > 0 &&
          (pfd.revents & (POLLOUT | POLLERR | POLLHUP)) != 0 &&
          get_so_error(fd, so_err) == 0 && so_err == 0) {
        std::memcpy(&out_addr, ai->ai_addr, ai->ai_addrlen);
        out_len = static_cast<SockLen>(ai->ai_addrlen);
        break;
      }
    }
    close_socket(fd);
    fd = kInvalidSocket;
  }
  ::freeaddrinfo(res);
  if (!is_valid_socket(fd)) {
    if (out_error != nullptr) {
      *out_error = "Failed to connect to " + config.host + ":" + std::to_string(config.port);
    }
    return false;
  }
  out_fd = fd;
  return true;
}

inline bool perform_ws_upgrade_fd(
    SocketHandle fd,
    SocketHandle wake_rx,
    const std::atomic<bool>& shutdown_flag,
    const PairClientConfig& config,
    std::string* out_error) {
  using Clock = std::chrono::steady_clock;
  std::string expected_accept;
  const std::string req = build_ws_upgrade_request(config, expected_accept);
  if (!write_all_stream_fd(
          fd, shutdown_flag, reinterpret_cast<const uint8_t*>(req.data()), req.size())) {
    if (out_error != nullptr) *out_error = "Failed writing WebSocket HTTP upgrade request";
    return false;
  }
  const auto deadline = Clock::now() + std::chrono::milliseconds(config.handshake_timeout_ms);
  std::string resp;
  resp.reserve(512u);
  while (Clock::now() < deadline) {
    uint8_t ch = 0u;
    const int rc = read_exact_stream_fd(
        fd, wake_rx, shutdown_flag, &ch, 1u, true, compute_wait_timeout_ms(Clock::now(), deadline),
        500);
    if (rc < 0) {
      if (out_error != nullptr) *out_error = "Connection closed during WebSocket HTTP upgrade";
      return false;
    }
    if (rc == 0) continue;
    resp.push_back(static_cast<char>(ch));
    if (resp.size() > kMaxWsUpgradeHeaderBytes) {
      if (out_error != nullptr) *out_error = "WebSocket upgrade response headers exceeded 8 KiB limit";
      return false;
    }
    if (resp.size() >= 4u && resp.compare(resp.size() - 4u, 4u, "\r\n\r\n") == 0) break;
  }
  if (resp.size() < 4u || resp.compare(resp.size() - 4u, 4u, "\r\n\r\n") != 0) {
    if (out_error != nullptr) *out_error = "WebSocket HTTP upgrade timed out";
    return false;
  }
  return verify_ws_upgrade_response(resp, expected_accept, out_error);
}
#endif // defined(NVE_PAIR_NEED_SOCKETS)

} // namespace nve::pair_detail
