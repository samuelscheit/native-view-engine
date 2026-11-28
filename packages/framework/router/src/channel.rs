//! Bidirectional [`RouterChannelBridge`] synchronizing `nve-router` with
//! `nve-interop::NveChannelBus` (`CHANNEL_ID_NAVIGATION = 4`) via binary `NVN1` packets.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nve_interop::{
    ChannelMessage, ChannelPayload, InteropError, NveChannelBus, PayloadEncoding, SubscriptionId,
    SurfaceId, CHANNEL_ID_NAVIGATION,
};

use crate::deeplink::DeepLinkConfig;
pub use crate::nvn1::{
    decode_nav_packet, decode_nav_packet_ref, encode_nav_packet, encode_nav_packet_into,
    NavPacket, NavPacketRef, Nvn1DecodeError, Nvn1EncodeError, NVN1_FLAG_CAN_GO_BACK,
    NVN1_FLAG_CAN_GO_FORWARD, NVN1_FLAG_IS_MODAL, NVN1_MAGIC, NVN1_MIN_PACKET_BYTES, NVN1_VERSION,
};
use crate::route::Routable;
use crate::stack::{NavigationAction, NavigationError, NavigationTransition, RouterState};
use crate::tabs::TabRouterState;

/// Error returned by [`RouterChannelBridge`] publish/attach operations.
#[derive(Debug)]
pub enum BridgeError {
    Encode(Nvn1EncodeError),
    Interop(InteropError),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(e) => write!(f, "NVN1 encode error: {e}"),
            Self::Interop(e) => write!(f, "NveChannelBus error: {e}"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl From<Nvn1EncodeError> for BridgeError {
    fn from(value: Nvn1EncodeError) -> Self {
        Self::Encode(value)
    }
}

impl From<InteropError> for BridgeError {
    fn from(value: InteropError) -> Self {
        Self::Interop(value)
    }
}

/// Decoded inbound external navigation command received via `CHANNEL_ID_NAVIGATION`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundNavCommand {
    pub seq: u64,
    pub sender_surface: Option<SurfaceId>,
    pub packet: NavPacket,
}

/// Outcome of applying one [`InboundNavCommand`] to a router state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundApplyOutcome<R> {
    Applied(NavigationTransition<R>),
    IgnoredSnapshot,
    UnmatchedUri(String),
    Rejected(NavigationError),
}

struct BridgeInbox {
    surface_id: SurfaceId,
    last_seq: AtomicU64,
    queue: Mutex<VecDeque<InboundNavCommand>>,
    decode_errors: AtomicU64,
}

