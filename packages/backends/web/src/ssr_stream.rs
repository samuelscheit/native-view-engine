//! Streaming SSR shell rendering (`StreamBoundary::stream_slot` -> `<slot name="nve-slot-{id}">`),
//! Declarative Shadow DOM parser hygiene validation, and stateful delta subtree extraction
//! ([`SsrStyleTracker`], [`StreamingShellOutput`], [`SsrDeltaChunk`]) over `NodeStore`.

use nve_abi::hash::HashSet;
use nve_components::tables::StreamBoundary;
use nve_store::{NodeId, NodeStore};

use crate::css::NVE_BASE_RESET_CSS;
use crate::html::FOSTER_PARENT_TRAP_TAGS;
use crate::ssr::SsrOutput;
use crate::streaming::{sanitize_stream_slot_id, StreamingError, NVE_STREAM_SLOT_PREFIX};
use crate::stylesheet::{CssWatermark, StyleSheetCache};
use crate::WebBackend;

/// Tracks already-emitted Option 5 Hybrid CSS class hashes across a streaming SSR response so
/// subsequent delta slot chunks emit ONLY newly introduced rules, in O(new rules) per chunk.
#[derive(Default)]
pub struct SsrStyleTracker {
    /// Every rule registered by the shell and earlier chunks (all of them already sent).
    sheet: StyleSheetCache,
    sent: CssWatermark,
}

impl SsrStyleTracker {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Tracker for a client/stream that already holds `hashes` (e.g. a resumed stream).
    pub fn with_sent_hashes<'a>(hashes: impl IntoIterator<Item = &'a str>) -> Self {
        let mut sheet = StyleSheetCache::new();
        sheet.seed_ssr_hashes(hashes);
        let sent = sheet.watermark();
        Self { sheet, sent }
    }

    #[inline]
    pub fn contains(&self, class_name: &str) -> bool {
        self.sheet.knows_structural(class_name)
            || self.sheet.media_seen.contains(class_name)
            || self.sheet.state_seen.contains(class_name)
    }

    /// Number of known rules (sent or seeded).
    #[inline]
    pub fn len(&self) -> usize {
        self.sheet.rule_count()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Serializes rules registered since the last call and marks them sent.
    fn take_unsent_css(&mut self) -> (String, Vec<String>) {
        let out = self.sheet.serialize_css_since(self.sent);
        self.sent = self.sheet.watermark();
        out
    }
}

impl core::fmt::Debug for SsrStyleTracker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SsrStyleTracker")
            .field("rules", &self.len())
            .field("sent", &self.sent)
            .finish()
    }
}

/// One pending `<slot name="nve-slot-{slot_id}">` discovered during
/// [`WebBackend::render_ssr_streaming_shell`], paired with its `data-nve-suspense` parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStreamSlot {
    pub slot_id: String,
    pub parent_node_id: NodeId,
}

/// Mid-stream state of one [`PendingStreamSlot`] (see [`WebBackend::stream_slot_state`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSlotState {
    /// The slot placeholder is still mounted under its suspense parent.
    Pending,
    /// The suspense parent swapped the placeholder for resolved content.
    Resolved,
    /// The suspense parent itself unmounted (e.g. a conditional branch flipped mid-stream).
    Unmounted,
}

/// Output produced by [`WebBackend::render_ssr_streaming_shell`].
#[derive(Debug)]
pub struct StreamingShellOutput {
    pub ssr: SsrOutput,
    pub style_tracker: SsrStyleTracker,
    pub pending_slots: Vec<PendingStreamSlot>,
}

/// Delta HTML and newly introduced Option 5 Hybrid CSS rules produced by
/// [`WebBackend::render_ssr_subtree_delta`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SsrDeltaChunk {
    pub html: String,
    pub style_css: String,
    pub rule_hashes: Vec<String>,
}

