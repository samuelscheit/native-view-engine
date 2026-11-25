#ifndef NVE_PAIR_ABI_H
#define NVE_PAIR_ABI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
#include <cstddef>
#include <cstdint>
#define NVE_PAIR_STATIC_ASSERT(cond, msg) static_assert(cond, msg)
extern "C" {
#else
#define NVE_PAIR_STATIC_ASSERT(cond, msg) _Static_assert(cond, msg)
#endif

/**
 * Shared C11/C++20 C ABI for NVE live pairing (`NVT1` transport + `NVD1` delta sync).
 *
 * Zero-JSON, zero-copy frame views, explicit caller-sized outbound buffers, and
 * compile-time layout assertions locked against `packages/modules/pair/src/c_api.rs`.
 */

enum {
  NVE_PAIR_ABI_VERSION = 1u,
  NVE_PAIR_TRANSPORT_HEADER_SIZE = 8u,
  NVE_PAIR_DELTA_HEADER_SIZE = 32u,
  NVE_PAIR_DEFAULT_SCHEMA_VERSION = 1u,
  NVE_PAIR_DEFAULT_MAX_WINDOW = 64u,
  NVE_PAIR_MSG_HELLO = 1u,
  NVE_PAIR_MSG_HELLO_ACK = 2u,
  NVE_PAIR_MSG_REJECT = 3u,
  NVE_PAIR_MSG_DELTA_SINGLE = 4u,
  NVE_PAIR_MSG_DELTA_FRAG = 5u,
  NVE_PAIR_MSG_FRAG_NACK = 6u,
  NVE_PAIR_MSG_PING = 7u,
  NVE_PAIR_MSG_PONG = 8u,
  NVE_PAIR_MSG_KEYFRAME_REQUEST = 9u,
  NVE_PAIR_MSG_BYE = 10u,
  NVE_PAIR_MSG_INPUT_EVENTS = 11u
};

/** Status codes returned by `nve_pair_*` C-ABI functions (`0` = success, negative = error). */
typedef enum NvePairStatus {
  NVE_PAIR_OK = 0,
  NVE_PAIR_ERR_NULL_ARG = -1,
  NVE_PAIR_ERR_INVALID_UTF8 = -2,
  NVE_PAIR_ERR_INVALID_URI = -3,
  NVE_PAIR_ERR_TRUNCATED = -4,
  NVE_PAIR_ERR_INVALID_MAGIC = -5,
  NVE_PAIR_ERR_UNSUPPORTED_VERSION = -6,
  NVE_PAIR_ERR_INVALID_KIND = -7,
  NVE_PAIR_ERR_CHECKSUM = -8,
  NVE_PAIR_ERR_BASE_SEQ_MISMATCH = -9,
  NVE_PAIR_ERR_AUTH_FAILED = -10,
  NVE_PAIR_ERR_SCHEMA_MISMATCH = -11,
  NVE_PAIR_ERR_SESSION_MISMATCH = -12,
  NVE_PAIR_ERR_NOT_PAIRED = -13,
  NVE_PAIR_ERR_BUFFER_TOO_SMALL = -14,
  NVE_PAIR_ERR_FRAME_TOO_LARGE = -15,
  NVE_PAIR_ERR_MALFORMED = -16,
  NVE_PAIR_ERR_HANDSHAKE_REJECTED = -17,
  NVE_PAIR_ERR_EMPTY_QUEUE = -18,
  NVE_PAIR_ERR_INTERNAL_PANIC = -99
} NvePairStatus;

/** Pairing client lifecycle state. */
typedef enum NvePairState {
  NVE_PAIR_STATE_DISCONNECTED = 0u,
  NVE_PAIR_STATE_CONNECTING = 1u,
  NVE_PAIR_STATE_PAIRED = 2u,
  NVE_PAIR_STATE_REJECTED = 3u,
  NVE_PAIR_STATE_ERROR = 4u
} NvePairState;

/** Parsed URI transport scheme. */
typedef enum NvePairTransportKind {
  NVE_PAIR_TRANSPORT_WS = 0u,
  NVE_PAIR_TRANSPORT_WSS = 1u,
  NVE_PAIR_TRANSPORT_TCP = 2u,
  NVE_PAIR_TRANSPORT_UDP = 3u
} NvePairTransportKind;

/** Event kind matching `nve_abi::NativeEventKind`. */
typedef enum NvePairEventKind {
  NVE_PAIR_EVENT_PRESS = 1u,
  NVE_PAIR_EVENT_LONG_PRESS = 2u,
  NVE_PAIR_EVENT_SCROLL = 3u,
  NVE_PAIR_EVENT_TEXT_CHANGE = 4u,
  NVE_PAIR_EVENT_FOCUS = 5u,
  NVE_PAIR_EVENT_BLUR = 6u,
  NVE_PAIR_EVENT_LAYOUT = 7u,
  NVE_PAIR_EVENT_CUSTOM = 8u
} NvePairEventKind;

/** Opaque handle owning a Rust `PairClientEngine` (`SyncDeltaSession` + `NVT1` state machine). */
typedef struct NvePairClient NvePairClient;

/** Zero-copy borrowed byte slice valid until the next mutating call on the owning client. */
typedef struct NvePairByteSlice {
  const uint8_t* ptr;
  size_t len;
} NvePairByteSlice;

/** Configuration passed to `nve_pair_client_create`. Zero numeric fields use defaults. */
typedef struct NvePairClientConfig {
  const uint8_t* token_ptr;
  size_t token_len;
  uint16_t schema_version;
  uint16_t max_window;
  uint32_t max_frame_bytes;
} NvePairClientConfig;

/** Flat zero-JSON input event encoded into `NVT1` `InputEvents = 11` (`EventRing` payload). */
typedef struct NvePairInputEvent {
  uint32_t surface_id;
  uint32_t node_id;
  uint64_t timestamp_us;
  uint8_t kind;
  uint8_t reserved[7];
  float x;
  float y;
  float width;
  float height;
  const uint8_t* text_ptr;
  size_t text_len;
  const uint8_t* data_ptr;
  size_t data_len;
} NvePairInputEvent;

/** Result summary populated by `nve_pair_client_ingest_packet`. */
typedef struct NvePairIngestOutcome {
  uint32_t applied_frame_count;
  uint32_t latest_seq;
  uint32_t outbound_packet_count;
  uint32_t state;
} NvePairIngestOutcome;

/** Fixed-layout runtime metrics snapshot populated by `nve_pair_client_metrics`. */
typedef struct NvePairMetrics {
  uint32_t session_id;
  uint32_t last_applied_seq;
  uint32_t state;
  uint32_t reorder_buffer_len;
  uint64_t frames_received;
  uint64_t keyframes_received;
  uint64_t deltas_received;
  uint64_t events_sent;
  uint64_t wire_bytes_received;
  uint64_t raw_bytes_received;
  uint64_t wire_bytes_sent;
  uint64_t malformed_packets_dropped;
  uint64_t acks_sent;
  uint64_t nacks_sent;
  uint64_t keyframe_resyncs;
  uint64_t last_pong_timestamp_us;
} NvePairMetrics;

/** Caller-provided fixed buffers for `nve_pair_parse_uri`. */
typedef struct NvePairParsedUri {
  uint8_t transport;
  uint8_t has_udp_port;
  uint16_t port;
  uint16_t udp_port;
  uint16_t schema_version;
  float width;
  float height;
  uint8_t host_buf[256];
  size_t host_len;
  uint8_t path_buf[256];
  size_t path_len;
  uint8_t token_buf[256];
  size_t token_len;
} NvePairParsedUri;

/* Compile-time ABI layout invariants matching `packages/modules/pair/src/c_api.rs`. */
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairByteSlice, ptr) == 0u, "NvePairByteSlice.ptr offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairByteSlice, len) == sizeof(void*), "NvePairByteSlice.len offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairIngestOutcome, applied_frame_count) == 0u, "NvePairIngestOutcome.applied_frame_count offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairIngestOutcome, latest_seq) == 4u, "NvePairIngestOutcome.latest_seq offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairIngestOutcome, outbound_packet_count) == 8u, "NvePairIngestOutcome.outbound_packet_count offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairIngestOutcome, state) == 12u, "NvePairIngestOutcome.state offset");
NVE_PAIR_STATIC_ASSERT(sizeof(NvePairIngestOutcome) == 16u, "NvePairIngestOutcome size");

NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, session_id) == 0u, "NvePairMetrics.session_id offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, last_applied_seq) == 4u, "NvePairMetrics.last_applied_seq offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, state) == 8u, "NvePairMetrics.state offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, reorder_buffer_len) == 12u, "NvePairMetrics.reorder_buffer_len offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, frames_received) == 16u, "NvePairMetrics.frames_received offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, keyframes_received) == 24u, "NvePairMetrics.keyframes_received offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, deltas_received) == 32u, "NvePairMetrics.deltas_received offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, events_sent) == 40u, "NvePairMetrics.events_sent offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, wire_bytes_received) == 48u, "NvePairMetrics.wire_bytes_received offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, raw_bytes_received) == 56u, "NvePairMetrics.raw_bytes_received offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, wire_bytes_sent) == 64u, "NvePairMetrics.wire_bytes_sent offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, malformed_packets_dropped) == 72u, "NvePairMetrics.malformed_packets_dropped offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, acks_sent) == 80u, "NvePairMetrics.acks_sent offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, nacks_sent) == 88u, "NvePairMetrics.nacks_sent offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, keyframe_resyncs) == 96u, "NvePairMetrics.keyframe_resyncs offset");
NVE_PAIR_STATIC_ASSERT(offsetof(NvePairMetrics, last_pong_timestamp_us) == 104u, "NvePairMetrics.last_pong_timestamp_us offset");
NVE_PAIR_STATIC_ASSERT(sizeof(NvePairMetrics) == 112u, "NvePairMetrics size");

