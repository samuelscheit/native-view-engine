#include "nve_event_batch.hpp"
#include "nve_pair_client.hpp"
#include "nve_wire_codecs.generated.hpp"

#include <algorithm>
#include <atomic>
#include <charconv>
#include <cstring>
#include <deque>
#include <mutex>
#include <thread>

#if defined(_WIN32)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
using NativeSocket = SOCKET;
constexpr NativeSocket kInvalidSocket = INVALID_SOCKET;
#else
#include <arpa/inet.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
using NativeSocket = int;
constexpr NativeSocket kInvalidSocket = -1;
#endif

namespace nve {
namespace {

void close_socket(NativeSocket fd) noexcept {
  if (fd == kInvalidSocket) return;
#if defined(_WIN32)
  ::shutdown(fd, SD_BOTH);
  ::closesocket(fd);
#else
  ::shutdown(fd, SHUT_RDWR);
  ::close(fd);
#endif
}

bool wait_readable(NativeSocket fd, int timeout_ms) noexcept {
#if defined(_WIN32)
  WSAPOLLFD pfd{};
  pfd.fd = fd;
  pfd.events = POLLRDNORM;
  return ::WSAPoll(&pfd, 1, timeout_ms) > 0 && ((pfd.revents & (POLLRDNORM | POLLHUP | POLLERR)) != 0);
#else
  pollfd pfd{};
  pfd.fd = fd;
  pfd.events = POLLIN;
  return ::poll(&pfd, 1, timeout_ms) > 0 && ((pfd.revents & (POLLIN | POLLHUP | POLLERR)) != 0);
#endif
}

bool send_all(NativeSocket fd, const uint8_t* data, size_t len) noexcept {
  size_t off = 0;
  while (off < len) {
#if defined(_WIN32)
    const int n = ::send(fd, reinterpret_cast<const char*>(data + off), static_cast<int>(len - off), 0);
#else
    const ssize_t n = ::send(fd, data + off, len - off, 0);
#endif
    if (n <= 0) return false;
    off += static_cast<size_t>(n);
  }
  return true;
}

bool recv_exact(NativeSocket fd, uint8_t* out, size_t len, int timeout_ms) noexcept {
  size_t off = 0;
  while (off < len) {
    if (timeout_ms >= 0 && !wait_readable(fd, timeout_ms)) return false;
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

bool read_wire_frame_stream(
    NativeSocket fd,
    std::vector<uint8_t>& out_full,
    WireFrameView& out_view,
    size_t max_frame_bytes,
    int timeout_ms) {
  std::array<uint8_t, kWireHeaderSize> hdr_bytes{};
  if (!recv_exact(fd, hdr_bytes.data(), hdr_bytes.size(), timeout_ms)) return false;
  WireHeader hdr{};
  if (!wire_v2::decode_header(hdr_bytes, hdr, max_frame_bytes)) return false;
  const size_t total = hdr.total_wire_len();
  out_full.resize(total);
  std::memcpy(out_full.data(), hdr_bytes.data(), kWireHeaderSize);
  if (total > kWireHeaderSize) {
    if (!recv_exact(fd, out_full.data() + kWireHeaderSize, total - kWireHeaderSize, timeout_ms)) {
      return false;
    }
  }
  return wire_v2::decode_frame(out_full, out_view, max_frame_bytes);
}

bool parse_u16_sv(std::string_view sv, uint16_t& out) noexcept {
  if (sv.empty()) return false;
  uint32_t v = 0;
  const auto res = std::from_chars(sv.data(), sv.data() + sv.size(), v);
  if (res.ec != std::errc{} || res.ptr != sv.data() + sv.size() || v > 65535u) return false;
  out = static_cast<uint16_t>(v);
  return true;
}

std::string to_lower_ascii(std::string_view sv) {
  std::string out(sv);
  for (char& c : out) {
    if (c >= 'A' && c <= 'Z') c = static_cast<char>(c - 'A' + 'a');
  }
  return out;
}

} // namespace

// =============================================================================
// Endpoint Parser & Direct EventRing Helper
// =============================================================================

bool parse_pair_endpoint(
    std::string_view endpoint,
    std::string_view transport_override,
    PairClientConfig& out,
    std::string* out_error) {
  auto fail = [&](std::string msg) {
    if (out_error != nullptr) *out_error = std::move(msg);
    return false;
  };
  if (endpoint.empty()) return fail("pair endpoint cannot be empty");

  PairTransport inferred = PairTransport::Auto;
  std::string_view rest = endpoint;

  if (rest.starts_with("nve://")) {
    const size_t qpos = rest.find('?');
    if (qpos == std::string_view::npos) return fail("nve://pair URI missing query parameters");
    std::string_view query = rest.substr(qpos + 1);
    uint16_t quic_port = 0;
    uint16_t wss_port = 0;
    while (!query.empty()) {
      const size_t amp = query.find('&');
      const std::string_view kv = amp == std::string_view::npos ? query : query.substr(0, amp);
      query = amp == std::string_view::npos ? std::string_view{} : query.substr(amp + 1);
      const size_t eq = kv.find('=');
      if (eq == std::string_view::npos) continue;
      const std::string_view k = kv.substr(0, eq);
      const std::string_view v = kv.substr(eq + 1);
      if (k == "host") out.host = std::string(v);
      else if (k == "quic" || k == "udp") (void)parse_u16_sv(v, quic_port);
      else if (k == "wss" || k == "ws" || k == "tcp") (void)parse_u16_sv(v, wss_port);
      else if (k == "token") out.pairing_token = std::string(v);
      else if (k == "cert_hash") out.cert_hash_hex = std::string(v);
      else if (k == "path") out.ws_path = std::string(v);
    }
    out.port = wss_port != 0u ? wss_port : quic_port;
    if (out.host.empty() || out.port == 0u) return fail("nve://pair URI missing host or port");
  } else {
    if (rest.starts_with("quic://")) { inferred = PairTransport::Quic; rest.remove_prefix(7); }
    else if (rest.starts_with("https://")) { inferred = PairTransport::WebTransport; rest.remove_prefix(8); }
    else if (rest.starts_with("wss://")) { inferred = PairTransport::Wss; rest.remove_prefix(6); }
    else if (rest.starts_with("ws://")) { inferred = PairTransport::Ws; rest.remove_prefix(5); }
    else if (rest.starts_with("tcp://")) { inferred = PairTransport::Tcp; rest.remove_prefix(6); }
    else if (rest.starts_with("udp://")) { inferred = PairTransport::Quic; rest.remove_prefix(6); }

    const size_t slash = rest.find('/');
    const std::string_view host_port = slash == std::string_view::npos ? rest : rest.substr(0, slash);
    if (slash != std::string_view::npos) out.ws_path = std::string(rest.substr(slash));

    const size_t colon = host_port.rfind(':');
    if (colon == std::string_view::npos) return fail("pair endpoint missing :port");
    out.host = std::string(host_port.substr(0, colon));
    if (out.host.size() >= 2 && out.host.front() == '[' && out.host.back() == ']') {
      out.host = out.host.substr(1, out.host.size() - 2);
    }
    if (!parse_u16_sv(host_port.substr(colon + 1), out.port) || out.port == 0u) {
      return fail("invalid port in pair endpoint");
    }
  }

  const std::string ov = to_lower_ascii(transport_override);
  if (ov.empty() || ov == "auto") out.transport = inferred;
  else if (ov == "quic" || ov == "udp") out.transport = PairTransport::Quic;
  else if (ov == "wt" || ov == "webtransport" || ov == "h3") out.transport = PairTransport::WebTransport;
  else if (ov == "wss") out.transport = PairTransport::Wss;
  else if (ov == "ws") out.transport = PairTransport::Ws;
  else if (ov == "tcp") out.transport = PairTransport::Tcp;
  else return fail("unknown pair transport: " + std::string(transport_override));

  return true;
}

bool encode_direct_event_batch(NveEventBatch batch, std::vector<uint8_t>& out_ring) {
  out_ring.clear();
  if (batch.event_count == 0u || batch.events == nullptr ||
      (batch.bytes_len > 0u && batch.bytes == nullptr)) {
    return false;
  }
  constexpr size_t kMaxSize = std::numeric_limits<size_t>::max();
  if (static_cast<size_t>(batch.event_count) >
      (kMaxSize - wire::kEventWireHeaderSize) / wire::kEventRecordSize) {
    return false;
  }
  const size_t ev_bytes = static_cast<size_t>(batch.event_count) * wire::kEventRecordSize;
  if (static_cast<size_t>(batch.bytes_len) > kMaxSize - wire::kEventWireHeaderSize - ev_bytes) {
    return false;
  }
  out_ring.reserve(wire::kEventWireHeaderSize + ev_bytes + static_cast<size_t>(batch.bytes_len));
  out_ring.insert(
      out_ring.end(), wire::kEventWireMagicBytes.begin(), wire::kEventWireMagicBytes.end());
  wire::append_le_u16(out_ring, wire::kEventWireVersion);
  wire::append_le_u32(out_ring, batch.event_count);
  wire::append_le_u32(out_ring, batch.bytes_len);
  for (uint32_t i = 0u; i < batch.event_count; ++i) {
    append_event_le(out_ring, batch.events[i]);
  }
  if (batch.bytes_len > 0u) {
    out_ring.insert(out_ring.end(), batch.bytes, batch.bytes + batch.bytes_len);
  }
  return true;
}

// =============================================================================
// Thin `PairClient` Implementation (`NveWireFrame` v2)
// =============================================================================

struct PairClient::Impl {
  NativeSocket sock{kInvalidSocket};
  PairClientConfig cfg{};
  WakeCallback wake_cb{};
  std::atomic<bool> connected{false};
  std::atomic<bool> stop{false};
  std::atomic<bool> has_frame{false};
  std::atomic<uint32_t> sid{0};
  std::atomic<uint32_t> next_seq{2};

  mutable std::mutex send_mtx;
  mutable std::mutex frame_mtx;
  mutable std::mutex metrics_mtx;
  std::deque<std::pair<uint32_t, std::vector<uint8_t>>> pending_frames;
  PairClientMetrics metrics{};
  std::thread reader;

  bool send_wire(WireChannel ch, WireKind kind, std::span<const uint8_t> payload) {
    const uint32_t seq = next_seq.fetch_add(1u, std::memory_order_relaxed);
    const auto encoded = wire_v2::encode_frame(ch, kind, seq, payload);
    std::lock_guard<std::mutex> lock(send_mtx);
    if (sock == kInvalidSocket || !send_all(sock, encoded.data(), encoded.size())) return false;
    std::lock_guard<std::mutex> ml(metrics_mtx);
    metrics.wire_bytes_sent += encoded.size();
    return true;
  }

  void run_reader() {
    std::vector<uint8_t> buf;
    while (!stop.load(std::memory_order_relaxed)) {
      if (!wait_readable(sock, 25)) continue;
      WireFrameView view{};
      if (!read_wire_frame_stream(sock, buf, view, cfg.max_frame_bytes, 250)) {
        break;
      }
      {
        std::lock_guard<std::mutex> ml(metrics_mtx);
        metrics.wire_bytes_received += buf.size();
      }
      if (view.header.kind == WireKind::CommandDelta || view.header.kind == WireKind::SceneKeyframe) {
        std::vector<uint8_t> frame(view.payload.begin(), view.payload.end());
        {
          std::lock_guard<std::mutex> fl(frame_mtx);
          pending_frames.emplace_back(view.header.sequence, std::move(frame));
          has_frame.store(true, std::memory_order_release);
        }
        {
          std::lock_guard<std::mutex> ml(metrics_mtx);
          metrics.frames_received += 1u;
          metrics.last_applied_seq = view.header.sequence;
        }
        if (wake_cb) wake_cb();
      } else if (view.header.kind == WireKind::Ping) {
        (void)send_wire(WireChannel::Control, WireKind::Pong, view.payload);
      } else if (view.header.kind == WireKind::Bye) {
        break;
      }
    }
    connected.store(false, std::memory_order_release);
  }
};

PairClient::PairClient() : impl_(std::make_unique<Impl>()) {}
PairClient::~PairClient() { disconnect(); }

bool PairClient::connect(
    const PairClientConfig& config,
    WakeCallback on_frame_ready,
    std::string* out_error) {
  disconnect();
  auto fail = [&](std::string msg) {
    if (out_error != nullptr) *out_error = std::move(msg);
    disconnect();
    return false;
  };
  if (config.host.empty() || config.port == 0u) return fail("invalid PairClientConfig host/port");

  addrinfo hints{};
  hints.ai_family = AF_UNSPEC;
  hints.ai_socktype = SOCK_STREAM;
  addrinfo* res = nullptr;
  const std::string port_str = std::to_string(config.port);
  if (::getaddrinfo(config.host.c_str(), port_str.c_str(), &hints, &res) != 0 || res == nullptr) {
    return fail("failed to resolve pair host");
  }

  NativeSocket fd = kInvalidSocket;
  for (addrinfo* p = res; p != nullptr; p = p->ai_next) {
    fd = ::socket(p->ai_family, p->ai_socktype, p->ai_protocol);
    if (fd == kInvalidSocket) continue;
    if (::connect(fd, p->ai_addr, static_cast<int>(p->ai_addrlen)) == 0) break;
    close_socket(fd);
    fd = kInvalidSocket;
  }
  ::freeaddrinfo(res);
  if (fd == kInvalidSocket) return fail("failed to connect to pair endpoint");

  const int nodelay = 1;
#if defined(_WIN32)
  (void)::setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, reinterpret_cast<const char*>(&nodelay), sizeof(nodelay));
#else
  (void)::setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &nodelay, sizeof(nodelay));
#endif

