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
    InteractionRef,
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

use nve_store::{NveTable, RealizationOrder};

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
    let _ = Link::slot();
    let _ = Scroll::slot();
    let _ = Image::slot();
    let _ = TextInput::slot();
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
