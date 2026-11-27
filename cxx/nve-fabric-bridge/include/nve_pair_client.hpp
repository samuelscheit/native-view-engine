#pragma once

#include "nve_abi.generated.h"
#include "nve_hash.generated.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace nve {

// =============================================================================
// NveWireFrame v2 Constants & Types (16-byte LE header + optional XXH64 trailer)
// =============================================================================

inline constexpr std::array<uint8_t, 2> kWireMagic{'N', 'W'};
inline constexpr uint8_t kWireVersion = 2u;
inline constexpr size_t kWireHeaderSize = 16u;
inline constexpr size_t kWireTrailerSize = 8u;
inline constexpr uint64_t kWireXxh64Seed = 0x4E56455749524532ULL; // "NVEWIRE2"
inline constexpr uint16_t kDefaultWireSchemaVersion = 2u;
inline constexpr size_t kDefaultMaxWireFrameBytes = 16u * 1024u * 1024u;
inline constexpr size_t kMaxWireFrameBytes = 64u * 1024u * 1024u;
inline constexpr uint8_t kWireFlagXxh64 = 1u << 0;
inline constexpr uint8_t kWireFlagDatagram = 1u << 1;
inline constexpr uint8_t kWireFlagFinalChunk = 1u << 2;

enum class WireChannel : uint8_t {
  Control = 0,
  UiScene = 1,
  AsyncSlot = 2,
  InputEvents = 3,
  StateSync = 4,
  Presence = 5,
};

enum class WireKind : uint8_t {
  Hello = 1,
  HelloAck = 2,
  Reject = 3,
  SceneKeyframe = 4,
  CommandDelta = 5,
  NodePatchDelta = 6,
  SlotChunk = 7,
  EventBatch = 8,
  CrdtDelta = 9,
  Presence = 10,
  HotPatch = 11,
  Ping = 12,
  Pong = 13,
  KeyframeRequest = 14,
  Bye = 15,
};

enum class WireRejectReason : uint8_t {
  Unauthorized = 1,
  SchemaVersionMismatch = 2,
  UnsupportedVersion = 3,
  ServerBusy = 4,
  FrameTooLarge = 5,
};

enum class PairTransport : uint8_t {
  Auto = 0,
  Quic = 1,
  WebTransport = 2,
  Wss = 3,
  Ws = 4,
  Tcp = 5,
};

struct WireHeader {
  WireChannel channel{WireChannel::Control};
  WireKind kind{WireKind::Hello};
  uint8_t flags{kWireFlagXxh64};
  uint32_t sequence{0};
  uint32_t payload_len{0};

  [[nodiscard]] bool has_xxh64() const noexcept { return (flags & kWireFlagXxh64) != 0u; }
  [[nodiscard]] size_t total_wire_len() const noexcept {
    return kWireHeaderSize + static_cast<size_t>(payload_len) +
           (has_xxh64() ? kWireTrailerSize : 0u);
  }
};

struct WireFrameView {
  WireHeader header{};
  std::span<const uint8_t> payload{};
};

struct WireHello {
  uint16_t schema_version{kDefaultWireSchemaVersion};
  uint32_t capabilities{0x0Fu};
  std::string bearer_token;
  std::string client_name{"nve-cxx-host"};
};

struct WireHelloAck {
  uint32_t session_id{0};
  uint16_t schema_version{kDefaultWireSchemaVersion};
  uint32_t server_capabilities{0};
};

