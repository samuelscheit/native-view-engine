//! Complete accessible component catalog built on `nve-dioxus`.

pub mod actions;
pub mod bottom_sheet;
pub mod combobox;
pub mod combobox_state;
pub mod dropdown;
pub mod error_boundary;
pub mod forms;
pub mod keyboard_avoiding;
pub mod layout;
pub mod overlays;
pub mod split_pane;
pub mod tabs;
pub mod toast;
#[cfg(feature = "virtual-list")]
pub mod virtual_list;

pub use actions::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, IconButton};
pub use bottom_sheet::{
    BottomSheet, BottomSheetState, DragHandoff, SheetDetent, SheetSnapOutcome,
};
pub use combobox::{
    default_combobox_filter, Combobox, ComboboxAction, ComboboxOption, ComboboxState,
};
pub use dropdown::{DropdownMenu, MenuItem};
pub use error_boundary::{
    throw_error, throw_ui_error, ui_error, CapturedError, ErrorBoundary, ErrorBoundaryReset,
    ErrorContext, ErrorFallback, UiError, UiErrorReport,
};
pub use forms::{Checkbox, Input, Switch};
pub use keyboard_avoiding::{
    effective_bottom_inset, use_effective_bottom_inset, use_keyboard_insets, KeyboardAvoidingView,
    KeyboardInsets,
};
pub use layout::{Card, CardVariant, Col, Row, Separator, Stack};
pub use overlays::{Dialog, Modal, Popover, Tooltip};
pub use split_pane::{SplitBound, SplitPane, SplitPaneState};
pub use tabs::{TabList, TabPanel, TabTrigger, TabsRoot};
pub use toast::{
    use_toast, ToastController, ToastItem, ToastLive, ToastOptions, ToastProvider, ToastQueue,
    ToastVariant, ToastViewport,
};
#[cfg(feature = "virtual-list")]
pub use virtual_list::{
    FenwickHeightTree, RowHeights, ScrollAlign, VirtualList, VirtualListState, VirtualSlice,
};
