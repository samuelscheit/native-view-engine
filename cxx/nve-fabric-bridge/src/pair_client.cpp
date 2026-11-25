#include "nve_pair_client.hpp"

#include "mock_jsi_recorder.hpp"

#include <algorithm>
#include <array>
#include <atomic>
#include <cctype>
#include <charconv>
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

#if !defined(_WIN32)
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

namespace nve {

namespace {

constexpr size_t kNveCmdHeaderSize = 24u;
constexpr uint8_t kCmdDeltaCopyRun = 1u;
constexpr uint8_t kCmdDeltaLiteralOp = 2u;
constexpr uint8_t kCmdDeltaXorOp = 3u;
constexpr size_t kMaxInflightFragMessages = 16u;
constexpr size_t kMaxWsUpgradeHeaderBytes = 8u * 1024u;
constexpr size_t kMaxPendingFrames = 256u;
constexpr size_t kMinEncodedEventBytes = 18u;
constexpr std::string_view kWsGuid = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

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
      byte |= 0x80u;
      out.push_back(byte);
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
    if (pos >= bytes.size()) {
      return false;
    }
    const uint8_t b = bytes[pos++];
    const uint64_t low = static_cast<uint64_t>(b & 0x7Fu);
    if (shift == 63u && low > 1u) {
      return false;
    }
    result |= (low << shift);
    if ((b & 0x80u) == 0u) {
      out = result;
      return true;
    }
    shift += 7u;
  }
  return false;
}

std::atomic<uint64_t> g_nonce_counter{1u};

uint64_t next_nonce(uint64_t seed_hint) noexcept {
  const uint64_t seq = g_nonce_counter.fetch_add(1u, std::memory_order_relaxed);
  uint64_t x = seq * 0x9E3779B97F4A7C15ULL + (seed_hint ^ 0xA0761D6478BD642FULL);
  x ^= x >> 30u;
  x *= 0xBF58476D1CE4E5B9ULL;
  x ^= x >> 27u;
  x *= 0x94D049BB133111EBULL;
  return x ^ (x >> 31u);
}

bool valid_transport_msg_kind(uint8_t v, TransportMsgKind& out) noexcept {
  if (v >= 1u && v <= 11u) {
    out = static_cast<TransportMsgKind>(v);
    return true;
  }
  return false;
}

bool valid_reject_reason(uint8_t v, PairRejectReason& out) noexcept {
  if (v >= 1u && v <= 4u) {
    out = static_cast<PairRejectReason>(v);
    return true;
  }
  return false;
}

bool valid_delta_packet_kind(uint8_t v, DeltaPacketKind& out) noexcept {
  if (v >= 1u && v <= 5u) {
    out = static_cast<DeltaPacketKind>(v);
    return true;
  }
  return false;
}

bool valid_ws_opcode(uint8_t v, WsOpcode& out) noexcept {
  switch (v) {
    case 0x0u:
    case 0x1u:
    case 0x2u:
    case 0x8u:
    case 0x9u:
    case 0xAu:
      out = static_cast<WsOpcode>(v);
      return true;
    default:
      return false;
  }
}

struct RawCmdHeader {
  uint16_t flags{0};
  uint32_t surface_id{0};
  uint64_t frame_id{0};
  uint32_t op_count{0};
};

bool scan_nve_command_slices(
    std::span<const uint8_t> bytes,
    RawCmdHeader& out_hdr,
    std::vector<std::pair<size_t, size_t>>& out_ranges) {
  if (bytes.size() < kNveCmdHeaderSize) {
    return false;
  }
  if (read_u32_le(bytes.data()) != NVE_ABI_MAGIC) {
    return false;
  }
  if (read_u16_le(bytes.data() + 4) != NVE_ABI_VERSION) {
    return false;
  }
  out_hdr.flags = read_u16_le(bytes.data() + 6);
  out_hdr.surface_id = read_u32_le(bytes.data() + 8);
  out_hdr.frame_id = read_u64_le(bytes.data() + 12);
  out_hdr.op_count = read_u32_le(bytes.data() + 20);

  size_t pos = kNveCmdHeaderSize;
  out_ranges.clear();
  out_ranges.reserve(std::min<size_t>(static_cast<size_t>(out_hdr.op_count), 4096u));

  auto take = [&](size_t n, const uint8_t*& ptr) -> bool {
    if (n > bytes.size() - pos) {
      return false;
    }
    ptr = bytes.data() + pos;
    pos += n;
    return true;
  };
  auto take_u16 = [&](uint16_t& v) -> bool {
    const uint8_t* p = nullptr;
    if (!take(2u, p)) return false;
    v = read_u16_le(p);
    return true;
  };
  auto take_u32 = [&](uint32_t& v) -> bool {
    const uint8_t* p = nullptr;
    if (!take(4u, p)) return false;
    v = read_u32_le(p);
    return true;
  };

  for (uint32_t idx = 0u; idx < out_hdr.op_count; ++idx) {
    const size_t start = pos;
    const uint8_t* op_ptr = nullptr;
    if (!take(1u, op_ptr)) {
      return false;
    }
    const uint8_t op = *op_ptr;
    const uint8_t* sink = nullptr;
    switch (op) {
      case 1u: {
        // CreateNode: u32 node_id + u16 kind + u16 name_len + name_bytes
        uint16_t name_len = 0u;
        if (!take(6u, sink) || !take_u16(name_len) || !take(name_len, sink)) return false;
        break;
      }
      case 2u: {
        // RemoveNode: u32 node_id
        if (!take(4u, sink)) return false;
        break;
      }
      case 3u: {
        // SetChildren: u32 parent_id + u32 count + count*u32
        uint32_t count = 0u;
        if (!take(4u, sink) || !take_u32(count)) return false;
        if (static_cast<size_t>(count) > (bytes.size() - pos) / 4u) return false;
        if (!take(static_cast<size_t>(count) * 4u, sink)) return false;
        break;
      }
      case 4u: {
        // SetText: u32 node_id + u32 len + text_bytes
        uint32_t len = 0u;
        if (!take(4u, sink) || !take_u32(len) || !take(static_cast<size_t>(len), sink)) {
          return false;
        }
        break;
      }
      case 5u: {
        // SetLayout: 81 bytes
        if (!take(81u, sink)) return false;
        break;
      }
      case 6u: {
        // SetStyle: 36 bytes
        if (!take(36u, sink)) return false;
        break;
      }
      case 7u: {
        // SetRawProp: u32 node_id + u16 key_len + key + u8 val_tag + val
        uint16_t key_len = 0u;
        const uint8_t* vtag_ptr = nullptr;
        if (!take(4u, sink) || !take_u16(key_len) || !take(key_len, sink) || !take(1u, vtag_ptr)) {
          return false;
        }
        switch (*vtag_ptr) {
          case 0u:
            break;
          case 1u:
            if (!take(1u, sink)) return false;
            break;
          case 2u:
          case 3u:
            if (!take(8u, sink)) return false;
            break;
          case 4u:
          case 5u: {
            uint32_t vlen = 0u;
            if (!take_u32(vlen) || !take(static_cast<size_t>(vlen), sink)) return false;
            break;
          }
          default:
            return false;
        }
        break;
      }
      case 8u: {
        // SetEventMask: u32 node_id + u64 mask
        if (!take(12u, sink)) return false;
        break;
      }
      case 9u: {
        // DispatchCommand: u32 node_id + u16 name_len + name + u32 payload_len + payload
        uint16_t nlen = 0u;
        uint32_t plen = 0u;
        if (!take(4u, sink) || !take_u16(nlen) || !take(nlen, sink) || !take_u32(plen) ||
            !take(static_cast<size_t>(plen), sink)) {
          return false;
        }
        break;
      }
      case 10u: {
        // Commit: u8 sync_mount
        if (!take(1u, sink)) return false;
        break;
      }
      default:
        return false;
    }
    out_ranges.emplace_back(start, pos);
  }
  return pos == bytes.size();
}

void encode_xor_rle_delta_into(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next,
    std::vector<uint8_t>& out) {
  out.clear();
  write_varint(static_cast<uint64_t>(next.size()), out);

  const size_t len = next.size();
  size_t i = 0u;
  auto base_at = [&](size_t idx) noexcept -> uint8_t {
    return idx < base.size() ? base[idx] : 0u;
  };

  while (i < len) {
    const size_t zero_start = i;
    while (i < len && static_cast<uint8_t>(base_at(i) ^ next[i]) == 0u) {
      ++i;
    }
    const size_t zero_run = i - zero_start;
    if (zero_run > 0u) {
      if (zero_run <= 127u) {
        out.push_back(static_cast<uint8_t>(zero_run));
      } else {
        out.push_back(0x00u);
        write_varint(static_cast<uint64_t>(zero_run), out);
      }
    }
    if (i >= len) {
      break;
    }

    const size_t lit_start = i;
    while (i < len) {
      if (static_cast<uint8_t>(base_at(i) ^ next[i]) == 0u) {
        const bool next_also_zero =
            (i + 1u < len) && (static_cast<uint8_t>(base_at(i + 1u) ^ next[i + 1u]) == 0u);
        if (next_also_zero || i + 1u == len) {
          break;
        }
      }
      ++i;
    }
    const size_t lit_len = i - lit_start;
    if (lit_len <= 127u) {
      out.push_back(static_cast<uint8_t>(0x80u | lit_len));
    } else {
      out.push_back(0x80u);
      write_varint(static_cast<uint64_t>(lit_len), out);
    }
    for (size_t idx = lit_start; idx < i; ++idx) {
      out.push_back(static_cast<uint8_t>(base_at(idx) ^ next[idx]));
    }
  }
}

bool raw_op_identity(std::span<const uint8_t> op_slice, uint64_t& out_key) noexcept {
  if (op_slice.empty()) {
    return false;
  }
  const uint8_t opcode = op_slice[0];
  if (opcode == 10u) {
    out_key = (static_cast<uint64_t>(opcode) << 32u);
    return true;
  }
  if (op_slice.size() < 5u) {
    return false;
  }
  const uint32_t node_id = read_u32_le(op_slice.data() + 1);
  out_key = (static_cast<uint64_t>(opcode) << 32u) | static_cast<uint64_t>(node_id);
  return true;
}

