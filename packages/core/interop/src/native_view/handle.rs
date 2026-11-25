//! `NativeViewSpec`, typed imperative `NativeViewHandle`, and wildcard-aware `NativeViewRegistry`.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use nve_abi::{CommandBufferBuilder, FrameId, NodeId, SurfaceId};

use super::codec::{
    NativeCommandDecoder, NativeCommandEncoder, NativePropDecoder, NativePropEncoder,
};
use super::wire::NativeViewCodecError;

/// Encodes typed native view props into a compact zero-JSON `NVNP` binary buffer.
pub trait EncodeNativeProps {
    fn encode_props(&self, enc: &mut NativePropEncoder) -> Result<(), NativeViewCodecError>;

    /// Convenience helper encoding directly into a standalone `Vec<u8>`.
    fn to_prop_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        let mut enc = NativePropEncoder::new();
        self.encode_props(&mut enc)?;
        Ok(enc.finish())
    }
}

/// Decodes typed native view props from a validated zero-JSON `NVNP` binary buffer.
pub trait DecodeNativeProps: Sized {
    fn decode_props(dec: &NativePropDecoder<'_>) -> Result<Self, NativeViewCodecError>;

    fn from_prop_bytes(bytes: &[u8]) -> Result<Self, NativeViewCodecError> {
        let dec = NativePropDecoder::decode(bytes)?;
        Self::decode_props(&dec)
    }
}

/// Encodes typed imperative native view commands into a zero-JSON `NVNC` binary payload
/// carried inside `nve_abi::Command::DispatchCommand`.
pub trait EncodeNativeCommand {
    /// Wire command name carried in `Command::DispatchCommand::command_name`.
    fn command_name(&self) -> &'static str;

    /// Encodes the command opcode and binary arguments into `NVNC` bytes.
    fn encode_command(&self) -> Result<Vec<u8>, NativeViewCodecError>;
}

/// Decodes typed imperative native view commands from a validated `NVNC` binary payload.
pub trait DecodeNativeCommand: Sized {
    fn decode_command(dec: &NativeCommandDecoder<'_>) -> Result<Self, NativeViewCodecError>;

    fn from_command_bytes(bytes: &[u8]) -> Result<Self, NativeViewCodecError> {
        let dec = NativeCommandDecoder::decode(bytes)?;
        Self::decode_command(&dec)
    }
}

/// Marker command type for declarative native views that do not expose imperative commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoCommand {}

impl EncodeNativeCommand for NoCommand {
    fn command_name(&self) -> &'static str {
        match *self {}
    }

    fn encode_command(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        match *self {}
    }
}

impl DecodeNativeCommand for NoCommand {
    fn decode_command(dec: &NativeCommandDecoder<'_>) -> Result<Self, NativeViewCodecError> {
        Err(NativeViewCodecError::UnknownCommandOpcode(dec.opcode()))
    }
}

/// First-class typed contract for a native platform view primitive mounted via `<NativeView>`.
pub trait NativeViewSpec: 'static {
    /// Canonical native view kind string (e.g. `"nve.blur_surface"`, `"nve.web_view"`, `"web.*"`).
    const KIND: &'static str;
    type Props: EncodeNativeProps + Clone + PartialEq + 'static;
    type Command: EncodeNativeCommand + 'static;

    /// Resolves the concrete native kind emitted for a specific props instance.
    /// Defaults to [`Self::KIND`]; overridden by dynamic passthrough specs like `WebElement`.
    #[inline]
    fn resolved_kind(_props: &Self::Props) -> Cow<'static, str> {
        Cow::Borrowed(Self::KIND)
    }
}

/// Binary imperative command queued on a [`NativeViewHandle`] awaiting flush into a `CommandBuffer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedNativeCommand {
    pub surface_id: SurfaceId,
    pub node_id: NodeId,
    pub command_name: &'static str,
    pub payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct HandleInner {
    binding: Option<(SurfaceId, NodeId)>,
    pending: Vec<QueuedNativeCommand>,
}

/// Type-erased mount/unmount binder extracted from [`NativeViewHandle`] so UI runtimes
/// (e.g. `nve-dioxus`) can bind `NodeId` on mount without leaking `NativeViewSpec` generics.
#[derive(Clone)]
pub struct NativeViewHandleBinder {
    inner: Arc<Mutex<HandleInner>>,
}

impl NativeViewHandleBinder {
    #[inline]
    pub fn bind(&self, surface_id: SurfaceId, node_id: NodeId) {
        let mut guard = self.inner.lock().unwrap();
        guard.binding = Some((surface_id, node_id));
    }

    #[inline]
    pub fn unbind(&self) {
        let mut guard = self.inner.lock().unwrap();
        guard.binding = None;
    }

    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl fmt::Debug for NativeViewHandleBinder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let guard = self.inner.lock().unwrap();
        f.debug_struct("NativeViewHandleBinder")
            .field("binding", &guard.binding)
            .finish()
    }
}

