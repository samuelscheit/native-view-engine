//! Zero-overhead typed mutation sink (`RenderSink`), single cross-boundary binary `BatchSink`,
//! and zero-copy `CommandBuffer` replay (`replay_command_buffer_into`).
//!
//! - **In-process paths** (Web WASM, Skia, Headless, static-LTO native builds) monomorphize
//!   producers directly over `S: RenderSink`, passing borrowed Rust structs (`&LayoutProps`,
//!   `&StyleProps`, `&LayoutExt`, `&[StateStyleRule]`, `&BackgroundFill`, `&SvgPicture`,
//!   `&DrawList`, `&TypedNativeViewProps`, etc.) with zero serialization, zero decoding, and
//!   zero vtable indirection.
//! - **Cross-boundary paths** (`NveApp::render` / `export_nve_app!` `cdylib` hot-reload,
//!   `nve-wire` streaming, snapshot/replay, C++20 Fabric bridge) route through the single shared
//!   [`BatchSink`], which encodes into `nve_abi::CommandBufferBuilder` and replays losslessly via
//!   [`replay_command_buffer_into`].

use nve_abi::{
    CommandBufferBuilder, CommandDecoder, CommandRef, ComponentKind, FrameId, LayoutProps, NodeId,
    RawPropValueRef, StyleProps, SurfaceId,
};
use nve_components::canvas_ir::DrawList;
use nve_components::extended_wire::{
    self, EXT_PROP_BACKGROUND_FILL, EXT_PROP_CURSOR, EXT_PROP_LAYOUT_EXT, EXT_PROP_PREFIX,
    EXT_PROP_RESPONSIVE_RULES, EXT_PROP_SHOW_ON, EXT_PROP_STATE_RULES, EXT_PROP_SVG,
    EXT_PROP_TRANSFORM, EXT_PROP_TRANSITIONS,
};
use nve_components::interaction::{CursorKind, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::native_view::{TypedNativeViewProps, NATIVE_VIEW_PROPS_PROP_KEY};
use nve_components::paint::BackgroundFill;
use nve_components::svg::SvgPicture;
use nve_components::transform::Transform2D;

use crate::{BackendApplyStats, BackendError};

/// Unified zero-overhead mutation receiver implemented directly by every in-process backend
/// (`WebBackend`, `SkiaBackend`, `HeadlessBackend`, and future `nve-runtime`) as well as the
/// single cross-boundary [`BatchSink`].
///
/// Covers all core tree/layout/style/event mutations and all extended typed properties in one
/// pass so no out-of-band `sync_*_from` side-channels are ever needed.
pub trait RenderSink {
    /// Starts a new mutation frame targeting `surface_id` with monotonic `frame_id`.
    fn begin_frame(&mut self, surface_id: SurfaceId, frame_id: FrameId);

    /// Creates a new node with `kind` and optional `custom_name` (for `ComponentKind::Custom`).
    fn create_node(&mut self, node_id: NodeId, kind: ComponentKind, custom_name: Option<&str>);

    /// Removes `node_id` from the tree.
    fn remove_node(&mut self, node_id: NodeId);

    /// Replaces the ordered child list of `parent_id`.
    fn set_children(&mut self, parent_id: NodeId, children: &[NodeId]);

    /// Sets the UTF-8 text content of `node_id`.
    fn set_text(&mut self, node_id: NodeId, text: &str);

    /// Sets core Flexbox `LayoutProps` on `node_id`.
    fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps);

    /// Sets core visual `StyleProps` on `node_id`.
    fn set_style(&mut self, node_id: NodeId, style: &StyleProps);

    /// Sets a borrowed raw property (ARIA, HTML5 semantic/input attributes, raw CSS fallbacks).
    fn set_raw_prop(&mut self, node_id: NodeId, key: &str, value: RawPropValueRef<'_>);

    /// Sets the subscribed `EventMask` bitmask for `node_id`.
    fn set_event_mask(&mut self, node_id: NodeId, mask: u64);

    /// Dispatches an imperative command payload to `node_id`.
    fn dispatch_command(&mut self, node_id: NodeId, command_name: &str, payload: &[u8]);

    /// Sets or clears extended layout metadata (CSS Grid, `position: sticky/absolute`, insets,
    /// `flex-basis`, `aspect-ratio`, RTL direction, logical inline edges).
    #[inline]
    fn set_layout_ext(&mut self, _node_id: NodeId, _ext: Option<&LayoutExt>) {}

    /// Sets or clears a typed 2D affine transform.
    #[inline]
    fn set_transform(&mut self, _node_id: NodeId, _transform: Option<Transform2D>) {}

    /// Sets or clears an explicit pointer cursor.
    #[inline]
    fn set_cursor(&mut self, _node_id: NodeId, _cursor: Option<CursorKind>) {}

    /// Sets interactive pseudo-state rules (`:hover`, `:focus`, `:focus-visible`, `:active`,
    /// `:disabled`). An empty slice clears all rules on `node_id`.
    #[inline]
    fn set_state_rules(&mut self, _node_id: NodeId, _rules: &[StateStyleRule]) {}

    /// Sets declarative style/transform transitions. An empty slice clears all transitions.
    #[inline]
    fn set_transitions(&mut self, _node_id: NodeId, _transitions: &[TransitionSpec]) {}

    /// Sets or clears a declarative `MediaQuery` visibility gate (`show_on`).
    #[inline]
    fn set_show_on(&mut self, _node_id: NodeId, _query: Option<MediaQuery>) {}

    /// Sets responsive `@media` style rules. An empty slice clears all responsive rules.
    #[inline]
    fn set_responsive_rules(&mut self, _node_id: NodeId, _rules: &[ResponsiveRule]) {}

    /// Sets or clears a typed background fill (solid, linear gradient, or radial gradient).
    #[inline]
    fn set_background_fill(&mut self, _node_id: NodeId, _fill: Option<&BackgroundFill>) {}

    /// Sets or clears a parsed, content-hashed vector `SvgPicture`.
    #[inline]
    fn set_svg(&mut self, _node_id: NodeId, _svg: Option<&SvgPicture>) {}

    /// Sets or clears a typed vector `DrawList` (`Arc<[CanvasOp]>`) for `SkiaCanvas` nodes.
    #[inline]
    fn set_draw_list(&mut self, _node_id: NodeId, _draw_list: Option<&DrawList>) {}

    /// Sets or clears typed `<NativeView>` properties (`BlurSurface`, `WebView`, `VideoPlayer`,
    /// `CameraPreview`, `WebElement`, etc.).
    #[inline]
    fn set_native_view_props(&mut self, _node_id: NodeId, _props: Option<&TypedNativeViewProps>) {}

    /// Commits the frame (runs layout, display-list rebuild, batched DOM/CSS flush, or finalizes
    /// the binary wire batch) and returns backend timing statistics or any validation error.
    fn commit_frame(&mut self, sync_mount: bool) -> Result<BackendApplyStats, BackendError>;

    /// Finalizes a frame whose stream ended without an explicit `Commit` op (used only when
    /// replaying partial/uncommitted binary `CommandBuffer` streams).
    #[inline]
    fn finish_uncommitted(&mut self) -> Result<BackendApplyStats, BackendError> {
        Ok(BackendApplyStats::default())
    }
}

/// Single cross-boundary [`RenderSink`] encoding mutations into a flat binary `nve-abi`
/// `CommandBuffer`.
///
/// Used ONLY when mutations cross a dynamic boundary (`NveApp::render` / `export_nve_app!`
/// `cdylib` hot-reload, `nve-wire` network transport, snapshot/replay, C++20 Fabric bridge).
/// Reuses its internal byte buffer and scratch encoding buffer across frames for zero steady-state
/// allocations.
pub struct BatchSink {
    builder: CommandBufferBuilder,
    ext_scratch: Vec<u8>,
    committed_len: usize,
}

impl BatchSink {
    #[inline]
    pub fn new(surface_id: SurfaceId) -> Self {
        Self {
            builder: CommandBufferBuilder::new(surface_id, 1),
            ext_scratch: Vec::new(),
            committed_len: 0,
        }
    }

    #[inline]
    pub fn with_capacity(surface_id: SurfaceId, capacity: usize) -> Self {
        Self {
            builder: CommandBufferBuilder::with_capacity(surface_id, 1, capacity),
            ext_scratch: Vec::new(),
            committed_len: 0,
        }
    }

    /// Returns the finalized binary `CommandBuffer` slice valid until the next `begin_frame`.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self.builder.as_bytes()[..self.committed_len]
    }

    /// Clones the finalized binary `CommandBuffer` into an owned `Vec<u8>`.
    #[inline]
    pub fn to_vec(&self) -> Vec<u8> {
        self.bytes().to_vec()
    }

    /// Consumes the sink and returns the finalized binary `CommandBuffer` `Vec<u8>`.
    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        let mut bytes = self.builder.into_bytes();
        bytes.truncate(self.committed_len);
        bytes
    }

    #[inline]
    pub fn op_count(&self) -> u32 {
        self.builder.op_count()
    }
}

