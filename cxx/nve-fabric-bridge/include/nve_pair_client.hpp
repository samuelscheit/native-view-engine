#pragma once

#include "nve_abi.generated.h"

#include <array>
#include <atomic>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <functional>
#include <map>
#include <memory>
#include <mutex>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nve {

// =============================================================================
// Wire Protocol Constants (Byte-for-Byte Match with `packages/core/interop`)
// =============================================================================

inline constexpr std::array<uint8_t, 2> kTransportMagic = {'N', 'T'};
inline constexpr uint8_t kTransportProtocolVersion = 1u;
inline constexpr size_t kTransportHeaderSize = 8u;

inline constexpr size_t kMaxUdpDatagramBytes = 1200u;
inline constexpr size_t kFragSubheaderSize = 12u;
inline constexpr size_t kMaxUdpSinglePayload = kMaxUdpDatagramBytes - kTransportHeaderSize;
inline constexpr size_t kMaxUdpFragChunk =
    kMaxUdpDatagramBytes - kTransportHeaderSize - kFragSubheaderSize;

inline constexpr uint16_t kDefaultPairSchemaVersion = 1u;
inline constexpr size_t kDefaultMaxWireFrameBytes = 4u * 1024u * 1024u;
inline constexpr size_t kMaxDecodedFrameBytes = 16u * 1024u * 1024u;
inline constexpr size_t kDeltaHeaderSize = 32u;
inline constexpr size_t kMaxAllowedWireFrameBytes = kMaxDecodedFrameBytes + kDeltaHeaderSize;
inline constexpr size_t kMaxInputEventBytes = 64u * 1024u;

inline constexpr std::array<uint8_t, 4> kSyncDeltaMagic = {'N', 'V', 'D', '1'};
inline constexpr uint8_t kSyncDeltaVersion = 1u;
inline constexpr uint16_t kDeltaFlagRetransmit = 1u << 0;

/// Transport envelope message kind (`NVT1`, matches `sync_transport::TransportMsgKind`).
enum class TransportMsgKind : uint8_t {
  Hello = 1,
  HelloAck = 2,
  Reject = 3,
  DeltaSingle = 4,
  DeltaFrag = 5,
  FragNack = 6,
  Ping = 7,
  Pong = 8,
  KeyframeRequest = 9,
  Bye = 10,
  InputEvents = 11,
};

/// Handshake rejection reason (`sync_transport::RejectReason`).
enum class PairRejectReason : uint8_t {
  InvalidPairingToken = 1,
  SchemaVersionMismatch = 2,
  UnsupportedProtocolVersion = 3,
  ServerBusy = 4,
};

/// `NVD1` delta packet kind (`sync_delta::PacketKind`).
enum class DeltaPacketKind : uint8_t {
  Keyframe = 1,
  XorRleDelta = 2,
  CommandOpDelta = 3,
  Ack = 4,
  Nack = 5,
};

/// RFC 6455 WebSocket opcode (`sync_transport::WsOpcode`).
enum class WsOpcode : uint8_t {
  Continuation = 0x0,
  Text = 0x1,
  Binary = 0x2,
  Close = 0x8,
  Ping = 0x9,
  Pong = 0xA,
};

/// Active transport mode for `nve::PairClient`.
enum class PairTransport : uint8_t {
  Udp = 0,
  Tcp = 1,
  WebSocket = 2,
};

/// Zero-copy view of a parsed `NVT1` transport envelope.
struct TransportEnvelopeView {
  TransportMsgKind kind{TransportMsgKind::Hello};
  uint32_t session_id{0};
  std::span<const uint8_t> payload{};
};

inline constexpr size_t kHelloPayloadWireSize = 32u;
inline constexpr size_t kHelloAckPayloadWireSize = 20u;

/// Decoded `Hello` handshake payload (32 bytes on the wire).
struct HelloPayload {
  uint16_t schema_version{kDefaultPairSchemaVersion};
  uint16_t max_window{64};
  uint32_t client_last_seq{0};
  uint64_t client_nonce{0};
  std::array<uint8_t, 16> token_proof{};
};

