#define NVE_PAIR_NEED_SOCKETS
#include "pair_internal.hpp"

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <deque>
#include <limits>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nve {

using namespace pair_detail;

struct PairClient::Impl {
  using Clock = std::chrono::steady_clock;

  PairClientConfig config{};
  WakeCallback on_frame_ready;
  Clock::time_point started_at{Clock::now()};

  std::atomic<bool> connected{false};
  std::atomic<bool> shutdown_flag{false};
  std::atomic<uint32_t> session_id{0u};
  std::atomic<bool> has_pending{false};

  mutable std::mutex metrics_mutex;
  PairClientMetrics metrics{};

  mutable std::mutex queue_mutex;
  std::deque<std::pair<uint32_t, std::vector<uint8_t>>> pending_frames;

  std::mutex write_mutex;
  std::thread reader_thread;

  SocketHandle sock_fd{kInvalidSocket};
  SocketHandle wake_fds[2]{kInvalidSocket, kInvalidSocket};
  sockaddr_storage peer_addr{};
  SockLen peer_addr_len{0};

  ~Impl() { disconnect(); }

  uint64_t now_us() const noexcept {
    return static_cast<uint64_t>(
        std::chrono::duration_cast<std::chrono::microseconds>(Clock::now() - started_at).count());
  }

  size_t clamped_max_frame_bytes() const noexcept {
    return std::clamp(config.max_frame_bytes, size_t{64u}, kMaxAllowedWireFrameBytes);
  }

  size_t clamped_max_event_bytes() const noexcept {
    const size_t base = std::min(clamped_max_frame_bytes(), kMaxInputEventBytes);
    return (config.transport == PairTransport::Udp) ? std::min(base, kMaxUdpSinglePayload) : base;
  }

  void push_applied_frames(
      std::vector<std::pair<uint32_t, std::vector<uint8_t>>>& frames,
      uint32_t last_seq) {
    if (frames.empty()) return;
    const size_t n = frames.size();
    {
      std::lock_guard<std::mutex> lock(queue_mutex);
      for (auto& item : frames) {
        while (pending_frames.size() >= kMaxPendingFrames) pending_frames.pop_front();
        pending_frames.push_back(std::move(item));
      }
      has_pending.store(!pending_frames.empty(), std::memory_order_release);
    }
    {
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.frames_received += static_cast<uint64_t>(n);
      metrics.last_applied_seq = last_seq;
    }
    if (on_frame_ready) on_frame_ready();
  }

  void close_sockets_locked() noexcept {
    if (is_valid_socket(sock_fd)) {
      shutdown_socket(sock_fd);
      close_socket(sock_fd);
      sock_fd = kInvalidSocket;
    }
    close_wake_handle(wake_fds[0]);
    close_wake_handle(wake_fds[1]);
  }

  bool send_stream_message_locked(SocketHandle fd, std::span<const uint8_t> env_bytes) {
    if (!is_valid_socket(fd)) return false;
    if (config.transport == PairTransport::Tcp) {
      if (env_bytes.size() > std::numeric_limits<uint32_t>::max()) return false;
      std::vector<uint8_t> framed;
      framed.reserve(4u + env_bytes.size());
      push_u32_le(framed, static_cast<uint32_t>(env_bytes.size()));
      framed.insert(framed.end(), env_bytes.begin(), env_bytes.end());
      if (!write_all_stream_fd(fd, shutdown_flag, framed.data(), framed.size())) return false;
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.wire_bytes_sent += framed.size();
      return true;
    }
    if (config.transport == PairTransport::WebSocket) {
      std::array<uint8_t, 4> mask{};
      write_u32_le(mask.data(), static_cast<uint32_t>(next_nonce(env_bytes.size())));
      const auto ws = pair_wire::encode_ws_frame(true, WsOpcode::Binary, env_bytes, mask);
      if (!write_all_stream_fd(fd, shutdown_flag, ws.data(), ws.size())) return false;
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.wire_bytes_sent += ws.size();
      return true;
    }
    return false;
  }

  bool send_wire_message(std::span<const uint8_t> env_bytes) {
    std::lock_guard<std::mutex> wlock(write_mutex);
    if (!is_valid_socket(sock_fd)) return false;
    if (config.transport == PairTransport::Udp) {
      const ptrdiff_t n = socket_sendto(
          sock_fd, env_bytes.data(), env_bytes.size(),
          reinterpret_cast<const sockaddr*>(&peer_addr), peer_addr_len);
      if (n != static_cast<ptrdiff_t>(env_bytes.size())) return false;
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.wire_bytes_sent += static_cast<uint64_t>(n);
      return true;
    }
    return send_stream_message_locked(sock_fd, env_bytes);
  }

