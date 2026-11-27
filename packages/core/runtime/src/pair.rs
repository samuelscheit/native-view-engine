//! Embedded `nve-wire` thin-client pairing session behind default `feature = "wire"`.
//!
//! Runs on ONE dedicated `current_thread` Tokio runtime thread inside [`nve_wire::ReconnectingWireClient`]
//! and forwards inbound frames/state transitions directly into the surface's [`WakeBridge`].

/// Inbound pairing event delivered in order from the embedded `nve-wire` client.
#[cfg_attr(not(all(feature = "wire", not(target_arch = "wasm32"))), allow(dead_code))]
pub(crate) enum PairInboundEvent {
    SessionStarted { session_id: u64, reconnected: bool },
    Keyframe { session_id: u64, payload: Vec<u8> },
    Delta { session_id: u64, payload: Vec<u8> },
    Overflowed { session_id: u64 },
}

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
pub(crate) const WIRE_PAIRING_SUPPORTED: bool = true;

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
pub(crate) const WIRE_PAIRING_SUPPORTED: bool = false;

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
mod enabled {
    use std::net::Ipv6Addr;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use nve_abi::{direct_event_to_native, encode_events, NveEventBatchRef, NveRuntimeMetrics};
    use nve_wire::{
        parse_wire_uri, NveWireClientConfig, NveWireKind, NveWireTransportKind, NveWireUri,
        ReconnectConfig, ReconnectingWireClient, WireInboundItem, WireResyncState,
    };

    use super::PairInboundEvent;
    use crate::builder::RuntimeError;
    use crate::wake::WakeBridge;

    pub(crate) struct ValidatedPairUri {
        uri: NveWireUri,
    }

    pub(crate) struct PairSession {
        client: ReconnectingWireClient,
        last_rx: u64,
        last_tx: u64,
        resync: WireResyncState,
    }

    fn is_valid_host(host: &str) -> bool {
        if host.is_empty() || host.bytes().any(|b| b.is_ascii_whitespace()) {
            return false;
        }
        let bare = host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(host);
        if bare.is_empty() {
            return false;
        }
        if host.starts_with('[') || host.ends_with(']') || bare.contains(':') {
            return bare.parse::<Ipv6Addr>().is_ok();
        }
        bare.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            && !bare.starts_with('.')
            && !bare.ends_with('.')
            && !bare.starts_with('-')
            && !bare.ends_with('-')
    }

    impl PairSession {
        /// Synchronously validates the URI syntax, host/port presence, TLS certificate pinning,
        /// and Bearer token BEFORE touching live surface state or spawning any worker thread.
        pub(crate) fn validate_and_parse(
            uri: &str,
            token: &str,
        ) -> Result<ValidatedPairUri, RuntimeError> {
            let trimmed_uri = uri.trim();
            if trimmed_uri.is_empty() {
                return Err(RuntimeError::InvalidArg("pairing URI must not be empty"));
            }
            let mut parsed = parse_wire_uri(trimmed_uri)
                .map_err(|_| RuntimeError::InvalidArg("invalid pairing URI"))?;
            if !is_valid_host(&parsed.host) {
                return Err(RuntimeError::InvalidArg("invalid pairing URI host"));
            }
            if parsed.quic_port == Some(0) || parsed.wss_port == Some(0) {
                return Err(RuntimeError::InvalidArg("pairing URI port must be non-zero"));
            }
            if parsed.quic_port.is_none() && parsed.wss_port.is_none() {
                return Err(RuntimeError::InvalidArg(
                    "pairing URI must specify host and port",
                ));
            }
            let requires_cert_pin = match parsed.transport {
                NveWireTransportKind::Quic
                | NveWireTransportKind::WebTransport
                | NveWireTransportKind::Wss
                | NveWireTransportKind::Auto => true,
                NveWireTransportKind::Ws => false,
            };
            if requires_cert_pin && parsed.cert_sha256.is_none() {
                return Err(RuntimeError::InvalidArg(
                    "TLS-pinned pairing transport requires cert_sha256",
                ));
            }
            let trimmed_token = token.trim();
            if !trimmed_token.is_empty() {
                parsed.token = trimmed_token.to_owned();
            }
            if parsed.token.trim().is_empty() {
                return Err(RuntimeError::InvalidArg("missing pairing token"));
            }
            Ok(ValidatedPairUri { uri: parsed })
        }

        pub(crate) fn spawn_parsed(
            validated: ValidatedPairUri,
            wake: Arc<WakeBridge>,
            connected_flag: Arc<AtomicBool>,
        ) -> Result<Self, RuntimeError> {
            let mut wire_cfg = NveWireClientConfig::new(validated.uri.token.clone(), "nve-runtime");
            wire_cfg.cert_sha256 = validated.uri.cert_sha256;

            let client = ReconnectingWireClient::spawn_with_connected_flag(
                validated.uri,
                wire_cfg,
                ReconnectConfig::default(),
                Some(Arc::new(move || wake.trigger())),
                connected_flag,
            )
            .map_err(|e| RuntimeError::Io(e.to_string()))?;

            Ok(Self {
                client,
                last_rx: 0,
                last_tx: 0,
                resync: WireResyncState::new(),
            })
        }

        #[inline]
        pub(crate) fn has_pending_frame(&self) -> bool {
            self.client.has_pending_item()
        }

        #[inline]
        pub(crate) fn on_session_started(
            &mut self,
            session_id: u64,
            reconnected: bool,
            has_prior_state: bool,
        ) {
            self.resync
                .on_session_started(session_id, reconnected, has_prior_state);
        }

