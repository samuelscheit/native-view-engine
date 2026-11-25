//! Accessible, themeable, zero-external-CSS Dioxus UI component library (`nve-ui`) built directly
//! on `nve-dioxus` primitives and `nve-interop` theme contexts.

pub mod components;
pub mod focus;
pub mod style_buf;
pub mod theme;

pub use components::{
    use_toast, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant, Checkbox,
    Col, Dialog, DropdownMenu, FenwickHeightTree, IconButton, Input, MenuItem, Modal, Popover, Row,
    RowHeights, ScrollAlign, Separator, Stack, Switch, TabList, TabPanel, TabTrigger, TabsRoot,
    ToastController, ToastItem, ToastLive, ToastOptions, ToastProvider, ToastQueue, ToastVariant,
    ToastViewport, Tooltip, VirtualList, VirtualListState, VirtualSlice,
};
pub use focus::{
    current_focused_id, notify_focus_scope, use_focus_scope, FocusScope, FocusScopeProvider,
    FocusTrap, FocusTrapAction, FocusableEntry, Orientation, RovingFocus,
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
    pub use crate::components::{
        use_toast, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Card, CardVariant,
        Checkbox, Col, Dialog, DropdownMenu, FenwickHeightTree, IconButton, Input, MenuItem, Modal,
        Popover, Row, RowHeights, ScrollAlign, Separator, Stack, Switch, TabList, TabPanel,
        TabTrigger, TabsRoot, ToastController, ToastItem, ToastLive, ToastOptions, ToastProvider,
        ToastQueue, ToastVariant, ToastViewport, Tooltip, VirtualList, VirtualListState,
        VirtualSlice,
    };
    pub use crate::focus::{
        use_focus_scope, FocusScope, FocusScopeProvider, FocusTrap, FocusTrapAction,
        FocusableEntry, Orientation, RovingFocus,
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