  int read_stream_message(
      SocketHandle fd,
      std::vector<uint8_t>& out_frame,
      size_t& out_wire_bytes,
      int idle_wait_ms) {
    out_frame.clear();
    out_wire_bytes = 0u;
    const size_t max_bytes = clamped_max_frame_bytes();
    auto read_ex = [&](uint8_t* buf, size_t n, bool idle_ok) {
      return read_exact_stream_fd(fd, wake_fds[0], shutdown_flag, buf, n, idle_ok, idle_wait_ms, 500);
    };

    if (config.transport == PairTransport::Tcp) {
      uint8_t len_buf[4]{};
      const int rc = read_ex(len_buf, 4u, true);
      if (rc <= 0) return rc;
      const size_t declared = static_cast<size_t>(read_u32_le(len_buf));
      if (declared == 0u || declared > max_bytes) return -1;
      out_frame.resize(declared);
      if (read_ex(out_frame.data(), declared, false) != 1) return -1;
      out_wire_bytes = 4u + declared;
      return 1;
    }

    bool assembling = false;
    size_t total_wire = 0u;
    for (;;) {
      uint8_t hdr[2]{};
      const int rc = read_ex(hdr, 2u, !assembling);
      if (rc <= 0) return rc;
      total_wire += 2u;
      const bool fin = (hdr[0] & 0x80u) != 0u;
      if ((hdr[0] & 0x70u) != 0u) return -1;
      WsOpcode opcode{};
      if (!valid_ws_opcode(static_cast<uint8_t>(hdr[0] & 0x0Fu), opcode)) return -1;
      const bool masked = (hdr[1] & 0x80u) != 0u;
      const uint8_t len7 = static_cast<uint8_t>(hdr[1] & 0x7Fu);
      size_t payload_len = 0u;
      if (len7 <= 125u) {
        payload_len = static_cast<size_t>(len7);
      } else if (len7 == 126u) {
        uint8_t b[2]{};
        if (read_ex(b, 2u, false) != 1) return -1;
        total_wire += 2u;
        payload_len = static_cast<size_t>(read_u16_be(b));
      } else {
        uint8_t b[8]{};
        if (read_ex(b, 8u, false) != 1) return -1;
        total_wire += 8u;
        const uint64_t n64 = read_u64_be(b);
        if ((n64 >> 63u) != 0u || n64 > static_cast<uint64_t>(max_bytes)) return -1;
        payload_len = static_cast<size_t>(n64);
      }

      const bool is_control =
          (opcode == WsOpcode::Close || opcode == WsOpcode::Ping || opcode == WsOpcode::Pong);
      const size_t existing = (!is_control && assembling) ? out_frame.size() : 0u;
      if (payload_len > max_bytes || payload_len > max_bytes - existing) return -1;

      std::array<uint8_t, 4> mask_key{};
      if (masked) {
        if (read_ex(mask_key.data(), 4u, false) != 1) return -1;
        total_wire += 4u;
      }

      if (is_control) {
        std::array<uint8_t, 128> stack_ctrl{};
        std::vector<uint8_t> heap_ctrl;
        uint8_t* ptr = stack_ctrl.data();
        if (payload_len > stack_ctrl.size()) {
          heap_ctrl.resize(payload_len);
          ptr = heap_ctrl.data();
        }
        if (payload_len > 0u) {
          if (read_ex(ptr, payload_len, false) != 1) return -1;
          total_wire += payload_len;
          if (masked) {
            for (size_t i = 0u; i < payload_len; ++i) ptr[i] ^= mask_key[i & 3u];
          }
        }
        if (opcode == WsOpcode::Close) return -1;
        if (opcode == WsOpcode::Ping) {
          std::array<uint8_t, 4> pm{};
          write_u32_le(pm.data(), static_cast<uint32_t>(next_nonce(7u)));
          const auto pong = pair_wire::encode_ws_frame(
              true, WsOpcode::Pong, std::span<const uint8_t>(ptr, payload_len), pm);
          std::lock_guard<std::mutex> wlock(write_mutex);
          if (is_valid_socket(sock_fd)) {
            (void)write_all_stream_fd(sock_fd, shutdown_flag, pong.data(), pong.size());
          }
        }
        continue;
      }

      if (opcode == WsOpcode::Continuation && !assembling) return -1;
      if ((opcode == WsOpcode::Binary || opcode == WsOpcode::Text) && !assembling) {
        out_frame.clear();
      }
      const size_t prev_size = out_frame.size();
      if (payload_len > 0u) {
        out_frame.resize(prev_size + payload_len);
        uint8_t* dst = out_frame.data() + prev_size;
        if (read_ex(dst, payload_len, false) != 1) return -1;
        total_wire += payload_len;
        if (masked) {
          for (size_t i = 0u; i < payload_len; ++i) dst[i] ^= mask_key[i & 3u];
        }
      }
      if (fin) {
        out_wire_bytes = total_wire;
        return 1;
      }
      assembling = true;
    }
  }