/// Decoded `HelloAck` handshake payload (20 bytes on the wire).
struct HelloAckPayload {
  uint16_t schema_version{kDefaultPairSchemaVersion};
  uint16_t server_max_window{64};
  uint32_t server_seq{0};
  uint64_t server_nonce{0};
  bool resync_required{false};
};

/// Decoded fixed 32-byte `NVD1` packet header (wire layout: 4B magic + 28B header fields).
struct DeltaPacketHeader {
  uint8_t version{kSyncDeltaVersion};
  DeltaPacketKind kind{DeltaPacketKind::Keyframe};
  uint16_t flags{0};
  uint32_t seq{0};
  uint32_t base_seq{0};
  uint32_t ack_seq{0};
  uint32_t payload_len{0};
  uint64_t checksum{0};
};

// Compile-time layout & wire-size verification for pair/NVD1 protocol headers.
static_assert(kTransportHeaderSize == 8u, "NVT1 transport header must be 8 bytes");
static_assert(kFragSubheaderSize == 12u, "NVT1 fragment subheader must be 12 bytes");
static_assert(kDeltaHeaderSize == 32u, "NVD1 wire packet header must be 32 bytes");
static_assert(sizeof(HelloPayload) == kHelloPayloadWireSize, "HelloPayload size must be 32 bytes");
static_assert(offsetof(HelloPayload, schema_version) == 0u, "HelloPayload::schema_version offset mismatch");
static_assert(offsetof(HelloPayload, max_window) == 2u, "HelloPayload::max_window offset mismatch");
static_assert(offsetof(HelloPayload, client_last_seq) == 4u, "HelloPayload::client_last_seq offset mismatch");
static_assert(offsetof(HelloPayload, client_nonce) == 8u, "HelloPayload::client_nonce offset mismatch");
static_assert(offsetof(HelloPayload, token_proof) == 16u, "HelloPayload::token_proof offset mismatch");
static_assert(offsetof(HelloAckPayload, schema_version) == 0u, "HelloAckPayload::schema_version offset mismatch");
static_assert(offsetof(HelloAckPayload, server_max_window) == 2u, "HelloAckPayload::server_max_window offset mismatch");
static_assert(offsetof(HelloAckPayload, server_seq) == 4u, "HelloAckPayload::server_seq offset mismatch");
static_assert(offsetof(HelloAckPayload, server_nonce) == 8u, "HelloAckPayload::server_nonce offset mismatch");
static_assert(offsetof(HelloAckPayload, resync_required) == 16u, "HelloAckPayload::resync_required offset mismatch");
static_assert(alignof(uint64_t) != 8u || sizeof(HelloAckPayload) == 24u, "HelloAckPayload size must be 24 bytes on 8-byte-aligned u64 targets");
static_assert(offsetof(DeltaPacketHeader, version) == 0u, "DeltaPacketHeader::version offset mismatch");
static_assert(offsetof(DeltaPacketHeader, kind) == 1u, "DeltaPacketHeader::kind offset mismatch");
static_assert(offsetof(DeltaPacketHeader, flags) == 2u, "DeltaPacketHeader::flags offset mismatch");
static_assert(offsetof(DeltaPacketHeader, seq) == 4u, "DeltaPacketHeader::seq offset mismatch");
static_assert(offsetof(DeltaPacketHeader, base_seq) == 8u, "DeltaPacketHeader::base_seq offset mismatch");
static_assert(offsetof(DeltaPacketHeader, ack_seq) == 12u, "DeltaPacketHeader::ack_seq offset mismatch");
static_assert(offsetof(DeltaPacketHeader, payload_len) == 16u, "DeltaPacketHeader::payload_len offset mismatch");
static_assert(alignof(uint64_t) != 8u || offsetof(DeltaPacketHeader, checksum) == 24u, "DeltaPacketHeader::checksum offset mismatch");
static_assert(alignof(uint64_t) != 8u || sizeof(DeltaPacketHeader) == kDeltaHeaderSize, "DeltaPacketHeader size must be 32 bytes on 8-byte-aligned u64 targets");

/// Zero-copy validated `NVD1` packet view.
struct DeltaPacketView {
  DeltaPacketHeader header{};
  std::span<const uint8_t> payload{};
};

