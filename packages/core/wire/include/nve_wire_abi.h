#ifndef NVE_WIRE_ABI_H
#define NVE_WIRE_ABI_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Opaque handle to a connected `nve-wire` client (QUIC / WebTransport / WSS Happy-Eyeballs). */
typedef struct NveWireClientHandle NveWireClientHandle;

/** Transport kind selected or negotiated by `nve-wire`. */
typedef enum NveWireTransportKindC {
  NVE_WIRE_TRANSPORT_AUTO = 0,
  NVE_WIRE_TRANSPORT_QUIC = 1,
  NVE_WIRE_TRANSPORT_WEBTRANSPORT = 2,
  NVE_WIRE_TRANSPORT_WSS = 3,
  NVE_WIRE_TRANSPORT_WS = 4,
} NveWireTransportKindC;

/** Wire frame kind discriminator matching `nve_wire_core::NveWireKind`. */
typedef enum NveWireKindC {
  NVE_WIRE_KIND_HELLO = 1,
  NVE_WIRE_KIND_HELLO_ACK = 2,
  NVE_WIRE_KIND_REJECT = 3,
  NVE_WIRE_KIND_SCENE_KEYFRAME = 4,
  NVE_WIRE_KIND_COMMAND_DELTA = 5,
  NVE_WIRE_KIND_NODE_PATCH_DELTA = 6,
  NVE_WIRE_KIND_SLOT_CHUNK = 7,
  NVE_WIRE_KIND_EVENT_BATCH = 8,
  NVE_WIRE_KIND_CRDT_DELTA = 9,
  NVE_WIRE_KIND_PRESENCE = 10,
  NVE_WIRE_KIND_HOT_PATCH = 11,
  NVE_WIRE_KIND_PING = 12,
  NVE_WIRE_KIND_PONG = 13,
  NVE_WIRE_KIND_KEYFRAME_REQUEST = 14,
  NVE_WIRE_KIND_BYE = 15,
} NveWireKindC;

/** Error codes returned by `nve_wire_*` C ABI functions (`0` = OK). */
typedef enum NveWireStatusC {
  NVE_WIRE_OK = 0,
  NVE_WIRE_ERR_INVALID_ARG = 1,
  NVE_WIRE_ERR_PARSE = 2,
  NVE_WIRE_ERR_CONNECT = 3,
  NVE_WIRE_ERR_AUTH = 4,
  NVE_WIRE_ERR_IO = 5,
  NVE_WIRE_ERR_EMPTY = 6,
  NVE_WIRE_ERR_BUFFER_TOO_SMALL = 7,
  NVE_WIRE_ERR_PANIC = 255,
} NveWireStatusC;

/** Client telemetry snapshot exposed over C ABI. */
typedef struct NveWireMetricsC {
  uint32_t session_id;
  uint32_t transport_kind;
  uint64_t frames_received;
  uint64_t keyframes_received;
  uint64_t command_deltas_received;
  uint64_t node_patch_deltas_received;
  uint64_t events_sent;
  uint64_t wire_bytes_rx;
  uint64_t wire_bytes_tx;
} NveWireMetricsC;

/** Connects a native `nve-wire` client using a pairing URI (`nve://pair?...`, `quic://`, `wss://`, `ws://`). */
NveWireStatusC nve_wire_client_connect_uri(
    const char *uri_utf8,
    const char *token_override_utf8,
    const char *client_name_utf8,
    uint32_t fallback_delay_ms,
    NveWireClientHandle **out_client);

/** Polls the next inbound frame without blocking. Returns `NVE_WIRE_ERR_EMPTY` if none queued. */
NveWireStatusC nve_wire_client_poll_frame(
    NveWireClientHandle *client,
    uint8_t *out_kind,
    uint32_t *out_sequence,
    uint8_t *out_payload_buf,
    size_t payload_buf_cap,
    size_t *out_payload_len);

/** Sends one raw `EventRing` (`NveEventBatch`) payload upstream on the `InputEvents` channel. */
NveWireStatusC nve_wire_client_send_events(
    NveWireClientHandle *client,
    const uint8_t *event_bytes,
    size_t event_len);

/** Sends one unreliable presence/cursor datagram (RFC 9221 on QUIC/WebTransport, stream on WSS). */
NveWireStatusC nve_wire_client_send_datagram(
    NveWireClientHandle *client,
    uint8_t kind,
    const uint8_t *payload,
    size_t payload_len);

/** Sends a `KeyframeRequest` control frame. */
NveWireStatusC nve_wire_client_request_keyframe(NveWireClientHandle *client);

/** Copies current telemetry counters into `*out_metrics`. */
NveWireStatusC nve_wire_client_metrics(
    const NveWireClientHandle *client,
    NveWireMetricsC *out_metrics);

/** Returns `true` while the client transport is connected. */
bool nve_wire_client_is_connected(const NveWireClientHandle *client);

/** Gracefully disconnects and frees `client`. Safe to call with `NULL`. */
void nve_wire_client_destroy(NveWireClientHandle *client);

#ifdef __cplusplus
}
#endif

#endif /* NVE_WIRE_ABI_H */