  impl_->sock = fd;
  impl_->cfg = config;
  impl_->wake_cb = std::move(on_frame_ready);
  impl_->stop.store(false, std::memory_order_relaxed);

  WireHello hello{};
  hello.schema_version = config.schema_version;
  hello.bearer_token = config.pairing_token;
  const auto hello_payload = wire_v2::encode_hello(hello);
  const auto hello_frame =
      wire_v2::encode_frame(WireChannel::Control, WireKind::Hello, 1u, hello_payload);
  if (!send_all(fd, hello_frame.data(), hello_frame.size())) {
    return fail("failed to send NveWireFrame Hello");
  }

  std::vector<uint8_t> reply_buf;
  WireFrameView reply{};
  if (!read_wire_frame_stream(
          fd, reply_buf, reply, config.max_frame_bytes, static_cast<int>(config.handshake_timeout_ms))) {
    return fail("timed out waiting for NveWireFrame HelloAck");
  }
  if (reply.header.kind == WireKind::Reject) {
    WireRejectReason reason{};
    std::string detail;
    (void)wire_v2::decode_reject(reply.payload, reason, detail);
    return fail("pairing rejected: " + detail);
  }
  WireHelloAck ack{};
  if (reply.header.kind != WireKind::HelloAck || !wire_v2::decode_hello_ack(reply.payload, ack)) {
    return fail("malformed NveWireFrame HelloAck");
  }