/// Typed imperative command handle bound to a mounted `<NativeView<P>>` node.
///
/// Emits compact binary `nve_abi::Command::DispatchCommand` frames without re-rendering RSX.
pub struct NativeViewHandle<P: NativeViewSpec> {
    inner: Arc<Mutex<HandleInner>>,
    _marker: PhantomData<fn() -> P>,
}

impl<P: NativeViewSpec> Clone for NativeViewHandle<P> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            _marker: PhantomData,
        }
    }
}

impl<P: NativeViewSpec> PartialEq for NativeViewHandle<P> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl<P: NativeViewSpec> Eq for NativeViewHandle<P> {}

impl<P: NativeViewSpec> Default for NativeViewHandle<P> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<P: NativeViewSpec> fmt::Debug for NativeViewHandle<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let guard = self.inner.lock().unwrap();
        f.debug_struct("NativeViewHandle")
            .field("kind", &P::KIND)
            .field("binding", &guard.binding)
            .field("pending", &guard.pending.len())
            .finish()
    }
}

impl<P: NativeViewSpec> NativeViewHandle<P> {
    /// Creates an unbound handle to be attached to a `<NativeView<P>>`.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HandleInner::default())),
            _marker: PhantomData,
        }
    }

    /// Creates a handle pre-bound to `(surface_id, node_id)`.
    pub fn bound(surface_id: SurfaceId, node_id: NodeId) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HandleInner {
                binding: Some((surface_id, node_id)),
                pending: Vec::new(),
            })),
            _marker: PhantomData,
        }
    }

    /// Binds this handle to a mounted `(surface_id, node_id)`.
    pub fn bind(&self, surface_id: SurfaceId, node_id: NodeId) {
        self.inner.lock().unwrap().binding = Some((surface_id, node_id));
    }

    /// Detaches this handle when the underlying native view unmounts.
    pub fn unbind(&self) {
        self.inner.lock().unwrap().binding = None;
    }

    #[inline]
    pub fn is_bound(&self) -> bool {
        self.inner.lock().unwrap().binding.is_some()
    }

    #[inline]
    pub fn node_id(&self) -> Option<NodeId> {
        self.inner.lock().unwrap().binding.map(|(_, nid)| nid)
    }

    #[inline]
    pub fn surface_id(&self) -> Option<SurfaceId> {
        self.inner.lock().unwrap().binding.map(|(sid, _)| sid)
    }

    /// Produces a type-erased binder for runtime mount/unmount wiring.
    #[inline]
    pub fn binder(&self) -> NativeViewHandleBinder {
        NativeViewHandleBinder {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Queues a typed imperative command encoded as binary `NVNC` bytes.
    pub fn dispatch(&self, command: P::Command) -> Result<(), NativeViewCodecError> {
        let payload = command.encode_command()?;
        let mut guard = self.inner.lock().unwrap();
        let (surface_id, node_id) = guard.binding.ok_or(NativeViewCodecError::UnboundHandle)?;
        guard.pending.push(QueuedNativeCommand {
            surface_id,
            node_id,
            command_name: command.command_name(),
            payload,
        });
        Ok(())
    }

    /// Immediately encodes a single typed imperative command into a standalone `nve-abi`
    /// binary `CommandBuffer` frame (without touching RSX or VirtualDom).
    pub fn encode_command_frame(
        &self,
        frame_id: FrameId,
        command: P::Command,
    ) -> Result<Vec<u8>, NativeViewCodecError> {
        let (surface_id, node_id) = self
            .inner
            .lock()
            .unwrap()
            .binding
            .ok_or(NativeViewCodecError::UnboundHandle)?;
        let payload = command.encode_command()?;
        let mut builder = CommandBufferBuilder::with_capacity(surface_id, frame_id, 128);
        builder.dispatch_command(node_id, command.command_name(), &payload);
        builder.commit(false);
        Ok(builder.into_bytes())
    }

    /// Drains all queued commands into an existing [`CommandBufferBuilder`].
    /// Returns the number of `Command::DispatchCommand` ops emitted.
    pub fn flush_into_builder(&self, builder: &mut CommandBufferBuilder) -> usize {
        let drained: Vec<QueuedNativeCommand> = {
            let mut guard = self.inner.lock().unwrap();
            std::mem::take(&mut guard.pending)
        };
        let count = drained.len();
        for cmd in drained {
            builder.dispatch_command(cmd.node_id, cmd.command_name, &cmd.payload);
        }
        count
    }

    /// Drains all queued commands into a standalone binary `nve-abi` `CommandBuffer` frame.
    /// Returns `None` if no commands are queued.
    pub fn take_command_frame(&self, frame_id: FrameId) -> Option<Vec<u8>> {
        let drained: Vec<QueuedNativeCommand> = {
            let mut guard = self.inner.lock().unwrap();
            if guard.pending.is_empty() {
                return None;
            }
            std::mem::take(&mut guard.pending)
        };
        let surface_id = drained[0].surface_id;
        let mut builder = CommandBufferBuilder::with_capacity(surface_id, frame_id, 256);
        for cmd in drained {
            builder.dispatch_command(cmd.node_id, cmd.command_name, &cmd.payload);
        }
        builder.commit(false);
        Some(builder.into_bytes())
    }

    /// Drains queued commands directly for inspection or custom host forwarding.
    pub fn drain_queued(&self) -> Vec<QueuedNativeCommand> {
        let mut guard = self.inner.lock().unwrap();
        std::mem::take(&mut guard.pending)
    }

    #[inline]
    pub fn pending_count(&self) -> usize {
        self.inner.lock().unwrap().pending.len()
    }
}

// =============================================================================
// NativeViewRegistry (Exact Kinds + Prefix Wildcards such as `"web.*"`)
// =============================================================================

/// Registry advertising which native platform view primitives are supported by the active host.
///
/// Supports exact kinds (e.g. `"nve.blur_surface"`, `"nve.web_view"`) and prefix wildcards
/// (e.g. `"web.*"` for Web DOM passthrough elements `<video>`, `<svg>`, `<iframe>`, Web Components).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NativeViewRegistry {
    exact: BTreeSet<String>,
    /// Normalized prefix strings before trailing `*` (e.g. `"web.*"` -> `"web."`, `"*"` -> `""`).
    prefix_wildcards: Vec<String>,
}

