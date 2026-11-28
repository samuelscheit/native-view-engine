//! Geometry, scroll, focus, and live `TextInput` trait contract for [`crate::AccessKitTreeBridge`].

use std::borrow::Cow;

use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore};
use nve_tables::tables::{is_node_display_none, Accessibility};

/// Deterministic 64-bit `XXH64` digest of a built [`accesskit::Node`] (and its synthetic `TextRun`
/// child, if present) used to skip unchanged nodes during incremental AccessKit updates.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct A11yNodeHash(u64);

impl A11yNodeHash {
    pub const EMPTY: Self = Self(0);

    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Geometry, scroll, focus, and live `TextInput` state queried by [`crate::AccessKitTreeBridge`].
///
/// Implemented by desktop/native hosts (`hosts/skia-shell` over `(&NodeStore, &SkiaBackend)`,
/// `hosts/windows-shell`, headless tests) without coupling `nve-accessibility` to any renderer.
pub trait A11yGeometryProvider {
    /// Device pixel scale factor applied on the root node's [`accesskit::Affine`] transform.
    fn scale_factor(&self) -> f32 {
        1.0
    }

    /// Node layout bounding box in window-origin logical pixels (unscrolled).
    fn layout_rect(&self, id: NodeId) -> Option<LayoutRect>;

    /// Current `(scroll_x, scroll_y)` offset in logical pixels for a `Scroll` container.
    fn scroll_offset(&self, _id: NodeId) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// Total scrollable `(content_width, content_height)` in logical pixels for a `Scroll` node.
    fn scroll_content_size(&self, _id: NodeId) -> Option<(f32, f32)> {
        None
    }

    /// Active `position: sticky` translation `(dx, dy)` in logical pixels, if any.
    fn sticky_delta(&self, _id: NodeId) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// Currently focused [`NodeId`], if any.
    fn focused_node(&self) -> Option<NodeId> {
        None
    }

    /// Live `TextInput` string value (allowing hosts with uncontrolled local editing buffers to
    /// override the store's `TextInput::value`).
    fn text_input_value(&self, _id: NodeId) -> Option<Cow<'_, str>> {
        None
    }

    /// Live `(selection_anchor_byte, caret_byte)` range for a focused `TextInput`.
    fn text_selection_bytes(&self, _id: NodeId) -> Option<(usize, usize)> {
        None
    }

    /// Host-resolved effective `display: none` check (e.g. including `:disabled`/`StateRules`
    /// overrides beyond raw `Layout::display == Display::None`).
    fn is_display_none(&self, _id: NodeId) -> bool {
        false
    }
}

/// Returns `true` when `id` (and its subtree) must be omitted from the accessibility tree because
/// it is dead, `display: none`, hidden `TextInput`, or marked `Accessibility::hidden`.
#[inline]
pub(crate) fn is_node_excluded(
    store: &NodeStore,
    geo: &dyn A11yGeometryProvider,
    id: NodeId,
) -> bool {
    if !store.is_live(id) {
        return true;
    }
    if id == NodeId::ROOT {
        return false;
    }
    if is_node_display_none(store, id) || geo.is_display_none(id) {
        return true;
    }
    store
        .try_row::<Accessibility>(id)
        .is_some_and(|a| a.hidden())
}