  impl_->sid.store(ack.session_id, std::memory_order_release);
  {
    std::lock_guard<std::mutex> ml(impl_->metrics_mtx);
    impl_->metrics = PairClientMetrics{};
    impl_->metrics.session_id = ack.session_id;
    impl_->metrics.wire_bytes_sent = hello_frame.size();
    impl_->metrics.wire_bytes_received = reply_buf.size();
  }
  impl_->connected.store(true, std::memory_order_release);
  impl_->reader = std::thread([this]() { impl_->run_reader(); });
  return true;
}

void PairClient::disconnect() noexcept {
  impl_->stop.store(true, std::memory_order_release);
  if (impl_->connected.exchange(false, std::memory_order_acq_rel)) {
    (void)impl_->send_wire(WireChannel::Control, WireKind::Bye, {});
  }
  if (impl_->sock != kInvalidSocket) {
    close_socket(impl_->sock);
    impl_->sock = kInvalidSocket;
  }
  if (impl_->reader.joinable()) impl_->reader.join();
  std::lock_guard<std::mutex> fl(impl_->frame_mtx);
  impl_->pending_frames.clear();
  impl_->has_frame.store(false, std::memory_order_release);
}

bool PairClient::is_connected() const noexcept {
  return impl_->connected.load(std::memory_order_acquire);
}

uint32_t PairClient::session_id() const noexcept {
  return impl_->sid.load(std::memory_order_acquire);
}

PairTransport PairClient::transport() const noexcept { return impl_->cfg.transport; }

PairClientMetrics PairClient::metrics() const {
  std::lock_guard<std::mutex> ml(impl_->metrics_mtx);
  return impl_->metrics;
}

bool PairClient::has_pending_frame() const noexcept {
  return impl_->has_frame.load(std::memory_order_acquire);
}

bool PairClient::pop_frame(std::vector<uint8_t>& out_frame, uint32_t* out_seq) {
  std::lock_guard<std::mutex> fl(impl_->frame_mtx);
  if (impl_->pending_frames.empty()) return false;
  auto [seq, bytes] = std::move(impl_->pending_frames.front());
  impl_->pending_frames.pop_front();
  impl_->has_frame.store(!impl_->pending_frames.empty(), std::memory_order_release);
  out_frame = std::move(bytes);
  if (out_seq != nullptr) *out_seq = seq;
  return true;
}

size_t PairClient::drain_pending_frames(std::vector<std::pair<uint32_t, std::vector<uint8_t>>>& out_frames) {
  std::lock_guard<std::mutex> fl(impl_->frame_mtx);
  const size_t count = impl_->pending_frames.size();
  while (!impl_->pending_frames.empty()) {
    out_frames.push_back(std::move(impl_->pending_frames.front()));
    impl_->pending_frames.pop_front();
  }
  impl_->has_frame.store(false, std::memory_order_release);
  return count;
}

bool PairClient::take_latest_frame(std::vector<uint8_t>& out_frame, uint32_t* out_seq) {
  std::lock_guard<std::mutex> fl(impl_->frame_mtx);
  if (impl_->pending_frames.empty()) return false;
  auto [seq, bytes] = std::move(impl_->pending_frames.back());
  impl_->pending_frames.clear();
  impl_->has_frame.store(false, std::memory_order_release);
  out_frame = std::move(bytes);
  if (out_seq != nullptr) *out_seq = seq;
  return true;
}

bool PairClient::send_events(std::span<const uint8_t> encoded_event_ring, std::string* out_error) {
  if (!is_connected() || encoded_event_ring.empty()) {
    if (out_error != nullptr) *out_error = "PairClient not connected or empty events";
    return false;
  }
  if (!impl_->send_wire(WireChannel::InputEvents, WireKind::EventBatch, encoded_event_ring)) {
    if (out_error != nullptr) *out_error = "failed to send EventBatch";
    return false;
  }
  std::lock_guard<std::mutex> ml(impl_->metrics_mtx);
  impl_->metrics.events_sent += 1u;
  return true;
}

bool PairClient::request_keyframe() {
  return is_connected() && impl_->send_wire(WireChannel::Control, WireKind::KeyframeRequest, {});
}

bool PairClient::send_ping() {
  return is_connected() && impl_->send_wire(WireChannel::Control, WireKind::Ping, {});
}

} // namespace nve
