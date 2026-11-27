//! Core `#[derive(NveTable)]` word tables, shared resource codecs, and realization priority order.
//!
//! Every node in `NodeStore` is a bare `NodeId` with attached tables. Backends derive a uniform
//! `primary_slot: TableSlot` from attached tables using [`core_realization_order`]; modules
//! (SVG, Canvas, Blur, WebView, Video, Camera, WebElement, WebHtml, 3P plugins) define their own
//! tables in standalone crates under `packages/modules/<cap>/`.

pub mod box_shadow;
pub mod interaction;
pub mod layout;
pub mod media;
pub mod resources;
pub(crate) mod style_patch_codec;
pub mod visual;

#[cfg(test)]
mod tests;

pub use box_shadow::{parse_box_shadow, write_box_shadows_css, BoxShadow};
pub use interaction::{
    Accessibility, AccessibilityMut, AccessibilityRef, AriaAutoComplete, AriaLive, AriaOrientation,
    AriaRole, AriaTriState, DebugId, DebugIdMut, DebugIdRef, Interaction, InteractionMut,
    InteractionRef, StreamBoundary, StreamBoundaryMut, StreamBoundaryRef, StreamSlotRole,
    NVE_STREAM_SLOT_ATTR, NVE_STREAM_SLOT_ROLE_ATTR, NVE_SUSPENSE_ATTR,
};
pub use layout::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, Direction, Display, FlexDirection, FlexWrap,
    GridItem, GridItemMut, GridItemRef, GridLayout, GridLayoutMut, GridLayoutRef, GridLine,
    JustifyContent, Layout, LayoutMut, LayoutRef, Position,
};
pub use media::{
    Image, ImageFit, ImageMut, ImageRef, InputKind, Link, LinkMut, LinkRef, LinkTarget,
    OptAutofillHint, Scroll, ScrollAxes, ScrollMut, ScrollRef, Scrollbars, TextInput, TextInputMut,
    TextInputRef,
};
pub use resources::{
    AriaRelations, BackgroundFillRes, BoxShadows, GridTracks, ImageSrc, ResponsiveRules,
    StateRules, Transitions,
};
pub use visual::{
    text_decoration, FontStyle, Motion, MotionMut, MotionRef, Overflow, OverflowWrap, Responsive,
    ResponsiveMut, ResponsiveRef, TextAlign, TextContent, TextContentMut, TextContentRef,
    Transform, TransformMut, TransformRef, Typography, TypographyMut, TypographyRef, VerticalAlign,
    Visibility, Visual, VisualMut, VisualRef, WhiteSpace, WordBreak,
};

use nve_store::{NveTable, RealizationOrder, TableDesc};

/// `TableDesc` of every core table, in registration order: the input of `nve codegen abi`, which
/// emits their keys and field specs for the static host runtimes.
pub const CORE_TABLE_DESCS: &[&TableDesc] = &[
    &Layout::DESC,
    &GridLayout::DESC,
    &GridItem::DESC,
    &Visual::DESC,
    &Typography::DESC,
    &TextContent::DESC,
    &Transform::DESC,
    &Motion::DESC,
    &Responsive::DESC,
    &Interaction::DESC,
    &Accessibility::DESC,
    &DebugId::DESC,
    &StreamBoundary::DESC,
    &Link::DESC,
    &Scroll::DESC,
    &Image::DESC,
    &TextInput::DESC,
];

/// Canonical descriptor list of all 17 cross-platform core tables.
pub const CORE_TABLE_DESCS: &[&nve_store::TableDesc] = &[
    &Layout::DESC,
    &GridLayout::DESC,
    &GridItem::DESC,
    &Visual::DESC,
    &Typography::DESC,
    &TextContent::DESC,
    &Transform::DESC,
    &Motion::DESC,
    &Responsive::DESC,
    &Interaction::DESC,
    &Accessibility::DESC,
    &DebugId::DESC,
    &StreamBoundary::DESC,
    &Link::DESC,
    &Scroll::DESC,
    &Image::DESC,
    &TextInput::DESC,
];

/// Deterministic 64-bit XXH64 schema digest over [`CORE_TABLE_DESCS`].
///
/// Independent of process-global lazy table registration order or optional module tables (such as
/// `Canvas` or `Svg`), so cold Frame-0 pre-warm always matches saved `frame0.nve2` footers.
pub fn core_tables_schema_hash() -> u64 {
    nve_store::tables_schema_hash(CORE_TABLE_DESCS)
}

/// Registers every cross-platform core table and its resource codecs in the process-global table registry.
/// Idempotent and fast after the first call.
pub fn register_core_tables() {
    let _ = Layout::slot();
    let _ = GridLayout::slot();
    let _ = GridItem::slot();
    let _ = Visual::slot();
    let _ = Typography::slot();
    let _ = TextContent::slot();
    let _ = Transform::slot();
    let _ = Motion::slot();
    let _ = Responsive::slot();
    let _ = Interaction::slot();
    let _ = Accessibility::slot();
    let _ = DebugId::slot();
    let _ = StreamBoundary::slot();
    let _ = Link::slot();
    let _ = Scroll::slot();
    let _ = Image::slot();
    let _ = TextInput::slot();
}

