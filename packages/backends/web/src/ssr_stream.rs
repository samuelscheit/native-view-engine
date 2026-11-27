//! Streaming SSR shell rendering (`data-nve-stream-slot` -> `<slot name="nve-slot-{id}">`),
//! Declarative Shadow DOM parser hygiene validation, and stateful delta subtree extraction
//! ([`SsrStyleTracker`], [`StreamingShellOutput`], [`SsrDeltaChunk`]).

use nve_abi::hash::HashSet;
use nve_abi::{ComponentKind, NodeId, RawPropValue, ROOT_NODE_ID};

use crate::css::{resolve_semantic_tag, NVE_BASE_RESET_CSS};
use crate::html::close_html_element;
use crate::ssr::{SsrOutput, SsrStyleCollector};
use crate::streaming::{sanitize_stream_slot_id, StreamingError, NVE_STREAM_SLOT_PREFIX};
use crate::tree::Visit;
use crate::WebBackend;

/// Tracks already-emitted Option 5 Hybrid CSS class hashes across a streaming SSR response so
/// subsequent delta slot chunks emit ONLY newly introduced rules.
#[derive(Debug, Clone, Default)]
pub struct SsrStyleTracker {
    pub sent_hashes: HashSet<String>,
}

impl SsrStyleTracker {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn contains(&self, class_name: &str) -> bool {
        self.sent_hashes.contains(class_name)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.sent_hashes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.sent_hashes.is_empty()
    }
}

/// One pending `<slot name="nve-slot-{slot_id}">` discovered during
/// [`WebBackend::render_ssr_streaming_shell`], paired with its `<div data-nve-suspense>` parent.
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
#[derive(Debug, Clone)]
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

/// HTML5 table-section tags whose non-cell children the parser foster-parents out of place.
#[inline]
fn is_foster_parent_trap_tag(tag: &str) -> bool {
    matches!(tag, "table" | "tbody" | "thead" | "tfoot" | "tr")
}

/// One open ancestor during the streaming shell walk. `<QuerySuspense>` wrappers
/// (`data-nve-suspense`) are transparent for placement checks: the slot's effective HTML parent
/// is the nearest non-wrapper ancestor.
#[derive(Clone, Copy)]
struct AncestorTag<'a> {
    tag: &'a str,
    is_suspense_wrapper: bool,
}

/// Rejects `data-nve-stream-slot` placements the HTML parser cannot keep intact inside
/// Declarative Shadow DOM.
fn check_slot_placement(clean_id: &str, ancestors: &[AncestorTag<'_>]) -> Result<(), StreamingError> {
    let placement = |reason: String| Err(StreamingError::InvalidSlotPlacement(reason));
    // `<form>` association breaks across the shadow boundary; `<select>` (and `<optgroup>`
    // inside it) drops every non-option child, anywhere below it.
    for banned in ["form", "select"] {
        if ancestors.iter().any(|a| a.tag == banned) {
            return placement(format!(
                "streaming slot {clean_id:?} cannot be placed inside <{banned}>"
            ));
        }
    }
    if let Some(parent) = ancestors.iter().rev().find(|a| !a.is_suspense_wrapper)
        && is_foster_parent_trap_tag(parent.tag)
    {
        return placement(format!(
            "streaming slot {clean_id:?} cannot be a direct child of <{}> (HTML5 foster-parenting trap)",
            parent.tag
        ));
    }
    Ok(())
}

struct OpenStreamingFrame<'a> {
    tag: &'a str,
    wrap_code: bool,
    wrapped_in_slot: bool,
}