  bool connect_impl(const PairClientConfig& cfg, WakeCallback wake_cb, std::string* out_error) {
    auto fail = [&](std::string msg) {
      close_sockets_locked();
      if (out_error != nullptr) *out_error = std::move(msg);
      return false;
    };

    disconnect();
#if defined(_WIN32)
    if (!WinsockRuntime::ensure_initialized()) return fail("WSAStartup failed initializing WinSock2");
#endif

    config = cfg;
    config.max_window = static_cast<uint16_t>(std::clamp<uint16_t>(cfg.max_window, 4u, 1024u));
    config.handshake_timeout_ms = std::max<uint32_t>(cfg.handshake_timeout_ms, 50u);
    config.heartbeat_interval_ms = std::max<uint32_t>(cfg.heartbeat_interval_ms, 10u);
    on_frame_ready = std::move(wake_cb);
    started_at = Clock::now();
    shutdown_flag.store(false, std::memory_order_release);
    connected.store(false, std::memory_order_release);
    session_id.store(0u, std::memory_order_release);
    has_pending.store(false, std::memory_order_release);
    { std::lock_guard<std::mutex> lock(queue_mutex); pending_frames.clear(); }
    { std::lock_guard<std::mutex> lock(metrics_mutex); metrics = PairClientMetrics{}; }

    if (!create_wake_pair(wake_fds)) return fail("Failed to create internal wake pipe/socket pair");

    std::string sub_err;
    if (!dial_pair_endpoint(config, sock_fd, peer_addr, peer_addr_len, &sub_err)) {
      return fail(std::move(sub_err));
    }
    if (config.transport == PairTransport::WebSocket &&
        !perform_ws_upgrade_fd(sock_fd, wake_fds[0], shutdown_flag, config, &sub_err)) {
      return fail(std::move(sub_err));
    }

    const uint64_t client_nonce = next_nonce(0x2468ACE0ULL ^ static_cast<uint64_t>(config.port));
    HelloPayload hello{config.schema_version, config.max_window, 0u, client_nonce,
                       pair_wire::derive_token_proof(config.pairing_token, config.schema_version)};
    const auto hello_env = pair_wire::encode_envelope(
        TransportMsgKind::Hello, 0u, pair_wire::encode_hello_payload(hello));

    const auto deadline = Clock::now() + std::chrono::milliseconds(config.handshake_timeout_ms);
    uint32_t negotiated_sid = 0u;
    size_t negotiated_window = config.max_window;
    bool handshake_ok = false;

    if (config.transport == PairTransport::Udp) {
      std::optional<Clock::time_point> next_resend;
      std::array<uint8_t, 2048> dgram{};
      while (Clock::now() < deadline) {
        const auto now = Clock::now();
        if (!next_resend.has_value() || now >= *next_resend) {
          if (!send_wire_message(hello_env)) return fail("Failed sending UDP Hello datagram");
          next_resend = now + std::chrono::milliseconds(25);
        }
        const int ready = wait_readable_fd(
            sock_fd, wake_fds[0], shutdown_flag,
            compute_wait_timeout_ms(Clock::now(), std::min(deadline, *next_resend)));
        if (ready < 0) return fail("Interrupted during UDP pairing handshake");
        if (ready == 0) continue;
        const ptrdiff_t n = socket_recvfrom(sock_fd, dgram.data(), dgram.size());
        if (n <= 0) continue;
        { std::lock_guard<std::mutex> lock(metrics_mutex); metrics.wire_bytes_received += static_cast<uint64_t>(n); }
        TransportEnvelopeView env{};
        if (!pair_wire::parse_envelope(std::span<const uint8_t>(dgram.data(), static_cast<size_t>(n)), env)) continue;
        if (env.kind == TransportMsgKind::HelloAck || env.kind == TransportMsgKind::Reject) {
          if (!verify_hello_ack_envelope(config, env, client_nonce, negotiated_sid, negotiated_window, &sub_err)) {
            return fail(std::move(sub_err));
          }
          handshake_ok = true;
          break;
        }
      }
    } else {
      if (!send_wire_message(hello_env)) return fail("Failed sending TCP/WS Hello handshake");
      std::vector<uint8_t> reply;
      size_t wire_len = 0u;
      while (Clock::now() < deadline) {
        const int rc = read_stream_message(
            sock_fd, reply, wire_len, compute_wait_timeout_ms(Clock::now(), deadline));
        if (rc < 0) return fail("Connection closed during pairing handshake");
        if (rc == 0) continue;
        { std::lock_guard<std::mutex> lock(metrics_mutex); metrics.wire_bytes_received += wire_len; }
        TransportEnvelopeView env{};
        if (!pair_wire::parse_envelope(reply, env)) return fail("Malformed NVT1 envelope during handshake");
        if (!verify_hello_ack_envelope(config, env, client_nonce, negotiated_sid, negotiated_window, &sub_err)) {
          return fail(std::move(sub_err));
        }
        handshake_ok = true;
        break;
      }
    }
    if (!handshake_ok) return fail("Pairing handshake timed out");

    session_id.store(negotiated_sid, std::memory_order_release);
    connected.store(true, std::memory_order_release);
    { std::lock_guard<std::mutex> lock(metrics_mutex); metrics.session_id = negotiated_sid; }
    (void)send_wire_message(pair_wire::encode_ping_or_pong(TransportMsgKind::Ping, negotiated_sid, now_us()));
    reader_thread = std::thread([this, negotiated_sid, negotiated_window]() {
      run_reader_loop(negotiated_sid, negotiated_window);
    });
    if (out_error != nullptr) out_error->clear();
    return true;
  }

