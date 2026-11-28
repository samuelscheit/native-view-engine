//! Exhaustive WAI-ARIA [`AriaRole`] and implicit `NodeStore` table to [`accesskit::Role`] mapping.

use accesskit::Role;
use nve_abi::InteractionEvent;
use nve_store::{NodeId, NodeStore};
use nve_tables::tables::{
    Accessibility, AriaRole, Image, InputKind, Interaction, Link, Scroll, TextContent, TextInput,
    TextInputRef,
};

/// Exhaustive 48-variant mapping from [`AriaRole`] to [`accesskit::Role`].
#[inline]
pub const fn aria_role_to_accesskit(role: AriaRole) -> Role {
    match role {
        AriaRole::Button => Role::Button,
        AriaRole::Link => Role::Link,
        AriaRole::Heading => Role::Heading,
        AriaRole::Text => Role::Label,
        AriaRole::Image => Role::Image,
        AriaRole::TextBox => Role::TextInput,
        AriaRole::SearchBox => Role::SearchInput,
        AriaRole::ScrollArea => Role::ScrollView,
        AriaRole::Checkbox => Role::CheckBox,
        AriaRole::Switch => Role::Switch,
        AriaRole::Radio => Role::RadioButton,
        AriaRole::Slider => Role::Slider,
        AriaRole::Tab => Role::Tab,
        AriaRole::TabList => Role::TabList,
        AriaRole::TabPanel => Role::TabPanel,
        AriaRole::Dialog => Role::Dialog,
        AriaRole::AlertDialog => Role::AlertDialog,
        AriaRole::Alert => Role::Alert,
        AriaRole::Status => Role::Status,
        AriaRole::Tooltip => Role::Tooltip,
        AriaRole::Navigation => Role::Navigation,
        AriaRole::Main => Role::Main,
        AriaRole::Article => Role::Article,
        AriaRole::Complementary => Role::Complementary,
        AriaRole::Banner => Role::Banner,
        AriaRole::ContentInfo => Role::ContentInfo,
        AriaRole::Form => Role::Form,
        AriaRole::Region => Role::Region,
        AriaRole::Group => Role::Group,
        AriaRole::List => Role::List,
        AriaRole::ListItem => Role::ListItem,
        AriaRole::ListBox => Role::ListBox,
        AriaRole::Option => Role::ListBoxOption,
        AriaRole::Menu => Role::Menu,
        AriaRole::MenuItem => Role::MenuItem,
        AriaRole::Separator => Role::Splitter,
        AriaRole::Presentation => Role::GenericContainer,
        AriaRole::Table => Role::Table,
        AriaRole::Row => Role::Row,
        AriaRole::Cell => Role::Cell,
        AriaRole::ColumnHeader => Role::ColumnHeader,
        AriaRole::RowHeader => Role::RowHeader,
        AriaRole::Code | AriaRole::CodeBlock => Role::Code,
        AriaRole::ProgressBar => Role::ProgressIndicator,
        AriaRole::ComboBox => Role::ComboBox,
        AriaRole::Grid => Role::Grid,
        AriaRole::GridCell => Role::GridCell,
    }
}

/// Maps a [`TextInput`] table row's `secure`/`multiline`/`input_kind` fields to its native
/// [`accesskit::Role`].
#[inline]
pub fn text_input_role(ti: TextInputRef<'_>) -> Role {
    if ti.secure() || ti.input_kind() == InputKind::Password {
        return Role::PasswordInput;
    }
    if ti.multiline() {
        return Role::MultilineTextInput;
    }
    match ti.input_kind() {
        InputKind::Text | InputKind::Hidden => Role::TextInput,
        InputKind::Email => Role::EmailInput,
        InputKind::Password => Role::PasswordInput,
        InputKind::Tel => Role::PhoneNumberInput,
        InputKind::Url => Role::UrlInput,
        InputKind::Number => Role::NumberInput,
        InputKind::Search => Role::SearchInput,
        InputKind::Date => Role::DateInput,
        InputKind::Time => Role::TimeInput,
    }
}

/// Resolves the final [`accesskit::Role`] for `id` in `store`, ALWAYS preserving
/// `Role::PasswordInput` for secure/password `TextInput`s (even when an explicit `Accessibility::role`
/// is attached), otherwise preferring explicit `Accessibility::role` (preserving
/// `MultilineTextInput` on explicit `AriaRole::TextBox`) and falling back to implicit component
/// tables (`TextInput`, `Link`, `Image`, `Scroll`, `TextContent`, pressable `Interaction`, root
/// `Window`, or `GenericContainer`).
pub fn resolve_node_role(store: &NodeStore, id: NodeId) -> Role {
    let text_input = store.try_row::<TextInput>(id);
    if let Some(ti) = text_input
        && (ti.secure() || ti.input_kind() == InputKind::Password)
    {
        return Role::PasswordInput;
    }
    if let Some(a11y) = store.try_row::<Accessibility>(id)
        && let Some(aria_role) = a11y.role()
    {
        let explicit = aria_role_to_accesskit(aria_role);
        if explicit == Role::TextInput
            && let Some(ti) = text_input
        {
            return text_input_role(ti);
        }
        return explicit;
    }
    if id == NodeId::ROOT {
        return Role::Window;
    }
    if let Some(ti) = text_input {
        return text_input_role(ti);
    }
    if store.has::<Link>(id) {
        return Role::Link;
    }
    if store.has::<Image>(id) {
        return Role::Image;
    }
    if store.has::<Scroll>(id) {
        return Role::ScrollView;
    }
    if store.has::<TextContent>(id) && store.children(id).is_empty() {
        return Role::Label;
    }
    if store
        .try_row::<Interaction>(id)
        .is_some_and(|i| i.listeners().contains(InteractionEvent::Press))
    {
        return Role::Button;
    }
    Role::GenericContainer
}
