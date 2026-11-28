//! Kitchen Sink application state, thread-local mount handoff, and versioned binary
//! state codec (`NVE_KITCHEN_SINK`) for 0ms web snapshots and Tier-2 hot-swap restore.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use nve_motion::SpringSpec;
use nve_i18n::{CalendarDate, Currency, LocaleId};
use nve_ui::{AppBrand, Appearance, DateRange, Theme, ThemeConfig, UiDensity};

/// The theme control uses the shared `nve_ui` modes: `System` ("Auto") follows the OS appearance.
pub use nve_ui::ThemeMode;

/// 16-byte self-documenting ASCII magic identifying a binary Kitchen Sink state payload.
pub const KITCHEN_SINK_STATE_MAGIC: [u8; 16] = *b"NVE_KITCHEN_SINK";

/// Schema version embedded in binary Kitchen Sink state snapshots.
pub const KITCHEN_SINK_STATE_VERSION: u32 = 2;

/// Next mode of the theme button: Auto → Light → Dark → Auto.
pub const fn next_theme_mode(mode: ThemeMode) -> ThemeMode {
    match mode {
        ThemeMode::System => ThemeMode::Light,
        ThemeMode::Light => ThemeMode::Dark,
        ThemeMode::Dark => ThemeMode::System,
    }
}

const MAX_STR_LEN: usize = 2048;
const MAX_MULTI_ITEMS: usize = 16;

/// Gallery navigation sections covering every `nve-ui` component and engine primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, nve_abi::ReprEnum)]
#[repr(u8)]
pub enum GallerySection {
    #[default]
    Overview = 0,
    Forms = 1,
    Selection = 2,
    Overlays = 3,
    Layout = 4,
    Motion = 5,
}

impl GallerySection {
    #[inline]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Forms => "forms",
            Self::Selection => "selection",
            Self::Overlays => "overlays",
            Self::Layout => "layout",
            Self::Motion => "motion",
        }
    }

    #[inline]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview & Primitives",
            Self::Forms => "Inputs & Forms",
            Self::Selection => "Selection & Dates",
            Self::Overlays => "Overlays & Feedback",
            Self::Layout => "Layout & Virtualization",
            Self::Motion => "Motion, Graphics & Resilience",
        }
    }

    #[inline]
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Forms => "Forms",
            Self::Selection => "Dates",
            Self::Overlays => "Overlays",
            Self::Layout => "Layout",
            Self::Motion => "Motion",
        }
    }

    #[inline]
    pub const fn keywords(self) -> &'static str {
        match self {
            Self::Overview => "overview primitives counter button iconbutton badge card separator row col stack hover active focus cursor",
            Self::Forms => "inputs forms input autofill inputmode otp checkbox switch formstate validation password undoredostack history notes",
            Self::Selection => "selection dates combobox multi tags datepicker daterangepicker calendar locale i18n icu plural currency number rtl",
            Self::Overlays => "overlays feedback dialog modal popover placement positioning dropdown menu tooltip toast bottomsheet detent peek half full",
            Self::Layout => "layout virtualization splitpane tabs tablist tabtrigger tabpanel virtuallist fenwick 1000 rows keyboardavoidingview",
            Self::Motion => "motion graphics resilience spring snappy gentle bouncy stiff transition reduced canvas draw1 sparkline bar chart errorboundary fallback retry",
        }
    }

    #[inline]
    pub const fn badge_count(self) -> &'static str {
        match self {
            Self::Overview => "8",
            Self::Forms => "5",
            Self::Selection => "4",
            Self::Overlays => "7",
            Self::Layout => "4",
            Self::Motion => "4",
        }
    }
}

/// Interactive `UiDensity` selector preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, nve_abi::ReprEnum)]
#[repr(u8)]
pub enum DensityPreset {
    Compact = 0,
    #[default]
    Comfortable = 1,
    Spacious = 2,
}

impl DensityPreset {
    #[inline]
    pub const fn to_ui_density(self) -> UiDensity {
        match self {
            Self::Compact => UiDensity::Compact,
            Self::Comfortable => UiDensity::Comfortable,
            Self::Spacious => UiDensity::Spacious,
        }
    }

