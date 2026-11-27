//! `Interaction`, `Accessibility`, and `DebugId` sparse tables.

use nve_store::{NveTable, Res, Str, WordEnum};

use crate::interaction::CursorKind;

use super::resources::{AriaRelations, StateRules};

/// Interactive subscriptions, pointer cursor, focusability, and pseudo-state style rules.
#[derive(NveTable)]
#[nve_table(
    name = "nve.interaction",
    storage = "sparse",
    flags = "publish",
    events = nve_abi::InteractionEvent
)]
pub struct Interaction {
    #[effect(paint)]
    pub cursor: CursorKind,
    #[effect(semantics, events, paint)]
    pub disabled: bool,
    #[effect(semantics, events)]
    pub focusable: bool,
    /// Optional explicit tab index (`None` = natural tab order).
    #[effect(semantics, events)]
    pub tab_index: Option<i16>,
    #[effect(paint)]
    pub state_rules: Res<StateRules>,
}

/// Closed cross-platform accessibility role enum covering WAI-ARIA, Apple NSAccessibility,
/// Android AccessibilityNodeInfo, GTK4, and Flutter Semantics; wrap in `Option<AriaRole>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum AriaRole {
    Button = 0,
    Link = 1,
    Heading = 2,
    Text = 3,
    Image = 4,
    TextBox = 5,
    SearchBox = 6,
    ScrollArea = 7,
    Checkbox = 8,
    Switch = 9,
    Radio = 10,
    Slider = 11,
    Tab = 12,
    TabList = 13,
    TabPanel = 14,
    Dialog = 15,
    AlertDialog = 16,
    Alert = 17,
    Status = 18,
    Tooltip = 19,
    Navigation = 20,
    Main = 21,
    Article = 22,
    Complementary = 23,
    Banner = 24,
    ContentInfo = 25,
    Form = 26,
    Region = 27,
    Group = 28,
    List = 29,
    ListItem = 30,
    ListBox = 31,
    Option = 32,
    Menu = 33,
    MenuItem = 34,
    Separator = 35,
    Presentation = 36,
    Table = 37,
    Row = 38,
    Cell = 39,
    ColumnHeader = 40,
    RowHeader = 41,
    Code = 42,
    CodeBlock = 43,
    ProgressBar = 44,
    ComboBox = 45,
    Grid = 46,
    GridCell = 47,
}

impl AriaRole {
    /// Canonical WAI-ARIA role token.
    #[inline]
    pub const fn as_aria_str(self) -> &'static str {
        match self {
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
    pub fn parse(raw: &str) -> Option<Self> {
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
    }
}

/// Tri-state ARIA boolean value (`False` / `True` / `Mixed`); wrap in `Option<AriaTriState>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum AriaTriState {
    False = 0,
    True = 1,
    Mixed = 2,
}

/// WAI-ARIA `aria-live` politeness level; wrap in `Option<AriaLive>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum AriaLive {
    Off = 0,
    Polite = 1,
    Assertive = 2,
}

impl AriaLive {
    /// Canonical WAI-ARIA `aria-live` attribute token.
    #[inline]
    pub const fn as_aria_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Polite => "polite",
            Self::Assertive => "assertive",
        }
    }
}

/// WAI-ARIA `aria-orientation`; wrap in `Option<AriaOrientation>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum AriaOrientation {
    Horizontal = 0,
    Vertical = 1,
}

/// WAI-ARIA `aria-autocomplete`; wrap in `Option<AriaAutoComplete>` for unset.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, WordEnum)]
pub enum AriaAutoComplete {
    None = 0,
    Inline = 1,
    List = 2,
}

/// Cross-platform accessibility semantics (WAI-ARIA + Apple/Android/GTK/Flutter).
#[derive(NveTable)]
#[nve_table(name = "nve.accessibility", storage = "sparse", flags = "publish")]
pub struct Accessibility {
    /// Drives semantic tag / native role realization (`None` = default container role).
    #[effect(semantics, realize)]
    pub role: Option<AriaRole>,
    /// Optional `aria-level` (`1..=6`; `None` = unset).
    #[effect(semantics, realize)]
    pub level: Option<u8>,
    #[effect(semantics, realize)]
    pub modal: bool,
    #[effect(semantics)]
    pub live: Option<AriaLive>,
    #[effect(semantics)]
    pub orientation: Option<AriaOrientation>,
    #[effect(semantics)]
    pub autocomplete: Option<AriaAutoComplete>,
    #[effect(semantics)]
    pub checked: Option<AriaTriState>,
    #[effect(semantics)]
    pub selected: Option<AriaTriState>,
    #[effect(semantics)]
    pub expanded: Option<AriaTriState>,
    #[effect(semantics)]
    pub pressed: Option<AriaTriState>,
    #[effect(semantics)]
    pub hidden: bool,
    #[effect(semantics)]
    pub invalid: bool,
    #[effect(semantics)]
    pub multiselectable: bool,
    #[effect(semantics)]
    pub busy: bool,
    #[effect(semantics)]
    pub label: Str,
    #[effect(semantics)]
    pub hint: Str,
    #[effect(semantics)]
    pub value_text: Str,
    /// Optional `aria-valuenow`.
    #[effect(semantics)]
    pub value_now: Option<f32>,
    /// Optional `aria-valuemin`.
    #[effect(semantics)]
    pub value_min: Option<f32>,
    /// Optional `aria-valuemax`.
    #[effect(semantics)]
    pub value_max: Option<f32>,
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