impl WebBackend {
    /// Emits the Stage-1 Declarative Shadow DOM streaming shell starting at `ROOT_NODE_ID`,
    /// wrapping every pending `data-nve-stream-slot="{id}"` node in
    /// `<slot name="nve-slot-{id}">...</slot>` while keeping the slot node itself inside `<slot>`
    /// so mid-stream WASM hydration sees the exact same tree depth as Client Frame 0.
    ///
    /// Enforces DSD parser hygiene and returns [`StreamingError::InvalidSlotPlacement`] when:
    /// 1. Any ancestor of `data-nve-stream-slot` is `<form>` (Shadow DOM breaks form association)
    ///    or `<select>` (the parser drops non-option content, also below `<optgroup>`).
    /// 2. The nearest non-`<QuerySuspense>`-wrapper ancestor is `table`/`tbody`/`thead`/`tfoot`/`tr`
    ///    (HTML5 foster-parenting trap).
    /// 3. A `data-nve-stream-slot` is nested inside another `data-nve-stream-slot`.
    /// 4. `data-nve-stream-slot` is placed on `ROOT_NODE_ID` or duplicates an existing slot ID.
    pub fn render_ssr_streaming_shell(&self) -> Result<StreamingShellOutput, StreamingError> {
        let root_hash = self.compute_ssr_tree_hash();
        let mut styles = SsrStyleCollector::new(None);
        let mut html = String::with_capacity(self.nodes.len().saturating_mul(104).max(64));
        let mut open: Vec<OpenStreamingFrame<'_>> = Vec::new();
        let mut ancestor_tags: Vec<AncestorTag<'_>> = Vec::new();
        let mut pending_slots: Vec<PendingStreamSlot> = Vec::new();
        let mut seen_slot_ids: HashSet<String> = HashSet::default();
        let mut slot_depth = 0usize;
        let mut error: Option<StreamingError> = None;

        self.walk(ROOT_NODE_ID, |id, parent, step| {
            if error.is_some() {
                return false;
            }
            match step {
                Visit::Enter => {
                    let Some(el) = self.nodes.get(&id) else {
                        return false;
                    };
                    let sem = resolve_semantic_tag(el.kind, &el.raw_props);
                    let tag = if el.kind == ComponentKind::Custom {
                        self.effective_html_tag(id, el)
                    } else {
                        sem.tag
                    };

                    let mut wrapped_in_slot = false;
                    if let Some(RawPropValue::String(raw_slot_id)) =
                        el.raw_props.get("data-nve-stream-slot")
                    {
                        let Some(clean_id) = sanitize_stream_slot_id(raw_slot_id) else {
                            error = Some(StreamingError::InvalidSlotId(raw_slot_id.clone()));
                            return false;
                        };
                        let Some(parent_node_id) = parent else {
                            error = Some(StreamingError::InvalidSlotPlacement(format!(
                                "streaming slot {clean_id:?} cannot be placed on ROOT_NODE_ID"
                            )));
                            return false;
                        };
                        if slot_depth > 0 {
                            error = Some(StreamingError::InvalidSlotPlacement(format!(
                                "nested streaming slot {clean_id:?} inside another data-nve-stream-slot is invalid"
                            )));
                            return false;
                        }
                        if let Err(err) = check_slot_placement(&clean_id, &ancestor_tags) {
                            error = Some(err);
                            return false;
                        }
                        if !seen_slot_ids.insert(clean_id.clone()) {
                            error = Some(StreamingError::InvalidSlotPlacement(format!(
                                "duplicate streaming slot id {clean_id:?} in shell"
                            )));
                            return false;
                        }

                        html.push_str("<slot name=\"");
                        html.push_str(NVE_STREAM_SLOT_PREFIX);
                        html.push_str(&clean_id);
                        html.push_str("\">");
                        pending_slots.push(PendingStreamSlot {
                            slot_id: clean_id,
                            parent_node_id,
                        });
                        wrapped_in_slot = true;
                    }

                    match self.open_ssr_element(id, parent, Some(root_hash), &mut styles, &mut html)
                    {
                        Some((opened_tag, wrap_code)) => {
                            ancestor_tags.push(AncestorTag {
                                tag,
                                is_suspense_wrapper: el.raw_props.contains_key("data-nve-suspense"),
                            });
                            if wrapped_in_slot {
                                slot_depth += 1;
                            }
                            open.push(OpenStreamingFrame {
                                tag: opened_tag,
                                wrap_code,
                                wrapped_in_slot,
                            });
                            true
                        }
                        None => {
                            if wrapped_in_slot {
                                html.push_str("</slot>");
                            }
                            false
                        }
                    }
                }
                Visit::Exit => {
                    let _ = ancestor_tags.pop();
                    if let Some(frame) = open.pop() {
                        if frame.wrap_code {
                            html.push_str("</code>");
                        }
                        close_html_element(frame.tag, &mut html);
                        if frame.wrapped_in_slot {
                            slot_depth = slot_depth.saturating_sub(1);
                            html.push_str("</slot>");
                        }
                    }
                    false
                }
            }
        });

        if let Some(err) = error {
            return Err(err);
        }

        let (style_css, rule_hashes) = styles.finish();
        let mut style_tracker = SsrStyleTracker::new();
        for hash in &rule_hashes {
            style_tracker.sent_hashes.insert(hash.clone());
        }

        Ok(StreamingShellOutput {
            ssr: SsrOutput {
                base_css: NVE_BASE_RESET_CSS,
                style_css,
                rule_hashes,
                html,
                root_hash,
            },
            style_tracker,
            pending_slots,
        })
    }

