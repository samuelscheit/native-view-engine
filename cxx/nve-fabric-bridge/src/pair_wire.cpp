#include "pair_internal.hpp"

#include "mock_jsi_recorder.hpp"

#include <algorithm>
#include <array>
#include <atomic>
#include <charconv>
#include <cstring>
#include <string>
#include <utility>
#include <vector>

namespace nve {

namespace pair_detail {

namespace {
std::atomic<uint64_t> g_nonce_counter{1u};
} // namespace

uint64_t next_nonce(uint64_t seed_hint) noexcept {
  const uint64_t seq = g_nonce_counter.fetch_add(1u, std::memory_order_relaxed);
  uint64_t x = seq * 0x9E3779B97F4A7C15ULL + (seed_hint ^ 0xA0761D6478BD642FULL);
  x ^= x >> 30u;
  x *= 0xBF58476D1CE4E5B9ULL;
  x ^= x >> 27u;
  x *= 0x94D049BB133111EBULL;
  return x ^ (x >> 31u);
}

bool verify_hello_ack_envelope(
    const PairClientConfig& config,
    const TransportEnvelopeView& env,
    uint64_t client_nonce,
    uint32_t& out_sid,
    size_t& out_window,
    std::string* out_error) {
  auto fail = [&](std::string msg) {
    if (out_error != nullptr) *out_error = std::move(msg);
    return false;
  };
  if (env.kind == TransportMsgKind::Reject) {
    PairRejectReason reason{};
    uint16_t exp_schema = 0u;
    if (pair_wire::decode_reject_payload(env.payload, reason, exp_schema)) {
      switch (reason) {
        case PairRejectReason::InvalidPairingToken:
          return fail("Pairing rejected: invalid pairing token");
        case PairRejectReason::SchemaVersionMismatch:
          return fail("Pairing rejected: schema version mismatch (expected " +
                      std::to_string(exp_schema) + ")");
        case PairRejectReason::UnsupportedProtocolVersion:
          return fail("Pairing rejected: unsupported protocol version");
        case PairRejectReason::ServerBusy:
          return fail("Pairing rejected: server busy");
      }
    }
    return fail("Pairing rejected by server");
  }
  if (env.kind != TransportMsgKind::HelloAck) {
    return fail("Unexpected message kind during handshake");
  }
  HelloAckPayload ack{};
  if (!pair_wire::decode_hello_ack_payload(env.payload, ack)) {
    return fail("Malformed HelloAck payload");
  }
  if (ack.schema_version != config.schema_version) {
    return fail("HelloAck schema version mismatch");
  }
  const uint32_t expected_sid = pair_wire::derive_session_id(
      config.pairing_token, config.schema_version, client_nonce, ack.server_nonce);
  if (env.session_id != expected_sid) {
    return fail("Negotiated session_id mismatch");
  }
  out_sid = env.session_id;
  out_window = std::max<size_t>(4u, static_cast<size_t>(ack.server_max_window));
  return true;
}

} // namespace pair_detail

namespace pair_wire {

using namespace pair_detail;

uint64_t fnv1a_64(std::span<const uint8_t> bytes) noexcept {
  return fnv1a_64_pair(bytes, {});
}

uint64_t fnv1a_64_pair(std::span<const uint8_t> a, std::span<const uint8_t> b) noexcept {
  constexpr uint64_t kOffsetBasis = 0xcbf29ce484222325ULL;
  constexpr uint64_t kPrime = 0x00000100000001b3ULL;
  uint64_t h = kOffsetBasis;
  for (uint8_t byte : a) {
    h ^= static_cast<uint64_t>(byte);
    h *= kPrime;
  }
  for (uint8_t byte : b) {
    h ^= static_cast<uint64_t>(byte);
    h *= kPrime;
  }
  return h;
}

std::array<uint8_t, 16> derive_token_proof(
    std::string_view pairing_token,
    uint16_t schema_version) noexcept {
  std::array<uint8_t, 16> prefix_a{};
  static constexpr char kPrefixTag[] = "NVE-PAIR-A-v1:";
  std::memcpy(prefix_a.data(), kPrefixTag, 14u);
  write_u16_le(prefix_a.data() + 14u, schema_version);

  const std::span<const uint8_t> token_bytes(
      reinterpret_cast<const uint8_t*>(pairing_token.data()), pairing_token.size());
  const uint64_t h1 = fnv1a_64_pair(prefix_a, token_bytes);

  std::array<uint8_t, 16> suffix_b{};
  static constexpr char kSuffixTag[] = ":NVE-PAIR-B-v1";
  std::memcpy(suffix_b.data(), kSuffixTag, 14u);
  write_u16_le(suffix_b.data() + 14u, schema_version);
  const uint64_t h2 = fnv1a_64_pair(token_bytes, suffix_b);

  std::array<uint8_t, 16> proof{};
  write_u64_le(proof.data(), h1);
  write_u64_le(proof.data() + 8u, h2);
  return proof;
}

uint32_t derive_session_id(
    std::string_view pairing_token,
    uint16_t schema_version,
    uint64_t client_nonce,
    uint64_t server_nonce) noexcept {
  std::array<uint8_t, 18> material{};
  write_u16_le(material.data(), schema_version);
  write_u64_le(material.data() + 2u, client_nonce);
  write_u64_le(material.data() + 10u, server_nonce);
  const std::span<const uint8_t> token_bytes(
      reinterpret_cast<const uint8_t*>(pairing_token.data()), pairing_token.size());
  const uint64_t h = fnv1a_64_pair(token_bytes, material);
  const uint32_t folded = static_cast<uint32_t>(h ^ (h >> 32u)) ^ 0x5A3C96F1u;
  return folded == 0u ? 1u : folded;
}

bool ct_eq_16(const std::array<uint8_t, 16>& a, const std::array<uint8_t, 16>& b) noexcept {
  uint8_t diff = 0u;
  for (size_t i = 0u; i < 16u; ++i) {
    diff |= static_cast<uint8_t>(a[i] ^ b[i]);
  }
  return diff == 0u;
}

std::vector<uint8_t> encode_envelope(
    TransportMsgKind kind,
    uint32_t session_id,
    std::span<const uint8_t> payload) {
  std::vector<uint8_t> out;
  out.reserve(kTransportHeaderSize + payload.size());
  out.push_back(kTransportMagic[0]);
  out.push_back(kTransportMagic[1]);
  out.push_back(kTransportProtocolVersion);
  out.push_back(static_cast<uint8_t>(kind));
  push_u32_le(out, session_id);
  out.insert(out.end(), payload.begin(), payload.end());
  return out;
}

bool parse_envelope(std::span<const uint8_t> bytes, TransportEnvelopeView& out) noexcept {
  if (bytes.size() < kTransportHeaderSize) {
    return false;
  }
  if (bytes[0] != kTransportMagic[0] || bytes[1] != kTransportMagic[1]) {
    return false;
  }
  if (bytes[2] != kTransportProtocolVersion) {
    return false;
  }
  TransportMsgKind kind{};
  if (!valid_transport_msg_kind(bytes[3], kind)) {
    return false;
  }
  out.kind = kind;
  out.session_id = read_u32_le(bytes.data() + 4u);
  out.payload = bytes.subspan(kTransportHeaderSize);
  return true;
}

std::array<uint8_t, 32> encode_hello_payload(const HelloPayload& hello) noexcept {
  std::array<uint8_t, 32> buf{};
  write_u16_le(buf.data(), hello.schema_version);
  write_u16_le(buf.data() + 2u, hello.max_window);
  write_u32_le(buf.data() + 4u, hello.client_last_seq);
  write_u64_le(buf.data() + 8u, hello.client_nonce);
  std::memcpy(buf.data() + 16u, hello.token_proof.data(), 16u);
  return buf;
}

bool decode_hello_payload(std::span<const uint8_t> bytes, HelloPayload& out) noexcept {
  if (bytes.size() != 32u) {
    return false;
  }
  out.schema_version = read_u16_le(bytes.data());
  out.max_window = read_u16_le(bytes.data() + 2u);
  out.client_last_seq = read_u32_le(bytes.data() + 4u);
  out.client_nonce = read_u64_le(bytes.data() + 8u);
  std::memcpy(out.token_proof.data(), bytes.data() + 16u, 16u);
  return true;
}

std::array<uint8_t, 20> encode_hello_ack_payload(const HelloAckPayload& ack) noexcept {
  std::array<uint8_t, 20> buf{};
  write_u16_le(buf.data(), ack.schema_version);
  write_u16_le(buf.data() + 2u, ack.server_max_window);
  write_u32_le(buf.data() + 4u, ack.server_seq);
  write_u64_le(buf.data() + 8u, ack.server_nonce);
  buf[16] = ack.resync_required ? 1u : 0u;
  return buf;
}

bool decode_hello_ack_payload(std::span<const uint8_t> bytes, HelloAckPayload& out) noexcept {
  if (bytes.size() != 20u) {
    return false;
  }
  out.schema_version = read_u16_le(bytes.data());
  out.server_max_window = read_u16_le(bytes.data() + 2u);
  out.server_seq = read_u32_le(bytes.data() + 4u);
  out.server_nonce = read_u64_le(bytes.data() + 8u);
  out.resync_required = (bytes[16] != 0u);
  return true;
}

std::vector<uint8_t> encode_reject_envelope(PairRejectReason reason, uint16_t expected_schema) {
  std::array<uint8_t, 4> payload{};
  payload[0] = static_cast<uint8_t>(reason);
  write_u16_le(payload.data() + 1u, expected_schema);
  return encode_envelope(TransportMsgKind::Reject, 0u, payload);
}

bool decode_reject_payload(
    std::span<const uint8_t> payload,
    PairRejectReason& out_reason,
    uint16_t& out_expected_schema) noexcept {
  if (payload.size() < 3u) {
    return false;
  }
  if (!valid_reject_reason(payload[0], out_reason)) {
    return false;
  }
  out_expected_schema = read_u16_le(payload.data() + 1u);
  return true;
}

std::vector<uint8_t> encode_ping_or_pong(
    TransportMsgKind kind,
    uint32_t session_id,
    uint64_t timestamp_us) {
  std::array<uint8_t, 8> payload{};
  write_u64_le(payload.data(), timestamp_us);
  return encode_envelope(kind, session_id, payload);
}

bool decode_timestamp_us(std::span<const uint8_t> payload, uint64_t& out_ts_us) noexcept {
  if (payload.size() != 8u) {
    return false;
  }
  out_ts_us = read_u64_le(payload.data());
  return true;
}

std::vector<uint8_t> encode_keyframe_request(
    uint32_t session_id,
    uint32_t client_last_seq,
    uint8_t reason) {
  std::array<uint8_t, 8> payload{};
  write_u32_le(payload.data(), client_last_seq);
  payload[4] = reason;
  return encode_envelope(TransportMsgKind::KeyframeRequest, session_id, payload);
}

bool decode_keyframe_request(
    std::span<const uint8_t> payload,
    uint32_t& out_last_seq,
    uint8_t& out_reason) noexcept {
  if (payload.size() != 8u) {
    return false;
  }
  out_last_seq = read_u32_le(payload.data());
  out_reason = payload[4];
  return true;
}

std::vector<uint8_t> encode_frag_nack(
    uint32_t session_id,
    uint32_t frag_msg_id,
    std::span<const uint16_t> missing) {
  const size_t capped = std::min<size_t>(missing.size(), 64u);
  std::vector<uint8_t> payload;
  payload.reserve(6u + capped * 2u);
  push_u32_le(payload, frag_msg_id);
  push_u16_le(payload, static_cast<uint16_t>(capped));
  for (size_t i = 0u; i < capped; ++i) {
    push_u16_le(payload, missing[i]);
  }
  return encode_envelope(TransportMsgKind::FragNack, session_id, payload);
}

bool decode_frag_nack(
    std::span<const uint8_t> payload,
    uint32_t& out_frag_msg_id,
    std::vector<uint16_t>& out_missing) {
  if (payload.size() < 6u) {
    return false;
  }
  out_frag_msg_id = read_u32_le(payload.data());
  const size_t count = static_cast<size_t>(read_u16_le(payload.data() + 4u));
  if (count == 0u || count > 64u || payload.size() != 6u + count * 2u) {
    return false;
  }
  out_missing.clear();
  out_missing.reserve(count);
  for (size_t i = 0u; i < count; ++i) {
    out_missing.push_back(read_u16_le(payload.data() + 6u + i * 2u));
  }
  return true;
}

std::vector<uint8_t> encode_nvd_packet(
    DeltaPacketKind kind,
    uint16_t flags,
    uint32_t seq,
    uint32_t base_seq,
    uint32_t ack_seq,
    std::span<const uint8_t> payload) {
  std::vector<uint8_t> out;
  out.reserve(kDeltaHeaderSize + payload.size());
  out.insert(out.end(), kSyncDeltaMagic.begin(), kSyncDeltaMagic.end());
  out.push_back(kSyncDeltaVersion);
  out.push_back(static_cast<uint8_t>(kind));
  push_u16_le(out, flags);
  push_u32_le(out, seq);
  push_u32_le(out, base_seq);
  push_u32_le(out, ack_seq);
  push_u32_le(out, static_cast<uint32_t>(payload.size()));
  const uint64_t checksum = fnv1a_64_pair(std::span<const uint8_t>(out.data(), 24u), payload);
  push_u64_le(out, checksum);
  out.insert(out.end(), payload.begin(), payload.end());
  return out;
}

bool parse_nvd_packet(std::span<const uint8_t> bytes, DeltaPacketView& out) noexcept {
  if (bytes.size() < kDeltaHeaderSize) {
    return false;
  }
  if (std::memcmp(bytes.data(), kSyncDeltaMagic.data(), 4u) != 0) {
    return false;
  }
  const uint8_t version = bytes[4];
  if (version != kSyncDeltaVersion) {
    return false;
  }
  DeltaPacketKind kind{};
  if (!valid_delta_packet_kind(bytes[5], kind)) {
    return false;
  }
  const uint16_t flags = read_u16_le(bytes.data() + 6u);
  const uint32_t seq = read_u32_le(bytes.data() + 8u);
  const uint32_t base_seq = read_u32_le(bytes.data() + 12u);
  const uint32_t ack_seq = read_u32_le(bytes.data() + 16u);
  const uint32_t payload_len = read_u32_le(bytes.data() + 20u);
  const uint64_t checksum = read_u64_le(bytes.data() + 24u);

  const size_t actual = bytes.size() - kDeltaHeaderSize;
  if (actual != static_cast<size_t>(payload_len)) {
    return false;
  }
  const std::span<const uint8_t> payload = bytes.subspan(kDeltaHeaderSize);
  const uint64_t computed = fnv1a_64_pair(bytes.subspan(0u, 24u), payload);
  if (computed != checksum) {
    return false;
  }
  out.header = DeltaPacketHeader{
      .version = version,
      .kind = kind,
      .flags = flags,
      .seq = seq,
      .base_seq = base_seq,
      .ack_seq = ack_seq,
      .payload_len = payload_len,
      .checksum = checksum,
  };
  out.payload = payload;
  return true;
}

} // namespace pair_wire

bool parse_pair_endpoint(
    std::string_view endpoint,
    std::string_view transport_override,
    PairClientConfig& out,
    std::string* out_error) {
  auto fail = [&](std::string msg) {
    if (out_error != nullptr) {
      *out_error = std::move(msg);
    }
    return false;
  };

  std::string_view raw = pair_detail::trim_ascii(endpoint);
  if (raw.empty()) {
    return fail("Empty pairing endpoint");
  }

  bool scheme_explicit = false;
  PairTransport parsed_transport = out.transport;
  std::string ws_path = "/";

  if (const size_t scheme_pos = raw.find("://"); scheme_pos != std::string_view::npos) {
    const std::string_view scheme = raw.substr(0u, scheme_pos);
    raw.remove_prefix(scheme_pos + 3u);
    scheme_explicit = true;
    if (pair_detail::ascii_iequals(scheme, "udp")) {
      parsed_transport = PairTransport::Udp;
    } else if (pair_detail::ascii_iequals(scheme, "tcp")) {
      parsed_transport = PairTransport::Tcp;
    } else if (pair_detail::ascii_iequals(scheme, "ws") ||
               pair_detail::ascii_iequals(scheme, "http")) {
      parsed_transport = PairTransport::WebSocket;
    } else {
      return fail("Unsupported pairing URL scheme: " + std::string(scheme));
    }
  }

  if (!transport_override.empty()) {
    const std::string_view tr = pair_detail::trim_ascii(transport_override);
    if (pair_detail::ascii_iequals(tr, "auto")) {
      if (!scheme_explicit) {
        parsed_transport = PairTransport::Tcp;
      }
    } else if (pair_detail::ascii_iequals(tr, "udp")) {
      parsed_transport = PairTransport::Udp;
    } else if (pair_detail::ascii_iequals(tr, "tcp")) {
      parsed_transport = PairTransport::Tcp;
    } else if (pair_detail::ascii_iequals(tr, "ws") ||
               pair_detail::ascii_iequals(tr, "websocket")) {
      parsed_transport = PairTransport::WebSocket;
    } else {
      return fail("Unknown --pair-transport value: " + std::string(tr));
    }
  } else if (!scheme_explicit) {
    parsed_transport = PairTransport::Tcp;
  }

  if (const size_t slash_pos = raw.find('/'); slash_pos != std::string_view::npos) {
    ws_path = std::string(raw.substr(slash_pos));
    raw = raw.substr(0u, slash_pos);
  }

  std::string host;
  std::string_view port_sv;
  if (!raw.empty() && raw.front() == '[') {
    const size_t close = raw.find(']');
    if (close == std::string_view::npos) {
      return fail("Invalid IPv6 bracketed host in endpoint");
    }
    host = std::string(raw.substr(1u, close - 1u));
    const std::string_view rest = raw.substr(close + 1u);
    if (rest.empty() || rest.front() != ':') {
      return fail("Missing port after IPv6 host");
    }
    port_sv = rest.substr(1u);
  } else {
    const size_t colon = raw.rfind(':');
    if (colon == std::string_view::npos) {
      return fail("Missing :port in pairing endpoint");
    }
    host = std::string(raw.substr(0u, colon));
    port_sv = raw.substr(colon + 1u);
  }

  if (host.empty() || port_sv.empty()) {
    return fail("Invalid host:port in pairing endpoint");
  }

  uint32_t port_num = 0u;
  const auto [ptr, ec] =
      std::from_chars(port_sv.data(), port_sv.data() + port_sv.size(), port_num);
  if (ec != std::errc{} || ptr != port_sv.data() + port_sv.size() || port_num == 0u ||
      port_num > 65535u) {
    return fail("Invalid port number in pairing endpoint: " + std::string(port_sv));
  }

  out.host = std::move(host);
  out.port = static_cast<uint16_t>(port_num);
  out.ws_path = ws_path.empty() ? std::string("/") : std::move(ws_path);
  out.transport = parsed_transport;
  if (out_error != nullptr) {
    out_error->clear();
  }
  return true;
}

bool encode_direct_event_batch(NveEventBatch batch, std::vector<uint8_t>& out_ring) {
  out_ring.clear();
  if (batch.ptr == nullptr || batch.len == 0u) {
    return false;
  }
  EventRingWriter writer;
  writer.clear();
  for (uint32_t i = 0u; i < batch.len; ++i) {
    if (!writer.push_direct_event(batch.ptr[i])) {
      return false;
    }
  }
  if (writer.count() == 0u) {
    return false;
  }
  out_ring = writer.finalize();
  return true;
}

} // namespace nve