  void handle_nvd_packet(
      uint32_t sid,
      pair_wire::SyncDeltaReceiver& receiver,
      std::span<const uint8_t> nvd_bytes,
      std::unordered_map<uint32_t, Clock::time_point>& nack_cooldowns,
      std::optional<Clock::time_point>& last_kf_req) {
    const auto now = Clock::now();
    DeltaPacketView parsed{};
    if (!pair_wire::parse_nvd_packet(nvd_bytes, parsed)) {
      std::lock_guard<std::mutex> lock(metrics_mutex);
      ++metrics.malformed_packets_dropped;
      return;
    }
    auto send_kf_req = [&](uint8_t reason) {
      if (!last_kf_req.has_value() ||
          std::chrono::duration_cast<std::chrono::milliseconds>(now - *last_kf_req).count() >= 25) {
        last_kf_req = now;
        if (send_wire_message(pair_wire::encode_keyframe_request(sid, receiver.last_applied_seq(), reason))) {
          std::lock_guard<std::mutex> lock(metrics_mutex);
          ++metrics.keyframe_resyncs;
        }
      }
    };

    const uint32_t last_applied = receiver.last_applied_seq();
    const uint32_t max_win = static_cast<uint32_t>(receiver.max_window());
    if (config.transport == PairTransport::Udp && parsed.header.kind != DeltaPacketKind::Keyframe &&
        parsed.header.seq > last_applied && (parsed.header.seq - last_applied) > max_win) {
      send_kf_req(2u);
      return;
    }
    DeltaReceiveOutcome outcome = receiver.receive_packet(nvd_bytes);
    if (outcome.status == DeltaReceiveStatus::BaseSequenceMismatch) {
      send_kf_req(3u);
      return;
    }
    if (outcome.status != DeltaReceiveStatus::Ok) {
      std::lock_guard<std::mutex> lock(metrics_mutex);
      ++metrics.malformed_packets_dropped;
      return;
    }
    for (const auto& [seq, _] : outcome.applied_frames) nack_cooldowns.erase(seq);
    if (!outcome.applied_frames.empty()) push_applied_frames(outcome.applied_frames, receiver.last_applied_seq());
    if (outcome.ack_packet.has_value() &&
        send_wire_message(pair_wire::encode_envelope(TransportMsgKind::DeltaSingle, sid, *outcome.ack_packet))) {
      std::lock_guard<std::mutex> lock(metrics_mutex);
      ++metrics.acks_sent;
    }
    if (config.transport == PairTransport::Udp) {
      for (const auto& nack_nvd : outcome.nack_packets) {
        DeltaPacketView np{};
        if (!pair_wire::parse_nvd_packet(nack_nvd, np) || np.payload.size() != 4u) continue;
        const uint32_t missing_seq = read_u32_le(np.payload.data());
        if (parsed.header.seq >= missing_seq && (parsed.header.seq - missing_seq) < 2u) continue;
        auto it = nack_cooldowns.find(missing_seq);
        if (it == nack_cooldowns.end() ||
            std::chrono::duration_cast<std::chrono::milliseconds>(now - it->second).count() >= 25) {
          nack_cooldowns[missing_seq] = now;
          if (send_wire_message(pair_wire::encode_envelope(TransportMsgKind::DeltaSingle, sid, nack_nvd))) {
            std::lock_guard<std::mutex> lock(metrics_mutex);
            ++metrics.nacks_sent;
          }
        }
      }
    }
  }