bool ascii_iequals(std::string_view a, std::string_view b) noexcept {
  if (a.size() != b.size()) return false;
  for (size_t i = 0u; i < a.size(); ++i) {
    const unsigned char ca = static_cast<unsigned char>(a[i]);
    const unsigned char cb = static_cast<unsigned char>(b[i]);
    if (std::tolower(ca) != std::tolower(cb)) return false;
  }
  return true;
}

std::string_view trim_ascii(std::string_view sv) noexcept {
  while (!sv.empty() && (sv.front() == ' ' || sv.front() == '\t' || sv.front() == '\r' ||
                         sv.front() == '\n')) {
    sv.remove_prefix(1u);
  }
  while (!sv.empty() && (sv.back() == ' ' || sv.back() == '\t' || sv.back() == '\r' ||
                         sv.back() == '\n')) {
    sv.remove_suffix(1u);
  }
  return sv;
}

bool validate_input_events_bytes(std::span<const uint8_t> payload, size_t max_bytes) noexcept {
  if (payload.size() > max_bytes || payload.size() < 4u) {
    return false;
  }
  const uint32_t count = read_u32_le(payload.data());
  if (count == 0u) {
    return payload.size() == 4u;
  }
  const size_t max_possible = (payload.size() - 4u) / kMinEncodedEventBytes;
  return static_cast<size_t>(count) <= max_possible;
}

} // namespace

// =============================================================================
// `nve::pair_wire` Implementation
// =============================================================================

namespace pair_wire {

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

std::vector<uint8_t> encode_xor_rle_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next) {
  std::vector<uint8_t> out;
  out.reserve(64u);
  encode_xor_rle_delta_into(base, next, out);
  return out;
}

bool decode_xor_rle_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> delta_payload,
    std::vector<uint8_t>& out) {
  size_t pos = 0u;
  uint64_t target_len_u64 = 0u;
  if (!read_varint(delta_payload, pos, target_len_u64)) {
    return false;
  }
  if (target_len_u64 > static_cast<uint64_t>(kMaxDecodedFrameBytes)) {
    return false;
  }
  const size_t target_len = static_cast<size_t>(target_len_u64);
  out.clear();
  out.reserve(target_len);

  while (out.size() < target_len) {
    if (pos >= delta_payload.size()) {
      return false;
    }
    const uint8_t tag = delta_payload[pos++];
    if ((tag & 0x80u) == 0u) {
      uint64_t run_len_u64 = 0u;
      if (tag == 0u) {
        if (!read_varint(delta_payload, pos, run_len_u64)) {
          return false;
        }
      } else {
        run_len_u64 = static_cast<uint64_t>(tag);
      }
      if (run_len_u64 == 0u || run_len_u64 > static_cast<uint64_t>(target_len - out.size())) {
        return false;
      }
      const size_t run_len = static_cast<size_t>(run_len_u64);
      const size_t start = out.size();
      const size_t end = start + run_len;
      const size_t from_base_end = std::min(end, base.size());
      if (start < from_base_end) {
        out.insert(out.end(), base.begin() + static_cast<ptrdiff_t>(start),
                   base.begin() + static_cast<ptrdiff_t>(from_base_end));
      }
      const size_t pad_start = std::max(from_base_end, start);
      if (end > pad_start) {
        out.resize(out.size() + (end - pad_start), 0u);
      }
    } else {
      uint64_t lit_len_u64 = 0u;
      if (tag == 0x80u) {
        if (!read_varint(delta_payload, pos, lit_len_u64)) {
          return false;
        }
      } else {
        lit_len_u64 = static_cast<uint64_t>(tag & 0x7Fu);
      }
      if (lit_len_u64 == 0u || lit_len_u64 > static_cast<uint64_t>(target_len - out.size())) {
        return false;
      }
      const size_t lit_len = static_cast<size_t>(lit_len_u64);
      if (delta_payload.size() - pos < lit_len) {
        return false;
      }
      const size_t base_offset = out.size();
      const uint8_t* xor_bytes = delta_payload.data() + pos;
      pos += lit_len;
      for (size_t k = 0u; k < lit_len; ++k) {
        const uint8_t b = (base_offset + k < base.size()) ? base[base_offset + k] : 0u;
        out.push_back(static_cast<uint8_t>(b ^ xor_bytes[k]));
      }
    }
  }
  return pos == delta_payload.size();
}

std::optional<std::vector<uint8_t>> encode_command_op_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next) {
  RawCmdHeader prev_hdr{};
  std::vector<std::pair<size_t, size_t>> prev_ranges;
  if (!scan_nve_command_slices(base, prev_hdr, prev_ranges)) {
    return std::nullopt;
  }
  RawCmdHeader next_hdr{};
  std::vector<std::pair<size_t, size_t>> next_ranges;
  if (!scan_nve_command_slices(next, next_hdr, next_ranges)) {
    return std::nullopt;
  }

  std::unordered_map<uint64_t, std::vector<size_t>> prev_exact;
  prev_exact.reserve(prev_ranges.size());
  std::unordered_map<uint64_t, size_t> prev_by_node;
  prev_by_node.reserve(prev_ranges.size());

  for (size_t idx = 0u; idx < prev_ranges.size(); ++idx) {
    const auto [s, e] = prev_ranges[idx];
    const auto slice = base.subspan(s, e - s);
    const uint64_t h = fnv1a_64(slice);
    auto& vec = prev_exact[h];
    if (vec.size() < 4u) {
      vec.push_back(idx);
    }
    uint64_t key = 0u;
    if (raw_op_identity(slice, key)) {
      prev_by_node[key] = idx;
    }
  }

  std::vector<uint8_t> out;
  out.reserve(64u);
  push_u16_le(out, next_hdr.flags);
  write_varint(static_cast<uint64_t>(next_hdr.surface_id), out);
  write_varint(next_hdr.frame_id, out);
  write_varint(static_cast<uint64_t>(next_hdr.op_count), out);

  std::vector<uint8_t> scratch_xor;
  std::vector<uint8_t> scratch_token;

  auto slices_equal = [](std::span<const uint8_t> a, std::span<const uint8_t> b) noexcept -> bool {
    return a.size() == b.size() && (a.empty() || std::memcmp(a.data(), b.data(), a.size()) == 0);
  };

  size_t ni = 0u;
  size_t expected_prev = 0u;
  const size_t n_next = next_ranges.size();
  const size_t n_prev = prev_ranges.size();

  while (ni < n_next) {
    const auto [ns, ne] = next_ranges[ni];
    const auto nslice = next.subspan(ns, ne - ns);

    std::optional<size_t> matched_prev;
    if (expected_prev < n_prev) {
      const auto [ps, pe] = prev_ranges[expected_prev];
      if (slices_equal(base.subspan(ps, pe - ps), nslice)) {
        matched_prev = expected_prev;
      }
    }
    if (!matched_prev.has_value()) {
      const auto it = prev_exact.find(fnv1a_64(nslice));
      if (it != prev_exact.end()) {
        for (size_t pidx : it->second) {
          const auto [ps, pe] = prev_ranges[pidx];
          if (slices_equal(base.subspan(ps, pe - ps), nslice)) {
            matched_prev = pidx;
            break;
          }
        }
      }
    }

    if (matched_prev.has_value()) {
      const size_t pstart = *matched_prev;
      size_t run = 1u;
      while (ni + run < n_next && pstart + run < n_prev) {
        const auto [ns2, ne2] = next_ranges[ni + run];
        const auto [ps2, pe2] = prev_ranges[pstart + run];
        if (!slices_equal(next.subspan(ns2, ne2 - ns2), base.subspan(ps2, pe2 - ps2))) {
          break;
        }
        ++run;
      }
      out.push_back(kCmdDeltaCopyRun);
      write_varint(static_cast<uint64_t>(pstart), out);
      write_varint(static_cast<uint64_t>(run), out);
      ni += run;
      expected_prev = pstart + run;
    } else {
      bool emitted_xor = false;
      uint64_t key = 0u;
      if (raw_op_identity(nslice, key)) {
        const auto it = prev_by_node.find(key);
        if (it != prev_by_node.end()) {
          const size_t pidx = it->second;
          const auto [ps, pe] = prev_ranges[pidx];
          encode_xor_rle_delta_into(base.subspan(ps, pe - ps), nslice, scratch_xor);
          scratch_token.clear();
          scratch_token.push_back(kCmdDeltaXorOp);
          write_varint(static_cast<uint64_t>(pidx), scratch_token);
          write_varint(static_cast<uint64_t>(scratch_xor.size()), scratch_token);
          scratch_token.insert(scratch_token.end(), scratch_xor.begin(), scratch_xor.end());

          const size_t lit_overhead = 1u + (nslice.size() < 128u ? 1u : 2u) + nslice.size();
          if (scratch_token.size() < lit_overhead) {
            out.insert(out.end(), scratch_token.begin(), scratch_token.end());
            emitted_xor = true;
            expected_prev = pidx + 1u;
          }
        }
      }
      if (!emitted_xor) {
        out.push_back(kCmdDeltaLiteralOp);
        write_varint(static_cast<uint64_t>(nslice.size()), out);
        out.insert(out.end(), nslice.begin(), nslice.end());
        if (expected_prev < std::numeric_limits<size_t>::max()) {
          ++expected_prev;
        }
      }
      ++ni;
    }
  }
  return out;
}