/// Decoded RFC 6455 WebSocket frame.
struct WsFrame {
  bool fin{true};
  WsOpcode opcode{WsOpcode::Binary};
  bool masked{false};
  std::vector<uint8_t> payload;
};

/// Status returned by `SyncDeltaReceiver::receive_packet`.
enum class DeltaReceiveStatus : uint8_t {
  Ok = 0,
  BaseSequenceMismatch = 1,
  InvalidPacket = 2,
};

/// Result of feeding one `NVD1` packet into `SyncDeltaReceiver`.
struct DeltaReceiveOutcome {
  DeltaReceiveStatus status{DeltaReceiveStatus::Ok};
  std::vector<std::pair<uint32_t, std::vector<uint8_t>>> applied_frames;
  std::optional<std::vector<uint8_t>> ack_packet;
  std::vector<std::vector<uint8_t>> nack_packets;
};

// =============================================================================
// Pure Wire-Protocol Helpers (Cross-Platform, Zero External Dependencies)
// =============================================================================

namespace pair_wire {

uint64_t fnv1a_64(std::span<const uint8_t> bytes) noexcept;
uint64_t fnv1a_64_pair(std::span<const uint8_t> a, std::span<const uint8_t> b) noexcept;

std::array<uint8_t, 16> derive_token_proof(
    std::string_view pairing_token,
    uint16_t schema_version) noexcept;

uint32_t derive_session_id(
    std::string_view pairing_token,
    uint16_t schema_version,
    uint64_t client_nonce,
    uint64_t server_nonce) noexcept;

bool ct_eq_16(const std::array<uint8_t, 16>& a, const std::array<uint8_t, 16>& b) noexcept;

std::vector<uint8_t> encode_envelope(
    TransportMsgKind kind,
    uint32_t session_id,
    std::span<const uint8_t> payload);

bool parse_envelope(std::span<const uint8_t> bytes, TransportEnvelopeView& out) noexcept;

std::array<uint8_t, 32> encode_hello_payload(const HelloPayload& hello) noexcept;
bool decode_hello_payload(std::span<const uint8_t> bytes, HelloPayload& out) noexcept;

std::array<uint8_t, 20> encode_hello_ack_payload(const HelloAckPayload& ack) noexcept;
bool decode_hello_ack_payload(std::span<const uint8_t> bytes, HelloAckPayload& out) noexcept;

std::vector<uint8_t> encode_reject_envelope(PairRejectReason reason, uint16_t expected_schema);
bool decode_reject_payload(
    std::span<const uint8_t> payload,
    PairRejectReason& out_reason,
    uint16_t& out_expected_schema) noexcept;

std::vector<uint8_t> encode_ping_or_pong(
    TransportMsgKind kind,
    uint32_t session_id,
    uint64_t timestamp_us);
bool decode_timestamp_us(std::span<const uint8_t> payload, uint64_t& out_ts_us) noexcept;

std::vector<uint8_t> encode_keyframe_request(
    uint32_t session_id,
    uint32_t client_last_seq,
    uint8_t reason);
bool decode_keyframe_request(
    std::span<const uint8_t> payload,
    uint32_t& out_last_seq,
    uint8_t& out_reason) noexcept;

std::vector<uint8_t> encode_frag_nack(
    uint32_t session_id,
    uint32_t frag_msg_id,
    std::span<const uint16_t> missing);
bool decode_frag_nack(
    std::span<const uint8_t> payload,
    uint32_t& out_frag_msg_id,
    std::vector<uint16_t>& out_missing);

std::vector<uint8_t> encode_nvd_packet(
    DeltaPacketKind kind,
    uint16_t flags,
    uint32_t seq,
    uint32_t base_seq,
    uint32_t ack_seq,
    std::span<const uint8_t> payload);

bool parse_nvd_packet(std::span<const uint8_t> bytes, DeltaPacketView& out) noexcept;

std::vector<uint8_t> encode_xor_rle_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next);

bool decode_xor_rle_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> delta_payload,
    std::vector<uint8_t>& out);

std::optional<std::vector<uint8_t>> encode_command_op_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next);