  void run_reader_loop(uint32_t sid, size_t window) {
    pair_wire::SyncDeltaReceiver receiver(window);
    pair_wire::FragReassembler reassembler(clamped_max_frame_bytes());
    std::unordered_map<uint32_t, Clock::time_point> nack_cooldowns;
    std::optional<Clock::time_point> last_kf_req;
    const auto hb_interval = std::chrono::milliseconds(config.heartbeat_interval_ms);
    constexpr auto kFragRetryInterval = std::chrono::milliseconds(15);
    constexpr auto kSeqNackCooldown = std::chrono::milliseconds(18);
    auto next_ping = Clock::now() + hb_interval;
    std::optional<Clock::time_point> next_frag_rto;
    std::array<uint8_t, 2048> udp_buf{};
    std::vector<uint8_t> stream_buf;

    while (!shutdown_flag.load(std::memory_order_acquire)) {
      const auto loop_now = Clock::now();
      Clock::time_point next_deadline = next_ping;
      if (config.transport == PairTransport::Udp) {
        if (reassembler.inflight_count() > 0u) {
          if (!next_frag_rto.has_value()) next_frag_rto = loop_now + kFragRetryInterval;
          next_deadline = std::min(next_deadline, *next_frag_rto);
        } else {
          next_frag_rto.reset();
        }
        if (const auto mseq = receiver.next_missing_seq(); mseq.has_value()) {
          const auto it = nack_cooldowns.find(*mseq);
          next_deadline = std::min(
              next_deadline, (it == nack_cooldowns.end()) ? loop_now : (it->second + kSeqNackCooldown));
        }
      }
      const int wait_ms = compute_wait_timeout_ms(loop_now, next_deadline);

      std::span<const uint8_t> raw_msg;
      if (config.transport == PairTransport::Udp) {
        const int ready = wait_readable_fd(sock_fd, wake_fds[0], shutdown_flag, wait_ms);
        if (ready < 0) break;
        if (ready > 0) {
          const ptrdiff_t n = socket_recvfrom(sock_fd, udp_buf.data(), udp_buf.size());
          if (n > 0) {
            {
              std::lock_guard<std::mutex> lock(metrics_mutex);
              metrics.wire_bytes_received += static_cast<uint64_t>(n);
            }
            raw_msg = std::span<const uint8_t>(udp_buf.data(), static_cast<size_t>(n));
          } else if (n < 0) {
            const int err = last_socket_error();
            if (!is_would_block(err) && !is_interrupted(err) && !is_udp_conn_reset(err)) break;
          }
        }
      } else {
        size_t wire_len = 0u;
        const int rc = read_stream_message(sock_fd, stream_buf, wire_len, wait_ms);
        if (rc < 0) break;
        if (rc > 0) {
          {
            std::lock_guard<std::mutex> lock(metrics_mutex);
            metrics.wire_bytes_received += wire_len;
          }
          raw_msg = stream_buf;
        }
      }

      if (!raw_msg.empty()) {
        TransportEnvelopeView env{};
        if (!pair_wire::parse_envelope(raw_msg, env) || env.session_id != sid) {
          std::lock_guard<std::mutex> lock(metrics_mutex);
          ++metrics.malformed_packets_dropped;
        } else if (env.kind == TransportMsgKind::DeltaSingle) {
          handle_nvd_packet(sid, receiver, env.payload, nack_cooldowns, last_kf_req);
        } else if (env.kind == TransportMsgKind::DeltaFrag) {
          std::optional<std::vector<uint8_t>> completed;
          const auto frag_now = Clock::now();
          if (!reassembler.push(env.payload, frag_now, completed)) {
            std::lock_guard<std::mutex> lock(metrics_mutex);
            ++metrics.malformed_packets_dropped;
          } else if (completed.has_value()) {
            handle_nvd_packet(sid, receiver, *completed, nack_cooldowns, last_kf_req);
          } else if (!next_frag_rto.has_value()) {
            next_frag_rto = frag_now + kFragRetryInterval;
          }
        } else if (env.kind == TransportMsgKind::Ping) {
          uint64_t ts = 0u;
          if (pair_wire::decode_timestamp_us(env.payload, ts)) {
            (void)send_wire_message(pair_wire::encode_ping_or_pong(TransportMsgKind::Pong, sid, ts));
          }
        } else if (env.kind == TransportMsgKind::Bye) {
          shutdown_flag.store(true, std::memory_order_release);
        } else if (env.kind != TransportMsgKind::Pong) {
          std::lock_guard<std::mutex> lock(metrics_mutex);
          ++metrics.malformed_packets_dropped;
        }
      }

      const auto now = Clock::now();
      if (config.transport == PairTransport::Udp) {
        if (reassembler.inflight_count() > 0u && next_frag_rto.has_value() && now >= *next_frag_rto) {
          for (const auto& [mid, missing] : reassembler.poll_stale_nacks(now, 15u)) {
            if (send_wire_message(pair_wire::encode_frag_nack(sid, mid, missing))) {
              std::lock_guard<std::mutex> lock(metrics_mutex);
              ++metrics.nacks_sent;
            }
          }
          next_frag_rto = (reassembler.inflight_count() > 0u)
                              ? std::optional<Clock::time_point>(now + kFragRetryInterval)
                              : std::nullopt;
        }
        if (const auto mseq = receiver.next_missing_seq(); mseq.has_value()) {
          auto it = nack_cooldowns.find(*mseq);
          if (it == nack_cooldowns.end() || (now - it->second) >= kSeqNackCooldown) {
            nack_cooldowns[*mseq] = now;
            const auto env = pair_wire::encode_envelope(
                TransportMsgKind::DeltaSingle, sid, receiver.build_nack_packet(*mseq));
            if (send_wire_message(env)) {
              std::lock_guard<std::mutex> lock(metrics_mutex);
              ++metrics.nacks_sent;
            }
          }
        }
      }
      if (now >= next_ping) {
        next_ping = now + hb_interval;
        if (!send_wire_message(pair_wire::encode_ping_or_pong(TransportMsgKind::Ping, sid, now_us())) &&
            config.transport != PairTransport::Udp) {
          break;
        }
      }
    }
    reassembler.clear();
    connected.store(false, std::memory_order_release);
  }