bool decode_command_op_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> delta_payload,
    std::vector<uint8_t>& out) {
  RawCmdHeader prev_hdr{};
  std::vector<std::pair<size_t, size_t>> prev_ranges;
  if (!scan_nve_command_slices(base, prev_hdr, prev_ranges)) {
    return false;
  }

  size_t pos = 0u;
  if (delta_payload.size() < 2u) {
    return false;
  }
  const uint16_t flags = read_u16_le(delta_payload.data());
  pos += 2u;

  uint64_t surface_id_u64 = 0u;
  uint64_t frame_id = 0u;
  uint64_t op_count_u64 = 0u;
  if (!read_varint(delta_payload, pos, surface_id_u64) ||
      !read_varint(delta_payload, pos, frame_id) ||
      !read_varint(delta_payload, pos, op_count_u64)) {
    return false;
  }
  if (surface_id_u64 > std::numeric_limits<uint32_t>::max() ||
      op_count_u64 > std::numeric_limits<uint32_t>::max()) {
    return false;
  }
  const uint32_t surface_id = static_cast<uint32_t>(surface_id_u64);
  const uint32_t op_count = static_cast<uint32_t>(op_count_u64);

  out.clear();
  out.reserve(std::max<size_t>(std::min<size_t>(base.size(), 64u * 1024u), kNveCmdHeaderSize));
  push_u32_le(out, NVE_ABI_MAGIC);
  push_u16_le(out, NVE_ABI_VERSION);
  push_u16_le(out, flags);
  push_u32_le(out, surface_id);
  push_u64_le(out, frame_id);
  push_u32_le(out, op_count);

  std::vector<uint8_t> xor_scratch;
  uint32_t emitted_ops = 0u;

  while (pos < delta_payload.size()) {
    const uint8_t tag = delta_payload[pos++];
    switch (tag) {
      case kCmdDeltaCopyRun: {
        uint64_t start_u64 = 0u;
        uint64_t count_u64 = 0u;
        if (!read_varint(delta_payload, pos, start_u64) ||
            !read_varint(delta_payload, pos, count_u64)) {
          return false;
        }
        if (count_u64 == 0u || start_u64 > prev_ranges.size() ||
            count_u64 > prev_ranges.size() - start_u64) {
          return false;
        }
        const size_t start_idx = static_cast<size_t>(start_u64);
        const size_t count = static_cast<size_t>(count_u64);
        for (size_t i = 0u; i < count; ++i) {
          const auto [ps, pe] = prev_ranges[start_idx + i];
          const size_t slice_len = pe - ps;
          if (slice_len > kMaxDecodedFrameBytes - out.size()) {
            return false;
          }
          out.insert(out.end(), base.begin() + static_cast<ptrdiff_t>(ps),
                     base.begin() + static_cast<ptrdiff_t>(pe));
          if (emitted_ops < std::numeric_limits<uint32_t>::max()) {
            ++emitted_ops;
          }
        }
        break;
      }
      case kCmdDeltaLiteralOp: {
        uint64_t len_u64 = 0u;
        if (!read_varint(delta_payload, pos, len_u64)) {
          return false;
        }
        if (len_u64 == 0u || len_u64 > static_cast<uint64_t>(delta_payload.size() - pos)) {
          return false;
        }
        const size_t len = static_cast<size_t>(len_u64);
        if (len > kMaxDecodedFrameBytes - out.size()) {
          return false;
        }
        out.insert(out.end(), delta_payload.begin() + static_cast<ptrdiff_t>(pos),
                   delta_payload.begin() + static_cast<ptrdiff_t>(pos + len));
        pos += len;
        if (emitted_ops < std::numeric_limits<uint32_t>::max()) {
          ++emitted_ops;
        }
        break;
      }
      case kCmdDeltaXorOp: {
        uint64_t prev_idx_u64 = 0u;
        uint64_t xor_len_u64 = 0u;
        if (!read_varint(delta_payload, pos, prev_idx_u64) ||
            prev_idx_u64 >= prev_ranges.size() ||
            !read_varint(delta_payload, pos, xor_len_u64)) {
          return false;
        }
        if (xor_len_u64 == 0u || xor_len_u64 > static_cast<uint64_t>(delta_payload.size() - pos)) {
          return false;
        }
        const auto [ps, pe] = prev_ranges[static_cast<size_t>(prev_idx_u64)];
        const size_t xor_len = static_cast<size_t>(xor_len_u64);
        if (!decode_xor_rle_delta(
                base.subspan(ps, pe - ps), delta_payload.subspan(pos, xor_len), xor_scratch)) {
          return false;
        }
        pos += xor_len;
        if (xor_scratch.size() > kMaxDecodedFrameBytes - out.size()) {
          return false;
        }
        out.insert(out.end(), xor_scratch.begin(), xor_scratch.end());
        if (emitted_ops < std::numeric_limits<uint32_t>::max()) {
          ++emitted_ops;
        }
        break;
      }
      default:
        return false;
    }
  }
  return emitted_ops == op_count;
}

std::vector<std::vector<uint8_t>> packetize_udp_datagrams(
    uint32_t session_id,
    std::span<const uint8_t> nvd_packet) {
  if (nvd_packet.size() <= kMaxUdpSinglePayload) {
    return {encode_envelope(TransportMsgKind::DeltaSingle, session_id, nvd_packet)};
  }

  uint32_t frag_msg_id = 0u;
  if (nvd_packet.size() >= kDeltaHeaderSize) {
    const uint32_t seq = read_u32_le(nvd_packet.data() + 8u);
    frag_msg_id = (seq != 0u) ? seq : static_cast<uint32_t>(fnv1a_64(nvd_packet));
  } else {
    frag_msg_id = static_cast<uint32_t>(fnv1a_64(nvd_packet));
  }

  const uint32_t total_len = static_cast<uint32_t>(nvd_packet.size());
  const size_t frag_count = (nvd_packet.size() + kMaxUdpFragChunk - 1u) / kMaxUdpFragChunk;
  const uint16_t frag_count_u16 = static_cast<uint16_t>(frag_count);

  std::vector<std::vector<uint8_t>> out;
  out.reserve(frag_count);
  for (size_t frag_idx = 0u; frag_idx < frag_count; ++frag_idx) {
    const size_t start = frag_idx * kMaxUdpFragChunk;
    const size_t end = std::min(start + kMaxUdpFragChunk, nvd_packet.size());
    std::vector<uint8_t> dgram;
    dgram.reserve(kTransportHeaderSize + kFragSubheaderSize + (end - start));
    dgram.push_back(kTransportMagic[0]);
    dgram.push_back(kTransportMagic[1]);
    dgram.push_back(kTransportProtocolVersion);
    dgram.push_back(static_cast<uint8_t>(TransportMsgKind::DeltaFrag));
    push_u32_le(dgram, session_id);
    push_u32_le(dgram, frag_msg_id);
    push_u16_le(dgram, static_cast<uint16_t>(frag_idx));
    push_u16_le(dgram, frag_count_u16);
    push_u32_le(dgram, total_len);
    dgram.insert(dgram.end(), nvd_packet.begin() + static_cast<ptrdiff_t>(start),
                 nvd_packet.begin() + static_cast<ptrdiff_t>(end));
    out.push_back(std::move(dgram));
  }
  return out;
}

// Inline zero-dependency SHA-1 (RFC 3174 / RFC 6455) matching `ws_codec.rs`.
std::array<uint8_t, 20> sha1_digest(std::span<const uint8_t> data) noexcept {
  uint32_t h[5] = {
      0x67452301u,
      0xEFCDAB89u,
      0x98BADCFEu,
      0x10325476u,
      0xC3D2E1F0u,
  };

  const uint64_t bit_len = static_cast<uint64_t>(data.size()) * 8ULL;
  std::vector<uint8_t> padded;
  padded.reserve(data.size() + 72u);
  padded.insert(padded.end(), data.begin(), data.end());
  padded.push_back(0x80u);
  while ((padded.size() % 64u) != 56u) {
    padded.push_back(0u);
  }
  push_u64_be(padded, bit_len);

  uint32_t w[80];
  for (size_t offset = 0u; offset < padded.size(); offset += 64u) {
    for (size_t i = 0u; i < 16u; ++i) {
      w[i] = read_u32_be(padded.data() + offset + i * 4u);
    }
    for (size_t i = 16u; i < 80u; ++i) {
      w[i] = rotl32(w[i - 3u] ^ w[i - 8u] ^ w[i - 14u] ^ w[i - 16u], 1u);
    }
    uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4];
    for (size_t i = 0u; i < 80u; ++i) {
      uint32_t f = 0u;
      uint32_t k = 0u;
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
      const uint32_t temp = rotl32(a, 5u) + f + e + k + w[i];
      e = d;
      d = c;
      c = rotl32(b, 30u);
      b = a;
      a = temp;
    }
    h[0] += a;
    h[1] += b;
    h[2] += c;
    h[3] += d;
    h[4] += e;
  }

  std::array<uint8_t, 20> out{};
  for (size_t i = 0u; i < 5u; ++i) {
    out[i * 4u] = static_cast<uint8_t>((h[i] >> 24u) & 0xFFu);
    out[i * 4u + 1u] = static_cast<uint8_t>((h[i] >> 16u) & 0xFFu);
    out[i * 4u + 2u] = static_cast<uint8_t>((h[i] >> 8u) & 0xFFu);
    out[i * 4u + 3u] = static_cast<uint8_t>(h[i] & 0xFFu);
  }
  return out;
}

std::string base64_encode(std::span<const uint8_t> input) {
  static constexpr char kTable[65] =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  std::string out;
  out.reserve(((input.size() + 2u) / 3u) * 4u);
  for (size_t i = 0u; i < input.size(); i += 3u) {
    const size_t rem = input.size() - i;
    const uint32_t b0 = static_cast<uint32_t>(input[i]);
    const uint32_t b1 = rem > 1u ? static_cast<uint32_t>(input[i + 1u]) : 0u;
    const uint32_t b2 = rem > 2u ? static_cast<uint32_t>(input[i + 2u]) : 0u;
    const uint32_t t = (b0 << 16u) | (b1 << 8u) | b2;
    out.push_back(kTable[(t >> 18u) & 0x3Fu]);
    out.push_back(kTable[(t >> 12u) & 0x3Fu]);
    out.push_back(rem > 1u ? kTable[(t >> 6u) & 0x3Fu] : '=');
    out.push_back(rem > 2u ? kTable[t & 0x3Fu] : '=');
  }
  return out;
}

std::string compute_ws_accept_key(std::string_view client_key) {
  const std::string_view trimmed = trim_ascii(client_key);
  std::string concat;
  concat.reserve(trimmed.size() + kWsGuid.size());
  concat.append(trimmed);
  concat.append(kWsGuid);
  const auto digest = sha1_digest(std::span<const uint8_t>(
      reinterpret_cast<const uint8_t*>(concat.data()), concat.size()));
  return base64_encode(digest);
}