/// One open ancestor during the streaming shell walk. `<QuerySuspense>` wrappers
/// (`StreamBoundary::suspense`) are transparent for placement checks: the slot's effective HTML
/// parent is the nearest non-wrapper ancestor.
#[derive(Clone, Copy)]
pub(crate) struct AncestorTag<'s> {
    pub(crate) tag: &'s str,
    pub(crate) is_suspense_wrapper: bool,
}

/// Rejects `stream_slot` placements the HTML parser cannot keep intact inside Declarative Shadow
/// DOM: below `<form>` (association breaks across the shadow boundary) or `<select>` (the parser
/// drops every non-option child, also below `<optgroup>`), or with a table-section foster-parenting
/// trap as the nearest non-wrapper ancestor.
fn check_slot_placement(clean_id: &str, ancestors: &[AncestorTag<'_>]) -> Result<(), StreamingError> {
    let placement = |reason: String| Err(StreamingError::InvalidSlotPlacement(reason));
    for banned in ["form", "select"] {
        if ancestors.iter().any(|a| a.tag == banned) {
            return placement(format!(
                "streaming slot {clean_id:?} cannot be placed inside <{banned}>"
            ));
        }
    }
    if let Some(parent) = ancestors.iter().rev().find(|a| !a.is_suspense_wrapper)
        && FOSTER_PARENT_TRAP_TAGS.contains(&parent.tag)
    {
        return placement(format!(
            "streaming slot {clean_id:?} cannot be a direct child of <{}> (HTML5 foster-parenting trap)",
            parent.tag
        ));
    }
    Ok(())
}

/// Mutable streaming-shell walk state threaded through `WebBackend::emit_ssr_node`.
#[derive(Default)]
pub(crate) struct StreamShellState<'s> {
    pub(crate) ancestor_tags: Vec<AncestorTag<'s>>,
    pending_slots: Vec<PendingStreamSlot>,
    seen_slot_ids: HashSet<String>,
    slot_depth: usize,
}

impl StreamShellState<'_> {
    /// Validates and opens `<slot name="nve-slot-{id}">` when `id` carries a non-empty
    /// `StreamBoundary::stream_slot`. Returns whether a slot was opened.
    pub(crate) fn open_slot(
        &mut self,
        store: &NodeStore,
        id: NodeId,
        out: &mut String,
    ) -> Result<bool, StreamingError> {
        let Some(raw) = store
            .try_row::<StreamBoundary>(id)
            .map(|b| b.stream_slot())
            .filter(|s| !s.is_empty())
        else {
            return Ok(false);
        };
        let placement = |msg: String| Err(StreamingError::InvalidSlotPlacement(msg));
        let Some(clean_id) = sanitize_stream_slot_id(raw) else {
            return Err(StreamingError::InvalidSlotId(raw.to_owned()));
        };
        let parent = store.parent(id).filter(|_| id != NodeId::ROOT);
        let Some(parent_node_id) = parent else {
            return placement(format!(
                "streaming slot {clean_id:?} cannot be placed on the root node"
            ));
        };
        if self.slot_depth > 0 {
            return placement(format!(
                "nested streaming slot {clean_id:?} inside another data-nve-stream-slot is invalid"
            ));
        }
        check_slot_placement(&clean_id, &self.ancestor_tags)?;
        if !self.seen_slot_ids.insert(clean_id.clone()) {
            return placement(format!("duplicate streaming slot id {clean_id:?} in shell"));
        }
        out.push_str("<slot name=\"");
        out.push_str(NVE_STREAM_SLOT_PREFIX);
        out.push_str(&clean_id);
        out.push_str("\">");
        self.pending_slots.push(PendingStreamSlot {
            slot_id: clean_id,
            parent_node_id,
        });
        self.slot_depth += 1;
        Ok(true)
    }

    pub(crate) fn close_slot(&mut self, out: &mut String) {
        self.slot_depth = self.slot_depth.saturating_sub(1);
        out.push_str("</slot>");
    }
}

