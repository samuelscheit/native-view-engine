//! Typed imperative [`NativeViewHandle`] (zero-serialization in-process command queueing +
//! on-demand cross-language `CommandBuffer` wire framing) and wildcard-aware
//! [`NativeViewRegistry`].

use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use nve_abi::{CommandBufferBuilder, FrameId, NodeId, SurfaceId};
use nve_components::native_view::{
    BlurSurface, EncodeNativeCommand, NativeCommand, NativeViewCodecError, NativeViewSpec,
    TypedNativeViewCommand, VideoPlayer, WebView,
};

/// Imperative command queued on a [`NativeViewHandle`].
///
/// Stores zero-serialization [`NativeCommand`] + concrete [`TypedNativeViewCommand`] directly
/// for in-process Rust hosts; encodes `NVNC` wire bytes only when flushed to an FFI `CommandBuffer`.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedNativeCommand {
    pub surface_id: SurfaceId,
    pub node_id: NodeId,
    pub command_name: &'static str,
    pub command: NativeCommand,
    pub typed: TypedNativeViewCommand,
}

impl QueuedNativeCommand {
    /// Encodes this command into cross-language `NVNC` binary wire bytes on demand.
    #[inline]
    pub fn to_wire_bytes(&self) -> Result<Vec<u8>, NativeViewCodecError> {
        self.command.to_nvnc_bytes()
    }
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
/// Queues zero-serialization typed commands in-process and can also emit cross-language
/// `nve_abi::Command::DispatchCommand` frames without re-rendering RSX.
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

    /// Queues a typed imperative command in-process with ZERO binary `NVNC` serialization.
    pub fn dispatch(&self, command: P::Command) -> Result<(), NativeViewCodecError> {
        let command_name = command.command_name();
        let native_cmd = command.to_native_command()?;
        let typed = TypedNativeViewCommand::with_typed(native_cmd.clone(), Arc::new(command));
        let mut guard = self.inner.lock().unwrap();
        let (surface_id, node_id) = guard.binding.ok_or(NativeViewCodecError::UnboundHandle)?;
        guard.pending.push(QueuedNativeCommand {
            surface_id,
            node_id,
            command_name,
            command: native_cmd,
            typed,
        });
        Ok(())
    }

    /// Immediately encodes a single typed imperative command into a standalone `nve-abi`
    /// binary `CommandBuffer` frame for cross-language FFI hosts.
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

    /// Drains all queued commands into an existing cross-language [`CommandBufferBuilder`].
    /// Returns the number of `Command::DispatchCommand` ops emitted.
    pub fn flush_into_builder(&self, builder: &mut CommandBufferBuilder) -> usize {
        let drained = self.drain_queued();
        let mut count = 0usize;
        for cmd in drained {
            if let Ok(payload) = cmd.to_wire_bytes() {
                builder.dispatch_command(cmd.node_id, cmd.command_name, &payload);
                count += 1;
            }
        }
        count
    }

    /// Drains all queued commands into a standalone cross-language `nve-abi` `CommandBuffer` frame.
    /// Returns `None` if no commands are queued.
    pub fn take_command_frame(&self, frame_id: FrameId) -> Option<Vec<u8>> {
        let drained = self.drain_queued();
        if drained.is_empty() {
            return None;
        }
        let surface_id = drained[0].surface_id;
        let mut builder = CommandBufferBuilder::with_capacity(surface_id, frame_id, 256);
        for cmd in drained {
            if let Ok(payload) = cmd.to_wire_bytes() {
                builder.dispatch_command(cmd.node_id, cmd.command_name, &payload);
            }
        }
        builder.commit(false);
        Some(builder.into_bytes())
    }

    /// Drains queued zero-serialization commands directly for in-process Rust backends.
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn headless() -> Self {
        Self::new()
    }

    pub fn web_dom() -> Self {
        Self::new().with("web.*")
    }

    pub fn apple() -> Self {
        Self::new()
            .with(BlurSurface::KIND)
            .with(WebView::KIND)
            .with(VideoPlayer::KIND)
    }

    pub fn android() -> Self {
        Self::new().with(WebView::KIND).with(VideoPlayer::KIND)
    }

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

    pub fn with(mut self, pattern: impl Into<String>) -> Self {
        self.register(pattern);
        self
    }

    pub fn register_spec<P: NativeViewSpec>(&mut self) -> &mut Self {
        self.register(P::KIND)
    }

    pub fn with_spec<P: NativeViewSpec>(mut self) -> Self {
        self.register_spec::<P>();
        self
    }

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

    #[inline]
    pub fn supports_spec<P: NativeViewSpec>(&self) -> bool {
        self.supports(P::KIND)
    }

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
