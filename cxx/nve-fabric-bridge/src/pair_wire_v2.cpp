#include "nve_pair_client.hpp"

#include <algorithm>
#include <cstring>

namespace nve {
namespace {

inline uint16_t read_u16_le(const uint8_t* p) noexcept {
  return static_cast<uint16_t>(p[0]) | (static_cast<uint16_t>(p[1]) << 8);
}

inline uint32_t read_u32_le(const uint8_t* p) noexcept {
  return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8) |
         (static_cast<uint32_t>(p[2]) << 16) | (static_cast<uint32_t>(p[3]) << 24);
}

inline uint64_t read_u64_le(const uint8_t* p) noexcept {
  return static_cast<uint64_t>(read_u32_le(p)) |
         (static_cast<uint64_t>(read_u32_le(p + 4)) << 32);
}

inline void write_u16_le(uint8_t* p, uint16_t v) noexcept {
  p[0] = static_cast<uint8_t>(v & 0xFFu);
  p[1] = static_cast<uint8_t>((v >> 8) & 0xFFu);
}

inline void write_u32_le(uint8_t* p, uint32_t v) noexcept {
  p[0] = static_cast<uint8_t>(v & 0xFFu);
  p[1] = static_cast<uint8_t>((v >> 8) & 0xFFu);
  p[2] = static_cast<uint8_t>((v >> 16) & 0xFFu);
  p[3] = static_cast<uint8_t>((v >> 24) & 0xFFu);
}

inline void write_u64_le(uint8_t* p, uint64_t v) noexcept {
  write_u32_le(p, static_cast<uint32_t>(v & 0xFFFFFFFFULL));
  write_u32_le(p + 4, static_cast<uint32_t>((v >> 32) & 0xFFFFFFFFULL));
}

inline uint64_t rotl64(uint64_t x, unsigned r) noexcept { return (x << r) | (x >> (64u - r)); }

} // namespace

// =============================================================================
// wire_v2 Codec Implementation
// =============================================================================

