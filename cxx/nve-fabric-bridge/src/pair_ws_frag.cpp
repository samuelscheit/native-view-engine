#include "pair_internal.hpp"

#include <algorithm>
#include <array>
#include <cstring>
#include <limits>
#include <string>
#include <utility>
#include <vector>

namespace nve {

namespace pair_detail {

bool parse_ws_frame_header(
    std::span<const uint8_t> bytes,
    size_t max_payload_len,
    WsFrameHeaderInfo& out) noexcept {
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
  std::array<uint8_t, 4> mask_key{};
  if (masked) {
    if (bytes.size() < pos + 4u) return false;
    std::memcpy(mask_key.data(), bytes.data() + pos, 4u);
    pos += 4u;
  }
  if (bytes.size() - pos < payload_len) {
    return false;
  }
  out.fin = fin;
  out.opcode = opcode;
  out.masked = masked;
  out.payload_offset = pos;
  out.payload_len = payload_len;
  out.mask_key = mask_key;
  return true;
}

std::string build_ws_upgrade_request(
    const PairClientConfig& config,
    std::string& out_expected_accept) {
  std::array<uint8_t, 16> key_raw{};
  write_u64_le(key_raw.data(), next_nonce(0xABCD1234ULL));
  write_u64_le(key_raw.data() + 8u, next_nonce(0x5678EF01ULL));
  const std::string client_key = pair_wire::base64_encode(key_raw);
  out_expected_accept = pair_wire::compute_ws_accept_key(client_key);
  const std::string host_hdr = config.host + ":" + std::to_string(config.port);
  return "GET " + config.ws_path +
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
}

bool verify_ws_upgrade_response(
    std::string_view resp,
    std::string_view expected_accept,
    std::string* out_error) {
  auto fail = [&](std::string msg) {
    if (out_error != nullptr) *out_error = std::move(msg);
    return false;
  };
  const size_t first_eol = resp.find("\r\n");
  const std::string_view status_line =
      (first_eol != std::string_view::npos) ? resp.substr(0u, first_eol) : resp;
  if (status_line.find("101") == std::string_view::npos) {
    return fail("WebSocket server did not reply with HTTP 101 Switching Protocols");
  }
  size_t line_start = (first_eol != std::string_view::npos) ? (first_eol + 2u) : resp.size();
  while (line_start < resp.size()) {
    const size_t line_end = resp.find("\r\n", line_start);
    const std::string_view line = (line_end != std::string_view::npos)
                                      ? resp.substr(line_start, line_end - line_start)
                                      : resp.substr(line_start);
    if (line.empty()) break;
    const size_t colon = line.find(':');
    if (colon != std::string_view::npos) {
      const std::string_view name = trim_ascii(line.substr(0u, colon));
      const std::string_view val = trim_ascii(line.substr(colon + 1u));
      if (ascii_iequals(name, "sec-websocket-accept") && val == expected_accept) {
        return true;
      }
    }
    if (line_end == std::string_view::npos) break;
    line_start = line_end + 2u;
  }
  return fail("Invalid or missing Sec-WebSocket-Accept header");
}

} // namespace pair_detail

namespace pair_wire {

using namespace pair_detail;

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
    dgram.insert(
        dgram.end(),
        nvd_packet.begin() + static_cast<ptrdiff_t>(start),
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
    const size_t dst_off = out.size();
    out.resize(dst_off + len);
    uint8_t* dst = out.data() + dst_off;
    for (size_t i = 0u; i < len; ++i) {
      dst[i] = static_cast<uint8_t>(payload[i] ^ mask[i & 3u]);
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
  WsFrameHeaderInfo hdr{};
  if (!parse_ws_frame_header(bytes, max_payload_len, hdr)) {
    return false;
  }
  out_frame.fin = hdr.fin;
  out_frame.opcode = hdr.opcode;
  out_frame.masked = hdr.masked;
  if (hdr.payload_len == 0u) {
    out_frame.payload.clear();
  } else {
    const uint8_t* src = bytes.data() + hdr.payload_offset;
    out_frame.payload.resize(hdr.payload_len);
    uint8_t* dst = out_frame.payload.data();
    if (hdr.masked) {
      const auto& mask = hdr.mask_key;
      for (size_t i = 0u; i < hdr.payload_len; ++i) {
        dst[i] = static_cast<uint8_t>(src[i] ^ mask[i & 3u]);
      }
    } else {
      std::memcpy(dst, src, hdr.payload_len);
    }
  }
  out_consumed = hdr.payload_offset + hdr.payload_len;
  return true;
}

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
  return reorder_buf_.empty() ? std::nullopt
                              : std::optional<uint32_t>(static_cast<uint32_t>(last_applied_seq_ + 1u));
}

std::vector<uint8_t> SyncDeltaReceiver::build_ack_packet() const {
  uint64_t sack = 0u;
  for (const auto& [buffered_seq, _] : reorder_buf_) {
    if (buffered_seq > last_applied_seq_) {
      const uint32_t diff = buffered_seq - last_applied_seq_;
      if (diff >= 1u && diff <= 64u) sack |= (1ULL << (diff - 1u));
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
      if (pkt.payload.size() > kMaxDecodedFrameBytes) return DeltaReceiveStatus::InvalidPacket;
      out.assign(pkt.payload.begin(), pkt.payload.end());
      return DeltaReceiveStatus::Ok;
    case DeltaPacketKind::XorRleDelta:
      if (pkt.header.base_seq != last_applied_seq_) return DeltaReceiveStatus::BaseSequenceMismatch;
      return decode_xor_rle_delta(last_applied_frame_, pkt.payload, out)
                 ? DeltaReceiveStatus::Ok
                 : DeltaReceiveStatus::InvalidPacket;
    case DeltaPacketKind::CommandOpDelta:
      if (pkt.header.base_seq != last_applied_seq_) return DeltaReceiveStatus::BaseSequenceMismatch;
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
    if (it == reorder_buf_.end()) break;
    const std::vector<uint8_t> raw = std::move(it->second);
    reorder_buf_.erase(it);
    DeltaPacketView pkt{};
    if (!parse_nvd_packet(raw, pkt)) return DeltaReceiveStatus::InvalidPacket;
    std::vector<uint8_t> frame;
    const DeltaReceiveStatus st = apply_single_packet(pkt, frame);
    if (st != DeltaReceiveStatus::Ok) return st;
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
      it = (it->first <= last_applied_seq_) ? reorder_buf_.erase(it) : std::next(it);
    }
    if (const auto st = drain_reorder_buffer(out.applied_frames); st != DeltaReceiveStatus::Ok) {
      out.status = st;
      return out;
    }
  } else if (hdr.seq == expected_next) {
    std::vector<uint8_t> frame;
    if (const auto st = apply_single_packet(parsed, frame); st != DeltaReceiveStatus::Ok) {
      out.status = st;
      return out;
    }
    last_applied_seq_ = hdr.seq;
    last_applied_frame_ = frame;
    out.applied_frames.emplace_back(hdr.seq, std::move(frame));
    if (const auto st = drain_reorder_buffer(out.applied_frames); st != DeltaReceiveStatus::Ok) {
      out.status = st;
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

} // namespace nve