std::vector<uint8_t> encode_ws_frame(
    bool fin,
    WsOpcode opcode,
    std::span<const uint8_t> payload,
    std::optional<std::array<uint8_t, 4>> mask_key) {
  std::vector<uint8_t> out;
  out.reserve(14u + payload.size());
  out.push_back(static_cast<uint8_t>((fin ? 0x80u : 0x00u) | static_cast<uint8_t>(opcode)));
  const uint8_t mask_bit = mask_key.has_value() ? 0x80u : 0x00u;
  const size_t len = payload.size();
  if (len <= 125u) {
    out.push_back(static_cast<uint8_t>(mask_bit | len));
  } else if (len <= std::numeric_limits<uint16_t>::max()) {
    out.push_back(static_cast<uint8_t>(mask_bit | 126u));
    push_u16_be(out, static_cast<uint16_t>(len));
  } else {
    out.push_back(static_cast<uint8_t>(mask_bit | 127u));
    push_u64_be(out, static_cast<uint64_t>(len));
  }
  if (mask_key.has_value()) {
    const auto& mask = *mask_key;
    out.insert(out.end(), mask.begin(), mask.end());
    for (size_t i = 0u; i < payload.size(); ++i) {
      out.push_back(static_cast<uint8_t>(payload[i] ^ mask[i & 3u]));
    }
  } else {
    out.insert(out.end(), payload.begin(), payload.end());
  }
  return out;
}

bool decode_ws_frame_slice(
    std::span<const uint8_t> bytes,
    size_t max_payload_len,
    WsFrame& out_frame,
    size_t& out_consumed) {
  if (bytes.size() < 2u) {
    return false;
  }
  const bool fin = (bytes[0] & 0x80u) != 0u;
  if ((bytes[0] & 0x70u) != 0u) {
    return false;
  }
  WsOpcode opcode{};
  if (!valid_ws_opcode(static_cast<uint8_t>(bytes[0] & 0x0Fu), opcode)) {
    return false;
  }
  const bool masked = (bytes[1] & 0x80u) != 0u;
  size_t pos = 2u;
  const uint8_t len7 = static_cast<uint8_t>(bytes[1] & 0x7Fu);
  size_t payload_len = 0u;
  if (len7 <= 125u) {
    payload_len = static_cast<size_t>(len7);
  } else if (len7 == 126u) {
    if (bytes.size() < pos + 2u) return false;
    payload_len = static_cast<size_t>(read_u16_be(bytes.data() + pos));
    pos += 2u;
  } else {
    if (bytes.size() < pos + 8u) return false;
    const uint64_t n64 = read_u64_be(bytes.data() + pos);
    pos += 8u;
    if ((n64 >> 63u) != 0u || n64 > static_cast<uint64_t>(max_payload_len)) {
      return false;
    }
    payload_len = static_cast<size_t>(n64);
  }
  if (payload_len > max_payload_len) {
    return false;
  }
  std::optional<std::array<uint8_t, 4>> mask_key;
  if (masked) {
    if (bytes.size() < pos + 4u) return false;
    std::array<uint8_t, 4> m{};
    std::memcpy(m.data(), bytes.data() + pos, 4u);
    pos += 4u;
    mask_key = m;
  }
  if (bytes.size() - pos < payload_len) {
    return false;
  }
  out_frame.fin = fin;
  out_frame.opcode = opcode;
  out_frame.masked = masked;
  out_frame.payload.assign(bytes.begin() + static_cast<ptrdiff_t>(pos),
                           bytes.begin() + static_cast<ptrdiff_t>(pos + payload_len));
  if (mask_key.has_value()) {
    const auto& mask = *mask_key;
    for (size_t i = 0u; i < out_frame.payload.size(); ++i) {
      out_frame.payload[i] = static_cast<uint8_t>(out_frame.payload[i] ^ mask[i & 3u]);
    }
  }
  out_consumed = pos + payload_len;
  return true;
}

// -----------------------------------------------------------------------------
// FragReassembler
// -----------------------------------------------------------------------------

FragReassembler::FragReassembler(size_t max_frame_bytes)
    : max_frame_bytes_(std::clamp(max_frame_bytes, size_t{64u}, kMaxAllowedWireFrameBytes)) {}

void FragReassembler::clear() noexcept {
  inflight_.clear();
  order_.clear();
}

bool FragReassembler::push(
    std::span<const uint8_t> payload,
    std::chrono::steady_clock::time_point now,
    std::optional<std::vector<uint8_t>>& out_completed) {
  out_completed.reset();
  if (payload.size() < kFragSubheaderSize) {
    return false;
  }
  const uint32_t frag_msg_id = read_u32_le(payload.data());
  const uint16_t frag_idx = read_u16_le(payload.data() + 4u);
  const uint16_t frag_count = read_u16_le(payload.data() + 6u);
  const size_t total_len = static_cast<size_t>(read_u32_le(payload.data() + 8u));
  const auto chunk = payload.subspan(kFragSubheaderSize);

  if (frag_count == 0u || frag_idx >= frag_count) {
    return false;
  }
  if (total_len == 0u || total_len > max_frame_bytes_) {
    return false;
  }
  const size_t expected_count = (total_len + kMaxUdpFragChunk - 1u) / kMaxUdpFragChunk;
  if (static_cast<size_t>(frag_count) != expected_count) {
    return false;
  }
  const size_t start = static_cast<size_t>(frag_idx) * kMaxUdpFragChunk;
  if (start >= total_len) {
    return false;
  }
  const size_t expected_chunk_len = std::min(total_len - start, kMaxUdpFragChunk);
  if (chunk.size() != expected_chunk_len) {
    return false;
  }

  auto it = inflight_.find(frag_msg_id);
  if (it == inflight_.end()) {
    while (inflight_.size() >= kMaxInflightFragMessages && !order_.empty()) {
      const uint32_t oldest = order_.front();
      order_.pop_front();
      inflight_.erase(oldest);
    }
    order_.push_back(frag_msg_id);
    InFlightFrag entry{};
    entry.frag_count = frag_count;
    entry.total_len = total_len;
    entry.buf.resize(total_len, 0u);
    entry.received.resize(static_cast<size_t>(frag_count), 0u);
    entry.first_seen = now;
    it = inflight_.emplace(frag_msg_id, std::move(entry)).first;
  }

  InFlightFrag& entry = it->second;
  if (entry.frag_count != frag_count || entry.total_len != total_len) {
    return false;
  }

  std::memcpy(entry.buf.data() + start, chunk.data(), chunk.size());
  if (entry.received[frag_idx] == 0u) {
    entry.received[frag_idx] = 1u;
    ++entry.received_count;
  }
  if (frag_idx > entry.highest_idx_seen) {
    entry.highest_idx_seen = frag_idx;
  }

  if (entry.received_count == static_cast<size_t>(entry.frag_count)) {
    out_completed = std::move(entry.buf);
    inflight_.erase(it);
    order_.erase(std::remove(order_.begin(), order_.end(), frag_msg_id), order_.end());
  }
  return true;
}

std::vector<std::pair<uint32_t, std::vector<uint16_t>>> FragReassembler::poll_stale_nacks(
    std::chrono::steady_clock::time_point now,
    uint64_t retry_ms) {
  std::vector<std::pair<uint32_t, std::vector<uint16_t>>> out;
  const auto retry = std::chrono::milliseconds(retry_ms);
  const auto max_age = std::chrono::seconds(5);
  std::vector<uint32_t> expired;

  for (auto& [msg_id, entry] : inflight_) {
    if (now - entry.first_seen > max_age) {
      expired.push_back(msg_id);
      continue;
    }
    const bool due = entry.last_nack_at.has_value() ? ((now - *entry.last_nack_at) >= retry)
                                                    : ((now - entry.first_seen) >= retry);
    if (due) {
      std::vector<uint16_t> missing;
      for (size_t i = 0u; i < entry.received.size(); ++i) {
        if (entry.received[i] == 0u) {
          missing.push_back(static_cast<uint16_t>(i));
        }
      }
      if (!missing.empty()) {
        entry.last_nack_at = now;
        out.emplace_back(msg_id, std::move(missing));
      }
    }
  }

  for (uint32_t id : expired) {
    inflight_.erase(id);
    order_.erase(std::remove(order_.begin(), order_.end(), id), order_.end());
  }
  return out;
}

// -----------------------------------------------------------------------------
// SyncDeltaReceiver
// -----------------------------------------------------------------------------

SyncDeltaReceiver::SyncDeltaReceiver(size_t max_window)
    : max_window_(std::clamp(max_window, size_t{4u}, size_t{1024u})) {}

void SyncDeltaReceiver::reset(size_t max_window) {
  if (max_window > 0u) {
    max_window_ = std::clamp(max_window, size_t{4u}, size_t{1024u});
  }
  last_applied_seq_ = 0u;
  last_applied_frame_.clear();
  reorder_buf_.clear();
}

std::optional<uint32_t> SyncDeltaReceiver::next_missing_seq() const noexcept {
  if (reorder_buf_.empty()) {
    return std::nullopt;
  }
  return static_cast<uint32_t>(last_applied_seq_ + 1u);
}

std::vector<uint8_t> SyncDeltaReceiver::build_ack_packet() const {
  uint64_t sack = 0u;
  for (const auto& [buffered_seq, _] : reorder_buf_) {
    if (buffered_seq > last_applied_seq_) {
      const uint32_t diff = buffered_seq - last_applied_seq_;
      if (diff >= 1u && diff <= 64u) {
        sack |= (1ULL << (diff - 1u));
      }
    }
  }
  std::array<uint8_t, 8> payload{};
  write_u64_le(payload.data(), sack);
  return encode_nvd_packet(DeltaPacketKind::Ack, 0u, 0u, 0u, last_applied_seq_, payload);
}

std::vector<uint8_t> SyncDeltaReceiver::build_nack_packet(uint32_t missing_seq) const {
  std::array<uint8_t, 4> payload{};
  write_u32_le(payload.data(), missing_seq);
  return encode_nvd_packet(DeltaPacketKind::Nack, 0u, 0u, 0u, last_applied_seq_, payload);
}