bool decode_command_op_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> delta_payload,
    std::vector<uint8_t>& out);

std::vector<std::vector<uint8_t>> packetize_udp_datagrams(
    uint32_t session_id,
    std::span<const uint8_t> nvd_packet);

std::array<uint8_t, 20> sha1_digest(std::span<const uint8_t> data) noexcept;
std::string base64_encode(std::span<const uint8_t> input);
std::string compute_ws_accept_key(std::string_view client_key);

std::vector<uint8_t> encode_ws_frame(
    bool fin,
    WsOpcode opcode,
    std::span<const uint8_t> payload,
    std::optional<std::array<uint8_t, 4>> mask_key = std::nullopt);

bool decode_ws_frame_slice(
    std::span<const uint8_t> bytes,
    size_t max_payload_len,
    WsFrame& out_frame,
    size_t& out_consumed);

/// Bounded out-of-order UDP `DeltaFrag` reassembler (matching `sync_transport::FragReassembler`).
class FragReassembler {
 public:
  explicit FragReassembler(size_t max_frame_bytes = kDefaultMaxWireFrameBytes);

  void clear() noexcept;

  /// Feed one `DeltaFrag` payload. Returns `false` on corrupt/oversized fragment metadata.
  /// When the full `NVD1` packet completes, `out_completed` is populated.
  bool push(
      std::span<const uint8_t> payload,
      std::chrono::steady_clock::time_point now,
      std::optional<std::vector<uint8_t>>& out_completed);

  std::vector<std::pair<uint32_t, std::vector<uint16_t>>> poll_stale_nacks(
      std::chrono::steady_clock::time_point now,
      uint64_t retry_ms = 15u);

  size_t inflight_count() const noexcept { return inflight_.size(); }

 private:
  struct InFlightFrag {
    uint16_t frag_count{0};
    size_t total_len{0};
    std::vector<uint8_t> buf;
    std::vector<uint8_t> received;
    size_t received_count{0};
    uint16_t highest_idx_seen{0};
    std::chrono::steady_clock::time_point first_seen{};
    std::optional<std::chrono::steady_clock::time_point> last_nack_at;
  };

  size_t max_frame_bytes_{kDefaultMaxWireFrameBytes};
  std::unordered_map<uint32_t, InFlightFrag> inflight_;
  std::deque<uint32_t> order_;
};

/// Stateful `NVD1` receiver reconstructing Keyframe / XorRleDelta / CommandOpDelta in strict
/// sequence order with bounded reorder window, cumulative ACK + 64-bit SACK, and gap NACKs.
class SyncDeltaReceiver {
 public:
  explicit SyncDeltaReceiver(size_t max_window = 64u);

  void reset(size_t max_window = 0u);
  DeltaReceiveOutcome receive_packet(std::span<const uint8_t> packet_bytes);

  std::vector<uint8_t> build_ack_packet() const;
  std::vector<uint8_t> build_nack_packet(uint32_t missing_seq) const;

  uint32_t last_applied_seq() const noexcept { return last_applied_seq_; }
  size_t max_window() const noexcept { return max_window_; }
  size_t reorder_buffer_len() const noexcept { return reorder_buf_.size(); }
  std::optional<uint32_t> next_missing_seq() const noexcept;
  const std::vector<uint8_t>& last_applied_frame() const noexcept { return last_applied_frame_; }

 private:
  DeltaReceiveStatus apply_single_packet(const DeltaPacketView& pkt, std::vector<uint8_t>& out);
  DeltaReceiveStatus drain_reorder_buffer(
      std::vector<std::pair<uint32_t, std::vector<uint8_t>>>& applied);

  size_t max_window_{64u};
  uint32_t last_applied_seq_{0u};
  std::vector<uint8_t> last_applied_frame_;
  std::map<uint32_t, std::vector<uint8_t>> reorder_buf_;
};

} // namespace pair_wire

// =============================================================================
// Pairing Endpoint Config, EventRing Bridge Helper & `nve::PairClient`
// =============================================================================

