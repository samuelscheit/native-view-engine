use std::collections::BTreeMap;
use std::sync::Arc;

use nve_abi::SurfaceId;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::error::InteropError;

/// C-friendly numeric channel identifier avoiding repeated string hashing across FFI boundaries.
pub type ChannelId = u32;

/// Unique identifier for an active subscription on [`NveChannelBus`].
pub type SubscriptionId = u64;

/// Built-in well-known cross-framework topics and stable numeric channel IDs.
pub const TOPIC_THEME: &str = "theme";
pub const TOPIC_LOCALE: &str = "locale";
pub const TOPIC_AUTH: &str = "auth";
pub const TOPIC_NAVIGATION: &str = "navigation";

pub const CHANNEL_ID_THEME: ChannelId = 1;
pub const CHANNEL_ID_LOCALE: ChannelId = 2;
pub const CHANNEL_ID_AUTH: ChannelId = 3;
pub const CHANNEL_ID_NAVIGATION: ChannelId = 4;
const FIRST_CUSTOM_CHANNEL_ID: ChannelId = 100;

/// Wire encoding of a [`ChannelMessage`] payload.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadEncoding {
    Binary = 0,
    Json = 1,
}

/// Zero-copy shared payload bytes delivered to every subscriber on a topic without per-subscriber clones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPayload(Arc<[u8]>);