    #[inline]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Comfortable => "Comfortable",
            Self::Spacious => "Spacious",
        }
    }

    #[inline]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
            Self::Spacious => "spacious",
        }
    }
}

/// Supported live `nve-i18n` locales in the toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, nve_abi::ReprEnum)]
#[repr(u8)]
pub enum LocalePreset {
    #[default]
    EnUs = 0,
    DeDe = 1,
    ArEg = 2,
    JaJp = 3,
}

impl LocalePreset {
    #[inline]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::DeDe => "de-DE",
            Self::ArEg => "ar-EG",
            Self::JaJp => "ja-JP",
        }
    }

    #[inline]
    pub fn locale_id(self) -> LocaleId {
        LocaleId::parse(self.tag()).unwrap_or_else(|_| LocaleId::en_us())
    }

    #[inline]
    pub const fn currency(self) -> Currency {
        match self {
            Self::EnUs => Currency::Usd,
            Self::DeDe => Currency::Eur,
            Self::ArEg => Currency::Usd,
            Self::JaJp => Currency::Jpy,
        }
    }

    #[inline]
    pub const fn currency_amount(self) -> f64 {
        match self {
            Self::EnUs => 1_249.50,
            Self::DeDe => 1_149.90,
            Self::ArEg => 4_850.25,
            Self::JaJp => 184_500.0,
        }
    }

    #[inline]
    pub const fn week_note(self) -> &'static str {
        match self {
            Self::EnUs => "Sunday start · MM/DD/YYYY · LTR",
            Self::DeDe => "Monday start · DD.MM.YYYY · LTR",
            Self::ArEg => "Saturday start · RTL",
            Self::JaJp => "Sunday start · YYYY/MM/DD · LTR",
        }
    }
}

/// Interactive `SpringSpec` preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, nve_abi::ReprEnum)]
#[repr(u8)]
pub enum SpringPreset {
    #[default]
    Snappy = 0,
    Gentle = 1,
    Bouncy = 2,
    Stiff = 3,
}

impl SpringPreset {
    #[inline]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Snappy => "snappy",
            Self::Gentle => "gentle",
            Self::Bouncy => "bouncy",
            Self::Stiff => "stiff",
        }
    }

    #[inline]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Snappy => "Snappy",
            Self::Gentle => "Gentle",
            Self::Bouncy => "Bouncy",
            Self::Stiff => "Stiff",
        }
    }

    #[inline]
    pub const fn spec(self) -> SpringSpec {
        match self {
            Self::Snappy => SpringSpec::snappy(),
            Self::Gentle => SpringSpec::gentle(),
            Self::Bouncy => SpringSpec::bouncy(),
            Self::Stiff => SpringSpec::stiff(),
        }
    }
}

/// Complete persisted state for `KitchenSinkApp` serialized as a versioned binary snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitchenSinkState {
    pub active_section: GallerySection,
    pub theme_mode: ThemeMode,
    /// Host OS appearance (runtime environment, not persisted), followed by [`ThemeMode::System`].
    pub os: Appearance,
    pub density: DensityPreset,
    pub locale: LocalePreset,
    pub pseudo_locale: bool,
    pub reduced_motion: bool,
    pub counter: i32,
    pub checkbox_terms: bool,
    pub checkbox_indeterminate: bool,
    pub switch_notifications: bool,
    pub switch_telemetry: bool,
    pub combobox_single: String,
    pub combobox_multi: Vec<String>,
    pub selected_date: Option<CalendarDate>,
    pub selected_range: Option<DateRange>,
    pub notes: String,
    pub spring_preset: SpringPreset,
    pub motion_toggled: bool,
    pub active_tab: String,
}

impl Default for KitchenSinkState {
    fn default() -> Self {
        Self {
            active_section: GallerySection::Overview,
            theme_mode: ThemeMode::System,
            os: Appearance::DARK,
            density: DensityPreset::Comfortable,
            locale: LocalePreset::EnUs,
            pseudo_locale: false,
            reduced_motion: false,
            counter: 0,
            checkbox_terms: true,
            checkbox_indeterminate: true,
            switch_notifications: true,
            switch_telemetry: false,
            combobox_single: "rust".to_owned(),
            combobox_multi: vec!["taffy".to_owned(), "skia".to_owned()],
            selected_date: CalendarDate::new(2026, 11, 25).ok(),
            selected_range: CalendarDate::new(2026, 11, 20)
                .ok()
                .zip(CalendarDate::new(2026, 11, 28).ok())
                .map(|(start, end)| DateRange::new(start, end)),
            notes: "NVE binary CommandBuffer renders identically on Web, Skia, Apple, Android & Win32."
                .to_owned(),
            spring_preset: SpringPreset::Snappy,
            motion_toggled: false,
            active_tab: "architecture".to_owned(),
        }
    }
}