  void disconnect() noexcept {
    const uint32_t sid = session_id.load(std::memory_order_acquire);
    if (connected.exchange(false, std::memory_order_acq_rel) && sid != 0u) {
      (void)send_wire_message(pair_wire::encode_envelope(TransportMsgKind::Bye, sid, {}));
    }
    shutdown_flag.store(true, std::memory_order_release);
    signal_wake_pair(wake_fds[1]);
    {
      std::lock_guard<std::mutex> wlock(write_mutex);
      if (is_valid_socket(sock_fd)) shutdown_socket(sock_fd);
    }
    if (reader_thread.joinable()) reader_thread.join();
    {
      std::lock_guard<std::mutex> wlock(write_mutex);
      close_sockets_locked();
    }
    session_id.store(0u, std::memory_order_release);
    on_frame_ready = {};
  }
};

PairClient::PairClient() : impl_(std::make_unique<Impl>()) {}
PairClient::~PairClient() = default;

bool PairClient::connect(
    const PairClientConfig& config, WakeCallback on_frame_ready, std::string* out_error) {
  return impl_->connect_impl(config, std::move(on_frame_ready), out_error);
}
void PairClient::disconnect() noexcept { impl_->disconnect(); }
bool PairClient::is_connected() const noexcept {
  return impl_->connected.load(std::memory_order_acquire);
}
uint32_t PairClient::session_id() const noexcept {
  return impl_->session_id.load(std::memory_order_acquire);
}
PairTransport PairClient::transport() const noexcept { return impl_->config.transport; }
PairClientMetrics PairClient::metrics() const {
  std::lock_guard<std::mutex> lock(impl_->metrics_mutex);
  return impl_->metrics;
}
bool PairClient::has_pending_frame() const noexcept {
  return impl_->has_pending.load(std::memory_order_acquire);
}