impl WebBackend {
    /// Emits the Stage-1 Declarative Shadow DOM streaming shell for `store`, wrapping every pending
    /// `data-nve-stream-slot="{id}"` node in `<slot name="nve-slot-{id}">...</slot>` while keeping
    /// the slot node itself inside `<slot>` so mid-stream hydration sees the exact same tree depth
    /// as Client Frame 0.
    ///
    /// Enforces DSD parser hygiene and returns [`StreamingError::InvalidSlotPlacement`] when:
    /// 1. Any ancestor of `data-nve-stream-slot` is `<form>` (Shadow DOM breaks form association)
    ///    or `<select>` (the parser drops non-option content, also below `<optgroup>`).
    /// 2. The nearest non-`<QuerySuspense>`-wrapper ancestor is `table`/`tbody`/`thead`/`tfoot`/`tr`
    ///    (HTML5 foster-parenting trap).
    /// 3. A `data-nve-stream-slot` is nested inside another `data-nve-stream-slot`.
    /// 4. `data-nve-stream-slot` is placed on the root node or duplicates an existing slot ID.
    pub fn render_ssr_streaming_shell(
        &self,
        store: &NodeStore,
    ) -> Result<StreamingShellOutput, StreamingError> {
        let root_hash = self.compute_ssr_tree_hash(store);
        let mut style_tracker = SsrStyleTracker::new();
        let mut html = String::with_capacity(1024);
        let mut state = StreamShellState::default();
        self.emit_ssr_node(
            store,
            NodeId::ROOT,
            root_hash,
            &mut style_tracker.sheet,
            &mut html,
            Some(&mut state),
        )?;
        let (style_css, rule_hashes) = style_tracker.take_unsent_css();
        Ok(StreamingShellOutput {
            ssr: SsrOutput {
                base_css: NVE_BASE_RESET_CSS,
                style_css,
                rule_hashes,
                html,
                root_hash,
            },
            style_tracker,
            pending_slots: state.pending_slots,
        })
    }

    /// Classifies a pending streaming slot against the `<QuerySuspense>` contract (pending state
    /// renders a `stream_slot` child; resolved state swaps the resolved children directly into the
    /// boundary): [`StreamSlotState::Unmounted`] when `parent_node_id` is no longer live,
    /// [`StreamSlotState::Pending`] while a direct child still carries `stream_slot == slot_id`,
    /// [`StreamSlotState::Resolved`] otherwise.
    pub fn stream_slot_state(
        &self,
        store: &NodeStore,
        parent_node_id: NodeId,
        slot_id: &str,
    ) -> StreamSlotState {
        if !store.is_live(parent_node_id) {
            return StreamSlotState::Unmounted;
        }
        let target_clean = sanitize_stream_slot_id(slot_id);
        let pending = store.children(parent_node_id).iter().any(|&cid| {
            store.try_row::<StreamBoundary>(cid).is_some_and(|b| {
                let raw = b.stream_slot();
                !raw.is_empty()
                    && (raw == slot_id
                        || (target_clean.is_some() && sanitize_stream_slot_id(raw) == target_clean))
            })
        });
        if pending {
            StreamSlotState::Pending
        } else {
            StreamSlotState::Resolved
        }
    }

    /// Emits ONLY `parent_node_id`'s direct children subtrees plus ONLY newly introduced Option 5
    /// Hybrid CSS rules not yet recorded in `style_tracker`, recording them as sent.
    pub fn render_ssr_subtree_delta(
        &self,
        store: &NodeStore,
        parent_node_id: NodeId,
        style_tracker: &mut SsrStyleTracker,
    ) -> SsrDeltaChunk {
        if !store.is_live(parent_node_id) {
            return SsrDeltaChunk::default();
        }
        let mut html = String::with_capacity(256);
        for &cid in store.children(parent_node_id) {
            // Infallible without a `StreamShellState` (no slot validation).
            let _ = self.emit_ssr_node(store, cid, 0, &mut style_tracker.sheet, &mut html, None);
        }
        let (style_css, rule_hashes) = style_tracker.take_unsent_css();
        SsrDeltaChunk {
            html,
            style_css,
            rule_hashes,
        }
    }
}