impl RenderSink for BatchSink {
    #[inline]
    fn begin_frame(&mut self, surface_id: SurfaceId, frame_id: FrameId) {
        self.builder.reset(surface_id, frame_id);
        self.committed_len = 0;
    }

    #[inline]
    fn create_node(&mut self, node_id: NodeId, kind: ComponentKind, custom_name: Option<&str>) {
        self.builder.create_node(node_id, kind, custom_name);
    }

    #[inline]
    fn remove_node(&mut self, node_id: NodeId) {
        self.builder.remove_node(node_id);
    }

    #[inline]
    fn set_children(&mut self, parent_id: NodeId, children: &[NodeId]) {
        self.builder.set_children(parent_id, children);
    }

    #[inline]
    fn set_text(&mut self, node_id: NodeId, text: &str) {
        self.builder.set_text(node_id, text);
    }

    #[inline]
    fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps) {
        self.builder.set_layout(node_id, layout);
    }

    #[inline]
    fn set_style(&mut self, node_id: NodeId, style: &StyleProps) {
        self.builder.set_style(node_id, style);
    }

    #[inline]
    fn set_raw_prop(&mut self, node_id: NodeId, key: &str, value: RawPropValueRef<'_>) {
        self.builder.set_raw_prop_ref(node_id, key, value);
    }

    #[inline]
    fn set_event_mask(&mut self, node_id: NodeId, mask: u64) {
        self.builder.set_event_mask(node_id, mask);
    }

    #[inline]
    fn dispatch_command(&mut self, node_id: NodeId, command_name: &str, payload: &[u8]) {
        self.builder
            .dispatch_command(node_id, command_name, payload);
    }

    fn set_layout_ext(&mut self, node_id: NodeId, ext: Option<&LayoutExt>) {
        match ext.filter(|e| !e.is_default()) {
            Some(e) => {
                self.ext_scratch.clear();
                extended_wire::encode_layout_ext(e, &mut self.ext_scratch);
                self.builder.set_raw_prop_ref(
                    node_id,
                    EXT_PROP_LAYOUT_EXT,
                    RawPropValueRef::Bytes(&self.ext_scratch),
                );
            }
            None => self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_LAYOUT_EXT,
                RawPropValueRef::Null,
            ),
        }
    }

    fn set_transform(&mut self, node_id: NodeId, transform: Option<Transform2D>) {
        match transform {
            Some(tf) => {
                self.ext_scratch.clear();
                extended_wire::encode_transform(tf, &mut self.ext_scratch);
                self.builder.set_raw_prop_ref(
                    node_id,
                    EXT_PROP_TRANSFORM,
                    RawPropValueRef::Bytes(&self.ext_scratch),
                );
            }
            None => self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_TRANSFORM,
                RawPropValueRef::Null,
            ),
        }
    }

    fn set_cursor(&mut self, node_id: NodeId, cursor: Option<CursorKind>) {
        match cursor {
            Some(c) => {
                self.ext_scratch.clear();
                extended_wire::encode_cursor(c, &mut self.ext_scratch);
                self.builder.set_raw_prop_ref(
                    node_id,
                    EXT_PROP_CURSOR,
                    RawPropValueRef::Bytes(&self.ext_scratch),
                );
            }
            None => {
                self.builder
                    .set_raw_prop_ref(node_id, EXT_PROP_CURSOR, RawPropValueRef::Null);
            }
        }
    }

    fn set_state_rules(&mut self, node_id: NodeId, rules: &[StateStyleRule]) {
        if rules.is_empty() {
            self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_STATE_RULES,
                RawPropValueRef::Null,
            );
        } else {
            self.ext_scratch.clear();
            extended_wire::encode_state_rules(rules, &mut self.ext_scratch);
            self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_STATE_RULES,
                RawPropValueRef::Bytes(&self.ext_scratch),
            );
        }
    }

    fn set_transitions(&mut self, node_id: NodeId, transitions: &[TransitionSpec]) {
        if transitions.is_empty() {
            self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_TRANSITIONS,
                RawPropValueRef::Null,
            );
        } else {
            self.ext_scratch.clear();
            extended_wire::encode_transitions(transitions, &mut self.ext_scratch);
            self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_TRANSITIONS,
                RawPropValueRef::Bytes(&self.ext_scratch),
            );
        }
    }

    fn set_show_on(&mut self, node_id: NodeId, query: Option<MediaQuery>) {
        match query {
            Some(q) => {
                self.ext_scratch.clear();
                extended_wire::encode_media_query(q, &mut self.ext_scratch);
                self.builder.set_raw_prop_ref(
                    node_id,
                    EXT_PROP_SHOW_ON,
                    RawPropValueRef::Bytes(&self.ext_scratch),
                );
            }
            None => {
                self.builder
                    .set_raw_prop_ref(node_id, EXT_PROP_SHOW_ON, RawPropValueRef::Null);
            }
        }
    }

    fn set_responsive_rules(&mut self, node_id: NodeId, rules: &[ResponsiveRule]) {
        if rules.is_empty() {
            self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_RESPONSIVE_RULES,
                RawPropValueRef::Null,
            );
        } else {
            self.ext_scratch.clear();
            extended_wire::encode_responsive_rules(rules, &mut self.ext_scratch);
            self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_RESPONSIVE_RULES,
                RawPropValueRef::Bytes(&self.ext_scratch),
            );
        }
    }

    fn set_background_fill(&mut self, node_id: NodeId, fill: Option<&BackgroundFill>) {
        match fill {
            Some(f) => {
                self.ext_scratch.clear();
                extended_wire::encode_background_fill(f, &mut self.ext_scratch);
                self.builder.set_raw_prop_ref(
                    node_id,
                    EXT_PROP_BACKGROUND_FILL,
                    RawPropValueRef::Bytes(&self.ext_scratch),
                );
            }
            None => self.builder.set_raw_prop_ref(
                node_id,
                EXT_PROP_BACKGROUND_FILL,
                RawPropValueRef::Null,
            ),
        }
    }

    fn set_svg(&mut self, node_id: NodeId, svg: Option<&SvgPicture>) {
        match svg {
            Some(pic) => {
                self.ext_scratch.clear();
                extended_wire::encode_svg_picture(pic, &mut self.ext_scratch);
                self.builder.set_raw_prop_ref(
                    node_id,
                    EXT_PROP_SVG,
                    RawPropValueRef::Bytes(&self.ext_scratch),
                );
            }
            None => {
                self.builder
                    .set_raw_prop_ref(node_id, EXT_PROP_SVG, RawPropValueRef::Null);
            }
        }
    }

    fn set_draw_list(&mut self, node_id: NodeId, draw_list: Option<&DrawList>) {
        match draw_list {
            Some(dl) => {
                let bytes = dl.to_nve_draw1_bytes();
                self.builder
                    .set_raw_prop_ref(node_id, "drawList", RawPropValueRef::Bytes(&bytes));
            }
            None => {
                self.builder
                    .set_raw_prop_ref(node_id, "drawList", RawPropValueRef::Null);
            }
        }
    }

    fn set_native_view_props(&mut self, node_id: NodeId, props: Option<&TypedNativeViewProps>) {
        match props {
            Some(p) => {
                if let Ok(bytes) = p.to_nvnp_bytes() {
                    self.builder.set_raw_prop_ref(
                        node_id,
                        NATIVE_VIEW_PROPS_PROP_KEY,
                        RawPropValueRef::Bytes(&bytes),
                    );
                }
            }
            None => {
                self.builder.set_raw_prop_ref(
                    node_id,
                    NATIVE_VIEW_PROPS_PROP_KEY,
                    RawPropValueRef::Null,
                );
            }
        }
    }

    fn commit_frame(&mut self, sync_mount: bool) -> Result<BackendApplyStats, BackendError> {
        self.builder.commit(sync_mount);
        let op_count = self.builder.op_count();
        self.committed_len = self.builder.finish().len();
        Ok(BackendApplyStats {
            op_count,
            mutation_count: op_count.saturating_sub(1),
            ..Default::default()
        })
    }
}