impl KitchenSinkState {
    /// Builds the active [`Theme`] for `ThemeProvider`.
    pub fn build_theme(&self) -> Theme {
        ThemeConfig {
            mode: self.theme_mode,
            density: self.density.to_ui_density(),
            ..ThemeConfig::new(AppBrand::Default)
        }
        .resolve(self.os)
    }
}

/// Shared synchronized store bridging `KitchenSinkModule` and the Dioxus `KitchenSinkApp` tree.
pub type SharedKitchenSinkStore = Arc<Mutex<KitchenSinkState>>;

thread_local! {
    static PENDING_STORE: RefCell<Option<SharedKitchenSinkStore>> = const { RefCell::new(None) };
}

pub(crate) fn set_pending_store(store: Option<SharedKitchenSinkStore>) {
    PENDING_STORE.with(|slot| *slot.borrow_mut() = store);
}

pub(crate) fn take_or_default_store() -> SharedKitchenSinkStore {
    PENDING_STORE
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_else(|| Arc::new(Mutex::new(KitchenSinkState::default())))
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(MAX_STR_LEN);
    // Ensure valid UTF-8 boundary when truncating.
    let valid_len = if len < bytes.len() {
        s.floor_char_boundary(len)
    } else {
        len
    };
    out.extend_from_slice(&(valid_len as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..valid_len]);
}

fn push_date(out: &mut Vec<u8>, date: CalendarDate) {
    out.extend_from_slice(&date.year.to_le_bytes());
    out.push(date.month);
    out.push(date.day);
}

fn read_u8(bytes: &[u8], pos: &mut usize) -> Option<u8> {
    let v = *bytes.get(*pos)?;
    *pos += 1;
    Some(v)
}