DeltaReceiveStatus SyncDeltaReceiver::apply_single_packet(
    const DeltaPacketView& pkt,
    std::vector<uint8_t>& out) {
  switch (pkt.header.kind) {
    case DeltaPacketKind::Keyframe:
      if (pkt.payload.size() > kMaxDecodedFrameBytes) {
        return DeltaReceiveStatus::InvalidPacket;
      }
      out.assign(pkt.payload.begin(), pkt.payload.end());
      return DeltaReceiveStatus::Ok;
    case DeltaPacketKind::XorRleDelta:
      if (pkt.header.base_seq != last_applied_seq_) {
        return DeltaReceiveStatus::BaseSequenceMismatch;
      }
      return decode_xor_rle_delta(last_applied_frame_, pkt.payload, out)
                 ? DeltaReceiveStatus::Ok
                 : DeltaReceiveStatus::InvalidPacket;
    case DeltaPacketKind::CommandOpDelta:
      if (pkt.header.base_seq != last_applied_seq_) {
        return DeltaReceiveStatus::BaseSequenceMismatch;
      }
      return decode_command_op_delta(last_applied_frame_, pkt.payload, out)
                 ? DeltaReceiveStatus::Ok
                 : DeltaReceiveStatus::InvalidPacket;
    case DeltaPacketKind::Ack:
    case DeltaPacketKind::Nack:
      return DeltaReceiveStatus::InvalidPacket;
  }
  return DeltaReceiveStatus::InvalidPacket;
}

DeltaReceiveStatus SyncDeltaReceiver::drain_reorder_buffer(
    std::vector<std::pair<uint32_t, std::vector<uint8_t>>>& applied) {
  for (;;) {
    auto it = reorder_buf_.find(last_applied_seq_ + 1u);
    if (it == reorder_buf_.end()) {
      break;
    }
    const std::vector<uint8_t> raw = std::move(it->second);
    reorder_buf_.erase(it);

    DeltaPacketView pkt{};
    if (!parse_nvd_packet(raw, pkt)) {
      return DeltaReceiveStatus::InvalidPacket;
    }
    std::vector<uint8_t> frame;
    const DeltaReceiveStatus st = apply_single_packet(pkt, frame);
    if (st != DeltaReceiveStatus::Ok) {
      return st;
    }
    last_applied_seq_ = pkt.header.seq;
    last_applied_frame_ = frame;
    applied.emplace_back(pkt.header.seq, std::move(frame));
  }
  return DeltaReceiveStatus::Ok;
}

DeltaReceiveOutcome SyncDeltaReceiver::receive_packet(std::span<const uint8_t> packet_bytes) {
  DeltaReceiveOutcome out{};
  DeltaPacketView parsed{};
  if (!parse_nvd_packet(packet_bytes, parsed)) {
    out.status = DeltaReceiveStatus::InvalidPacket;
    return out;
  }
  const DeltaPacketHeader& hdr = parsed.header;
  if (hdr.kind == DeltaPacketKind::Ack || hdr.kind == DeltaPacketKind::Nack) {
    out.status = DeltaReceiveStatus::Ok;
    return out;
  }
  if (hdr.seq <= last_applied_seq_) {
    out.status = DeltaReceiveStatus::Ok;
    out.ack_packet = build_ack_packet();
    return out;
  }

  const uint32_t expected_next = last_applied_seq_ + 1u;
  if (hdr.kind == DeltaPacketKind::Keyframe && hdr.seq >= expected_next) {
    if (parsed.payload.size() > kMaxDecodedFrameBytes) {
      out.status = DeltaReceiveStatus::InvalidPacket;
      return out;
    }
    std::vector<uint8_t> frame(parsed.payload.begin(), parsed.payload.end());
    last_applied_seq_ = hdr.seq;
    last_applied_frame_ = frame;
    out.applied_frames.emplace_back(hdr.seq, std::move(frame));
    for (auto it = reorder_buf_.begin(); it != reorder_buf_.end();) {
      if (it->first <= last_applied_seq_) {
        it = reorder_buf_.erase(it);
      } else {
        ++it;
      }
    }
    const DeltaReceiveStatus drain_st = drain_reorder_buffer(out.applied_frames);
    if (drain_st != DeltaReceiveStatus::Ok) {
      out.status = drain_st;
      return out;
    }
  } else if (hdr.seq == expected_next) {
    std::vector<uint8_t> frame;
    const DeltaReceiveStatus st = apply_single_packet(parsed, frame);
    if (st != DeltaReceiveStatus::Ok) {
      out.status = st;
      return out;
    }
    last_applied_seq_ = hdr.seq;
    last_applied_frame_ = frame;
    out.applied_frames.emplace_back(hdr.seq, std::move(frame));
    const DeltaReceiveStatus drain_st = drain_reorder_buffer(out.applied_frames);
    if (drain_st != DeltaReceiveStatus::Ok) {
      out.status = drain_st;
      return out;
    }
  } else {
    if (reorder_buf_.size() < max_window_) {
      reorder_buf_[hdr.seq] = std::vector<uint8_t>(packet_bytes.begin(), packet_bytes.end());
    }
    const uint32_t gap_end = std::min(hdr.seq, expected_next + 8u);
    for (uint32_t missing = expected_next; missing < gap_end; ++missing) {
      if (reorder_buf_.find(missing) == reorder_buf_.end()) {
        out.nack_packets.push_back(build_nack_packet(missing));
      }
    }
  }

  out.status = DeltaReceiveStatus::Ok;
  out.ack_packet = build_ack_packet();
  return out;
}

} // namespace pair_wire