bool PairClient::pop_frame(std::vector<uint8_t>& out_frame, uint32_t* out_seq) {
  std::lock_guard<std::mutex> lock(impl_->queue_mutex);
  if (impl_->pending_frames.empty()) {
    impl_->has_pending.store(false, std::memory_order_release);
    return false;
  }
  auto item = std::move(impl_->pending_frames.front());
  impl_->pending_frames.pop_front();
  impl_->has_pending.store(!impl_->pending_frames.empty(), std::memory_order_release);
  if (out_seq != nullptr) *out_seq = item.first;
  out_frame = std::move(item.second);
  return true;
}

size_t PairClient::drain_pending_frames(
    std::vector<std::pair<uint32_t, std::vector<uint8_t>>>& out_frames) {
  out_frames.clear();
  std::lock_guard<std::mutex> lock(impl_->queue_mutex);
  if (impl_->pending_frames.empty()) {
    impl_->has_pending.store(false, std::memory_order_release);
    return 0u;
  }
  out_frames.reserve(impl_->pending_frames.size());
  while (!impl_->pending_frames.empty()) {
    out_frames.push_back(std::move(impl_->pending_frames.front()));
    impl_->pending_frames.pop_front();
  }
  impl_->has_pending.store(false, std::memory_order_release);
  return out_frames.size();
}

bool PairClient::take_latest_frame(std::vector<uint8_t>& out_frame, uint32_t* out_seq) {
  std::lock_guard<std::mutex> lock(impl_->queue_mutex);
  if (impl_->pending_frames.empty()) {
    impl_->has_pending.store(false, std::memory_order_release);
    return false;
  }
  auto latest = std::move(impl_->pending_frames.back());
  impl_->pending_frames.clear();
  impl_->has_pending.store(false, std::memory_order_release);
  if (out_seq != nullptr) *out_seq = latest.first;
  out_frame = std::move(latest.second);
  return true;
}

bool PairClient::send_events(
    std::span<const uint8_t> encoded_event_ring, std::string* out_error) {
  if (!is_connected()) {
    if (out_error != nullptr) *out_error = "PairClient is not connected";
    return false;
  }
  if (!validate_input_events_bytes(encoded_event_ring, impl_->clamped_max_event_bytes())) {
    if (out_error != nullptr) *out_error = "Invalid or oversized EventRing payload";
    return false;
  }
  const auto env =
      pair_wire::encode_envelope(TransportMsgKind::InputEvents, session_id(), encoded_event_ring);
  if (!impl_->send_wire_message(env)) {
    if (out_error != nullptr) *out_error = "Failed writing InputEvents frame";
    return false;
  }
  {
    std::lock_guard<std::mutex> lock(impl_->metrics_mutex);
    ++impl_->metrics.events_sent;
  }
  if (out_error != nullptr) out_error->clear();
  return true;
}

bool PairClient::request_keyframe() {
  if (!is_connected()) return false;
  uint32_t last_seq = 0u;
  {
    std::lock_guard<std::mutex> lock(impl_->metrics_mutex);
    last_seq = impl_->metrics.last_applied_seq;
  }
  if (!impl_->send_wire_message(pair_wire::encode_keyframe_request(session_id(), last_seq, 1u))) {
    return false;
  }
  std::lock_guard<std::mutex> lock(impl_->metrics_mutex);
  ++impl_->metrics.keyframe_resyncs;
  return true;
}

bool PairClient::send_ping() {
  if (!is_connected()) return false;
  return impl_->send_wire_message(
      pair_wire::encode_ping_or_pong(TransportMsgKind::Ping, session_id(), impl_->now_us()));
}

} // namespace nve
