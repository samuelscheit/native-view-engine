//! Accessible, themeable, zero-external-CSS Dioxus UI component library (`nve-ui`) built directly
//! on `nve-dioxus` primitives and `nve-interop` theme contexts.

pub mod components;
pub mod focus;
pub mod form;
pub mod form_controller;
pub mod form_draft;
pub mod history;
pub mod positioning;
#[cfg(feature = "spatial-focus")]
pub mod spatial_focus;
pub mod style_buf;
pub mod theme;

#[cfg(feature = "virtual-list")]
pub use components::{
    FenwickHeightTree, RowHeights, ScrollAlign, VirtualList, VirtualListState, VirtualSlice,
};
pub use components::{
    add_days, add_months_clamped, add_years_clamped, civil_from_days, days_from_civil,
    days_in_month, default_combobox_filter, effective_bottom_inset, end_of_week, start_of_week,
    throw_error, throw_ui_error, ui_error, use_effective_bottom_inset, use_form_context,
    use_keyboard_insets, use_toast, Accordion, AccordionItem, Badge, BadgeVariant, BottomSheet,
    BottomSheetState, Button, ButtonSize, ButtonVariant, CalendarCell, CalendarMonthGrid,
    CapturedError, Card, CardVariant, Checkbox, Col, Combobox, ComboboxAction, ComboboxOption,
    ComboboxState, DateConstraints, DatePicker, DatePickerAction, DatePickerState, DateRange,
    DateRangeAction, DateRangePicker, DateRangePickerState, DateSegment, DateSegmentInput,
    DateSegmentOrder, Dialog, DisabledDatePredicate, Disclosure, DisclosureSpring, DragHandoff,
    DropdownMenu, ErrorBoundary, ErrorBoundaryReset, ErrorContext, ErrorFallback, Form, FormField,
    FormMethod, FormSubmitButton, FormSubmitPayload, HourCycle, IconButton, Input,
    KeyboardAvoidingView, KeyboardInsets, MenuItem, Modal, Popover, RangeEndpoint, Row,
    SegmentKeyOutcome, Separator, SheetDetent, SheetSnapOutcome, SplitBound, SplitPane,
    SplitPaneState, Stack, Switch, TabList, TabPanel, TabTrigger, TabsRoot, TimeOfDay,
    ToastController, ToastItem, ToastLive, ToastOptions, ToastProvider, ToastQueue, ToastVariant,
    ToastViewport, Tooltip, UiError, UiErrorReport,
};
pub use nve_dioxus::{AutofillHint, InputMode};
pub use focus::{
    current_focused_id, notify_focus_scope, use_focus_scope, FocusScope, FocusScopeProvider,
    FocusTrap, FocusTrapAction, FocusableEntry, Orientation, RovingFocus,
};
pub use form::{FieldHandle, FieldState, FormState, ValidationToken};
#[cfg(target_arch = "wasm32")]
pub use form_draft::BrowserLocalStorageFormDraftStorage;
pub use form_draft::{
    decode_form_draft, encode_form_draft, FormController, FormDraftError, FormDraftStorage,
    FormDraftStore, MemoryFormDraftStorage, NativeMmapFormDraftStorage,
    StringKeyValueDraftBackend, StringKeyValueFormDraftStorage, DEFAULT_MAX_FORM_DRAFT_BYTES,
    FORM_DRAFT_KEY_PREFIX, FORM_DRAFT_MAGIC, FORM_DRAFT_SCHEMA_VERSION,
};
pub use history::UndoRedoStack;
pub use positioning::{
    compute_overlay_position, point_in_safe_polygon, Alignment, ComputedPosition, Placement, Point,
    PositionOptions, Rect, Side, Size, Viewport,
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
        add_days, add_months_clamped, add_years_clamped, civil_from_days, days_from_civil,
        days_in_month, default_combobox_filter, effective_bottom_inset, end_of_week, start_of_week,
        throw_error, throw_ui_error, ui_error, use_effective_bottom_inset, use_form_context,
        use_keyboard_insets, use_toast, Accordion, AccordionItem, Badge, BadgeVariant, BottomSheet,
        BottomSheetState, Button, ButtonSize, ButtonVariant, CalendarCell, CalendarMonthGrid,
        CapturedError, Card, CardVariant, Checkbox, Col, Combobox, ComboboxAction, ComboboxOption,
        ComboboxState, DateConstraints, DatePicker, DatePickerAction, DatePickerState, DateRange,
        DateRangeAction, DateRangePicker, DateRangePickerState, DateSegment, DateSegmentInput,
        DateSegmentOrder, Dialog, DisabledDatePredicate, Disclosure, DisclosureSpring, DragHandoff,
        DropdownMenu, ErrorBoundary as UiErrorBoundary, ErrorBoundaryReset, ErrorContext,
        ErrorFallback, Form, FormField, FormMethod, FormSubmitButton, FormSubmitPayload, HourCycle,
        IconButton, Input, KeyboardAvoidingView, KeyboardInsets, MenuItem, Modal, Popover,
        RangeEndpoint, Row, SegmentKeyOutcome, Separator, SheetDetent, SheetSnapOutcome,
        SplitBound, SplitPane, SplitPaneState, Stack, Switch, TabList, TabPanel, TabTrigger,
        TabsRoot, TimeOfDay, ToastController, ToastItem, ToastLive, ToastOptions, ToastProvider,
        ToastQueue, ToastVariant, ToastViewport, Tooltip, UiError, UiErrorReport,
    };
    pub use nve_dioxus::{AutofillHint, InputMode};
    pub use crate::focus::{
        use_focus_scope, FocusScope, FocusScopeProvider, FocusTrap, FocusTrapAction,
        FocusableEntry, Orientation, RovingFocus,
    };
    pub use crate::form::{FieldHandle, FieldState, FormState, ValidationToken};
    #[cfg(target_arch = "wasm32")]
    pub use crate::form_draft::BrowserLocalStorageFormDraftStorage;
    pub use crate::form_draft::{
        decode_form_draft, encode_form_draft, FormController, FormDraftError, FormDraftStorage,
        FormDraftStore, MemoryFormDraftStorage, NativeMmapFormDraftStorage,
        StringKeyValueDraftBackend, StringKeyValueFormDraftStorage,
    };
    pub use crate::history::UndoRedoStack;
    pub use crate::positioning::{
        compute_overlay_position, point_in_safe_polygon, Alignment, ComputedPosition, Placement,
        Point, PositionOptions, Rect, Side, Size, Viewport,
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