    /// Classifies a pending streaming slot against the `<QuerySuspense>` DOM contract (pending
    /// state renders `<div data-nve-stream-slot="{id}">`; resolved state swaps the resolved
    /// children directly inside `<div data-nve-suspense="{id}">`):
    /// [`StreamSlotState::Unmounted`] when `parent_node_id` no longer exists,
    /// [`StreamSlotState::Pending`] while a direct child still carries `data-nve-stream-slot ==
    /// slot_id`, [`StreamSlotState::Resolved`] otherwise.
    pub fn stream_slot_state(&self, parent_node_id: NodeId, slot_id: &str) -> StreamSlotState {
        let Some(parent) = self.nodes.get(&parent_node_id) else {
            return StreamSlotState::Unmounted;
        };
        let target_clean = sanitize_stream_slot_id(slot_id);
        let pending = parent.children.iter().any(|cid| {
            self.nodes.get(cid).is_some_and(|child| {
                matches!(
                    child.raw_props.get("data-nve-stream-slot"),
                    Some(RawPropValue::String(raw))
                        if raw == slot_id
                            || (target_clean.is_some()
                                && sanitize_stream_slot_id(raw) == target_clean)
                )
            })
        });
        if pending {
            StreamSlotState::Pending
        } else {
            StreamSlotState::Resolved
        }
    }

    /// Walks ONLY `parent_node_id`'s direct children and emits their HTML plus ONLY newly
    /// introduced Option 5 Hybrid CSS rules not yet recorded in `style_tracker.sent_hashes`.
    pub fn render_ssr_subtree_delta(
        &self,
        parent_node_id: NodeId,
        style_tracker: &mut SsrStyleTracker,
    ) -> SsrDeltaChunk {
        let Some(parent) = self.nodes.get(&parent_node_id) else {
            return SsrDeltaChunk::default();
        };

        let mut styles = SsrStyleCollector::new(Some(&style_tracker.sent_hashes));
        let mut html = String::with_capacity(parent.children.len().saturating_mul(96).max(64));
        let mut open: Vec<(&str, bool)> = Vec::new();
        let mut stack: Vec<(NodeId, Option<NodeId>, Visit)> = Vec::new();

        for &child_id in parent.children.iter().rev() {
            stack.push((child_id, Some(parent_node_id), Visit::Enter));
        }

        while let Some((id, p_id, step)) = stack.pop() {
            match step {
                Visit::Enter => {
                    let Some(el) = self.nodes.get(&id) else {
                        continue;
                    };
                    if let Some(closing) =
                        self.open_ssr_element(id, p_id, None, &mut styles, &mut html)
                    {
                        open.push(closing);
                        stack.push((id, p_id, Visit::Exit));
                        stack.extend(
                            el.children
                                .iter()
                                .rev()
                                .map(|&cid| (cid, Some(id), Visit::Enter)),
                        );
                    }
                }
                Visit::Exit => {
                    let (tag, wrap_code) = open.pop().unwrap_or(("div", false));
                    if wrap_code {
                        html.push_str("</code>");
                    }
                    close_html_element(tag, &mut html);
                }
            }
        }

        let (style_css, rule_hashes) = styles.finish();
        for hash in &rule_hashes {
            style_tracker.sent_hashes.insert(hash.clone());
        }

        SsrDeltaChunk {
            html,
            style_css,
            rule_hashes,
        }
    }
}