impl NativeViewRegistry {
    /// Empty registry (every `<NativeView>` renders its graceful `fallback` slot).
    pub fn new() -> Self {
        Self::default()
    }

    /// Preset for headless test hosts (no native primitives registered).
    pub fn headless() -> Self {
        Self::new()
    }

    /// Preset for Web DOM hosts supporting `"web.*"` passthrough elements.
    pub fn web_dom() -> Self {
        Self::new().with("web.*")
    }

    /// Preset for Apple UIKit/AppKit hosts (`BlurSurface`, `WebView`, `VideoPlayer`).
    pub fn apple() -> Self {
        Self::new()
            .with(super::specs::BlurSurface::KIND)
            .with(super::specs::WebView::KIND)
            .with(super::specs::VideoPlayer::KIND)
    }

    /// Preset for Android hosts (`WebView`, `VideoPlayer`).
    pub fn android() -> Self {
        Self::new()
            .with(super::specs::WebView::KIND)
            .with(super::specs::VideoPlayer::KIND)
    }

    /// Registers an exact kind or prefix wildcard pattern (e.g. `"web.*"`).
    pub fn register(&mut self, pattern: impl Into<String>) -> &mut Self {
        let raw = pattern.into();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return self;
        }
        if let Some(prefix) = trimmed.strip_suffix('*') {
            let owned = prefix.to_owned();
            if !self.prefix_wildcards.contains(&owned) {
                self.prefix_wildcards.push(owned);
                self.prefix_wildcards.sort();
            }
        } else {
            self.exact.insert(trimmed.to_owned());
        }
        self
    }

    /// Builder-style variant of [`Self::register`].
    pub fn with(mut self, pattern: impl Into<String>) -> Self {
        self.register(pattern);
        self
    }

    /// Registers `P::KIND` in this registry.
    pub fn register_spec<P: NativeViewSpec>(&mut self) -> &mut Self {
        self.register(P::KIND)
    }

    /// Builder-style variant of [`Self::register_spec`].
    pub fn with_spec<P: NativeViewSpec>(mut self) -> Self {
        self.register_spec::<P>();
        self
    }

    /// Removes a previously registered exact kind or prefix wildcard pattern.
    pub fn unregister(&mut self, pattern: &str) -> bool {
        let trimmed = pattern.trim();
        if let Some(prefix) = trimmed.strip_suffix('*') {
            let before = self.prefix_wildcards.len();
            self.prefix_wildcards.retain(|p| p != prefix);
            self.prefix_wildcards.len() != before
        } else {
            self.exact.remove(trimmed)
        }
    }

    /// Returns `true` if `kind` matches an exact entry or a registered prefix wildcard.
    pub fn supports(&self, kind: &str) -> bool {
        let trimmed = kind.trim();
        if trimmed.is_empty() {
            return false;
        }
        if self.exact.contains(trimmed) {
            return true;
        }
        self.prefix_wildcards.iter().any(|prefix| {
            if prefix.is_empty() {
                true
            } else {
                trimmed.starts_with(prefix.as_str()) && trimmed.len() > prefix.len()
            }
        })
    }

    /// Returns `true` if `P::KIND` is registered (for static-kind specs).
    #[inline]
    pub fn supports_spec<P: NativeViewSpec>(&self) -> bool {
        self.supports(P::KIND)
    }

    /// Returns `true` if the resolved kind for `(P, props)` is supported by this host.
    #[inline]
    pub fn supports_props<P: NativeViewSpec>(&self, props: &P::Props) -> bool {
        let kind = P::resolved_kind(props);
        self.supports(kind.as_ref())
    }

    #[inline]
    pub fn exact_kinds(&self) -> &BTreeSet<String> {
        &self.exact
    }

    #[inline]
    pub fn wildcard_prefixes(&self) -> &[String] {
        &self.prefix_wildcards
    }
}

/// Helper re-exported for specs implementing [`EncodeNativeCommand`].
#[inline]
pub(crate) fn cmd_encoder(opcode: u16) -> NativeCommandEncoder {
    NativeCommandEncoder::new(opcode)
}