// =============================================================================
// Endpoint Parser & EventRing Batch Encoder
// =============================================================================

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

  std::string_view raw = trim_ascii(endpoint);
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
    if (ascii_iequals(scheme, "udp")) {
      parsed_transport = PairTransport::Udp;
    } else if (ascii_iequals(scheme, "tcp")) {
      parsed_transport = PairTransport::Tcp;
    } else if (ascii_iequals(scheme, "ws") || ascii_iequals(scheme, "http")) {
      parsed_transport = PairTransport::WebSocket;
    } else {
      return fail("Unsupported pairing URL scheme: " + std::string(scheme));
    }
  }

  if (!transport_override.empty()) {
    const std::string_view tr = trim_ascii(transport_override);
    if (ascii_iequals(tr, "udp")) {
      parsed_transport = PairTransport::Udp;
    } else if (ascii_iequals(tr, "tcp")) {
      parsed_transport = PairTransport::Tcp;
    } else if (ascii_iequals(tr, "ws") || ascii_iequals(tr, "websocket")) {
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
  const auto [ptr, ec] = std::from_chars(port_sv.data(), port_sv.data() + port_sv.size(), port_num);
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

// =============================================================================
// `nve::PairClient::Impl` (POSIX Sockets + Clean Windows Stub)
// =============================================================================

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

#if !defined(_WIN32)
  int sock_fd{-1};
  int wake_pipe[2]{-1, -1};
  sockaddr_storage peer_addr{};
  socklen_t peer_addr_len{0};
#endif

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
    if (frames.empty()) {
      return;
    }
    const size_t n = frames.size();
    {
      std::lock_guard<std::mutex> lock(queue_mutex);
      for (auto& item : frames) {
        while (pending_frames.size() >= kMaxPendingFrames) {
          pending_frames.pop_front();
        }
        pending_frames.push_back(std::move(item));
      }
      has_pending.store(!pending_frames.empty(), std::memory_order_release);
    }
    {
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.frames_received += static_cast<uint64_t>(n);
      metrics.last_applied_seq = last_seq;
    }
    if (on_frame_ready) {
      on_frame_ready();
    }
  }

#if defined(_WIN32)
  bool connect_impl(const PairClientConfig&, WakeCallback, std::string* out_error) {
    if (out_error != nullptr) {
      *out_error = "PairClient POSIX socket runtime is disabled on _WIN32 builds";
    }
    return false;
  }
  bool send_wire_message(std::span<const uint8_t>) { return false; }
  void disconnect() noexcept {
    connected.store(false, std::memory_order_release);
    shutdown_flag.store(true, std::memory_order_release);
    session_id.store(0u, std::memory_order_release);
  }
#else
  void close_sockets_locked() noexcept {
    if (sock_fd >= 0) {
      ::shutdown(sock_fd, SHUT_RDWR);
      ::close(sock_fd);
      sock_fd = -1;
    }
    if (wake_pipe[0] >= 0) {
      ::close(wake_pipe[0]);
      wake_pipe[0] = -1;
    }
    if (wake_pipe[1] >= 0) {
      ::close(wake_pipe[1]);
      wake_pipe[1] = -1;
    }
  }

  void signal_wake_pipe() noexcept {
    if (wake_pipe[1] >= 0) {
      const uint8_t b = 1u;
      const ssize_t ignored = ::write(wake_pipe[1], &b, 1u);
      (void)ignored;
    }
  }

  static void disable_sigpipe(int fd) noexcept {
#if defined(SO_NOSIGPIPE)
    const int one = 1;
    (void)::setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof(one));
#else
    (void)fd;
#endif
  }

  static bool set_nonblocking(int fd, bool nonblocking) noexcept {
    const int flags = ::fcntl(fd, F_GETFL, 0);
    if (flags < 0) return false;
    const int next = nonblocking ? (flags | O_NONBLOCK) : (flags & ~O_NONBLOCK);
    return ::fcntl(fd, F_SETFL, next) == 0;
  }

  // Returns: 1 = readable, 0 = timeout/idle, -1 = shutdown/error.
  int wait_readable(int fd, int timeout_ms) const noexcept {
    pollfd pfds[2]{};
    pfds[0].fd = fd;
    pfds[0].events = POLLIN;
    nfds_t nfds = 1u;
    if (wake_pipe[0] >= 0) {
      pfds[1].fd = wake_pipe[0];
      pfds[1].events = POLLIN;
      nfds = 2u;
    }
    for (;;) {
      if (shutdown_flag.load(std::memory_order_acquire)) {
        return -1;
      }
      const int rc = ::poll(pfds, nfds, timeout_ms);
      if (rc > 0) {
        if (nfds == 2u && (pfds[1].revents & (POLLIN | POLLHUP | POLLERR)) != 0) {
          return -1;
        }
        if ((pfds[0].revents & (POLLIN | POLLHUP | POLLERR)) != 0) {
          return 1;
        }
        return 0;
      }
      if (rc == 0) {
        return 0;
      }
      if (errno == EINTR) {
        continue;
      }
      return -1;
    }
  }

  bool write_all_stream(int fd, const uint8_t* data, size_t len) noexcept {
    size_t sent = 0u;
    const auto deadline = Clock::now() + std::chrono::milliseconds(2000);
    while (sent < len) {
      if (shutdown_flag.load(std::memory_order_acquire)) {
        return false;
      }
#if defined(MSG_NOSIGNAL)
      const int flags = MSG_NOSIGNAL;
#else
      const int flags = 0;
#endif
      const ssize_t n = ::send(fd, data + sent, len - sent, flags);
      if (n > 0) {
        sent += static_cast<size_t>(n);
        continue;
      }
      if (n < 0 && errno == EINTR) {
        continue;
      }
      if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
        const auto now = Clock::now();
        if (now >= deadline) {
          return false;
        }
        pollfd pfd{};
        pfd.fd = fd;
        pfd.events = POLLOUT;
        const int rem = static_cast<int>(
            std::chrono::duration_cast<std::chrono::milliseconds>(deadline - now).count());
        const int rc = ::poll(&pfd, 1u, std::max(1, std::min(rem, 50)));
        if (rc < 0 && errno != EINTR) {
          return false;
        }
        continue;
      }
      return false;
    }
    return true;
  }

  // Returns: 1 = read exact `len` bytes, 0 = idle (only when allow_idle_on_zero && offset==0),
  // -1 = EOF/error/shutdown.
  int read_exact_stream(
      int fd,
      uint8_t* buf,
      size_t len,
      bool allow_idle_on_zero,
      int idle_wait_ms,
      int stall_timeout_ms) noexcept {
    size_t offset = 0u;
    std::optional<Clock::time_point> started;
    while (offset < len) {
      if (shutdown_flag.load(std::memory_order_acquire)) {
        return -1;
      }
      const ssize_t n = ::recv(fd, buf + offset, len - offset, 0);
      if (n > 0) {
        offset += static_cast<size_t>(n);
        if (!started.has_value()) {
          started = Clock::now();
        }
        continue;
      }
      if (n == 0) {
        return -1;
      }
      if (errno == EINTR) {
        continue;
      }
      if (errno == EAGAIN || errno == EWOULDBLOCK) {
        if (offset == 0u && allow_idle_on_zero) {
          const int ready = wait_readable(fd, idle_wait_ms);
          if (ready <= 0) {
            return ready;
          }
          continue;
        }
        if (!started.has_value()) {
          started = Clock::now();
        }
        const auto elapsed =
            std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - *started).count();
        if (elapsed >= stall_timeout_ms) {
          return -1;
        }
        const int rem = std::max(1, std::min(50, stall_timeout_ms - static_cast<int>(elapsed)));
        const int ready = wait_readable(fd, rem);
        if (ready < 0) {
          return -1;
        }
        continue;
      }
      return -1;
    }
    return 1;
  }

  bool send_stream_message_locked(int fd, std::span<const uint8_t> env_bytes) {
    if (fd < 0) return false;
    if (config.transport == PairTransport::Tcp) {
      if (env_bytes.size() > std::numeric_limits<uint32_t>::max()) return false;
      std::vector<uint8_t> framed;
      framed.reserve(4u + env_bytes.size());
      push_u32_le(framed, static_cast<uint32_t>(env_bytes.size()));
      framed.insert(framed.end(), env_bytes.begin(), env_bytes.end());
      if (!write_all_stream(fd, framed.data(), framed.size())) return false;
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.wire_bytes_sent += framed.size();
      return true;
    }
    if (config.transport == PairTransport::WebSocket) {
      const uint32_t m32 = static_cast<uint32_t>(next_nonce(env_bytes.size()));
      std::array<uint8_t, 4> mask{};
      write_u32_le(mask.data(), m32);
      const std::vector<uint8_t> ws =
          pair_wire::encode_ws_frame(true, WsOpcode::Binary, env_bytes, mask);
      if (!write_all_stream(fd, ws.data(), ws.size())) return false;
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.wire_bytes_sent += ws.size();
      return true;
    }
    return false;
  }

  bool send_wire_message(std::span<const uint8_t> env_bytes) {
    std::lock_guard<std::mutex> wlock(write_mutex);
    if (sock_fd < 0) {
      return false;
    }
    if (config.transport == PairTransport::Udp) {
      const ssize_t n = ::sendto(
          sock_fd,
          env_bytes.data(),
          env_bytes.size(),
          0,
          reinterpret_cast<const sockaddr*>(&peer_addr),
          peer_addr_len);
      if (n != static_cast<ssize_t>(env_bytes.size())) {
        return false;
      }
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.wire_bytes_sent += static_cast<uint64_t>(n);
      return true;
    }
    return send_stream_message_locked(sock_fd, env_bytes);
  }

  // Returns: 1 = frame decoded into `out_frame`, 0 = idle, -1 = closed/error.
  int read_stream_message(
      int fd,
      std::vector<uint8_t>& out_frame,
      size_t& out_wire_bytes,
      int idle_wait_ms) {
    out_frame.clear();
    out_wire_bytes = 0u;
    const size_t max_bytes = clamped_max_frame_bytes();

    if (config.transport == PairTransport::Tcp) {
      uint8_t len_buf[4]{};
      const int rc = read_exact_stream(fd, len_buf, 4u, true, idle_wait_ms, 500);
      if (rc <= 0) {
        return rc;
      }
      const size_t declared = static_cast<size_t>(read_u32_le(len_buf));
      if (declared == 0u || declared > max_bytes) {
        return -1;
      }
      out_frame.resize(declared);
      if (read_exact_stream(fd, out_frame.data(), declared, false, idle_wait_ms, 500) != 1) {
        return -1;
      }
      out_wire_bytes = 4u + declared;
      return 1;
    }

    // RFC 6455 WebSocket binary frame reader with continuation & ping/pong support.
    std::optional<std::vector<uint8_t>> assembled;
    size_t total_wire = 0u;
    for (;;) {
      uint8_t hdr[2]{};
      const int rc =
          read_exact_stream(fd, hdr, 2u, !assembled.has_value(), idle_wait_ms, 500);
      if (rc <= 0) {
        return rc;
      }
      total_wire += 2u;
      const bool fin = (hdr[0] & 0x80u) != 0u;
      if ((hdr[0] & 0x70u) != 0u) {
        return -1;
      }
      WsOpcode opcode{};
      if (!valid_ws_opcode(static_cast<uint8_t>(hdr[0] & 0x0Fu), opcode)) {
        return -1;
      }
      const bool masked = (hdr[1] & 0x80u) != 0u;
      const uint8_t len7 = static_cast<uint8_t>(hdr[1] & 0x7Fu);
      size_t payload_len = 0u;
      if (len7 <= 125u) {
        payload_len = static_cast<size_t>(len7);
      } else if (len7 == 126u) {
        uint8_t b[2]{};
        if (read_exact_stream(fd, b, 2u, false, idle_wait_ms, 500) != 1) return -1;
        total_wire += 2u;
        payload_len = static_cast<size_t>(read_u16_be(b));
      } else {
        uint8_t b[8]{};
        if (read_exact_stream(fd, b, 8u, false, idle_wait_ms, 500) != 1) return -1;
        total_wire += 8u;
        const uint64_t n64 = read_u64_be(b);
        if ((n64 >> 63u) != 0u || n64 > static_cast<uint64_t>(max_bytes)) return -1;
        payload_len = static_cast<size_t>(n64);
      }

      const size_t existing = assembled.has_value() ? assembled->size() : 0u;
      if (payload_len > max_bytes || payload_len > max_bytes - existing) {
        return -1;
      }

      std::optional<std::array<uint8_t, 4>> mask_key;
      if (masked) {
        std::array<uint8_t, 4> m{};
        if (read_exact_stream(fd, m.data(), 4u, false, idle_wait_ms, 500) != 1) return -1;
        total_wire += 4u;
        mask_key = m;
      }

      std::vector<uint8_t> payload(payload_len);
      if (payload_len > 0u) {
        if (read_exact_stream(fd, payload.data(), payload_len, false, idle_wait_ms, 500) != 1) {
          return -1;
        }
        total_wire += payload_len;
        if (mask_key.has_value()) {
          const auto& m = *mask_key;
          for (size_t i = 0u; i < payload.size(); ++i) {
            payload[i] = static_cast<uint8_t>(payload[i] ^ m[i & 3u]);
          }
        }
      }

      switch (opcode) {
        case WsOpcode::Close:
          return -1;
        case WsOpcode::Ping: {
          const uint32_t m32 = static_cast<uint32_t>(next_nonce(7u));
          std::array<uint8_t, 4> pm{};
          write_u32_le(pm.data(), m32);
          const auto pong = pair_wire::encode_ws_frame(true, WsOpcode::Pong, payload, pm);
          std::lock_guard<std::mutex> wlock(write_mutex);
          if (sock_fd >= 0) {
            (void)write_all_stream(sock_fd, pong.data(), pong.size());
          }
          break;
        }
        case WsOpcode::Pong:
          break;
        case WsOpcode::Binary:
        case WsOpcode::Text:
          if (fin && !assembled.has_value()) {
            out_frame = std::move(payload);
            out_wire_bytes = total_wire;
            return 1;
          }
          if (!assembled.has_value()) {
            assembled.emplace();
          }
          assembled->insert(assembled->end(), payload.begin(), payload.end());
          if (fin) {
            out_frame = std::move(*assembled);
            out_wire_bytes = total_wire;
            return 1;
          }
          break;
        case WsOpcode::Continuation:
          if (!assembled.has_value()) {
            return -1;
          }
          assembled->insert(assembled->end(), payload.begin(), payload.end());
          if (fin) {
            out_frame = std::move(*assembled);
            out_wire_bytes = total_wire;
            return 1;
          }
          break;
      }
    }
  }

  bool perform_ws_upgrade(int fd, std::string* out_error) {
    auto fail = [&](std::string msg) {
      if (out_error != nullptr) *out_error = std::move(msg);
      return false;
    };

    std::array<uint8_t, 16> key_raw{};
    write_u64_le(key_raw.data(), next_nonce(0xABCD1234ULL));
    write_u64_le(key_raw.data() + 8u, next_nonce(0x5678EF01ULL));
    const std::string client_key = pair_wire::base64_encode(key_raw);
    const std::string expected_accept = pair_wire::compute_ws_accept_key(client_key);

    const std::string host_hdr = config.host + ":" + std::to_string(config.port);
    const std::string req = "GET " + config.ws_path +
                            " HTTP/1.1\r\n"
                            "Host: " +
                            host_hdr +
                            "\r\n"
                            "Upgrade: websocket\r\n"
                            "Connection: Upgrade\r\n"
                            "Sec-WebSocket-Key: " +
                            client_key +
                            "\r\n"
                            "Sec-WebSocket-Version: 13\r\n\r\n";
    if (!write_all_stream(fd, reinterpret_cast<const uint8_t*>(req.data()), req.size())) {
      return fail("Failed writing WebSocket HTTP upgrade request");
    }

    const auto deadline = Clock::now() + std::chrono::milliseconds(config.handshake_timeout_ms);
    std::string resp;
    resp.reserve(512u);
    while (Clock::now() < deadline) {
      uint8_t ch = 0u;
      const int rem = static_cast<int>(
          std::chrono::duration_cast<std::chrono::milliseconds>(deadline - Clock::now()).count());
      const int rc = read_exact_stream(fd, &ch, 1u, true, std::max(1, std::min(rem, 20)), 500);
      if (rc < 0) {
        return fail("Connection closed during WebSocket HTTP upgrade");
      }
      if (rc == 0) {
        continue;
      }
      resp.push_back(static_cast<char>(ch));
      if (resp.size() > kMaxWsUpgradeHeaderBytes) {
        return fail("WebSocket upgrade response headers exceeded 8 KiB limit");
      }
      if (resp.size() >= 4u && resp.compare(resp.size() - 4u, 4u, "\r\n\r\n") == 0) {
        break;
      }
    }
    if (resp.size() < 4u || resp.compare(resp.size() - 4u, 4u, "\r\n\r\n") != 0) {
      return fail("WebSocket HTTP upgrade timed out");
    }

    const size_t first_eol = resp.find("\r\n");
    const std::string_view status_line =
        (first_eol != std::string::npos) ? std::string_view(resp).substr(0u, first_eol)
                                         : std::string_view(resp);
    if (status_line.find("101") == std::string_view::npos) {
      return fail("WebSocket server did not reply with HTTP 101 Switching Protocols");
    }

    bool accept_matched = false;
    size_t line_start = (first_eol != std::string::npos) ? (first_eol + 2u) : resp.size();
    while (line_start < resp.size()) {
      const size_t line_end = resp.find("\r\n", line_start);
      const std::string_view line =
          (line_end != std::string::npos)
              ? std::string_view(resp).substr(line_start, line_end - line_start)
              : std::string_view(resp).substr(line_start);
      if (line.empty()) break;
      const size_t colon = line.find(':');
      if (colon != std::string_view::npos) {
        const std::string_view name = trim_ascii(line.substr(0u, colon));
        const std::string_view val = trim_ascii(line.substr(colon + 1u));
        if (ascii_iequals(name, "sec-websocket-accept") && val == expected_accept) {
          accept_matched = true;
          break;
        }
      }
      if (line_end == std::string::npos) break;
      line_start = line_end + 2u;
    }
    if (!accept_matched) {
      return fail("Invalid or missing Sec-WebSocket-Accept header");
    }
    return true;
  }

  bool verify_hello_ack_envelope(
      const TransportEnvelopeView& env,
      uint64_t client_nonce,
      uint32_t& out_sid,
      size_t& out_window,
      std::string* out_error) const {
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

  bool connect_impl(
      const PairClientConfig& cfg,
      WakeCallback wake_cb,
      std::string* out_error) {
    auto fail = [&](std::string msg) {
      close_sockets_locked();
      if (out_error != nullptr) *out_error = std::move(msg);
      return false;
    };

    disconnect();

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
    {
      std::lock_guard<std::mutex> lock(queue_mutex);
      pending_frames.clear();
    }
    {
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics = PairClientMetrics{};
    }

    if (::pipe(wake_pipe) != 0) {
      return fail("Failed to create internal wake pipe");
    }
    (void)set_nonblocking(wake_pipe[0], true);
    (void)set_nonblocking(wake_pipe[1], true);

    addrinfo hints{};
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = (config.transport == PairTransport::Udp) ? SOCK_DGRAM : SOCK_STREAM;
    const std::string port_str = std::to_string(config.port);
    addrinfo* res = nullptr;
    const int gai = ::getaddrinfo(config.host.c_str(), port_str.c_str(), &hints, &res);
    if (gai != 0 || res == nullptr) {
      return fail(std::string("getaddrinfo failed: ") + ::gai_strerror(gai));
    }

    int fd = -1;
    sockaddr_storage chosen_addr{};
    socklen_t chosen_len = 0;

    for (addrinfo* ai = res; ai != nullptr; ai = ai->ai_next) {
      fd = ::socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
      if (fd < 0) continue;
      disable_sigpipe(fd);
      (void)set_nonblocking(fd, true);

      if (config.transport == PairTransport::Udp) {
        std::memcpy(&chosen_addr, ai->ai_addr, ai->ai_addrlen);
        chosen_len = static_cast<socklen_t>(ai->ai_addrlen);
        break;
      }

      const int one = 1;
      (void)::setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
      const int crc = ::connect(fd, ai->ai_addr, ai->ai_addrlen);
      if (crc == 0) {
        std::memcpy(&chosen_addr, ai->ai_addr, ai->ai_addrlen);
        chosen_len = static_cast<socklen_t>(ai->ai_addrlen);
        break;
      }
      if (errno == EINPROGRESS) {
        pollfd pfd{};
        pfd.fd = fd;
        pfd.events = POLLOUT;
        const int prc = ::poll(&pfd, 1u, static_cast<int>(config.handshake_timeout_ms));
        int so_err = 0;
        socklen_t so_len = sizeof(so_err);
        if (prc > 0 && ::getsockopt(fd, SOL_SOCKET, SO_ERROR, &so_err, &so_len) == 0 &&
            so_err == 0) {
          std::memcpy(&chosen_addr, ai->ai_addr, ai->ai_addrlen);
          chosen_len = static_cast<socklen_t>(ai->ai_addrlen);
          break;
        }
      }
      ::close(fd);
      fd = -1;
    }
    ::freeaddrinfo(res);

    if (fd < 0) {
      return fail("Failed to connect to " + config.host + ":" + std::to_string(config.port));
    }
    sock_fd = fd;
    peer_addr = chosen_addr;
    peer_addr_len = chosen_len;

    if (config.transport == PairTransport::WebSocket) {
      std::string ws_err;
      if (!perform_ws_upgrade(sock_fd, &ws_err)) {
        return fail(std::move(ws_err));
      }
    }

    // Build and send NVT1 Hello handshake.
    const uint64_t client_nonce = next_nonce(0x2468ACE0ULL ^ static_cast<uint64_t>(config.port));
    HelloPayload hello{};
    hello.schema_version = config.schema_version;
    hello.max_window = config.max_window;
    hello.client_last_seq = 0u;
    hello.client_nonce = client_nonce;
    hello.token_proof =
        pair_wire::derive_token_proof(config.pairing_token, config.schema_version);
    const auto hello_bytes = pair_wire::encode_hello_payload(hello);
    const auto hello_env = pair_wire::encode_envelope(TransportMsgKind::Hello, 0u, hello_bytes);

    const auto deadline = Clock::now() + std::chrono::milliseconds(config.handshake_timeout_ms);
    uint32_t negotiated_sid = 0u;
    size_t negotiated_window = config.max_window;
    bool handshake_ok = false;

    if (config.transport == PairTransport::Udp) {
      std::optional<Clock::time_point> last_send;
      std::array<uint8_t, 2048> dgram{};
      while (Clock::now() < deadline) {
        const auto now = Clock::now();
        if (!last_send.has_value() ||
            std::chrono::duration_cast<std::chrono::milliseconds>(now - *last_send).count() >= 25) {
          if (!send_wire_message(hello_env)) {
            return fail("Failed sending UDP Hello datagram");
          }
          last_send = now;
        }
        const int ready = wait_readable(sock_fd, 20);
        if (ready < 0) {
          return fail("Interrupted during UDP pairing handshake");
        }
        if (ready == 0) {
          continue;
        }
        const ssize_t n = ::recvfrom(sock_fd, dgram.data(), dgram.size(), 0, nullptr, nullptr);
        if (n <= 0) {
          continue;
        }
        {
          std::lock_guard<std::mutex> lock(metrics_mutex);
          metrics.wire_bytes_received += static_cast<uint64_t>(n);
        }
        TransportEnvelopeView env{};
        if (!pair_wire::parse_envelope(
                std::span<const uint8_t>(dgram.data(), static_cast<size_t>(n)), env)) {
          continue;
        }
        if (env.kind == TransportMsgKind::HelloAck || env.kind == TransportMsgKind::Reject) {
          std::string herr;
          if (!verify_hello_ack_envelope(
                  env, client_nonce, negotiated_sid, negotiated_window, &herr)) {
            return fail(std::move(herr));
          }
          handshake_ok = true;
          break;
        }
      }
    } else {
      if (!send_wire_message(hello_env)) {
        return fail("Failed sending TCP/WS Hello handshake");
      }
      std::vector<uint8_t> reply;
      size_t wire_len = 0u;
      while (Clock::now() < deadline) {
        const int rem = static_cast<int>(
            std::chrono::duration_cast<std::chrono::milliseconds>(deadline - Clock::now()).count());
        const int rc = read_stream_message(sock_fd, reply, wire_len, std::max(1, std::min(rem, 20)));
        if (rc < 0) {
          return fail("Connection closed during pairing handshake");
        }
        if (rc == 0) {
          continue;
        }
        {
          std::lock_guard<std::mutex> lock(metrics_mutex);
          metrics.wire_bytes_received += wire_len;
        }
        TransportEnvelopeView env{};
        if (!pair_wire::parse_envelope(reply, env)) {
          return fail("Malformed NVT1 envelope during handshake");
        }
        std::string herr;
        if (!verify_hello_ack_envelope(
                env, client_nonce, negotiated_sid, negotiated_window, &herr)) {
          return fail(std::move(herr));
        }
        handshake_ok = true;
        break;
      }
    }

    if (!handshake_ok) {
      return fail("Pairing handshake timed out");
    }

    session_id.store(negotiated_sid, std::memory_order_release);
    connected.store(true, std::memory_order_release);
    {
      std::lock_guard<std::mutex> lock(metrics_mutex);
      metrics.session_id = negotiated_sid;
    }

    // Send initial Ping so server can measure RTT immediately.
    const auto ping = pair_wire::encode_ping_or_pong(TransportMsgKind::Ping, negotiated_sid, now_us());
    (void)send_wire_message(ping);

    reader_thread = std::thread(
        [this, negotiated_sid, negotiated_window]() { run_reader_loop(negotiated_sid, negotiated_window); });

    if (out_error != nullptr) {
      out_error->clear();
    }
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
        const auto req =
            pair_wire::encode_keyframe_request(sid, receiver.last_applied_seq(), reason);
        if (send_wire_message(req)) {
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

    for (const auto& [seq, _] : outcome.applied_frames) {
      nack_cooldowns.erase(seq);
    }
    if (!outcome.applied_frames.empty()) {
      push_applied_frames(outcome.applied_frames, receiver.last_applied_seq());
    }

    if (outcome.ack_packet.has_value()) {
      const auto ack_env =
          pair_wire::encode_envelope(TransportMsgKind::DeltaSingle, sid, *outcome.ack_packet);
      if (send_wire_message(ack_env)) {
        std::lock_guard<std::mutex> lock(metrics_mutex);
        ++metrics.acks_sent;
      }
    }

    if (config.transport == PairTransport::Udp) {
      for (const auto& nack_nvd : outcome.nack_packets) {
        DeltaPacketView np{};
        if (!pair_wire::parse_nvd_packet(nack_nvd, np) || np.payload.size() != 4u) continue;
        const uint32_t missing_seq = read_u32_le(np.payload.data());
        if (parsed.header.seq >= missing_seq && (parsed.header.seq - missing_seq) < 2u) {
          continue;
        }
        auto it = nack_cooldowns.find(missing_seq);
        if (it == nack_cooldowns.end() ||
            std::chrono::duration_cast<std::chrono::milliseconds>(now - it->second).count() >= 25) {
          nack_cooldowns[missing_seq] = now;
          const auto nack_env =
              pair_wire::encode_envelope(TransportMsgKind::DeltaSingle, sid, nack_nvd);
          if (send_wire_message(nack_env)) {
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
    auto last_ping = Clock::now();
    auto last_tick = Clock::now();
    const auto hb_interval = std::chrono::milliseconds(config.heartbeat_interval_ms);

    std::array<uint8_t, 2048> udp_buf{};
    std::vector<uint8_t> stream_buf;

    while (!shutdown_flag.load(std::memory_order_acquire)) {
      std::span<const uint8_t> raw_msg;
      if (config.transport == PairTransport::Udp) {
        const int ready = wait_readable(sock_fd, 10);
        if (ready < 0) {
          break;
        }
        if (ready > 0) {
          const ssize_t n =
              ::recvfrom(sock_fd, udp_buf.data(), udp_buf.size(), 0, nullptr, nullptr);
          if (n > 0) {
            {
              std::lock_guard<std::mutex> lock(metrics_mutex);
              metrics.wire_bytes_received += static_cast<uint64_t>(n);
            }
            raw_msg = std::span<const uint8_t>(udp_buf.data(), static_cast<size_t>(n));
          } else if (n < 0 && errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR) {
            break;
          }
        }
      } else {
        size_t wire_len = 0u;
        const int rc = read_stream_message(sock_fd, stream_buf, wire_len, 10);
        if (rc < 0) {
          break;
        }
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
        } else {
          switch (env.kind) {
            case TransportMsgKind::DeltaSingle:
              handle_nvd_packet(sid, receiver, env.payload, nack_cooldowns, last_kf_req);
              break;
            case TransportMsgKind::DeltaFrag: {
              std::optional<std::vector<uint8_t>> completed;
              if (!reassembler.push(env.payload, Clock::now(), completed)) {
                std::lock_guard<std::mutex> lock(metrics_mutex);
                ++metrics.malformed_packets_dropped;
              } else if (completed.has_value()) {
                handle_nvd_packet(sid, receiver, *completed, nack_cooldowns, last_kf_req);
              }
              break;
            }
            case TransportMsgKind::Ping: {
              uint64_t ts = 0u;
              if (pair_wire::decode_timestamp_us(env.payload, ts)) {
                const auto pong = pair_wire::encode_ping_or_pong(TransportMsgKind::Pong, sid, ts);
                (void)send_wire_message(pong);
              }
              break;
            }
            case TransportMsgKind::Pong:
              break;
            case TransportMsgKind::Bye:
              shutdown_flag.store(true, std::memory_order_release);
              break;
            default: {
              std::lock_guard<std::mutex> lock(metrics_mutex);
              ++metrics.malformed_packets_dropped;
              break;
            }
          }
        }
      }

      const auto now = Clock::now();
      if (config.transport == PairTransport::Udp &&
          std::chrono::duration_cast<std::chrono::milliseconds>(now - last_tick).count() >= 10) {
        last_tick = now;
        for (const auto& [mid, missing] : reassembler.poll_stale_nacks(now, 15u)) {
          const auto fnack = pair_wire::encode_frag_nack(sid, mid, missing);
          if (send_wire_message(fnack)) {
            std::lock_guard<std::mutex> lock(metrics_mutex);
            ++metrics.nacks_sent;
          }
        }
        if (const auto mseq = receiver.next_missing_seq(); mseq.has_value()) {
          auto it = nack_cooldowns.find(*mseq);
          if (it == nack_cooldowns.end() ||
              std::chrono::duration_cast<std::chrono::milliseconds>(now - it->second).count() >=
                  18) {
            nack_cooldowns[*mseq] = now;
            const auto nack_pkt = receiver.build_nack_packet(*mseq);
            const auto env =
                pair_wire::encode_envelope(TransportMsgKind::DeltaSingle, sid, nack_pkt);
            if (send_wire_message(env)) {
              std::lock_guard<std::mutex> lock(metrics_mutex);
              ++metrics.nacks_sent;
            }
          }
        }
      }

      if ((now - last_ping) >= hb_interval) {
        last_ping = now;
        const auto ping = pair_wire::encode_ping_or_pong(TransportMsgKind::Ping, sid, now_us());
        if (!send_wire_message(ping) && config.transport != PairTransport::Udp) {
          break;
        }
      }
    }

    reassembler.clear();
    connected.store(false, std::memory_order_release);
  }

  void disconnect() noexcept {
    const uint32_t sid = session_id.load(std::memory_order_acquire);
    const bool was_connected = connected.exchange(false, std::memory_order_acq_rel);
    if (was_connected && sid != 0u) {
      const auto bye = pair_wire::encode_envelope(TransportMsgKind::Bye, sid, {});
      (void)send_wire_message(bye);
    }
    shutdown_flag.store(true, std::memory_order_release);
    signal_wake_pipe();
    {
      std::lock_guard<std::mutex> wlock(write_mutex);
      if (sock_fd >= 0) {
        ::shutdown(sock_fd, SHUT_RDWR);
      }
    }
    if (reader_thread.joinable()) {
      reader_thread.join();
    }
    {
      std::lock_guard<std::mutex> wlock(write_mutex);
      close_sockets_locked();
    }
    session_id.store(0u, std::memory_order_release);
    on_frame_ready = {};
  }
#endif
};

// =============================================================================
// `nve::PairClient` Public Wrapper
// =============================================================================

PairClient::PairClient() : impl_(std::make_unique<Impl>()) {}
PairClient::~PairClient() = default;

bool PairClient::connect(
    const PairClientConfig& config,
    WakeCallback on_frame_ready,
    std::string* out_error) {
  return impl_->connect_impl(config, std::move(on_frame_ready), out_error);
}

void PairClient::disconnect() noexcept {
  impl_->disconnect();
}

bool PairClient::is_connected() const noexcept {
  return impl_->connected.load(std::memory_order_acquire);
}

uint32_t PairClient::session_id() const noexcept {
  return impl_->session_id.load(std::memory_order_acquire);
}

PairTransport PairClient::transport() const noexcept {
  return impl_->config.transport;
}

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
  if (out_seq != nullptr) {
    *out_seq = item.first;
  }
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
  if (out_seq != nullptr) {
    *out_seq = latest.first;
  }
  out_frame = std::move(latest.second);
  return true;
}

bool PairClient::send_events(
    std::span<const uint8_t> encoded_event_ring,
    std::string* out_error) {
  if (!is_connected()) {
    if (out_error != nullptr) *out_error = "PairClient is not connected";
    return false;
  }
  const size_t max_ev = impl_->clamped_max_event_bytes();
  if (!validate_input_events_bytes(encoded_event_ring, max_ev)) {
    if (out_error != nullptr) *out_error = "Invalid or oversized EventRing payload";
    return false;
  }
  const uint32_t sid = session_id();
  const auto env =
      pair_wire::encode_envelope(TransportMsgKind::InputEvents, sid, encoded_event_ring);
  if (!impl_->send_wire_message(env)) {
    if (out_error != nullptr) *out_error = "Failed writing InputEvents frame";
    return false;
  }
  {
    std::lock_guard<std::mutex> lock(impl_->metrics_mutex);
    ++impl_->metrics.events_sent;
  }
  if (out_error != nullptr) {
    out_error->clear();
  }
  return true;
}

bool PairClient::request_keyframe() {
  if (!is_connected()) return false;
  const uint32_t sid = session_id();
  uint32_t last_seq = 0u;
  {
    std::lock_guard<std::mutex> lock(impl_->metrics_mutex);
    last_seq = impl_->metrics.last_applied_seq;
  }
  const auto req = pair_wire::encode_keyframe_request(sid, last_seq, 1u);
  if (!impl_->send_wire_message(req)) return false;
  std::lock_guard<std::mutex> lock(impl_->metrics_mutex);
  ++impl_->metrics.keyframe_resyncs;
  return true;
}

bool PairClient::send_ping() {
  if (!is_connected()) return false;
  const uint32_t sid = session_id();
  const auto ping =
      pair_wire::encode_ping_or_pong(TransportMsgKind::Ping, sid, impl_->now_us());
  return impl_->send_wire_message(ping);
}

} // namespace nve