/// Decodes a binary `CommandBuffer` stream and replays every core & extended mutation into `sink`.
///
/// Used by `RenderBackend::submit_frame` implementations so every backend maintains a single
/// [`RenderSink`] mutation implementation with zero duplicated command-handling logic.
pub fn replay_command_buffer_into<S: RenderSink>(
    command_bytes: &[u8],
    sink: &mut S,
) -> Result<BackendApplyStats, BackendError> {
    #[cfg(not(target_arch = "wasm32"))]
    let t_decode = std::time::Instant::now();

    let decoder = CommandDecoder::new(command_bytes)?;
    let header = decoder.header();
    sink.begin_frame(header.surface_id, header.frame_id);

    let mut children_buf: Vec<NodeId> = Vec::new();
    let mut committed = false;
    let mut last_sync_mount = true;

    decoder.for_each(|cmd| {
        match cmd {
            CommandRef::CreateNode {
                node_id,
                kind,
                custom_name,
            } => sink.create_node(node_id, kind, custom_name),
            CommandRef::RemoveNode { node_id } => sink.remove_node(node_id),
            CommandRef::SetChildren {
                parent_id,
                children,
            } => {
                children_buf.clear();
                children_buf.reserve(children.len());
                children_buf.extend(children);
                sink.set_children(parent_id, &children_buf);
            }
            CommandRef::SetText { node_id, text } => sink.set_text(node_id, text),
            CommandRef::SetLayout { node_id, layout } => sink.set_layout(node_id, &layout),
            CommandRef::SetStyle { node_id, style } => sink.set_style(node_id, &style),
            CommandRef::SetRawProp {
                node_id,
                key,
                value,
            } => {
                if key.starts_with(EXT_PROP_PREFIX) {
                    dispatch_extended_raw_prop(sink, node_id, key, value);
                } else {
                    sink.set_raw_prop(node_id, key, value);
                }
            }
            CommandRef::SetEventMask { node_id, mask } => sink.set_event_mask(node_id, mask),
            CommandRef::DispatchCommand {
                node_id,
                command_name,
                payload,
            } => sink.dispatch_command(node_id, command_name, payload),
            CommandRef::Commit { sync_mount } => {
                committed = true;
                last_sync_mount = sync_mount;
            }
        }
        Ok::<(), BackendError>(())
    })?;

    #[cfg(not(target_arch = "wasm32"))]
    let decode_us = t_decode.elapsed().as_micros() as u64;
    #[cfg(target_arch = "wasm32")]
    let decode_us = 0u64;

    let mut stats = if committed {
        sink.commit_frame(last_sync_mount)?
    } else {
        sink.finish_uncommitted()?
    };
    stats.op_count = header.op_count;
    stats.decode_us = decode_us;
    Ok(stats)
}

