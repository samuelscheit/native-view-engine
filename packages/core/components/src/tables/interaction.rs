//! `Interaction`, `Accessibility`, `DebugId`, and `StreamBoundary` sparse tables.

use nve_store::{word_enum, NveTable, Res, Str};

pub use crate::interaction::CursorKind;

use super::resources::{AriaRelations, StateRules};

/// Interactive subscriptions, pointer cursor, focusability, and pseudo-state style rules.
#[derive(NveTable)]
#[nve_table(name = "nve.interaction", storage = "sparse", flags = "publish")]
pub struct Interaction {
    /// Bitmask of subscribed `EventMask` bits.
    #[effect(events, realize)]
    pub event_mask: u32,
    #[effect(paint)]
    #[bits(4)]
    pub cursor: CursorKind,
    #[effect(semantics, events, paint)]
    #[bits(1)]
    pub disabled: bool,
    #[effect(semantics, events)]
    #[bits(1)]
    pub focusable: bool,
    #[effect(semantics, events)]
    #[bits(1)]
    pub tab_index_set: bool,
    #[effect(semantics, events)]
    #[bits(16)]
    pub tab_index: i16,
    #[effect(paint)]
    pub state_rules: Res<StateRules>,
}

word_enum! {
    /// Closed cross-platform accessibility role enum covering WAI-ARIA, Apple NSAccessibility,
    /// Android AccessibilityNodeInfo, GTK4, and Flutter Semantics.
    pub enum AriaRole: 6 {
        None = 0,
        Button = 1,
        Link = 2,
        Heading = 3,
        Text = 4,
        Image = 5,
        TextBox = 6,
        SearchBox = 7,
        ScrollArea = 8,
        Checkbox = 9,
        Switch = 10,
        Radio = 11,
        Slider = 12,
        Tab = 13,
        TabList = 14,
        TabPanel = 15,
        Dialog = 16,
        AlertDialog = 17,
        Alert = 18,
        Status = 19,
        Tooltip = 20,
        Navigation = 21,
        Main = 22,
        Article = 23,
        Complementary = 24,
        Banner = 25,
        ContentInfo = 26,
        Form = 27,
        Region = 28,
        Group = 29,
        List = 30,
        ListItem = 31,
        ListBox = 32,
        Option = 33,
        Menu = 34,
        MenuItem = 35,
        Separator = 36,
        Presentation = 37,
        Table = 38,
        Row = 39,
        Cell = 40,
        ColumnHeader = 41,
        RowHeader = 42,
        Code = 43,
        CodeBlock = 44,
        ProgressBar = 45,
        ComboBox = 46,
        Grid = 47,
        GridCell = 48,
    }
}

