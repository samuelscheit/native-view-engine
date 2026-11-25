//! Localization & `PseudoLocalizer` helpers powered by `nve-i18n`.

use nve_i18n::{
    first_day_of_week, format_currency, format_date, format_number, format_relative_time,
    plural_category_for_int, pseudo_localize, CalendarDate, DateStyle, NumberFormatOptions,
    PluralKind, RelativeUnit, TextDirection, Weekday,
};

use crate::state::LocalePreset;

/// Applies `nve_i18n::pseudo_localize` when the toolbar PseudoLocalizer toggle is enabled.
#[inline]
pub fn copy(text: &str, pseudo: bool) -> String {
    if pseudo {
        pseudo_localize(text)
    } else {
        text.to_owned()
    }
}

/// Live ICU formatting readout derived directly from `nve-i18n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleReadout {
    pub canonical_tag: String,
    pub direction_label: &'static str,
    pub first_weekday: &'static str,
    pub date_short: String,
    pub date_medium: String,
    pub date_long: String,
    pub currency_formatted: String,
    pub number_formatted: String,
    pub relative_past: String,
    pub relative_future: String,
    pub plural_label: String,
}

fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Sunday => "Sunday",
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
    }
}

/// Computes live ICU formatting metrics for the active `LocalePreset`, `CalendarDate`, and count.
pub fn compute_locale_readout(
    preset: LocalePreset,
    date: CalendarDate,
    count: i32,
) -> LocaleReadout {
    let loc = preset.locale_id();
    let dir = match loc.text_direction() {
        TextDirection::Rtl => "RTL (Right-to-Left)",
        TextDirection::Ltr => "LTR (Left-to-Right)",
    };
    let first_day = weekday_name(first_day_of_week(&loc));
    let date_short = format_date(&loc, date, DateStyle::Short);
    let date_medium = format_date(&loc, date, DateStyle::Medium);
    let date_long = format_date(&loc, date, DateStyle::Long);
    let currency_formatted = format_currency(&loc, preset.currency_amount(), preset.currency());
    let number_formatted = format_number(
        &loc,
        1_234_567.89,
        NumberFormatOptions {
            use_grouping: true,
            min_fraction_digits: Some(2),
            max_fraction_digits: Some(2),
        },
    );
    let relative_past = format_relative_time(&loc, -3, RelativeUnit::Day);
    let relative_future = format_relative_time(&loc, 2, RelativeUnit::Hour);
    let plural_cat = plural_category_for_int(&loc, PluralKind::Cardinal, i64::from(count.abs()));
    let plural_label = format!("n={} -> CLDR `{}`", count, plural_cat.as_str());

    LocaleReadout {
        canonical_tag: loc.canonical_tag(),
        direction_label: dir,
        first_weekday: first_day,
        date_short,
        date_medium,
        date_long,
        currency_formatted,
        number_formatted,
        relative_past,
        relative_future,
        plural_label,
    }
}