namespace wire_v2 {

/// Constant-time byte comparison for Bearer tokens.
bool constant_time_eq(std::string_view a, std::string_view b) noexcept;

/// Encodes one `NveWireFrame` v2 packet (16B LE header + payload + optional 8B XXH64 trailer).
std::vector<uint8_t> encode_frame(
    WireChannel channel,
    WireKind kind,
    uint32_t sequence,
    std::span<const uint8_t> payload,
    uint8_t flags = kWireFlagXxh64);

/// Parses and validates a 16-byte `NveWireFrame` header before any payload allocation.
bool decode_header(
    std::span<const uint8_t> bytes,
    WireHeader& out,
    size_t max_frame_bytes = kDefaultMaxWireFrameBytes) noexcept;

/// Validates a complete `NveWireFrame` v2 packet including XXH64 trailer if flagged.
bool decode_frame(
    std::span<const uint8_t> bytes,
    WireFrameView& out,
    size_t max_frame_bytes = kDefaultMaxWireFrameBytes) noexcept;

std::vector<uint8_t> encode_hello(const WireHello& hello);
bool decode_hello(std::span<const uint8_t> payload, WireHello& out);

std::array<uint8_t, 12> encode_hello_ack(const WireHelloAck& ack) noexcept;
bool decode_hello_ack(std::span<const uint8_t> payload, WireHelloAck& out) noexcept;

std::vector<uint8_t> encode_reject(WireRejectReason reason, std::string_view detail);
bool decode_reject(
    std::span<const uint8_t> payload,
    WireRejectReason& out_reason,
    std::string& out_detail);

} // namespace wire_v2

// =============================================================================
// Pairing Endpoint Config, EventRing Bridge Helper & Thin `nve::PairClient`
// =============================================================================

struct PairClientConfig {
  std::string host{"127.0.0.1"};
  uint16_t port{0};
  std::string ws_path{"/nve-wire"};
  PairTransport transport{PairTransport::Auto};
  std::string pairing_token;
  std::string cert_hash_hex;
  uint16_t schema_version{kDefaultWireSchemaVersion};
  size_t max_frame_bytes{kDefaultMaxWireFrameBytes};
  uint32_t handshake_timeout_ms{2000};
};

/// Parses `--pair-connect <uri|host:port>` and optional `--pair-transport` into `PairClientConfig`.
bool parse_pair_endpoint(
    std::string_view endpoint,
    std::string_view transport_override,
    PairClientConfig& out,
    std::string* out_error = nullptr);

/// Encodes a zero-copy `NveEventBatch` into a binary `EventRing` payload for `EventBatch`.
bool encode_direct_event_batch(NveEventBatch batch, std::vector<uint8_t>& out_ring);

struct PairClientMetrics {
  uint32_t session_id{0};
  uint32_t last_applied_seq{0};
  uint64_t frames_received{0};
  uint64_t events_sent{0};
  uint64_t wire_bytes_received{0};
  uint64_t wire_bytes_sent{0};
  uint64_t malformed_packets_dropped{0};
};

/// Thin C++20 `NveWireFrame` v2 paired client adapter (zero legacy NVT1/NVD1/UDP-frag/XOR-RLE).
class PairClient {
 public:
  using WakeCallback = std::function<void()>;

  PairClient();
  ~PairClient();

  PairClient(const PairClient&) = delete;
  PairClient& operator=(const PairClient&) = delete;
  PairClient(PairClient&&) = delete;
  PairClient& operator=(PairClient&&) = delete;

  bool connect(
      const PairClientConfig& config,
      WakeCallback on_frame_ready = {},
      std::string* out_error = nullptr);

  void disconnect() noexcept;

  bool is_connected() const noexcept;
  uint32_t session_id() const noexcept;
  PairTransport transport() const noexcept;
  PairClientMetrics metrics() const;

  bool has_pending_frame() const noexcept;
  bool pop_frame(std::vector<uint8_t>& out_frame, uint32_t* out_seq = nullptr);
  size_t drain_pending_frames(std::vector<std::pair<uint32_t, std::vector<uint8_t>>>& out_frames);
  bool take_latest_frame(std::vector<uint8_t>& out_frame, uint32_t* out_seq = nullptr);

  bool send_events(std::span<const uint8_t> encoded_event_ring, std::string* out_error = nullptr);
  bool request_keyframe();
  bool send_ping();

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

} // namespace nve