impl AriaRole {
    /// Canonical WAI-ARIA role token (empty for `None`).
    #[inline]
    pub const fn as_aria_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Button => "button",
            Self::Link => "link",
            Self::Heading => "heading",
            Self::Text => "text",
            Self::Image => "img",
            Self::TextBox => "textbox",
            Self::SearchBox => "searchbox",
            Self::ScrollArea => "region",
            Self::Checkbox => "checkbox",
            Self::Switch => "switch",
            Self::Radio => "radio",
            Self::Slider => "slider",
            Self::Tab => "tab",
            Self::TabList => "tablist",
            Self::TabPanel => "tabpanel",
            Self::Dialog => "dialog",
            Self::AlertDialog => "alertdialog",
            Self::Alert => "alert",
            Self::Status => "status",
            Self::Tooltip => "tooltip",
            Self::Navigation => "navigation",
            Self::Main => "main",
            Self::Article => "article",
            Self::Complementary => "complementary",
            Self::Banner => "banner",
            Self::ContentInfo => "contentinfo",
            Self::Form => "form",
            Self::Region => "region",
            Self::Group => "group",
            Self::List => "list",
            Self::ListItem => "listitem",
            Self::ListBox => "listbox",
            Self::Option => "option",
            Self::Menu => "menu",
            Self::MenuItem => "menuitem",
            Self::Separator => "separator",
            Self::Presentation => "presentation",
            Self::Table => "table",
            Self::Row => "row",
            Self::Cell => "cell",
            Self::ColumnHeader => "columnheader",
            Self::RowHeader => "rowheader",
            Self::Code => "code",
            Self::CodeBlock => "codeblock",
            Self::ProgressBar => "progressbar",
            Self::ComboBox => "combobox",
            Self::Grid => "grid",
            Self::GridCell => "gridcell",
        }
    }

    /// Parses a WAI-ARIA / RN accessibility role string without allocating.
    pub fn parse(raw: &str) -> Self {
        let s = raw.trim();
        const TABLE: &[(&str, AriaRole)] = &[
            ("button", AriaRole::Button),
            ("link", AriaRole::Link),
            ("heading", AriaRole::Heading),
            ("header", AriaRole::Heading),
            ("text", AriaRole::Text),
            ("statictext", AriaRole::Text),
            ("label", AriaRole::Text),
            ("img", AriaRole::Image),
            ("image", AriaRole::Image),
            ("textbox", AriaRole::TextBox),
            ("textinput", AriaRole::TextBox),
            ("searchbox", AriaRole::SearchBox),
            ("search", AriaRole::SearchBox),
            ("scrollarea", AriaRole::ScrollArea),
            ("scrollview", AriaRole::ScrollArea),
            ("checkbox", AriaRole::Checkbox),
            ("switch", AriaRole::Switch),
            ("radio", AriaRole::Radio),
            ("slider", AriaRole::Slider),
            ("tab", AriaRole::Tab),
            ("tablist", AriaRole::TabList),
            ("tabpanel", AriaRole::TabPanel),
            ("dialog", AriaRole::Dialog),
            ("alertdialog", AriaRole::AlertDialog),
            ("alert", AriaRole::Alert),
            ("status", AriaRole::Status),
            ("tooltip", AriaRole::Tooltip),
            ("navigation", AriaRole::Navigation),
            ("nav", AriaRole::Navigation),
            ("main", AriaRole::Main),
            ("article", AriaRole::Article),
            ("complementary", AriaRole::Complementary),
            ("banner", AriaRole::Banner),
            ("contentinfo", AriaRole::ContentInfo),
            ("form", AriaRole::Form),
            ("region", AriaRole::Region),
            ("group", AriaRole::Group),
            ("list", AriaRole::List),
            ("listitem", AriaRole::ListItem),
            ("listbox", AriaRole::ListBox),
            ("option", AriaRole::Option),
            ("menu", AriaRole::Menu),
            ("menuitem", AriaRole::MenuItem),
            ("separator", AriaRole::Separator),
            ("presentation", AriaRole::Presentation),
            ("table", AriaRole::Table),
            ("row", AriaRole::Row),
            ("cell", AriaRole::Cell),
            ("columnheader", AriaRole::ColumnHeader),
            ("rowheader", AriaRole::RowHeader),
            ("code", AriaRole::Code),
            ("codeblock", AriaRole::CodeBlock),
            ("progressbar", AriaRole::ProgressBar),
            ("combobox", AriaRole::ComboBox),
            ("grid", AriaRole::Grid),
            ("gridcell", AriaRole::GridCell),
        ];
        TABLE
            .iter()
            .find_map(|&(name, role)| s.eq_ignore_ascii_case(name).then_some(role))
            .unwrap_or(Self::None)
    }
}

word_enum! {
    /// Tri-state ARIA boolean attribute (`Unset` / `False` / `True` / `Mixed`).
    pub enum AriaTriState: 2 { Unset = 0, False = 1, True = 2, Mixed = 3 }
}

word_enum! {
    pub enum AriaLive: 2 { Unset = 0, Off = 1, Polite = 2, Assertive = 3 }
}

impl AriaLive {
    /// Canonical WAI-ARIA `aria-live` attribute token (`None` when `Unset`).
    #[inline]
    pub const fn as_aria_str(self) -> Option<&'static str> {
        match self {
            Self::Unset => None,
            Self::Off => Some("off"),
            Self::Polite => Some("polite"),
            Self::Assertive => Some("assertive"),
        }
    }
}

word_enum! {
    pub enum AriaOrientation: 2 { Unset = 0, Horizontal = 1, Vertical = 2 }
}

word_enum! {
    pub enum AriaAutoComplete: 2 { Unset = 0, None = 1, Inline = 2, List = 3 }
}

