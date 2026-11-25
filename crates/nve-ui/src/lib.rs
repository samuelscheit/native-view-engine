//! Accessible, themeable, zero-external-CSS Dioxus UI component library (`nve-ui`) built directly
//! on `nve-dioxus` primitives and `nve-interop` theme contexts.

pub mod components;
pub mod focus;
#[cfg(feature = "spatial-focus")]
pub mod spatial_focus;
pub mod style_buf;
pub mod theme;

#[cfg(feature = "virtual-list")]
pub use components::{
    FenwickHeightTree, RowHeights, ScrollAlign, VirtualList, VirtualListState, VirtualSlice,
};
pub use components::{
    use_toast, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Checkbox,
    Col, Dialog, DropdownMenu, IconButton, Input, MenuItem, Modal, Popover, Row, Separator, Stack,
    Switch, TabList, TabPanel, TabTrigger, TabsRoot, ToastController, ToastItem, ToastLive,
    ToastOptions, ToastProvider, ToastQueue, ToastVariant, ToastViewport, Tooltip,
};
pub use focus::{
    current_focused_id, notify_focus_scope, use_focus_scope, FocusScope, FocusScopeProvider,
    FocusTrap, FocusTrapAction, FocusableEntry, Orientation, RovingFocus,
};
#[cfg(feature = "spatial-focus")]
pub use spatial_focus::{
    FocusDirection, RemoteAction, ScrollAdjustment, ScrollRegion, SpatialFocusEngine,
    SpatialFocusGroupId, SpatialFocusNode, SpatialRect,
};
pub use nve_dioxus::TypedStyle;
pub use style_buf::{
    parse_color_rgba, StyleBuf, SurfaceStyle, TextStyle, TypedStyleBuilder, TypedSurfaceStyle,
    TypedTextStyle,
};
pub use theme::{
    rgba_to_hex, use_theme, use_theme_rgba, use_theme_signal, RadiiScale, SpacingScale,
    ThemeProvider, TypographyScale, UiDensity, UiPalette, UiPaletteRgba, UiTheme,
};

pub mod prelude {
    pub use dioxus_core_macro::Props;
    pub use nve_dioxus::TypedStyle;
    #[cfg(feature = "virtual-list")]
    pub use crate::components::{
        FenwickHeightTree, RowHeights, ScrollAlign, VirtualList, VirtualListState, VirtualSlice,
    };
    pub use crate::components::{
        use_toast, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant,
        Checkbox, Col, Dialog, DropdownMenu, IconButton, Input, MenuItem, Modal, Popover, Row,
        Separator, Stack, Switch, TabList, TabPanel, TabTrigger, TabsRoot, ToastController,
        ToastItem, ToastLive, ToastOptions, ToastProvider, ToastQueue, ToastVariant, ToastViewport,
        Tooltip,
    };
    pub use crate::focus::{
        use_focus_scope, FocusScope, FocusScopeProvider, FocusTrap, FocusTrapAction,
        FocusableEntry, Orientation, RovingFocus,
    };
    #[cfg(feature = "spatial-focus")]
    pub use crate::spatial_focus::{
        FocusDirection, RemoteAction, ScrollAdjustment, ScrollRegion, SpatialFocusEngine,
        SpatialFocusGroupId, SpatialFocusNode, SpatialRect,
    };
    pub use crate::style_buf::{
        parse_color_rgba, StyleBuf, SurfaceStyle, TextStyle, TypedStyleBuilder, TypedSurfaceStyle,
        TypedTextStyle,
    };
    pub use crate::theme::{
        rgba_to_hex, use_theme, use_theme_rgba, use_theme_signal, RadiiScale, SpacingScale,
        ThemeProvider, TypographyScale, UiDensity, UiPalette, UiPaletteRgba, UiTheme,
    };
}