/**
 * Parse a pairing URI (`nve://pair?...`, `ws://`, `wss://`, `tcp://`, `udp://`, or `host:port`)
 * into `out_uri` without allocating across the FFI boundary.
 */
int32_t nve_pair_parse_uri(
    const uint8_t* uri_ptr,
    size_t uri_len,
    const uint8_t* fallback_token_ptr,
    size_t fallback_token_len,
    NvePairParsedUri* out_uri);

/** Create a new pairing client from `config`. Returns NULL on invalid config/UTF-8. */
NvePairClient* nve_pair_client_create(const NvePairClientConfig* config);

/** Create a new pairing client directly from a pairing URI and optional fallback token. */
NvePairClient* nve_pair_client_create_from_uri(
    const uint8_t* uri_ptr,
    size_t uri_len,
    const uint8_t* fallback_token_ptr,
    size_t fallback_token_len);

/** Destroy a client previously returned by `nve_pair_client_create*`. Safe on NULL. */
void nve_pair_client_destroy(NvePairClient* client);

/**
 * Begin a pairing handshake by encoding an `NVT1` `Hello` envelope into `out_buf`.
 * Always writes the required byte length into `*out_len`.
 */
int32_t nve_pair_client_begin_handshake(
    NvePairClient* client,
    uint64_t client_nonce,
    uint8_t* out_buf,
    size_t out_cap,
    size_t* out_len);

/**
 * Ingest one incoming `NVT1` transport envelope (or standalone `NVD1` delta packet).
 * Reconstructs keyframes/XOR-RLE/CommandOp deltas in strict sequence order and queues
 * any outbound `Ack` / `Nack` / `Pong` / `KeyframeRequest` packets for retrieval via
 * `nve_pair_client_pop_outbound_packet`.
 */
int32_t nve_pair_client_ingest_packet(
    NvePairClient* client,
    const uint8_t* packet_ptr,
    size_t packet_len,
    NvePairIngestOutcome* out_outcome);

/**
 * Pop the next queued outbound packet produced by `nve_pair_client_ingest_packet`.
 * Returns `NVE_PAIR_ERR_EMPTY_QUEUE` when no outbound packets remain.
 * If `out_cap` is too small (or `out_buf == NULL`), writes required size to `*out_len`,
 * keeps the packet at the front of the queue, and returns `NVE_PAIR_ERR_BUFFER_TOO_SMALL`.
 */
int32_t nve_pair_client_pop_outbound_packet(
    NvePairClient* client,
    uint8_t* out_buf,
    size_t out_cap,
    size_t* out_len);

/**
 * Encode one native UI input event as an `NVT1` `InputEvents = 11` envelope carrying a
 * binary `EventRing` payload. Writes required byte length to `*out_len`.
 */
int32_t nve_pair_client_encode_input_event(
    NvePairClient* client,
    const NvePairInputEvent* event,
    uint8_t* out_buf,
    size_t out_cap,
    size_t* out_len);

/**
 * Return a zero-copy slice pointing at the latest reconstructed `NVE1` frame buffer.
 * Returns `{NULL, 0}` if `client` is NULL or no frame has been applied yet.
 */
NvePairByteSlice nve_pair_client_current_frame(const NvePairClient* client);

/** Copy the current metrics snapshot into `*out_metrics`. */
int32_t nve_pair_client_metrics(
    const NvePairClient* client,
    NvePairMetrics* out_metrics);

#ifdef __cplusplus
}
#endif

#endif /* NVE_PAIR_ABI_H */