fn read_u16(bytes: &[u8], pos: &mut usize) -> Option<u16> {
    let slice = bytes.get(*pos..*pos + 2)?;
    *pos += 2;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_i32(bytes: &[u8], pos: &mut usize) -> Option<i32> {
    let slice = bytes.get(*pos..*pos + 4)?;
    *pos += 4;
    Some(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_date(bytes: &[u8], pos: &mut usize) -> Option<CalendarDate> {
    let year = read_i32(bytes, pos)?;
    let month = read_u8(bytes, pos)?;
    let day = read_u8(bytes, pos)?;
    CalendarDate::new(year, month, day).ok()
}

fn read_str(bytes: &[u8], pos: &mut usize) -> Option<String> {
    let len = read_u16(bytes, pos)? as usize;
    if len > MAX_STR_LEN {
        return None;
    }
    let slice = bytes.get(*pos..*pos + len)?;
    *pos += len;
    std::str::from_utf8(slice).ok().map(str::to_owned)
}

/// Encodes `KitchenSinkState` into a compact little-endian binary payload.
pub fn encode_kitchen_sink_state(state: &KitchenSinkState) -> Vec<u8> {
    let mut out = Vec::with_capacity(176);
    out.extend_from_slice(&KITCHEN_SINK_STATE_MAGIC);
    out.extend_from_slice(&(KITCHEN_SINK_STATE_VERSION as u16).to_le_bytes());
    out.push(state.active_section as u8);
    out.push(state.theme_mode.as_u8());
    out.push(state.density as u8);
    out.push(state.locale as u8);

    let mut flags = 0u8;
    if state.pseudo_locale {
        flags |= 1 << 0;
    }
    if state.reduced_motion {
        flags |= 1 << 1;
    }
    if state.checkbox_terms {
        flags |= 1 << 2;
    }
    if state.checkbox_indeterminate {
        flags |= 1 << 3;
    }
    if state.switch_notifications {
        flags |= 1 << 4;
    }
    if state.switch_telemetry {
        flags |= 1 << 5;
    }
    if state.motion_toggled {
        flags |= 1 << 6;
    }
    out.push(flags);
    out.push(state.spring_preset as u8);
    out.extend_from_slice(&state.counter.to_le_bytes());

    match state.selected_date {
        Some(d) => {
            out.push(1);
            push_date(&mut out, d);
        }
        None => out.push(0),
    }

    match state.selected_range {
        Some(r) => {
            let norm = r.normalized();
            out.push(1);
            push_date(&mut out, norm.start);
            push_date(&mut out, norm.end);
        }
        None => out.push(0),
    }

    push_str(&mut out, &state.combobox_single);
    let multi_count = state.combobox_multi.len().min(MAX_MULTI_ITEMS);
    out.push(multi_count as u8);
    for item in state.combobox_multi.iter().take(multi_count) {
        push_str(&mut out, item);
    }
    push_str(&mut out, &state.notes);
    push_str(&mut out, &state.active_tab);
    out
}

/// Decodes a versioned binary payload back into `KitchenSinkState`, rejecting
/// corrupt, truncated, version-mismatched, or trailing-byte inputs cleanly.
pub fn decode_kitchen_sink_state(bytes: &[u8]) -> Option<KitchenSinkState> {
    if bytes.len() < KITCHEN_SINK_STATE_MAGIC.len() + 12
        || bytes[0..KITCHEN_SINK_STATE_MAGIC.len()] != KITCHEN_SINK_STATE_MAGIC
    {
        return None;
    }
    let mut pos = KITCHEN_SINK_STATE_MAGIC.len();
    let ver = read_u16(bytes, &mut pos)? as u32;
    if ver != KITCHEN_SINK_STATE_VERSION {
        return None;
    }
    let active_section = GallerySection::try_from_u8(read_u8(bytes, &mut pos)?)?;
    let theme_mode = ThemeMode::try_from_u8(read_u8(bytes, &mut pos)?)?;
    let density = DensityPreset::try_from_u8(read_u8(bytes, &mut pos)?)?;
    let locale = LocalePreset::try_from_u8(read_u8(bytes, &mut pos)?)?;
    let flags = read_u8(bytes, &mut pos)?;
    if flags & (1 << 7) != 0 {
        return None;
    }
    let spring_preset = SpringPreset::try_from_u8(read_u8(bytes, &mut pos)?)?;
    let counter = read_i32(bytes, &mut pos)?;

    let selected_date = match read_u8(bytes, &mut pos)? {
        0 => None,
        1 => Some(read_date(bytes, &mut pos)?),
        _ => return None,
    };

    let selected_range = match read_u8(bytes, &mut pos)? {
        0 => None,
        1 => {
            let start = read_date(bytes, &mut pos)?;
            let end = read_date(bytes, &mut pos)?;
            Some(DateRange::new(start, end))
        }
        _ => return None,
    };

    let combobox_single = read_str(bytes, &mut pos)?;
    let multi_count = read_u8(bytes, &mut pos)? as usize;
    if multi_count > MAX_MULTI_ITEMS {
        return None;
    }
    let mut combobox_multi = Vec::with_capacity(multi_count);
    for _ in 0..multi_count {
        combobox_multi.push(read_str(bytes, &mut pos)?);
    }
    let notes = read_str(bytes, &mut pos)?;
    let active_tab = read_str(bytes, &mut pos)?;

    if pos != bytes.len() {
        return None;
    }

    Some(KitchenSinkState {
        active_section,
        theme_mode,
        os: Appearance::DARK,
        density,
        locale,
        pseudo_locale: (flags & (1 << 0)) != 0,
        reduced_motion: (flags & (1 << 1)) != 0,
        counter,
        checkbox_terms: (flags & (1 << 2)) != 0,
        checkbox_indeterminate: (flags & (1 << 3)) != 0,
        switch_notifications: (flags & (1 << 4)) != 0,
        switch_telemetry: (flags & (1 << 5)) != 0,
        combobox_single,
        combobox_multi,
        selected_date,
        selected_range,
        notes,
        spring_preset,
        motion_toggled: (flags & (1 << 6)) != 0,
        active_tab,
    })
}