/// Cross-platform accessibility semantics (WAI-ARIA + Apple/Android/GTK/Flutter).
#[derive(NveTable)]
#[nve_table(name = "nve.accessibility", storage = "sparse", flags = "publish")]
pub struct Accessibility {
    /// Drives semantic tag / native role realization.
    #[effect(semantics, realize)]
    #[bits(6)]
    pub role: AriaRole,
    /// `aria-level` (`1..=6`; `0` = unset).
    #[effect(semantics, realize)]
    #[bits(8)]
    pub level: u8,
    #[effect(semantics, realize)]
    #[bits(1)]
    pub modal: bool,
    #[effect(semantics)]
    #[bits(2)]
    pub live: AriaLive,
    #[effect(semantics)]
    #[bits(2)]
    pub orientation: AriaOrientation,
    #[effect(semantics)]
    #[bits(2)]
    pub autocomplete: AriaAutoComplete,
    #[effect(semantics)]
    #[bits(2)]
    pub checked: AriaTriState,
    #[effect(semantics)]
    #[bits(2)]
    pub selected: AriaTriState,
    #[effect(semantics)]
    #[bits(2)]
    pub expanded: AriaTriState,
    #[effect(semantics)]
    #[bits(2)]
    pub pressed: AriaTriState,
    #[effect(semantics)]
    #[bits(1)]
    pub hidden: bool,
    #[effect(semantics)]
    #[bits(1)]
    pub invalid: bool,
    #[effect(semantics)]
    #[bits(1)]
    pub multiselectable: bool,
    #[effect(semantics)]
    #[bits(1)]
    pub busy: bool,
    #[effect(semantics)]
    pub label: Str,
    #[effect(semantics)]
    pub hint: Str,
    #[effect(semantics)]
    pub value_text: Str,
    /// `aria-valuenow`; `NaN` = unset.
    #[effect(semantics)]
    #[default(f32::NAN)]
    pub value_now: f32,
    /// `aria-valuemin`; `NaN` = unset.
    #[effect(semantics)]
    #[default(f32::NAN)]
    pub value_min: f32,
    /// `aria-valuemax`; `NaN` = unset.
    #[effect(semantics)]
    #[default(f32::NAN)]
    pub value_max: f32,
    #[effect(semantics)]
    pub relations: Res<AriaRelations>,
}

/// Element identity & test automation handles (`testID`, DOM `id`, `slot`).
/// Kept separate from `Accessibility` so test-tagged nodes do not materialize a full a11y row.
#[derive(NveTable)]
#[nve_table(name = "nve.debug_id", storage = "sparse", flags = "publish")]
pub struct DebugId {
    #[effect(semantics)]
    pub test_id: Str,
    #[effect(semantics)]
    pub id: Str,
    #[effect(semantics)]
    pub slot: Str,
}

/// DOM attribute marking a pending Declarative Shadow DOM streaming slot root
/// (`<slot name="nve-slot-{id}">` in the streaming SSR shell).
pub const NVE_STREAM_SLOT_ATTR: &str = "data-nve-stream-slot";

/// DOM attribute marking a `<QuerySuspense>` boundary container whose direct children swap from
/// the pending stream slot to resolved content.
pub const NVE_SUSPENSE_ATTR: &str = "data-nve-suspense";

/// DOM attribute distinguishing replaceable output slots from stable input slots.
pub const NVE_STREAM_SLOT_ROLE_ATTR: &str = "data-nve-slot-role";

word_enum! {
    /// Role of a streaming slot boundary ([`NVE_STREAM_SLOT_ROLE_ATTR`]).
    pub enum StreamSlotRole: 2 {
        /// No role (not a stream slot, or role never set): no attribute emitted.
        None = 0,
        /// Replaceable async/live output slot (receives out-of-order Light DOM chunks).
        Output = 1,
        /// Stable input slot hosting active `<Form>` / `<input>` controls (never overwritten live).
        Input = 2,
    }
}

impl StreamSlotRole {
    /// The [`NVE_STREAM_SLOT_ROLE_ATTR`] value, `None` for [`Self::None`].
    #[inline]
    pub const fn as_attr(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Output => Some("output"),
            Self::Input => Some("input"),
        }
    }

    /// Parses an [`NVE_STREAM_SLOT_ROLE_ATTR`] value; unknown values are [`Self::None`].
    #[inline]
    pub fn from_attr(value: &str) -> Self {
        match value {
            "output" => Self::Output,
            "input" => Self::Input,
            _ => Self::None,
        }
    }
}

/// Streaming SSR boundary markers ([`NVE_STREAM_SLOT_ATTR`], [`NVE_SUSPENSE_ATTR`],
/// [`NVE_STREAM_SLOT_ROLE_ATTR`]) consumed by web streaming shell rendering, slot resolution
/// checks, and structural hydration.
#[derive(NveTable)]
#[nve_table(name = "nve.stream_boundary", storage = "sparse", flags = "publish")]
pub struct StreamBoundary {
    #[effect(semantics)]
    pub stream_slot: Str,
    #[effect(semantics)]
    pub suspense: Str,
    #[effect(semantics)]
    #[bits(2)]
    pub slot_role: StreamSlotRole,
}

