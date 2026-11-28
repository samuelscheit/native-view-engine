//! Geometry, scroll, focus, and live `TextInput` trait contract for [`crate::AccessKitTreeBridge`].

use std::borrow::Cow;

use accesskit::Affine;
use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore};
use nve_tables::tables::{is_display_contents, is_node_display_none, Accessibility, Transform};
use nve_tables::transform::Transform2D;

/// Converts a 2D node [`Transform2D`] anchored at `rect` into an AccessKit [`Affine`] matrix in
/// logical surface coordinates.
#[inline]
pub fn transform_2d_to_affine(tf: Transform2D, rect: LayoutRect) -> Affine {
    let m = tf.to_surface_matrix_2d(rect.x, rect.y, rect.width, rect.height);
    Affine::new([
        f64::from(m[0]),
        f64::from(m[1]),
        f64::from(m[2]),
        f64::from(m[3]),
        f64::from(m[4]),
        f64::from(m[5]),
    ])
}

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

    /// Visual child shift `(shift_x, shift_y)` in logical pixels for a `Scroll` container
    /// (subtracting any virtualized `Scroll::virtual_top_spacer`). Returns `None` to derive
    /// automatically from `scroll_offset(id)` and `Scroll::virtual_top_spacer`.
    fn visual_scroll_shift(&self, _id: NodeId) -> Option<(f32, f32)> {
        None
    }

    /// Total scrollable `(content_width, content_height)` in logical pixels for a `Scroll` node.
    fn scroll_content_size(&self, _id: NodeId) -> Option<(f32, f32)> {
        None
    }

    /// Active `position: sticky` translation `(dx, dy)` in logical pixels, if any.
    fn sticky_delta(&self, _id: NodeId) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// Optional effective 2D affine transform (e.g. animated or static `Transform2D`) in logical
    /// coordinates, composed with sticky/scroll/scale transforms.
    ///
    /// Defaults to projecting any non-identity static [`Transform`] row from `store` against
    /// [`Self::layout_rect`]. Hosts with dynamic `StateRules` or frame-hook animations override
    /// this as the single source of truth.
    fn node_affine(&self, store: &NodeStore, id: NodeId) -> Option<Affine> {
        let t = store.try_row::<Transform>(id)?;
        let tf = Transform2D {
            translate_x: t.translate_x(),
            translate_y: t.translate_y(),
            scale_x: t.scale_x(),
            scale_y: t.scale_y(),
            rotate_deg: t.rotate_deg(),
            origin_x: t.origin_x(),
            origin_y: t.origin_y(),
        };
        if tf.is_identity() {
            return None;
        }
        Some(transform_2d_to_affine(tf, self.layout_rect(id)?))
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
    /// overrides and `InputKind::Hidden`). Defaults to static [`is_node_display_none`]; hosts with
    /// dynamic `StateRules` override this as the single source of truth so a rule restoring a node
    /// from static `Display::None` is respected.
    fn is_display_none(&self, store: &NodeStore, id: NodeId) -> bool {
        is_node_display_none(store, id)
    }

    /// Host-resolved effective `display: contents` check (e.g. including `StateRules` overrides).
    /// Defaults to static [`is_display_contents`]; hosts with dynamic `StateRules` override this
    /// as the single source of truth so a rule restoring a node from static `Display::Contents` is
    /// respected.
    fn is_display_contents(&self, store: &NodeStore, id: NodeId) -> bool {
        is_display_contents(store, id)
    }
}

/// Returns `true` when `id` (and its subtree) must be omitted from the accessibility tree because
/// it is dead, effectively `display: none`, hidden `TextInput`, or marked `Accessibility::hidden`.
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
    if geo.is_display_none(store, id) {
        return true;
    }
    store
        .try_row::<Accessibility>(id)
        .is_some_and(|a| a.hidden())
}

/// Returns `true` when `id` is a non-root effectively `display: contents` node regardless of
/// `aria-hidden`.
#[inline]
pub(crate) fn is_node_display_contents_any(
    store: &NodeStore,
    geo: &dyn A11yGeometryProvider,
    id: NodeId,
) -> bool {
    id != NodeId::ROOT && geo.is_display_contents(store, id)
}

/// Returns `true` when `id` is a non-root, non-excluded `display: contents` node whose visible
/// children should be flattened transparently into its box-generating accessibility parent.
#[inline]
pub(crate) fn is_node_display_contents_transparent(
    store: &NodeStore,
    geo: &dyn A11yGeometryProvider,
    id: NodeId,
) -> bool {
    !is_node_excluded(store, geo, id) && is_node_display_contents_any(store, geo, id)
}