namespace wire_v2 {

uint64_t xxh64(std::span<const uint8_t> data, uint64_t seed) noexcept {
  constexpr uint64_t P1 = 0x9E3779B185EBCA87ULL;
  constexpr uint64_t P2 = 0xC2B2AE3D27D4EB4FULL;
  constexpr uint64_t P3 = 0x165667B19E3779F9ULL;
  constexpr uint64_t P4 = 0x85EBCA77C2B2AE63ULL;
  constexpr uint64_t P5 = 0x27D4EB2F165667C5ULL;

  const uint8_t* p = data.data();
  const size_t len = data.size();
  const uint8_t* const end = p + len;
  uint64_t h64 = 0;

  auto round = [](uint64_t acc, uint64_t input) noexcept {
    acc += input * P2;
    acc = rotl64(acc, 31);
    acc *= P1;
    return acc;
  };
  auto merge_round = [&](uint64_t acc, uint64_t val) noexcept {
    acc ^= round(0, val);
    return acc * P1 + P4;
  };

  if (len >= 32u) {
    const uint8_t* const limit = end - 32u;
    uint64_t v1 = seed + P1 + P2;
    uint64_t v2 = seed + P2;
    uint64_t v3 = seed;
    uint64_t v4 = seed - P1;
    do {
      v1 = round(v1, read_u64_le(p)); p += 8;
      v2 = round(v2, read_u64_le(p)); p += 8;
      v3 = round(v3, read_u64_le(p)); p += 8;
      v4 = round(v4, read_u64_le(p)); p += 8;
    } while (p <= limit);
    h64 = rotl64(v1, 1) + rotl64(v2, 7) + rotl64(v3, 12) + rotl64(v4, 18);
    h64 = merge_round(h64, v1);
    h64 = merge_round(h64, v2);
    h64 = merge_round(h64, v3);
    h64 = merge_round(h64, v4);
  } else {
    h64 = seed + P5;
  }

  h64 += static_cast<uint64_t>(len);
  while (p + 8u <= end) {
    h64 ^= round(0, read_u64_le(p));
    h64 = rotl64(h64, 27) * P1 + P4;
    p += 8;
  }
  if (p + 4u <= end) {
    h64 ^= static_cast<uint64_t>(read_u32_le(p)) * P1;
    h64 = rotl64(h64, 23) * P2 + P3;
    p += 4;
  }
  while (p < end) {
    h64 ^= static_cast<uint64_t>(*p++) * P5;
    h64 = rotl64(h64, 11) * P1;
  }

  h64 ^= h64 >> 33;
  h64 *= P2;
  h64 ^= h64 >> 29;
  h64 *= P3;
  h64 ^= h64 >> 32;
  return h64;
}

bool constant_time_eq(std::string_view a, std::string_view b) noexcept {
  const size_t min_len = std::min(a.size(), b.size());
  uint8_t diff = 0;
  for (size_t i = 0; i < min_len; ++i) {
    diff |= static_cast<uint8_t>(a[i] ^ b[i]);
  }
  return a.size() == b.size() && diff == 0u;
}

std::vector<uint8_t> encode_frame(
    WireChannel channel,
    WireKind kind,
    uint32_t sequence,
    std::span<const uint8_t> payload,
    uint8_t flags) {
  const bool has_ck = (flags & kWireFlagXxh64) != 0u;
  const size_t total = kWireHeaderSize + payload.size() + (has_ck ? kWireTrailerSize : 0u);
  std::vector<uint8_t> out(total, 0u);
  out[0] = kWireMagic[0];
  out[1] = kWireMagic[1];
  out[2] = kWireVersion;
  out[3] = static_cast<uint8_t>(channel);
  out[4] = static_cast<uint8_t>(kind);
  out[5] = flags;
  write_u16_le(out.data() + 6, 0u);
  write_u32_le(out.data() + 8, sequence);
  write_u32_le(out.data() + 12, static_cast<uint32_t>(payload.size()));
  if (!payload.empty()) {
    std::memcpy(out.data() + kWireHeaderSize, payload.data(), payload.size());
  }
  if (has_ck) {
    const size_t body_len = kWireHeaderSize + payload.size();
    const uint64_t digest = xxh64(std::span<const uint8_t>(out.data(), body_len));
    write_u64_le(out.data() + body_len, digest);
  }
  return out;
}

bool decode_header(std::span<const uint8_t> bytes, WireHeader& out, size_t max_frame_bytes) noexcept {
  if (bytes.size() < kWireHeaderSize) return false;
  if (bytes[0] != kWireMagic[0] || bytes[1] != kWireMagic[1]) return false;
  if (bytes[2] != kWireVersion) return false;
  if (bytes[3] > static_cast<uint8_t>(WireChannel::Presence)) return false;
  if (bytes[4] < static_cast<uint8_t>(WireKind::Hello) ||
      bytes[4] > static_cast<uint8_t>(WireKind::Bye)) {
    return false;
  }
  if (read_u16_le(bytes.data() + 6) != 0u) return false;
  const uint32_t payload_len = read_u32_le(bytes.data() + 12);
  const size_t cap = std::min(max_frame_bytes, kMaxWireFrameBytes);
  if (static_cast<size_t>(payload_len) > cap) return false;

  out.channel = static_cast<WireChannel>(bytes[3]);
  out.kind = static_cast<WireKind>(bytes[4]);
  out.flags = bytes[5];
  out.sequence = read_u32_le(bytes.data() + 8);
  out.payload_len = payload_len;
  return true;
}

bool decode_frame(std::span<const uint8_t> bytes, WireFrameView& out, size_t max_frame_bytes) noexcept {
  WireHeader hdr{};
  if (!decode_header(bytes, hdr, max_frame_bytes)) return false;
  if (bytes.size() != hdr.total_wire_len()) return false;
  const size_t payload_end = kWireHeaderSize + static_cast<size_t>(hdr.payload_len);
  if (hdr.has_xxh64()) {
    const uint64_t expected = read_u64_le(bytes.data() + payload_end);
    const uint64_t actual = xxh64(bytes.subspan(0, payload_end));
    if (actual != expected) return false;
  }
  out.header = hdr;
  out.payload = bytes.subspan(kWireHeaderSize, hdr.payload_len);
  return true;
}

std::vector<uint8_t> encode_hello(const WireHello& hello) {
  const uint16_t tok_len = static_cast<uint16_t>(std::min<size_t>(hello.bearer_token.size(), 512u));
  const uint16_t name_len = static_cast<uint16_t>(std::min<size_t>(hello.client_name.size(), 128u));
  std::vector<uint8_t> out(10u + tok_len + name_len, 0u);
  write_u16_le(out.data(), hello.schema_version);
  write_u32_le(out.data() + 2, hello.capabilities);
  write_u16_le(out.data() + 6, tok_len);
  write_u16_le(out.data() + 8, name_len);
  if (tok_len > 0u) std::memcpy(out.data() + 10, hello.bearer_token.data(), tok_len);
  if (name_len > 0u) std::memcpy(out.data() + 10 + tok_len, hello.client_name.data(), name_len);
  return out;
}

bool decode_hello(std::span<const uint8_t> payload, WireHello& out) {
  if (payload.size() < 10u) return false;
  const uint16_t tok_len = read_u16_le(payload.data() + 6);
  const uint16_t name_len = read_u16_le(payload.data() + 8);
  if (tok_len > 512u || name_len > 128u || payload.size() != 10u + tok_len + name_len) return false;
  out.schema_version = read_u16_le(payload.data());
  out.capabilities = read_u32_le(payload.data() + 2);
  out.bearer_token.assign(reinterpret_cast<const char*>(payload.data() + 10), tok_len);
  out.client_name.assign(reinterpret_cast<const char*>(payload.data() + 10 + tok_len), name_len);
  return true;
}

std::array<uint8_t, 12> encode_hello_ack(const WireHelloAck& ack) noexcept {
  std::array<uint8_t, 12> out{};
  write_u32_le(out.data(), ack.session_id);
  write_u16_le(out.data() + 4, ack.schema_version);
  write_u16_le(out.data() + 6, 0u);
  write_u32_le(out.data() + 8, ack.server_capabilities);
  return out;
}

bool decode_hello_ack(std::span<const uint8_t> payload, WireHelloAck& out) noexcept {
  if (payload.size() != 12u) return false;
  out.session_id = read_u32_le(payload.data());
  out.schema_version = read_u16_le(payload.data() + 4);
  out.server_capabilities = read_u32_le(payload.data() + 8);
  return true;
}

std::vector<uint8_t> encode_reject(WireRejectReason reason, std::string_view detail) {
  const uint16_t len = static_cast<uint16_t>(std::min<size_t>(detail.size(), 256u));
  std::vector<uint8_t> out(4u + len, 0u);
  out[0] = static_cast<uint8_t>(reason);
  out[1] = 0u;
  write_u16_le(out.data() + 2, len);
  if (len > 0u) std::memcpy(out.data() + 4, detail.data(), len);
  return out;
}

bool decode_reject(
    std::span<const uint8_t> payload,
    WireRejectReason& out_reason,
    std::string& out_detail) {
  if (payload.size() < 4u) return false;
  const uint16_t len = read_u16_le(payload.data() + 2);
  if (payload.size() != 4u + len) return false;
  out_reason = static_cast<WireRejectReason>(payload[0]);
  out_detail.assign(reinterpret_cast<const char*>(payload.data() + 4), len);
  return true;
}

} // namespace wire_v2
} // namespace nve