/// Connection configuration for `nve::PairClient`.
struct PairClientConfig {
  std::string host{"127.0.0.1"};
  uint16_t port{0};
  std::string ws_path{"/"};
  PairTransport transport{PairTransport::Tcp};
  std::string pairing_token;
  uint16_t schema_version{kDefaultPairSchemaVersion};
  uint16_t max_window{64};
  size_t max_frame_bytes{kDefaultMaxWireFrameBytes};
  uint32_t handshake_timeout_ms{2000};
  uint32_t heartbeat_interval_ms{250};
};

/// Parse `--pair-connect <host:port|url>` and optional `--pair-transport <udp|tcp|ws>` into
/// `PairClientConfig`. Preserves `pairing_token`, `schema_version`, and timeouts on `out`.
bool parse_pair_endpoint(
    std::string_view endpoint,
    std::string_view transport_override,
    PairClientConfig& out,
    std::string* out_error = nullptr);

/// Encode a zero-copy `NveEventBatch` into a binary `EventRing` payload suitable for
/// `PairClient::send_events` (`TransportMsgKind::InputEvents = 11`).
bool encode_direct_event_batch(NveEventBatch batch, std::vector<uint8_t>& out_ring);

/// Runtime counters exposed by `PairClient::metrics()`.
struct PairClientMetrics {
  uint32_t session_id{0};
  uint32_t last_applied_seq{0};
  uint64_t frames_received{0};
  uint64_t events_sent{0};
  uint64_t wire_bytes_received{0};
  uint64_t wire_bytes_sent{0};
  uint64_t malformed_packets_dropped{0};
  uint64_t acks_sent{0};
  uint64_t nacks_sent{0};
  uint64_t keyframe_resyncs{0};
};

/// Zero-dependency C++20 `NVT1` / `NVD1` paired thin-client over UDP, framed TCP, or RFC 6455
/// binary WebSocket.
///
/// Runs a background socket reader thread that decodes incoming `NVD1` Keyframe/XorRleDelta/
/// CommandOpDelta packets into reconstructed `NVE1` `CommandBuffer`s, hands them off via a
/// mutex-guarded queue, and invokes `on_frame_ready` to wake the platform UI thread. Native UI
/// input events stream upstream over the same connection as `TransportMsgKind::InputEvents = 11`.
class PairClient {
 public:
  using WakeCallback = std::function<void()>;

  PairClient();
  ~PairClient();

  PairClient(const PairClient&) = delete;
  PairClient& operator=(const PairClient&) = delete;
  PairClient(PairClient&&) = delete;
  PairClient& operator=(PairClient&&) = delete;

  /// Connect, perform optional RFC 6455 WebSocket upgrade and `NVT1` Hello/HelloAck handshake,
  /// and start the background reader thread.
  bool connect(
      const PairClientConfig& config,
      WakeCallback on_frame_ready = {},
      std::string* out_error = nullptr);

  /// Send `Bye`, stop the background reader thread, and close sockets cleanly.
  void disconnect() noexcept;

  bool is_connected() const noexcept;
  uint32_t session_id() const noexcept;
  PairTransport transport() const noexcept;
  PairClientMetrics metrics() const;

  /// Fast lock-free check indicating whether at least one reconstructed `NVE1` frame is queued.
  bool has_pending_frame() const noexcept;

  /// Pop the oldest queued reconstructed `NVE1` `CommandBuffer` in strict sequence order.
  bool pop_frame(std::vector<uint8_t>& out_frame, uint32_t* out_seq = nullptr);

  /// Drain all queued reconstructed `NVE1` frames in strict sequence order in a single O(1) swap.
  size_t drain_pending_frames(std::vector<std::pair<uint32_t, std::vector<uint8_t>>>& out_frames);

  /// Move out the newest queued `NVE1` frame and clear any older queued frames.
  bool take_latest_frame(std::vector<uint8_t>& out_frame, uint32_t* out_seq = nullptr);

  /// Send a raw binary `EventRing` payload upstream as `TransportMsgKind::InputEvents = 11`.
  bool send_events(std::span<const uint8_t> encoded_event_ring, std::string* out_error = nullptr);

  /// Request an immediate keyframe resync from the paired host.
  bool request_keyframe();

  /// Send an `NVT1` heartbeat Ping to the paired host.
  bool send_ping();

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

} // namespace nve