/// Returns `true` when `id` carries a [`TextInput`] row with [`InputKind::Hidden`].
#[inline]
pub fn is_hidden_text_input(store: &nve_store::NodeStore, id: nve_store::NodeId) -> bool {
    store
        .try_row::<TextInput>(id)
        .is_some_and(|t| t.input_kind() == InputKind::Hidden)
}

/// Returns `true` when `id` is excluded from layout, paint, hit-testing, and focus
/// (`Layout::display == Display::None` or `TextInput::input_kind == InputKind::Hidden`).
#[inline]
pub fn is_node_display_none(store: &nve_store::NodeStore, id: nve_store::NodeId) -> bool {
    store
        .try_row::<Layout>(id)
        .is_some_and(|l| l.display() == Display::None)
        || is_hidden_text_input(store, id)
}

/// `display: contents`: `id` generates no box; its children are laid out, painted, and hit-tested
/// as children of its parent box. A `display: none` node (including a hidden [`TextInput`]) is
/// never contents: it removes its whole subtree instead.
#[inline]
pub fn is_display_contents(store: &nve_store::NodeStore, id: nve_store::NodeId) -> bool {
    store
        .try_row::<Layout>(id)
        .is_some_and(|l| l.display() == Display::Contents)
        && !is_hidden_text_input(store, id)
}

/// Whether `id` generates a box: every node except non-root `display: contents` nodes (the
/// layout root always generates the viewport box). `display: none` nodes are *not* excluded here:
/// they and their subtree are skipped separately via [`is_node_display_none`].
#[inline]
pub fn generates_box(store: &nve_store::NodeStore, id: nve_store::NodeId) -> bool {
    id == nve_store::NodeId::ROOT || !is_display_contents(store, id)
}

/// `id`'s [`Visual`] row when `id` paints a box: a box-less node has no background, border,
/// opacity, stacking, shadows, transform, or clip; inherited text styling is unaffected.
#[inline]
pub fn box_visual(store: &nve_store::NodeStore, id: nve_store::NodeId) -> Option<VisualRef<'_>> {
    store
        .try_row::<Visual>(id)
        .filter(|_| generates_box(store, id))
}

/// Whether `id`'s box clips its descendants (`overflow` hidden / scroll / auto on either axis).
#[inline]
pub fn node_clips_overflow(store: &nve_store::NodeStore, id: nve_store::NodeId) -> bool {
    box_visual(store, id).is_some_and(|v| visual_clips_overflow(&v))
}

/// Whether a box with this [`Visual`] row clips its descendants; for callers that already know
/// the node generates a box.
#[inline]
pub fn visual_clips_overflow(v: &VisualRef<'_>) -> bool {
    let clips = |o: Overflow| matches!(o, Overflow::Hidden | Overflow::Scroll | Overflow::Auto);
    clips(v.overflow_x()) || clips(v.overflow_y())
}

/// `v` resolved for `id` (e.g. after state rules) with box paint dropped when `id` generates no
/// box, so no pseudo-state can paint, fade, or cull a box-less node's subtree.
#[inline]
pub fn paint_of_box(
    store: &nve_store::NodeStore,
    id: nve_store::NodeId,
    v: crate::style_patch::VisualSnapshot,
) -> crate::style_patch::VisualSnapshot {
    if generates_box(store, id) {
        v
    } else {
        v.without_box_paint()
    }
}

/// Returns `true` when `id` or any ancestor is excluded from layout, paint, hit-testing, and focus
/// (`Layout::display == Display::None` or `TextInput::input_kind == InputKind::Hidden`).
#[inline]
pub fn is_subtree_display_none(store: &nve_store::NodeStore, id: nve_store::NodeId) -> bool {
    let mut cur = id;
    for _ in 0..256 {
        if is_node_display_none(store, cur) {
            return true;
        }
        let Some(parent) = store.parent(cur) else {
            break;
        };
        if parent == cur {
            break;
        }
        cur = parent;
    }
    false
}

/// Default cross-platform core realization priority order (`Image > TextInput > Scroll > Link > TextContent`).
/// Backends/hosts prepend any higher-priority module tables (`Svg`, `Canvas`, `WebView`, `Video`,
/// `Camera`, `WebHtml`, 3P tables) before installing onto a [`nve_store::NodeStore`].
pub fn core_realization_order() -> RealizationOrder {
    RealizationOrder::new()
        .with::<Image>()
        .with::<TextInput>()
        .with::<Scroll>()
        .with::<Link>()
        .with::<TextContent>()
}
