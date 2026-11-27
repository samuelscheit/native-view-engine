//! Embedded `nve-wire` thin-client pairing session behind default `feature = "wire"`.
//!
//! Runs on ONE dedicated `current_thread` Tokio runtime thread inside [`nve_wire::ReconnectingWireClient`]
//! and forwards inbound frames/state transitions directly into the surface's [`WakeBridge`].

/// Inbound scene payload decoded from a paired `nve-wire` server.
#[cfg_attr(not(all(feature = "wire", not(target_arch = "wasm32"))), allow(dead_code))]
pub(crate) enum PairInboundFrame {
    Keyframe(Vec<u8>),
    Delta(Vec<u8>),
}

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
pub(crate) const WIRE_PAIRING_SUPPORTED: bool = true;

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
pub(crate) const WIRE_PAIRING_SUPPORTED: bool = false;

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
mod enabled {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use nve_abi::{direct_event_to_native, encode_events, NveEventBatchRef, NveRuntimeMetrics};
    use nve_wire::{
        parse_wire_uri, NveWireClientConfig, NveWireKind, ReconnectConfig, ReconnectingWireClient,
    };

    use super::PairInboundFrame;
    use crate::builder::RuntimeError;
    use crate::wake::WakeBridge;

    pub(crate) struct PairSession {
        client: ReconnectingWireClient,
        last_rx: u64,
        last_tx: u64,
    }

    impl PairSession {
        pub(crate) fn connect(
            uri: &str,
            token: &str,
            wake: Arc<WakeBridge>,
        ) -> Result<Self, RuntimeError> {
            let trimmed_uri = uri.trim();
            if trimmed_uri.is_empty() {
                return Err(RuntimeError::InvalidArg("pairing URI must not be empty"));
            }
            let mut parsed = parse_wire_uri(trimmed_uri)
                .map_err(|_| RuntimeError::InvalidArg("invalid pairing URI"))?;
            let trimmed_token = token.trim();
            if !trimmed_token.is_empty() {
                parsed.token = trimmed_token.to_owned();
            }
            if parsed.token.trim().is_empty() {
                return Err(RuntimeError::InvalidArg("missing pairing token"));
            }

            let mut wire_cfg = NveWireClientConfig::new(parsed.token.clone(), "nve-runtime");
            wire_cfg.cert_sha256 = parsed.cert_sha256;

            let client = ReconnectingWireClient::spawn(
                parsed,
                wire_cfg,
                ReconnectConfig::default(),
                Some(Arc::new(move || wake.trigger())),
            )
            .map_err(|e| RuntimeError::Io(e.to_string()))?;

            Ok(Self {
                client,
                last_rx: 0,
                last_tx: 0,
            })
        }

        #[inline]
        pub(crate) fn connected_flag(&self) -> Arc<AtomicBool> {
            self.client.connected_flag()
        }

        #[inline]
        pub(crate) fn has_pending_frame(&self) -> bool {
            self.client.has_pending_frame()
        }

        pub(crate) fn try_recv_scene_frame(&self) -> Option<PairInboundFrame> {
            while let Some(frame) = self.client.try_recv_frame() {
                match frame.kind {
                    NveWireKind::SceneKeyframe => {
                        return Some(PairInboundFrame::Keyframe(frame.payload));
                    }
                    NveWireKind::CommandDelta => {
                        return Some(PairInboundFrame::Delta(frame.payload));
                    }
                    _ => continue,
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
            self.client.send_events(&encode_events(&owned)).is_ok()
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

        pub(crate) fn shutdown(&mut self) {
            self.client.shutdown();
        }
    }
}

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
mod disabled {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use nve_abi::{NveEventBatchRef, NveRuntimeMetrics};

    use super::PairInboundFrame;
    use crate::builder::RuntimeError;
    use crate::wake::WakeBridge;

    pub(crate) struct PairSession;

    impl PairSession {
        pub(crate) fn connect(
            _uri: &str,
            _token: &str,
            _wake: Arc<WakeBridge>,
        ) -> Result<Self, RuntimeError> {
            Err(RuntimeError::Unsupported(
                "nve-runtime built without wire pairing support",
            ))
        }

        pub(crate) fn connected_flag(&self) -> Arc<AtomicBool> {
            Arc::new(AtomicBool::new(false))
        }

        pub(crate) fn has_pending_frame(&self) -> bool {
            false
        }

        pub(crate) fn try_recv_scene_frame(&self) -> Option<PairInboundFrame> {
            None
        }

        pub(crate) fn send_events(&self, _events: NveEventBatchRef<'_>) -> bool {
            false
        }

        pub(crate) fn sync_metrics(&mut self, dst: &mut NveRuntimeMetrics) {
            dst.pair_connected = 0;
        }

        pub(crate) fn shutdown(&mut self) {}
    }
}

#[cfg(all(feature = "wire", not(target_arch = "wasm32")))]
pub(crate) use enabled::PairSession;

#[cfg(not(all(feature = "wire", not(target_arch = "wasm32"))))]
pub(crate) use disabled::PairSession;