impl BridgeInbox {
    fn on_message(&self, msg: &ChannelMessage) {
        if msg.sender_surface == Some(self.surface_id) {
            return;
        }
        if msg.encoding != PayloadEncoding::Binary {
            return;
        }
        let mut prev = self.last_seq.load(Ordering::Acquire);
        loop {
            if msg.seq <= prev {
                return;
            }
            match self.last_seq.compare_exchange_weak(
                prev,
                msg.seq,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => prev = actual,
            }
        }

        match decode_nav_packet(msg.as_bytes()) {
            Ok(packet) => {
                if let Ok(mut q) = self.queue.lock() {
                    q.push_back(InboundNavCommand {
                        seq: msg.seq,
                        sender_surface: msg.sender_surface,
                        packet,
                    });
                }
            }
            Err(_) => {
                self.decode_errors.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Bidirectional bridge between `nve-router` state machines and `nve-interop::NveChannelBus` on
/// `CHANNEL_ID_NAVIGATION (4)` (`TOPIC_NAVIGATION`).
///
/// Publishes every committed transition with `retain = true` and consumes external navigation
/// commands with echo + monotonic sequence loop prevention.
pub struct RouterChannelBridge {
    surface_id: SurfaceId,
    subscription_id: Option<SubscriptionId>,
    inbox: Arc<BridgeInbox>,
    published_count: u64,
}

impl RouterChannelBridge {
    /// Subscribes to `CHANNEL_ID_NAVIGATION` on `bus` for `surface_id`.
    pub fn attach(
        bus: &mut NveChannelBus,
        surface_id: SurfaceId,
        replay_retained: bool,
    ) -> Result<Self, InteropError> {
        let inbox = Arc::new(BridgeInbox {
            surface_id,
            last_seq: AtomicU64::new(0),
            queue: Mutex::new(VecDeque::new()),
            decode_errors: AtomicU64::new(0),
        });
        let inbox_cb = Arc::clone(&inbox);
        let sub_id = bus.subscribe_by_id(
            CHANNEL_ID_NAVIGATION,
            Some(surface_id),
            replay_retained,
            move |msg| inbox_cb.on_message(msg),
        )?;
        Ok(Self {
            surface_id,
            subscription_id: Some(sub_id),
            inbox,
            published_count: 0,
        })
    }

    /// Unsubscribes this bridge from `bus`.
    pub fn detach(&mut self, bus: &mut NveChannelBus) -> Result<(), InteropError> {
        if let Some(id) = self.subscription_id.take() {
            bus.unsubscribe(id)?;
        }
        Ok(())
    }

    #[inline]
    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    #[inline]
    pub fn subscription_id(&self) -> Option<SubscriptionId> {
        self.subscription_id
    }

    #[inline]
    pub fn last_inbound_seq(&self) -> u64 {
        self.inbox.last_seq.load(Ordering::Acquire)
    }

    #[inline]
    pub fn decode_error_count(&self) -> u64 {
        self.inbox.decode_errors.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn published_count(&self) -> u64 {
        self.published_count
    }

    /// Encodes `transition` as `NVN1` and publishes it on `CHANNEL_ID_NAVIGATION` with `retain = true`.
    pub fn publish_transition<R: Routable>(
        &mut self,
        bus: &mut NveChannelBus,
        transition: &NavigationTransition<R>,
    ) -> Result<usize, BridgeError> {
        let packet = NavPacket::from_transition(transition);
        self.publish_packet(bus, &packet)
    }

    /// Encodes `packet` as `NVN1` and publishes it on `CHANNEL_ID_NAVIGATION` with `retain = true`.
    pub fn publish_packet(
        &mut self,
        bus: &mut NveChannelBus,
        packet: &NavPacket,
    ) -> Result<usize, BridgeError> {
        let bytes = packet.encode()?;
        let delivered = bus.publish_by_id(
            CHANNEL_ID_NAVIGATION,
            Some(self.surface_id),
            PayloadEncoding::Binary,
            ChannelPayload::from_vec(bytes),
            true,
        )?;
        self.published_count = self.published_count.saturating_add(1);
        Ok(delivered)
    }

    /// Drains all buffered inbound external navigation commands in FIFO sequence order.
    pub fn drain_inbound(&self) -> Vec<InboundNavCommand> {
        let Ok(mut q) = self.inbox.queue.lock() else {
            return Vec::new();
        };
        q.drain(..).collect()
    }

    /// Drains and applies all queued inbound commands to a [`RouterState`] without republishing.
    pub fn apply_inbound_to_router<R: Routable>(
        &self,
        router: &mut RouterState<R>,
        deeplink: Option<&DeepLinkConfig>,
    ) -> Vec<InboundApplyOutcome<R>> {
        self.drain_inbound()
            .into_iter()
            .map(|cmd| apply_command_to_router(router, &cmd.packet, deeplink))
            .collect()
    }

    /// Drains and applies all queued inbound commands to a [`TabRouterState`] without republishing.
    pub fn apply_inbound_to_tabs<T: Copy + Eq + Ord + 'static, R: Routable>(
        &self,
        tabs: &mut TabRouterState<T, R>,
        deeplink: Option<&DeepLinkConfig>,
    ) -> Vec<InboundApplyOutcome<R>> {
        self.drain_inbound()
            .into_iter()
            .map(|cmd| apply_command_to_tabs(tabs, &cmd.packet, deeplink))
            .collect()
    }
}

fn resolve_packet_route<R: Routable>(uri: &str, deeplink: Option<&DeepLinkConfig>) -> Option<R> {
    if let Some(cfg) = deeplink {
        cfg.match_uri::<R>(uri).ok().map(|m| m.route)
    } else {
        R::parse_uri(uri)
    }
}

trait NavCommandTarget<R: Routable> {
    fn nav_pop(&mut self) -> Result<NavigationTransition<R>, NavigationError>;
    fn nav_pop_to_root(&mut self) -> Result<NavigationTransition<R>, NavigationError>;
    fn nav_dismiss_modal(&mut self) -> Result<NavigationTransition<R>, NavigationError>;
    fn nav_push(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError>;
    fn nav_replace(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError>;
    fn nav_deep_link(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError>;
    fn nav_open_modal(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError>;
    fn nav_switch_tab(
        &mut self,
        packet: &NavPacket,
        deeplink: Option<&DeepLinkConfig>,
    ) -> InboundApplyOutcome<R>;
}

fn transition_outcome<R>(res: Result<NavigationTransition<R>, NavigationError>) -> InboundApplyOutcome<R> {
    match res {
        Ok(tr) => InboundApplyOutcome::Applied(tr),
        Err(e) => InboundApplyOutcome::Rejected(e),
    }
}

impl<R: Routable> NavCommandTarget<R> for RouterState<R> {
    fn nav_pop(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.pop()
    }
    fn nav_pop_to_root(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.pop_to_root()
    }
    fn nav_dismiss_modal(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.dismiss_modal()
    }
    fn nav_push(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.push(route)
    }
    fn nav_replace(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.replace(route)
    }
    fn nav_deep_link(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.apply_deep_link(route)
    }
    fn nav_open_modal(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.open_modal(route)
    }
    fn nav_switch_tab(
        &mut self,
        packet: &NavPacket,
        deeplink: Option<&DeepLinkConfig>,
    ) -> InboundApplyOutcome<R> {
        let Some(route) = resolve_packet_route::<R>(&packet.uri, deeplink) else {
            return InboundApplyOutcome::UnmatchedUri(packet.uri.clone());
        };
        transition_outcome(self.push(route))
    }
}

impl<T: Copy + Eq + Ord + 'static, R: Routable> NavCommandTarget<R> for TabRouterState<T, R> {
    fn nav_pop(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.pop()
    }
    fn nav_pop_to_root(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.pop_to_root()
    }
    fn nav_dismiss_modal(&mut self) -> Result<NavigationTransition<R>, NavigationError> {
        self.dismiss_modal()
    }
    fn nav_push(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.push(route)
    }
    fn nav_replace(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.replace(route)
    }
    fn nav_deep_link(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.apply_deep_link(route)
    }
    fn nav_open_modal(&mut self, route: R) -> Result<NavigationTransition<R>, NavigationError> {
        self.open_modal(route)
    }
    fn nav_switch_tab(
        &mut self,
        packet: &NavPacket,
        _deeplink: Option<&DeepLinkConfig>,
    ) -> InboundApplyOutcome<R> {
        let res = if !packet.tab_id.is_empty() {
            self.switch_tab_by_id(&packet.tab_id)
        } else {
            self.switch_tab_by_index(packet.active_tab_index as usize)
        };
        transition_outcome(res)
    }
}

fn apply_command<R: Routable>(
    target: &mut impl NavCommandTarget<R>,
    packet: &NavPacket,
    deeplink: Option<&DeepLinkConfig>,
) -> InboundApplyOutcome<R> {
    match packet.action {
        NavigationAction::Snapshot => InboundApplyOutcome::IgnoredSnapshot,
        NavigationAction::SwitchTab => target.nav_switch_tab(packet, deeplink),
        NavigationAction::Pop => transition_outcome(target.nav_pop()),
        NavigationAction::PopToRoot => transition_outcome(target.nav_pop_to_root()),
        NavigationAction::DismissModal => transition_outcome(target.nav_dismiss_modal()),
        NavigationAction::Push
        | NavigationAction::Replace
        | NavigationAction::DeepLink
        | NavigationAction::OpenModal => {
            let Some(route) = resolve_packet_route::<R>(&packet.uri, deeplink) else {
                return InboundApplyOutcome::UnmatchedUri(packet.uri.clone());
            };
            let res = match packet.action {
                NavigationAction::Push => target.nav_push(route),
                NavigationAction::Replace => target.nav_replace(route),
                NavigationAction::DeepLink => target.nav_deep_link(route),
                NavigationAction::OpenModal => target.nav_open_modal(route),
                _ => unreachable!(),
            };
            transition_outcome(res)
        }
    }
}

fn apply_command_to_router<R: Routable>(
    router: &mut RouterState<R>,
    packet: &NavPacket,
    deeplink: Option<&DeepLinkConfig>,
) -> InboundApplyOutcome<R> {
    apply_command(router, packet, deeplink)
}

fn apply_command_to_tabs<T: Copy + Eq + Ord + 'static, R: Routable>(
    tabs: &mut TabRouterState<T, R>,
    packet: &NavPacket,
    deeplink: Option<&DeepLinkConfig>,
) -> InboundApplyOutcome<R> {
    apply_command(tabs, packet, deeplink)
}