fn dispatch_extended_raw_prop<S: RenderSink>(
    sink: &mut S,
    node_id: NodeId,
    key: &str,
    value: RawPropValueRef<'_>,
) {
    match key {
        EXT_PROP_LAYOUT_EXT => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(ext) = extended_wire::decode_layout_ext(b) {
                    sink.set_layout_ext(node_id, Some(&ext));
                }
            }
            RawPropValueRef::Null => sink.set_layout_ext(node_id, None),
            _ => {}
        },
        EXT_PROP_TRANSFORM => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(tf) = extended_wire::decode_transform(b) {
                    sink.set_transform(node_id, Some(tf));
                }
            }
            RawPropValueRef::Null => sink.set_transform(node_id, None),
            _ => {}
        },
        EXT_PROP_CURSOR => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(c) = extended_wire::decode_cursor(b) {
                    sink.set_cursor(node_id, Some(c));
                }
            }
            RawPropValueRef::Null => sink.set_cursor(node_id, None),
            _ => {}
        },
        EXT_PROP_STATE_RULES => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(rules) = extended_wire::decode_state_rules(b) {
                    sink.set_state_rules(node_id, &rules);
                }
            }
            RawPropValueRef::Null => sink.set_state_rules(node_id, &[]),
            _ => {}
        },
        EXT_PROP_TRANSITIONS => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(specs) = extended_wire::decode_transitions(b) {
                    sink.set_transitions(node_id, &specs);
                }
            }
            RawPropValueRef::Null => sink.set_transitions(node_id, &[]),
            _ => {}
        },
        EXT_PROP_SHOW_ON => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(q) = extended_wire::decode_media_query(b) {
                    sink.set_show_on(node_id, Some(q));
                }
            }
            RawPropValueRef::Null => sink.set_show_on(node_id, None),
            _ => {}
        },
        EXT_PROP_RESPONSIVE_RULES => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(rules) = extended_wire::decode_responsive_rules(b) {
                    sink.set_responsive_rules(node_id, &rules);
                }
            }
            RawPropValueRef::Null => sink.set_responsive_rules(node_id, &[]),
            _ => {}
        },
        EXT_PROP_BACKGROUND_FILL => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(fill) = extended_wire::decode_background_fill(b) {
                    sink.set_background_fill(node_id, Some(&fill));
                }
            }
            RawPropValueRef::Null => sink.set_background_fill(node_id, None),
            _ => {}
        },
        EXT_PROP_SVG => match value {
            RawPropValueRef::Bytes(b) => {
                if let Some(pic) = extended_wire::decode_svg_picture(b) {
                    sink.set_svg(node_id, Some(&pic));
                }
            }
            RawPropValueRef::Null => sink.set_svg(node_id, None),
            _ => {}
        },
        _ => {}
    }
}