        #[inline]
        pub(crate) fn on_overflowed(&mut self, session_id: u64) {
            let client = &self.client;
            self.resync
                .on_overflowed(session_id, || client.request_keyframe().is_ok());
        }

        #[inline]
        pub(crate) fn on_decode_error(&mut self, session_id: u64) {
            let client = &self.client;
            self.resync
                .on_decode_error(session_id, || client.request_keyframe().is_ok());
        }

        #[inline]
        pub(crate) fn should_apply(&self, is_keyframe: bool, is_frame1: bool) -> bool {
            self.resync.should_apply(is_keyframe, is_frame1)
        }

        #[inline]
        pub(crate) fn mark_keyframe_applied(&mut self) {
            self.resync.mark_keyframe_applied();
        }

        pub(crate) fn try_recv_event(&self) -> Option<PairInboundEvent> {
            while let Some(item) = self.client.try_recv_item() {
                match item {
                    WireInboundItem::SessionStarted {
                        session_id,
                        reconnected,
                    } => {
                        return Some(PairInboundEvent::SessionStarted {
                            session_id,
                            reconnected,
                        });
                    }
                    WireInboundItem::Overflowed { session_id } => {
                        return Some(PairInboundEvent::Overflowed { session_id });
                    }
                    WireInboundItem::Frame { session_id, frame } => match frame.kind {
                        NveWireKind::SceneKeyframe => {
                            return Some(PairInboundEvent::Keyframe {
                                session_id,
                                payload: frame.payload,
                            });
                        }
                        NveWireKind::CommandDelta => {
                            return Some(PairInboundEvent::Delta {
                                session_id,
                                payload: frame.payload,
                            });
                        }
                        _ => continue,
                    },
                }
            }
            None
        }

        pub(crate) fn send_events(&self, events: NveEventBatchRef<'_>) -> bool {
            if events.is_empty() || !self.client.is_connected() {
                return false;
            }
            let owned: Vec<_> = events.iter().filter_map(direct_event_to_native).collect();
            if owned.is_empty() {
                return false;
            }
            self.client.send_events(encode_events(&owned)).is_ok()
        }

        pub(crate) fn sync_metrics(&mut self, dst: &mut NveRuntimeMetrics) {
            let m = self.client.metrics();
            let drx = m.wire_bytes_rx.saturating_sub(self.last_rx);
            let dtx = m.wire_bytes_tx.saturating_sub(self.last_tx);
            self.last_rx = m.wire_bytes_rx;
            self.last_tx = m.wire_bytes_tx;
            dst.wire_bytes_rx = dst.wire_bytes_rx.saturating_add(drx);
            dst.wire_bytes_tx = dst.wire_bytes_tx.saturating_add(dtx);
            dst.pair_connected = u32::from(self.client.is_connected());
        }

        pub(crate) fn overlay_live_metrics(&self, dst: &mut NveRuntimeMetrics) {
            let m = self.client.metrics();
            let drx = m.wire_bytes_rx.saturating_sub(self.last_rx);
            let dtx = m.wire_bytes_tx.saturating_sub(self.last_tx);
            dst.wire_bytes_rx = dst.wire_bytes_rx.saturating_add(drx);
            dst.wire_bytes_tx = dst.wire_bytes_tx.saturating_add(dtx);
            dst.pair_connected = u32::from(self.client.is_connected());
        }

        pub(crate) fn shutdown(&mut self) {
            self.client.shutdown();
        }
    }
}

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
mod disabled {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use nve_abi::{NveEventBatchRef, NveRuntimeMetrics};

    use super::PairInboundEvent;
    use crate::builder::RuntimeError;
    use crate::wake::WakeBridge;

    pub(crate) struct ValidatedPairUri;

    pub(crate) struct PairSession;

    impl PairSession {
        pub(crate) fn validate_and_parse(
            _uri: &str,
            _token: &str,
        ) -> Result<ValidatedPairUri, RuntimeError> {
            Err(RuntimeError::Unsupported(
                "nve-runtime built without wire pairing support",
            ))
        }

        pub(crate) fn spawn_parsed(
            _validated: ValidatedPairUri,
            _wake: Arc<WakeBridge>,
            _connected_flag: Arc<AtomicBool>,
        ) -> Result<Self, RuntimeError> {
            Err(RuntimeError::Unsupported(
                "nve-runtime built without wire pairing support",
            ))
        }

        pub(crate) fn has_pending_frame(&self) -> bool {
            false
        }

        pub(crate) fn on_session_started(
            &mut self,
            _session_id: u64,
            _reconnected: bool,
            _has_prior_state: bool,
        ) {
        }

        pub(crate) fn on_overflowed(&mut self, _session_id: u64) {}

        pub(crate) fn on_decode_error(&mut self, _session_id: u64) {}

        pub(crate) fn should_apply(&self, _is_keyframe: bool, _is_frame1: bool) -> bool {
            true
        }

        pub(crate) fn mark_keyframe_applied(&mut self) {}

        pub(crate) fn try_recv_event(&self) -> Option<PairInboundEvent> {
            None
        }

        pub(crate) fn send_events(&self, _events: NveEventBatchRef<'_>) -> bool {
            false
        }

        pub(crate) fn sync_metrics(&mut self, dst: &mut NveRuntimeMetrics) {
            dst.pair_connected = 0;
        }

        pub(crate) fn overlay_live_metrics(&self, dst: &mut NveRuntimeMetrics) {
            dst.pair_connected = 0;
        }

        pub(crate) fn shutdown(&mut self) {}
    }
}

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
pub(crate) use enabled::PairSession;

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
pub(crate) use disabled::PairSession;
