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
pub mod overlay;
pub mod resources;
pub mod visual;

#[cfg(test)]
mod tests;

pub use box_shadow::BoxShadow;
pub use interaction::{
    Accessibility, AccessibilityMut, AccessibilityRef, AriaAutoComplete, AriaLive, AriaOrientation,
    AriaRole, AriaTriState, CursorKind, DebugId, DebugIdMut, DebugIdRef, Interaction, InteractionMut,
    InteractionRef, PointerCaptureMode, NVE_STREAM_SLOT_ATTR, NVE_STREAM_SLOT_ROLE_ATTR,
    NVE_SUSPENSE_ATTR, StreamBoundary, StreamBoundaryMut, StreamBoundaryRef, StreamSlotRole,
};
pub use layout::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, Direction, Display, FlexDirection, FlexWrap,
    GridItem, GridItemMut, GridItemRef, GridLayout, GridLayoutMut, GridLayoutRef, GridLine,
    JustifyContent, Layout, LayoutMut, LayoutRef, Position,
};
pub use media::{
    Image, ImageFit, ImageMut, ImageRef, InputKind, Link, LinkMut, LinkRef, LinkRel, LinkTarget,
    Scroll, ScrollAxes, ScrollMut, ScrollRef, Scrollbars, TextInput, TextInputMut, TextInputRef,
};
pub use overlay::{
    Alignment, Overlay, OverlayFlags, OverlayLayer, OverlayMut, OverlayPlacement, OverlayRef, Side,
};
pub use resources::{
    AriaRelations, AriaRelationsView, BackgroundFillRes, BackgroundFillView, BoxShadows,
    BoxShadowsIter, BoxShadowsView, GradientStopsIter, GradientStopsView, GridTracks,
    GridTracksIter, GridTracksView, ImageSrc, ImageSrcView, ResponsiveRules, ResponsiveRulesIter,
    ResponsiveRulesView, StateRules, StateRulesIter, StateRulesView,
};
pub use visual::{
    FontStyle, Overflow, OverflowWrap, Responsive, ResponsiveMut,
    ResponsiveRef, TextAlign, TextContent, TextContentMut, TextContentRef, TextDecoration,
    Transform, TransformMut, TransformRef, Typography, TypographyMut, TypographyRef, VerticalAlign,
    Visibility, Visual, VisualMut, VisualRef, WhiteSpace, WordBreak,
};

pub use nve_abi::{EventSet, InteractionEvent, LayoutEvent, ScrollEvent, TextInputEvent};
use nve_store::{NveTable, RealizationOrder};

const _: () = {
    assert!(Interaction::KEY.0 == nve_abi::NVE_TABLE_INTERACTION.0);
    assert!(TextInput::KEY.0 == nve_abi::NVE_TABLE_TEXT_INPUT.0);
    assert!(Scroll::KEY.0 == nve_abi::NVE_TABLE_SCROLL.0);
    assert!(Layout::KEY.0 == nve_abi::NVE_TABLE_LAYOUT.0);
};

macro_rules! define_core_tables {
    ($($table:ident),+ $(,)?) => {
        /// Canonical descriptor list of every cross-platform core table.
        pub const CORE_TABLE_DESCS: &[&nve_store::TableDesc] = &[$(&$table::DESC),+];

        /// Registers every cross-platform core table in the process-global table registry.
        /// Idempotent and fast after the first call.
        pub fn register_core_tables() {
            $(let _ = $table::slot();)+
        }
    };
}

define_core_tables!(
    Layout,
    GridLayout,
    GridItem,
    Visual,
    Typography,
    TextContent,
    Transform,
    Responsive,
    Interaction,
    Accessibility,
    DebugId,
    StreamBoundary,
    Link,
    Scroll,
    Image,
    TextInput,
    Overlay,
);

/// Deterministic 64-bit XXH64 schema digest over [`CORE_TABLE_DESCS`].
///
/// Independent of process-global lazy table registration order or optional module tables (such as
/// `Canvas` or `Svg`), so cold Frame-0 pre-warm always matches saved `frame0.nve2` footers.
pub fn core_tables_schema_hash() -> u64 {
    nve_store::tables_schema_hash(CORE_TABLE_DESCS)
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
    v: crate::snapshots::VisualSnapshot,
) -> crate::snapshots::VisualSnapshot {
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
///
/// Also registers every core table first: replicas are built with this order, and a wire
/// declaration for a core table must resolve to the static slot typed lookups use — not to a
/// dynamic slot because this process happened not to touch that table yet.
pub fn core_realization_order() -> RealizationOrder {
    register_core_tables();
    RealizationOrder::new()
        .with::<Image>()
        .with::<TextInput>()
        .with::<Scroll>()
        .with::<Link>()
        .with::<TextContent>()
}
