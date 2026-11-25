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
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

#include "command_encoder.hpp"
#include "mock_jsi_recorder.hpp"
#include "nve_host_runtime.hpp"
#include "nve_pair_client.hpp"

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#define NVE_CHECK(expr)                                                                         \
  do {                                                                                          \
    if (!(expr)) {                                                                              \
      std::cerr << "NVE_CHECK failed at " << __FILE__ << ":" << __LINE__ << ": " #expr << "\n"; \
      std::abort();                                                                             \
    }                                                                                           \
  } while (0)

namespace {

#if defined(_WIN32)
using TestSocket = SOCKET;
using TestSockLen = int;

struct TestWinsockGuard {
  TestWinsockGuard() {
    WSADATA data{};
    NVE_CHECK(::WSAStartup(MAKEWORD(2, 2), &data) == 0);
  }
  ~TestWinsockGuard() { ::WSACleanup(); }
};

inline bool test_sock_valid(TestSocket s) noexcept { return s != INVALID_SOCKET; }
inline void test_close(TestSocket s) noexcept {
  if (test_sock_valid(s)) ::closesocket(s);
}
inline int test_poll_read(TestSocket s, int timeout_ms) noexcept {
  WSAPOLLFD pfd{};
  pfd.fd = s;
  pfd.events = POLLIN;
  return ::WSAPoll(&pfd, 1u, timeout_ms);
}
inline ptrdiff_t test_send(TestSocket s, const uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(
      ::send(s, reinterpret_cast<const char*>(data), static_cast<int>(len), 0));
}
inline ptrdiff_t test_recv(TestSocket s, uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(
      ::recv(s, reinterpret_cast<char*>(data), static_cast<int>(len), 0));
}
inline ptrdiff_t test_sendto(
    TestSocket s,
    const uint8_t* data,
    size_t len,
    const sockaddr* addr,
    TestSockLen alen) noexcept {
  return static_cast<ptrdiff_t>(
      ::sendto(s, reinterpret_cast<const char*>(data), static_cast<int>(len), 0, addr, alen));
}
inline ptrdiff_t test_recvfrom(
    TestSocket s,
    uint8_t* data,
    size_t len,
    sockaddr* addr,
    TestSockLen* alen) noexcept {
  return static_cast<ptrdiff_t>(
      ::recvfrom(s, reinterpret_cast<char*>(data), static_cast<int>(len), 0, addr, alen));
}
inline void test_set_reuseaddr(TestSocket s) noexcept {
  const int reuse = 1;
  (void)::setsockopt(
      s, SOL_SOCKET, SO_REUSEADDR, reinterpret_cast<const char*>(&reuse), sizeof(reuse));
}
#else
using TestSocket = int;
using TestSockLen = socklen_t;

struct TestWinsockGuard {};

inline bool test_sock_valid(TestSocket s) noexcept { return s >= 0; }
inline void test_close(TestSocket s) noexcept {
  if (test_sock_valid(s)) ::close(s);
}
inline int test_poll_read(TestSocket s, int timeout_ms) noexcept {
  pollfd pfd{s, POLLIN, 0};
  return ::poll(&pfd, 1u, timeout_ms);
}
inline ptrdiff_t test_send(TestSocket s, const uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(::send(s, data, len, 0));
}
inline ptrdiff_t test_recv(TestSocket s, uint8_t* data, size_t len) noexcept {
  return static_cast<ptrdiff_t>(::recv(s, data, len, 0));
}
inline ptrdiff_t test_sendto(
    TestSocket s,
    const uint8_t* data,
    size_t len,
    const sockaddr* addr,
    TestSockLen alen) noexcept {
  return static_cast<ptrdiff_t>(::sendto(s, data, len, 0, addr, alen));
}
inline ptrdiff_t test_recvfrom(
    TestSocket s,
    uint8_t* data,
    size_t len,
    sockaddr* addr,
    TestSockLen* alen) noexcept {
  return static_cast<ptrdiff_t>(::recvfrom(s, data, len, 0, addr, alen));
}
inline void test_set_reuseaddr(TestSocket s) noexcept {
  const int reuse = 1;
  (void)::setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
}
#endif

std::vector<uint8_t> build_scene_cmd_buf(
    uint32_t surface_id,
    uint64_t frame_id,
    const std::string& label_text,
    bool extra_nodes = false) {
  nve::CommandBufferEncoder enc(surface_id, frame_id);
  enc.create_node(2u, NVE_COMP_VIEW);
  enc.create_node(3u, NVE_COMP_TEXT);
  enc.create_node(4u, NVE_COMP_PRESSABLE);
  enc.create_node(5u, NVE_COMP_TEXT);

  std::vector<uint32_t> root_children{2u};
  std::vector<uint32_t> container_children{3u, 4u};
  if (extra_nodes) {
    for (uint32_t i = 0u; i < 24u; ++i) {
      const uint32_t nid = 20u + i;
      enc.create_node(nid, NVE_COMP_TEXT);
      enc.set_text(
          nid,
          "UDP Fragmentation Payload Row #" + std::to_string(i) +
              " — verifying out-of-order 1200B MTU DeltaFrag reassembly end-to-end!");
      container_children.push_back(nid);
    }
  }

  enc.set_children(NVE_ROOT_NODE_ID, root_children);
  enc.set_children(2u, container_children);
  enc.set_children(4u, {5u});

  nve::LayoutProps box{};
  box.width = {1u, 320.0f};
  box.height = {1u, 200.0f};
  box.padding = {12.0f, 12.0f, 12.0f, 12.0f};
  box.gap = 8.0f;
  enc.set_layout(2u, box);

  nve::LayoutProps btn{};
  btn.width = {1u, 180.0f};
  btn.height = {1u, 44.0f};
  enc.set_layout(4u, btn);

  enc.set_text(3u, label_text);
  enc.set_text(5u, "Tap");

  nve::StyleProps st{};
  st.background_rgba = 0xFF1E293Bu;
  st.text_color_rgba = 0xFFFFFFFFu;
  st.border_radius = 10.0f;
  st.font_size = 18.0f;
  enc.set_style(2u, st);
  enc.set_event_mask(4u, nve::kEventMaskPress);
  enc.commit(true);

  const NveBufferSlice slice = enc.finish();
  return std::vector<uint8_t>(slice.ptr, slice.ptr + slice.len);
}

bool send_all(TestSocket fd, const uint8_t* data, size_t len) {
  size_t off = 0u;
  while (off < len) {
    const ptrdiff_t n = test_send(fd, data + off, len - off);
    if (n <= 0) return false;
    off += static_cast<size_t>(n);
  }
  return true;
}

bool recv_exact(TestSocket fd, uint8_t* buf, size_t len, int timeout_ms = 2000) {
  size_t off = 0u;
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(timeout_ms);
  while (off < len) {
    const auto now = std::chrono::steady_clock::now();
    if (now >= deadline) return false;
    const int rem = static_cast<int>(
        std::chrono::duration_cast<std::chrono::milliseconds>(deadline - now).count());
    const int rc = test_poll_read(fd, std::max(1, std::min(rem, 50)));
    if (rc <= 0) continue;
    const ptrdiff_t n = test_recv(fd, buf + off, len - off);
    if (n <= 0) return false;
    off += static_cast<size_t>(n);
  }
  return true;
}

bool write_tcp_framed(TestSocket fd, std::span<const uint8_t> payload) {
  uint8_t hdr[4]{
      static_cast<uint8_t>(payload.size() & 0xFFu),
      static_cast<uint8_t>((payload.size() >> 8u) & 0xFFu),
      static_cast<uint8_t>((payload.size() >> 16u) & 0xFFu),
      static_cast<uint8_t>((payload.size() >> 24u) & 0xFFu),
  };
  return send_all(fd, hdr, 4u) && send_all(fd, payload.data(), payload.size());
}

bool read_tcp_framed(TestSocket fd, std::vector<uint8_t>& out, int timeout_ms = 2000) {
  uint8_t hdr[4]{};
  if (!recv_exact(fd, hdr, 4u, timeout_ms)) return false;
  const uint32_t len = static_cast<uint32_t>(hdr[0]) | (static_cast<uint32_t>(hdr[1]) << 8u) |
                       (static_cast<uint32_t>(hdr[2]) << 16u) |
                       (static_cast<uint32_t>(hdr[3]) << 24u);
  if (len == 0u || len > 4u * 1024u * 1024u) return false;
  out.resize(len);
  return recv_exact(fd, out.data(), len, timeout_ms);
}

bool write_ws_binary(TestSocket fd, std::span<const uint8_t> payload, bool fragmented = false) {
  if (!fragmented || payload.size() < 4u) {
    const auto frame = nve::pair_wire::encode_ws_frame(true, nve::WsOpcode::Binary, payload);
    return send_all(fd, frame.data(), frame.size());
  }
  const size_t half = payload.size() / 2u;
  const auto f1 =
      nve::pair_wire::encode_ws_frame(false, nve::WsOpcode::Binary, payload.subspan(0u, half));
  const auto f2 =
      nve::pair_wire::encode_ws_frame(true, nve::WsOpcode::Continuation, payload.subspan(half));
  return send_all(fd, f1.data(), f1.size()) && send_all(fd, f2.data(), f2.size());
}

bool read_ws_binary(TestSocket fd, std::vector<uint8_t>& out, int timeout_ms = 2000) {
  for (;;) {
    uint8_t hdr[2]{};
    if (!recv_exact(fd, hdr, 2u, timeout_ms)) return false;
    const uint8_t opcode = hdr[0] & 0x0Fu;
    const bool masked = (hdr[1] & 0x80u) != 0u;
    size_t len = hdr[1] & 0x7Fu;
    if (len == 126u) {
      uint8_t b[2]{};
      if (!recv_exact(fd, b, 2u, timeout_ms)) return false;
      len = (static_cast<size_t>(b[0]) << 8u) | static_cast<size_t>(b[1]);
    } else if (len == 127u) {
      uint8_t b[8]{};
      if (!recv_exact(fd, b, 8u, timeout_ms)) return false;
      len = 0u;
      for (int i = 0; i < 8; ++i) len = (len << 8u) | static_cast<size_t>(b[i]);
    }
    std::array<uint8_t, 4> mask{};
    if (masked && !recv_exact(fd, mask.data(), 4u, timeout_ms)) return false;
    out.resize(len);
    if (len > 0u && !recv_exact(fd, out.data(), len, timeout_ms)) return false;
    if (masked) {
      for (size_t i = 0u; i < len; ++i) out[i] ^= mask[i & 3u];
    }
    if (opcode == static_cast<uint8_t>(nve::WsOpcode::Binary)) return true;
    if (opcode == static_cast<uint8_t>(nve::WsOpcode::Close)) return false;
  }
}

bool handle_server_ws_upgrade(TestSocket fd) {
  std::string req;
  req.reserve(512u);
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
  while (std::chrono::steady_clock::now() < deadline) {
    uint8_t ch = 0u;
    if (!recv_exact(fd, &ch, 1u, 1000)) return false;
    req.push_back(static_cast<char>(ch));
    if (req.size() >= 4u && req.compare(req.size() - 4u, 4u, "\r\n\r\n") == 0) break;
  }
  const size_t kpos = req.find("Sec-WebSocket-Key:");
  if (kpos == std::string::npos) return false;
  size_t vstart = kpos + std::strlen("Sec-WebSocket-Key:");
  while (vstart < req.size() && req[vstart] == ' ') ++vstart;
  const size_t vend = req.find("\r\n", vstart);
  if (vend == std::string::npos) return false;
  const std::string key = req.substr(vstart, vend - vstart);
  const std::string accept = nve::pair_wire::compute_ws_accept_key(key);
  const std::string resp = "HTTP/1.1 101 Switching Protocols\r\n"
                           "Upgrade: websocket\r\n"
                           "Connection: Upgrade\r\n"
                           "Sec-WebSocket-Accept: " +
                           accept + "\r\n\r\n";
  return send_all(fd, reinterpret_cast<const uint8_t*>(resp.data()), resp.size());
}

// -----------------------------------------------------------------------------
// 1. Exact Rust `nve-interop` FNV-1a64 + Token Proof + Session ID + Endpoint Parsing
// -----------------------------------------------------------------------------
void test_rust_parity_vectors_and_endpoint_parser() {
  using namespace nve::pair_wire;

  const std::array<uint8_t, 4> nvd_magic = {'N', 'V', 'D', '1'};
  NVE_CHECK(fnv1a_64(nvd_magic) == 0x89cbe9c5c4abc8c6ULL);

  const auto proof = derive_token_proof("nve-live-secret-token", 1u);
  const std::array<uint8_t, 16> expected_proof = {
      0x5bu, 0x11u, 0x82u, 0x9bu, 0x5fu, 0x1bu, 0xb8u, 0x65u,
      0x8eu, 0x5cu, 0xb6u, 0x99u, 0x56u, 0x6fu, 0x22u, 0xfbu,
  };
  NVE_CHECK(ct_eq_16(proof, expected_proof));
  NVE_CHECK(!ct_eq_16(proof, derive_token_proof("nve-live-secret-token", 2u)));
  NVE_CHECK(!ct_eq_16(proof, derive_token_proof("wrong-token", 1u)));

  const uint32_t sid = derive_session_id(
      "nve-live-secret-token", 1u, 0x1122334455667788ULL, 0x99AABBCCDDEEFF00ULL);
  NVE_CHECK(sid == 0x3479b62eu);

  // Endpoint parser checks (including `auto`)
  nve::PairClientConfig cfg{};
  std::string err;
  NVE_CHECK(nve::parse_pair_endpoint("127.0.0.1:7777", "", cfg, &err));
  NVE_CHECK(cfg.host == "127.0.0.1" && cfg.port == 7777u && cfg.transport == nve::PairTransport::Tcp);

  NVE_CHECK(nve::parse_pair_endpoint("127.0.0.1:7777", "auto", cfg, &err));
  NVE_CHECK(cfg.transport == nve::PairTransport::Tcp && cfg.port == 7777u);

  NVE_CHECK(nve::parse_pair_endpoint("udp://127.0.0.1:7777", "auto", cfg, &err));
  NVE_CHECK(cfg.transport == nve::PairTransport::Udp && cfg.port == 7777u);

  NVE_CHECK(nve::parse_pair_endpoint("127.0.0.1:7777", "udp", cfg, &err));
  NVE_CHECK(cfg.transport == nve::PairTransport::Udp && cfg.port == 7777u);

  NVE_CHECK(nve::parse_pair_endpoint("ws://192.168.1.5:8088/nve/sync", "", cfg, &err));
  NVE_CHECK(cfg.host == "192.168.1.5" && cfg.port == 8088u && cfg.ws_path == "/nve/sync" &&
            cfg.transport == nve::PairTransport::WebSocket);

  NVE_CHECK(nve::parse_pair_endpoint("udp://[::1]:9001", "", cfg, &err));
  NVE_CHECK(cfg.host == "::1" && cfg.port == 9001u && cfg.transport == nve::PairTransport::Udp);

  // Invalid endpoints rejected cleanly
  NVE_CHECK(!nve::parse_pair_endpoint("", "", cfg, &err));
  NVE_CHECK(!nve::parse_pair_endpoint("127.0.0.1", "", cfg, &err));
  NVE_CHECK(!nve::parse_pair_endpoint("127.0.0.1:0", "", cfg, &err));
  NVE_CHECK(!nve::parse_pair_endpoint("127.0.0.1:70000", "", cfg, &err));
  NVE_CHECK(!nve::parse_pair_endpoint("ftp://127.0.0.1:21", "", cfg, &err));
  NVE_CHECK(!nve::parse_pair_endpoint("127.0.0.1:7777", "quic", cfg, &err));
}

// -----------------------------------------------------------------------------
// 2. NVD1 Keyframe + XorRleDelta + CommandOpDelta + Reorder + SACK/NACK
// -----------------------------------------------------------------------------
void test_nvd1_codecs_and_reorder_receiver() {
  using namespace nve::pair_wire;

  const auto f1 = build_scene_cmd_buf(7u, 1u, "Frame 1 Initial");
  const auto f2 = build_scene_cmd_buf(7u, 2u, "Frame 2 CommandOp");
  const auto f3 = build_scene_cmd_buf(7u, 3u, "Frame 3 XorRle Final!");

  const auto pkt1 = encode_nvd_packet(nve::DeltaPacketKind::Keyframe, 0u, 1u, 0u, 0u, f1);
  const auto cmd_delta = encode_command_op_delta(f1, f2);
  NVE_CHECK(cmd_delta.has_value());
  NVE_CHECK(cmd_delta->size() < f2.size());
  const auto pkt2 =
      encode_nvd_packet(nve::DeltaPacketKind::CommandOpDelta, 0u, 2u, 1u, 0u, *cmd_delta);
  const auto xor_delta = encode_xor_rle_delta(f2, f3);
  NVE_CHECK(xor_delta.size() < f3.size());
  const auto pkt3 =
      encode_nvd_packet(nve::DeltaPacketKind::XorRleDelta, 0u, 3u, 2u, 0u, xor_delta);

  SyncDeltaReceiver rx(64u);
  const auto r1 = rx.receive_packet(pkt1);
  NVE_CHECK(r1.status == nve::DeltaReceiveStatus::Ok);
  NVE_CHECK(r1.applied_frames.size() == 1u && r1.applied_frames[0].first == 1u);
  NVE_CHECK(r1.applied_frames[0].second == f1);
  NVE_CHECK(r1.ack_packet.has_value());

  const auto r3 = rx.receive_packet(pkt3);
  NVE_CHECK(r3.status == nve::DeltaReceiveStatus::Ok);
  NVE_CHECK(r3.applied_frames.empty());
  NVE_CHECK(rx.reorder_buffer_len() == 1u);
  NVE_CHECK(rx.next_missing_seq().has_value() && *rx.next_missing_seq() == 2u);
  NVE_CHECK(r3.nack_packets.size() == 1u);
  NVE_CHECK(r3.ack_packet.has_value());
  nve::DeltaPacketView ack3{};
  NVE_CHECK(parse_nvd_packet(*r3.ack_packet, ack3));
  NVE_CHECK(ack3.header.kind == nve::DeltaPacketKind::Ack);
  NVE_CHECK(ack3.header.ack_seq == 1u);
  NVE_CHECK(ack3.payload.size() == 8u);
  NVE_CHECK((ack3.payload[0] & 0x02u) != 0u);

  const auto r2 = rx.receive_packet(pkt2);
  NVE_CHECK(r2.status == nve::DeltaReceiveStatus::Ok);
  NVE_CHECK(r2.applied_frames.size() == 2u);
  NVE_CHECK(r2.applied_frames[0].first == 2u && r2.applied_frames[0].second == f2);
  NVE_CHECK(r2.applied_frames[1].first == 3u && r2.applied_frames[1].second == f3);
  NVE_CHECK(rx.last_applied_seq() == 3u && rx.reorder_buffer_len() == 0u);

  const auto dup = rx.receive_packet(pkt2);
  NVE_CHECK(dup.status == nve::DeltaReceiveStatus::Ok && dup.applied_frames.empty() &&
            dup.ack_packet.has_value());

  const auto bad_base =
      encode_nvd_packet(nve::DeltaPacketKind::XorRleDelta, 0u, 4u, 99u, 0u, xor_delta);
  const auto r_bad = rx.receive_packet(bad_base);
  NVE_CHECK(r_bad.status == nve::DeltaReceiveStatus::BaseSequenceMismatch);

  // Large XOR-RLE roundtrip (base shorter than next, exercising pre-reserve & tail copy).
  std::vector<uint8_t> base_big(2048u, 0x3Cu);
  std::vector<uint8_t> next_big(8192u, 0x3Cu);
  for (size_t i = 512u; i < 4096u; i += 3u) {
    next_big[i] = static_cast<uint8_t>(i & 0xFFu);
  }
  const auto big_delta = encode_xor_rle_delta(base_big, next_big);
  std::vector<uint8_t> decoded_big;
  NVE_CHECK(decode_xor_rle_delta(base_big, big_delta, decoded_big));
  NVE_CHECK(decoded_big == next_big);
}

// -----------------------------------------------------------------------------
// 3. Out-of-order UDP 1200B `DeltaFrag` Reassembly + Stale NACKs + Adversarial Checks
// -----------------------------------------------------------------------------
void test_udp_fragmentation_and_adversarial_packets() {
  using namespace nve::pair_wire;

  const auto big_frame = build_scene_cmd_buf(3u, 1u, "Big Fragmented Keyframe", /*extra_nodes=*/true);
  NVE_CHECK(big_frame.size() > 2500u);
  const auto big_nvd =
      encode_nvd_packet(nve::DeltaPacketKind::Keyframe, 0u, 1u, 0u, 0u, big_frame);
  const auto dgrams = packetize_udp_datagrams(0x12345678u, big_nvd);
  NVE_CHECK(dgrams.size() >= 3u);
  for (const auto& dg : dgrams) {
    NVE_CHECK(dg.size() <= nve::kMaxUdpDatagramBytes);
  }

  FragReassembler reasm(nve::kDefaultMaxWireFrameBytes);
  const auto t0 = std::chrono::steady_clock::now();
  std::optional<std::vector<uint8_t>> completed;

  for (size_t i = dgrams.size() - 1u; i >= 1u; --i) {
    nve::TransportEnvelopeView env{};
    NVE_CHECK(parse_envelope(dgrams[i], env));
    NVE_CHECK(env.kind == nve::TransportMsgKind::DeltaFrag && env.session_id == 0x12345678u);
    NVE_CHECK(reasm.push(env.payload, t0, completed));
    NVE_CHECK(!completed.has_value());
  }
  {
    nve::TransportEnvelopeView env{};
    NVE_CHECK(parse_envelope(dgrams[1], env));
    NVE_CHECK(reasm.push(env.payload, t0, completed));
    NVE_CHECK(!completed.has_value());
  }
  const auto stale = reasm.poll_stale_nacks(t0 + std::chrono::milliseconds(20), 15u);
  NVE_CHECK(stale.size() == 1u && stale[0].second.size() == 1u && stale[0].second[0] == 0u);

  {
    nve::TransportEnvelopeView env{};
    NVE_CHECK(parse_envelope(dgrams[0], env));
    NVE_CHECK(reasm.push(env.payload, t0 + std::chrono::milliseconds(25), completed));
    NVE_CHECK(completed.has_value());
    NVE_CHECK(*completed == big_nvd);
    NVE_CHECK(reasm.inflight_count() == 0u);
  }

  {
    nve::TransportEnvelopeView env{};
    const std::array<uint8_t, 4> short_env = {'N', 'T', 1u, 4u};
    NVE_CHECK(!parse_envelope(short_env, env));
    const std::array<uint8_t, 8> bad_magic = {'X', 'T', 1u, 4u, 0u, 0u, 0u, 0u};
    NVE_CHECK(!parse_envelope(bad_magic, env));
    const std::array<uint8_t, 8> bad_kind = {'N', 'T', 1u, 99u, 0u, 0u, 0u, 0u};
    NVE_CHECK(!parse_envelope(bad_kind, env));

    auto flipped = big_nvd;
    flipped.back() ^= 0x01u;
    nve::DeltaPacketView dv{};
    NVE_CHECK(!parse_nvd_packet(flipped, dv));
    NVE_CHECK(!parse_nvd_packet(std::span<const uint8_t>(big_nvd.data(), 16u), dv));

    std::vector<uint8_t> huge_frag(nve::kFragSubheaderSize + 10u, 0u);
    huge_frag[4] = 0u;
    huge_frag[6] = 1u;
    huge_frag[8] = 0xFFu;
    huge_frag[9] = 0xFFu;
    huge_frag[10] = 0xFFu;
    huge_frag[11] = 0x7Fu;
    NVE_CHECK(!reasm.push(huge_frag, t0, completed));

    const std::array<uint8_t, 12> bad_varint = {
        0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0x01u};
    std::vector<uint8_t> dec_out;
    NVE_CHECK(!decode_xor_rle_delta(big_frame, bad_varint, dec_out));
  }
}

// -----------------------------------------------------------------------------
// 4. RFC 6455 SHA-1 + Base64 + WebSocket Frame Codec (with frame reuse)
// -----------------------------------------------------------------------------
void test_rfc6455_ws_codec() {
  using namespace nve::pair_wire;

  NVE_CHECK(compute_ws_accept_key("dGhlIHNhbXBsZSBub25jZQ==") == "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");

  const std::vector<uint8_t> small_payload = {1u, 2u, 3u, 4u, 5u};
  const std::array<uint8_t, 4> mask = {0x37u, 0xFAu, 0x21u, 0x3Du};
  const auto encoded_small = encode_ws_frame(true, nve::WsOpcode::Binary, small_payload, mask);
  nve::WsFrame decoded{};
  size_t consumed = 0u;
  NVE_CHECK(decode_ws_frame_slice(encoded_small, 1024u, decoded, consumed));
  NVE_CHECK(consumed == encoded_small.size());
  NVE_CHECK(decoded.fin && decoded.opcode == nve::WsOpcode::Binary && decoded.masked);
  NVE_CHECK(decoded.payload == small_payload);

  std::vector<uint8_t> med_payload(300u, 0xABu);
  const auto encoded_med = encode_ws_frame(true, nve::WsOpcode::Binary, med_payload);
  NVE_CHECK(decode_ws_frame_slice(encoded_med, 1024u, decoded, consumed));
  NVE_CHECK(consumed == encoded_med.size() && decoded.payload == med_payload);
  NVE_CHECK(!decode_ws_frame_slice(encoded_med, 100u, decoded, consumed));

  // Empty payload into reused WsFrame clears cleanly without allocating
  const auto encoded_empty = encode_ws_frame(true, nve::WsOpcode::Pong, {});
  NVE_CHECK(decode_ws_frame_slice(encoded_empty, 1024u, decoded, consumed));
  NVE_CHECK(consumed == encoded_empty.size() && decoded.payload.empty());

  auto bad_rsv = encoded_small;
  bad_rsv[0] |= 0x40u;
  NVE_CHECK(!decode_ws_frame_slice(bad_rsv, 1024u, decoded, consumed));
}

// -----------------------------------------------------------------------------
// 5. Live Loopback TCP + WebSocket + UDP Roundtrip with NativeHostSession & FabricHost
// -----------------------------------------------------------------------------
void test_live_loopback_stream_roundtrip(bool use_ws) {
  const std::string token = "nve-wave10-secret-token";
  const auto expected_proof = nve::pair_wire::derive_token_proof(token, 1u);
  const auto frame1 = build_scene_cmd_buf(1u, 1u, "Paired Frame 1");
  const auto frame2 = build_scene_cmd_buf(1u, 2u, "Paired Frame 2 Delta!");

  TestSocket listen_fd = ::socket(AF_INET, SOCK_STREAM, 0);
  NVE_CHECK(test_sock_valid(listen_fd));
  test_set_reuseaddr(listen_fd);
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = 0;
  NVE_CHECK(::bind(listen_fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
  TestSockLen alen = static_cast<TestSockLen>(sizeof(addr));
  NVE_CHECK(::getsockname(listen_fd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
  const uint16_t port = ntohs(addr.sin_port);
  NVE_CHECK(::listen(listen_fd, 4) == 0);

  std::atomic<bool> server_saw_input_event{false};
  std::atomic<bool> server_saw_ack{false};
  std::atomic<bool> server_rejected_bad_token{false};
  std::vector<uint8_t> received_event_ring;
  std::mutex ev_mutex;

  std::thread server_thread([&]() {
    {
      TestSocket cfd = ::accept(listen_fd, nullptr, nullptr);
      NVE_CHECK(test_sock_valid(cfd));
      if (use_ws) NVE_CHECK(handle_server_ws_upgrade(cfd));
      std::vector<uint8_t> msg;
      NVE_CHECK(use_ws ? read_ws_binary(cfd, msg) : read_tcp_framed(cfd, msg));
      nve::TransportEnvelopeView env{};
      NVE_CHECK(nve::pair_wire::parse_envelope(msg, env) &&
                env.kind == nve::TransportMsgKind::Hello);
      nve::HelloPayload hp{};
      NVE_CHECK(nve::pair_wire::decode_hello_payload(env.payload, hp));
      NVE_CHECK(!nve::pair_wire::ct_eq_16(hp.token_proof, expected_proof));
      const auto rej =
          nve::pair_wire::encode_reject_envelope(nve::PairRejectReason::InvalidPairingToken, 1u);
      NVE_CHECK(use_ws ? write_ws_binary(cfd, rej) : write_tcp_framed(cfd, rej));
      server_rejected_bad_token.store(true);
      test_close(cfd);
    }

    {
      TestSocket cfd = ::accept(listen_fd, nullptr, nullptr);
      NVE_CHECK(test_sock_valid(cfd));
      if (use_ws) NVE_CHECK(handle_server_ws_upgrade(cfd));
      std::vector<uint8_t> msg;
      NVE_CHECK(use_ws ? read_ws_binary(cfd, msg) : read_tcp_framed(cfd, msg));
      nve::TransportEnvelopeView env{};
      NVE_CHECK(nve::pair_wire::parse_envelope(msg, env) &&
                env.kind == nve::TransportMsgKind::Hello);
      nve::HelloPayload hp{};
      NVE_CHECK(nve::pair_wire::decode_hello_payload(env.payload, hp));
      NVE_CHECK(nve::pair_wire::ct_eq_16(hp.token_proof, expected_proof));

      const uint64_t server_nonce = 0xCAFEBABE12345678ULL;
      const uint32_t sid =
          nve::pair_wire::derive_session_id(token, 1u, hp.client_nonce, server_nonce);
      nve::HelloAckPayload ack{};
      ack.schema_version = 1u;
      ack.server_max_window = 64u;
      ack.server_seq = 0u;
      ack.server_nonce = server_nonce;
      ack.resync_required = true;
      const auto ack_bytes = nve::pair_wire::encode_hello_ack_payload(ack);
      const auto ack_env = nve::pair_wire::encode_envelope(
          nve::TransportMsgKind::HelloAck, sid, ack_bytes);
      NVE_CHECK(use_ws ? write_ws_binary(cfd, ack_env) : write_tcp_framed(cfd, ack_env));

      const auto kf1 = nve::pair_wire::encode_nvd_packet(
          nve::DeltaPacketKind::Keyframe, 0u, 1u, 0u, 0u, frame1);
      const auto env1 =
          nve::pair_wire::encode_envelope(nve::TransportMsgKind::DeltaSingle, sid, kf1);
      NVE_CHECK(use_ws ? write_ws_binary(cfd, env1, /*fragmented=*/true)
                       : write_tcp_framed(cfd, env1));

      const auto delta2 = nve::pair_wire::encode_command_op_delta(frame1, frame2);
      NVE_CHECK(delta2.has_value());
      const auto pkt2 = nve::pair_wire::encode_nvd_packet(
          nve::DeltaPacketKind::CommandOpDelta, 0u, 2u, 1u, 0u, *delta2);
      const auto env2 =
          nve::pair_wire::encode_envelope(nve::TransportMsgKind::DeltaSingle, sid, pkt2);
      NVE_CHECK(use_ws ? write_ws_binary(cfd, env2) : write_tcp_framed(cfd, env2));

      while (use_ws ? read_ws_binary(cfd, msg, 3000) : read_tcp_framed(cfd, msg, 3000)) {
        nve::TransportEnvelopeView in{};
        if (!nve::pair_wire::parse_envelope(msg, in) || in.session_id != sid) continue;
        if (in.kind == nve::TransportMsgKind::DeltaSingle) {
          nve::DeltaPacketView dv{};
          if (nve::pair_wire::parse_nvd_packet(in.payload, dv) &&
              dv.header.kind == nve::DeltaPacketKind::Ack) {
            server_saw_ack.store(true);
          }
        } else if (in.kind == nve::TransportMsgKind::InputEvents) {
          std::lock_guard<std::mutex> lock(ev_mutex);
          received_event_ring.assign(in.payload.begin(), in.payload.end());
          server_saw_input_event.store(true);
          break;
        } else if (in.kind == nve::TransportMsgKind::Bye) {
          break;
        }
      }
      test_close(cfd);
    }
    test_close(listen_fd);
  });

  {
    nve::PairClientConfig bad_cfg{};
    bad_cfg.host = "127.0.0.1";
    bad_cfg.port = port;
    bad_cfg.transport = use_ws ? nve::PairTransport::WebSocket : nve::PairTransport::Tcp;
    bad_cfg.ws_path = "/nve-sync";
    bad_cfg.pairing_token = "wrong-token";
    nve::NativeHostSession bad_session;
    std::string err;
    NVE_CHECK(!bad_session.connect_paired(bad_cfg, {}, &err));
    NVE_CHECK(!bad_session.is_paired());
    NVE_CHECK(err.find("token") != std::string::npos);
  }

  nve::PairClientConfig cfg{};
  cfg.host = "127.0.0.1";
  cfg.port = port;
  cfg.transport = use_ws ? nve::PairTransport::WebSocket : nve::PairTransport::Tcp;
  cfg.ws_path = "/nve-sync";
  cfg.pairing_token = token;
  // Long heartbeat interval to verify deadline-driven poll blocks cleanly and wakes immediately
  // on both incoming frames and disconnect() wake_fd!
  cfg.heartbeat_interval_ms = 5000u;

  std::atomic<uint32_t> wake_count{0u};
  nve::NativeHostSession session;
  NveSurfaceConfig scfg{.surface_id = 1u, .width = 400.0f, .height = 300.0f, .scale_factor = 2.0f};
  std::string err;
  NVE_CHECK(session.connect_paired(
      cfg,
      scfg,
      [&]() { wake_count.fetch_add(1u, std::memory_order_relaxed); },
      &err));
  NVE_CHECK(session.is_paired() && session.is_paired_connected());
  NVE_CHECK(session.bridge() == nullptr);

  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (session.paired_frame_count() < 2u && std::chrono::steady_clock::now() < deadline) {
    (void)session.poll_paired_frames();
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(session.paired_frame_count() == 2u);
  NVE_CHECK(wake_count.load() >= 1u);

  const NveSceneView view = session.scene_view();
  NVE_CHECK(view.schema_hash == NVE_SCENE_SCHEMA_HASH);
  NVE_CHECK(view.node_count == 5u);
  bool found_updated_text = false;
  for (uint32_t i = 0u; i < view.node_count; ++i) {
    if (view.nodes[i].node_id == 3u) {
      NVE_CHECK(nve::scene_node_text(view, view.nodes[i]) == "Paired Frame 2 Delta!");
      found_updated_text = true;
    }
  }
  NVE_CHECK(found_updated_text);

  NVE_CHECK(session.dispatch_press(4u, 22.5f, 11.25f) == 1u);

  const auto ev_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (!server_saw_input_event.load() && std::chrono::steady_clock::now() < ev_deadline) {
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(server_rejected_bad_token.load());
  NVE_CHECK(server_saw_ack.load());
  NVE_CHECK(server_saw_input_event.load());

  std::vector<nve::DecodedEvent> decoded;
  {
    std::lock_guard<std::mutex> lock(ev_mutex);
    NVE_CHECK(nve::decode_event_ring(
        NveBufferSlice{received_event_ring.data(), received_event_ring.size()}, decoded));
  }
  NVE_CHECK(decoded.size() == 1u);
  NVE_CHECK(decoded[0].kind == nve::EventKind::Press && decoded[0].node_id == 4u);
  NVE_CHECK(std::fabs(decoded[0].x - 22.5f) < 0.01f && std::fabs(decoded[0].y - 11.25f) < 0.01f);

  const auto disc_start = std::chrono::steady_clock::now();
  session.disconnect_paired();
  const auto disc_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                           std::chrono::steady_clock::now() - disc_start)
                           .count();
  // Even with heartbeat_interval_ms=5000, wake_fd must interrupt poll/WSAPoll promptly!
  NVE_CHECK(disc_ms < 500);
  NVE_CHECK(!session.is_paired());
  server_thread.join();
}

void test_live_loopback_udp_fragmented_roundtrip() {
  const std::string token = "nve-udp-secret";
  const auto expected_proof = nve::pair_wire::derive_token_proof(token, 1u);
  const auto frame1 = build_scene_cmd_buf(1u, 1u, "UDP Keyframe 1", /*extra_nodes=*/true);
  const auto frame2 = build_scene_cmd_buf(1u, 2u, "UDP XorRle Frame 2", /*extra_nodes=*/true);
  NVE_CHECK(frame1.size() > 2500u);

  TestSocket srv_fd = ::socket(AF_INET, SOCK_DGRAM, 0);
  NVE_CHECK(test_sock_valid(srv_fd));
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  addr.sin_port = 0;
  NVE_CHECK(::bind(srv_fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0);
  TestSockLen alen = static_cast<TestSockLen>(sizeof(addr));
  NVE_CHECK(::getsockname(srv_fd, reinterpret_cast<sockaddr*>(&addr), &alen) == 0);
  const uint16_t port = ntohs(addr.sin_port);

  std::atomic<bool> saw_input{false};
  std::vector<uint8_t> rx_ev;
  std::mutex ev_mtx;

  std::thread udp_srv([&]() {
    std::array<uint8_t, 2048> buf{};
    sockaddr_storage peer{};
    TestSockLen peer_len = static_cast<TestSockLen>(sizeof(peer));
    uint32_t sid = 0u;
    bool sent_frames = false;
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(4);
    while (std::chrono::steady_clock::now() < deadline) {
      const int rc = test_poll_read(srv_fd, 20);
      if (rc <= 0) continue;
      peer_len = static_cast<TestSockLen>(sizeof(peer));
      const ptrdiff_t n = test_recvfrom(
          srv_fd, buf.data(), buf.size(), reinterpret_cast<sockaddr*>(&peer), &peer_len);
      if (n <= 0) continue;
      nve::TransportEnvelopeView env{};
      if (!nve::pair_wire::parse_envelope(
              std::span<const uint8_t>(buf.data(), static_cast<size_t>(n)), env)) {
        continue;
      }
      if (env.kind == nve::TransportMsgKind::Hello && !sent_frames) {
        nve::HelloPayload hp{};
        NVE_CHECK(nve::pair_wire::decode_hello_payload(env.payload, hp));
        NVE_CHECK(nve::pair_wire::ct_eq_16(hp.token_proof, expected_proof));
        const uint64_t sn = 0x0102030405060708ULL;
        sid = nve::pair_wire::derive_session_id(token, 1u, hp.client_nonce, sn);
        nve::HelloAckPayload ack{};
        ack.schema_version = 1u;
        ack.server_max_window = 64u;
        ack.server_nonce = sn;
        const auto ack_bytes = nve::pair_wire::encode_hello_ack_payload(ack);
        const auto ack_env =
            nve::pair_wire::encode_envelope(nve::TransportMsgKind::HelloAck, sid, ack_bytes);
        (void)test_sendto(
            srv_fd, ack_env.data(), ack_env.size(), reinterpret_cast<sockaddr*>(&peer), peer_len);

        const auto kf1 = nve::pair_wire::encode_nvd_packet(
            nve::DeltaPacketKind::Keyframe, 0u, 1u, 0u, 0u, frame1);
        auto frags = nve::pair_wire::packetize_udp_datagrams(sid, kf1);
        NVE_CHECK(frags.size() >= 3u);
        std::reverse(frags.begin(), frags.end());
        for (const auto& dg : frags) {
          (void)test_sendto(
              srv_fd, dg.data(), dg.size(), reinterpret_cast<sockaddr*>(&peer), peer_len);
        }

        const auto delta2 = nve::pair_wire::encode_xor_rle_delta(frame1, frame2);
        const auto pkt2 = nve::pair_wire::encode_nvd_packet(
            nve::DeltaPacketKind::XorRleDelta, 0u, 2u, 1u, 0u, delta2);
        for (const auto& dg : nve::pair_wire::packetize_udp_datagrams(sid, pkt2)) {
          (void)test_sendto(
              srv_fd, dg.data(), dg.size(), reinterpret_cast<sockaddr*>(&peer), peer_len);
        }
        sent_frames = true;
      } else if (env.kind == nve::TransportMsgKind::InputEvents && env.session_id == sid) {
        std::lock_guard<std::mutex> lock(ev_mtx);
        rx_ev.assign(env.payload.begin(), env.payload.end());
        saw_input.store(true);
        break;
      }
    }
    test_close(srv_fd);
  });

  nve::PairClientConfig cfg{};
  cfg.host = "127.0.0.1";
  cfg.port = port;
  cfg.transport = nve::PairTransport::Udp;
  cfg.pairing_token = token;

  nve::NativeHostSession session;
  std::string err;
  NVE_CHECK(session.connect_paired(cfg, {}, &err));

  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (session.paired_frame_count() < 2u && std::chrono::steady_clock::now() < deadline) {
    (void)session.poll_paired_frames();
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(session.paired_frame_count() == 2u);
  const NveSceneView view = session.scene_view();
  NVE_CHECK(view.node_count == 29u);
  bool found = false;
  for (uint32_t i = 0u; i < view.node_count; ++i) {
    if (view.nodes[i].node_id == 3u) {
      NVE_CHECK(nve::scene_node_text(view, view.nodes[i]) == "UDP XorRle Frame 2");
      found = true;
    }
  }
  NVE_CHECK(found);

  NveDirectEvent ev{};
  ev.surface_id = 1u;
  ev.node_id = 5u;
  ev.timestamp_us = 424242u;
  ev.kind = static_cast<uint8_t>(nve::EventKind::TextChange);
  ev.payload_tag = NVE_EVENT_PAYLOAD_TEXT;
  ev.text = nve::make_str_slice("udp-typed");
  NVE_CHECK(session.dispatch_direct_events(NveEventBatch{&ev, 1u}) == 1u);

  const auto ev_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(3);
  while (!saw_input.load() && std::chrono::steady_clock::now() < ev_deadline) {
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  NVE_CHECK(saw_input.load());
  std::vector<nve::DecodedEvent> decoded;
  {
    std::lock_guard<std::mutex> lock(ev_mtx);
    NVE_CHECK(nve::decode_event_ring(NveBufferSlice{rx_ev.data(), rx_ev.size()}, decoded));
  }
  NVE_CHECK(decoded.size() == 1u && decoded[0].kind == nve::EventKind::TextChange &&
            decoded[0].text == "udp-typed");

  session.disconnect_paired();
  udp_srv.join();
}

} // namespace

int main() {
  TestWinsockGuard winsock_guard;
  (void)winsock_guard;
  test_rust_parity_vectors_and_endpoint_parser();
  test_nvd1_codecs_and_reorder_receiver();
  test_udp_fragmentation_and_adversarial_packets();
  test_rfc6455_ws_codec();
  test_live_loopback_stream_roundtrip(/*use_ws=*/false);
  test_live_loopback_stream_roundtrip(/*use_ws=*/true);
  test_live_loopback_udp_fragmented_roundtrip();

  std::cout << "nve_pair_client_test passed (NVT1/NVD1 + TCP + WS + UDP Frag + InputEvents)\n";
  return 0;
}