impl ChannelPayload {
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self(bytes.into())
    }

    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self(Arc::from(bytes.into_boxed_slice()))
    }

    pub fn from_slice(bytes: &[u8]) -> Self {
        Self(Arc::from(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Raw pointer equality check useful for asserting zero-copy fan-out across subscribers.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Message published on [`NveChannelBus`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMessage {
    pub channel_id: ChannelId,
    pub topic: String,
    pub seq: u64,
    pub sender_surface: Option<SurfaceId>,
    pub encoding: PayloadEncoding,
    pub payload: ChannelPayload,
}

impl ChannelMessage {
    pub fn as_bytes(&self) -> &[u8] {
        self.payload.as_bytes()
    }

    pub fn decode_json<T: DeserializeOwned>(&self) -> Result<T, InteropError> {
        serde_json::from_slice(self.payload.as_bytes())
            .map_err(|e| InteropError::Serialization(e.to_string()))
    }
}

type SubscriberCallback = Arc<dyn Fn(&ChannelMessage) + Send + Sync + 'static>;

struct SubscriptionEntry {
    id: SubscriptionId,
    owner_surface: Option<SurfaceId>,
    callback: SubscriberCallback,
}

struct ChannelState {
    id: ChannelId,
    topic: String,
    seq: u64,
    retained: Option<ChannelMessage>,
    subscribers: Vec<SubscriptionEntry>,
}

/// Typed binary & JSON pub/sub and shared-state channel registry.
///
/// Replaces O(N^2) ad-hoc framework bridges with a single O(N) bus shared across
/// Rust, React Native, Flutter, SwiftUI, Jetpack Compose, and Web WASM.
pub struct NveChannelBus {
    next_custom_id: ChannelId,
    next_sub_id: SubscriptionId,
    topic_to_id: BTreeMap<String, ChannelId>,
    channels: BTreeMap<ChannelId, ChannelState>,
    sub_to_channel: BTreeMap<SubscriptionId, ChannelId>,
}

impl Default for NveChannelBus {
    fn default() -> Self {
        Self::new()
    }
}

impl NveChannelBus {
    pub fn new() -> Self {
        let mut bus = Self {
            next_custom_id: FIRST_CUSTOM_CHANNEL_ID,
            next_sub_id: 1,
            topic_to_id: BTreeMap::new(),
            channels: BTreeMap::new(),
            sub_to_channel: BTreeMap::new(),
        };
        for (id, topic) in [
            (CHANNEL_ID_THEME, TOPIC_THEME),
            (CHANNEL_ID_LOCALE, TOPIC_LOCALE),
            (CHANNEL_ID_AUTH, TOPIC_AUTH),
            (CHANNEL_ID_NAVIGATION, TOPIC_NAVIGATION),
        ] {
            bus.register_builtin_channel(id, topic);
        }
        bus
    }

    fn register_builtin_channel(&mut self, id: ChannelId, topic: &str) {
        self.topic_to_id.insert(topic.to_owned(), id);
        self.channels.insert(
            id,
            ChannelState {
                id,
                topic: topic.to_owned(),
                seq: 0,
                retained: None,
                subscribers: Vec::new(),
            },
        );
    }

    /// Resolves an existing topic to its numeric [`ChannelId`], or registers a new custom channel.
    pub fn ensure_channel(&mut self, topic: &str) -> Result<ChannelId, InteropError> {
        let trimmed = topic.trim();
        if trimmed.is_empty() {
            return Err(InteropError::InvalidTopic("topic must not be empty".into()));
        }
        if let Some(&existing) = self.topic_to_id.get(trimmed) {
            return Ok(existing);
        }
        let id = self.next_custom_id;
        self.next_custom_id = self.next_custom_id.saturating_add(1);
        self.topic_to_id.insert(trimmed.to_owned(), id);
        self.channels.insert(
            id,
            ChannelState {
                id,
                topic: trimmed.to_owned(),
                seq: 0,
                retained: None,
                subscribers: Vec::new(),
            },
        );
        Ok(id)
    }

    pub fn channel_id(&self, topic: &str) -> Option<ChannelId> {
        self.topic_to_id.get(topic).copied()
    }

    pub fn topic_for(&self, channel_id: ChannelId) -> Option<&str> {
        self.channels.get(&channel_id).map(|c| c.topic.as_str())
    }

    pub fn topics(&self) -> Vec<&str> {
        self.topic_to_id.keys().map(String::as_str).collect()
    }

    pub fn subscriber_count(&self, topic: &str) -> usize {
        self.channel_id(topic)
            .and_then(|id| self.channels.get(&id))
            .map(|c| c.subscribers.len())
            .unwrap_or(0)
    }

    /// Subscribes to `topic` (creating it if needed). When `replay_retained` is `true` and the
    /// channel has retained shared state, `handler` is invoked immediately with that snapshot.
    pub fn subscribe<F>(
        &mut self,
        topic: &str,
        owner_surface: Option<SurfaceId>,
        replay_retained: bool,
        handler: F,
    ) -> Result<SubscriptionId, InteropError>
    where
        F: Fn(&ChannelMessage) + Send + Sync + 'static,
    {
        let channel_id = self.ensure_channel(topic)?;
        self.subscribe_by_id(channel_id, owner_surface, replay_retained, handler)
    }

    /// Subscribes directly using a C-friendly [`ChannelId`].
    pub fn subscribe_by_id<F>(
        &mut self,
        channel_id: ChannelId,
        owner_surface: Option<SurfaceId>,
        replay_retained: bool,
        handler: F,
    ) -> Result<SubscriptionId, InteropError>
    where
        F: Fn(&ChannelMessage) + Send + Sync + 'static,
    {
        let cb: SubscriberCallback = Arc::new(handler);
        let retained_snapshot = {
            let channel = self
                .channels
                .get_mut(&channel_id)
                .ok_or(InteropError::ChannelNotFound(channel_id))?;
            let sub_id = self.next_sub_id;
            self.next_sub_id = self.next_sub_id.saturating_add(1);
            channel.subscribers.push(SubscriptionEntry {
                id: sub_id,
                owner_surface,
                callback: Arc::clone(&cb),
            });
            self.sub_to_channel.insert(sub_id, channel_id);
            (sub_id, if replay_retained { channel.retained.clone() } else { None })
        };

        if let Some(msg) = retained_snapshot.1 {
            cb(&msg);
        }

        Ok(retained_snapshot.0)
    }

    /// Unsubscribes a single subscription by ID.
    pub fn unsubscribe(&mut self, subscription_id: SubscriptionId) -> Result<(), InteropError> {
        let channel_id = self
            .sub_to_channel
            .remove(&subscription_id)
            .ok_or(InteropError::SubscriptionNotFound(subscription_id))?;
        if let Some(channel) = self.channels.get_mut(&channel_id) {
            channel.subscribers.retain(|s| s.id != subscription_id);
        }
        Ok(())
    }

    /// Removes every subscription owned by `surface_id` across all channels (leak-free teardown).
    pub fn unsubscribe_surface(&mut self, surface_id: SurfaceId) -> usize {
        let mut removed = 0usize;
        for channel in self.channels.values_mut() {
            let before = channel.subscribers.len();
            channel.subscribers.retain(|s| {
                let keep = s.owner_surface != Some(surface_id);
                if !keep {
                    self.sub_to_channel.remove(&s.id);
                }
                keep
            });
            removed += before - channel.subscribers.len();
        }
        removed
    }

    /// Publishes zero-copy binary bytes to `topic`. Returns the number of notified subscribers.
    pub fn publish_bytes(
        &mut self,
        topic: &str,
        sender_surface: Option<SurfaceId>,
        payload: ChannelPayload,
        retain: bool,
    ) -> Result<usize, InteropError> {
        let channel_id = self.ensure_channel(topic)?;
        self.publish_by_id(
            channel_id,
            sender_surface,
            PayloadEncoding::Binary,
            payload,
            retain,
        )
    }

    /// Serializes `value` once as JSON and fans out the zero-copy byte buffer to all subscribers.
    pub fn publish_json<T: Serialize>(
        &mut self,
        topic: &str,
        sender_surface: Option<SurfaceId>,
        value: &T,
        retain: bool,
    ) -> Result<usize, InteropError> {
        let bytes =
            serde_json::to_vec(value).map_err(|e| InteropError::Serialization(e.to_string()))?;
        let channel_id = self.ensure_channel(topic)?;
        self.publish_by_id(
            channel_id,
            sender_surface,
            PayloadEncoding::Json,
            ChannelPayload::from_vec(bytes),
            retain,
        )
    }

    /// Publishes directly by C-friendly [`ChannelId`].
    pub fn publish_by_id(
        &mut self,
        channel_id: ChannelId,
        sender_surface: Option<SurfaceId>,
        encoding: PayloadEncoding,
        payload: ChannelPayload,
        retain: bool,
    ) -> Result<usize, InteropError> {
        let (msg, callbacks) = {
            let channel = self
                .channels
                .get_mut(&channel_id)
                .ok_or(InteropError::ChannelNotFound(channel_id))?;
            channel.seq = channel.seq.saturating_add(1);
            let msg = ChannelMessage {
                channel_id: channel.id,
                topic: channel.topic.clone(),
                seq: channel.seq,
                sender_surface,
                encoding,
                payload,
            };
            if retain {
                channel.retained = Some(msg.clone());
            }
            let callbacks: Vec<SubscriberCallback> = channel
                .subscribers
                .iter()
                .map(|s| Arc::clone(&s.callback))
                .collect();
            (msg, callbacks)
        };

        for cb in &callbacks {
            cb(&msg);
        }
        Ok(callbacks.len())
    }

    /// Returns the latest retained shared-state message for `topic`, if any.
    pub fn retained(&self, topic: &str) -> Option<&ChannelMessage> {
        let id = self.channel_id(topic)?;
        self.channels.get(&id)?.retained.as_ref()
    }
}
