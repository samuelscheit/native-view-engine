//! Comprehensive tests for locale formatting (number, currency, calendar date,
//! first-day-of-week, relative-time), pseudo-localization, and `MessageCatalog`/`Translator`.

use nve_i18n::{
    BuiltinI18nBackend, CalendarDate, Currency, DateStyle, I18nError, LocaleId,
    MessageCatalog, NameWidth, NumberFormatOptions, RelativeUnit, Translator, Weekday,
    first_day_of_week, format_currency, format_currency_minor, format_date, format_decimal,
    format_integer, format_message, format_number, format_relative_time,
    format_relative_time_auto, month_name, ordered_weekdays, parse_message, pseudo_localize,
    weekday_name,
};

fn loc(tag: &str) -> LocaleId {
    LocaleId::parse(tag).expect("valid locale")
}

#[test]
fn number_formatting_across_locales_and_edge_cases() {
    assert_eq!(format_decimal(&loc("en-US"), 1234.56), "1,234.56");
    assert_eq!(format_decimal(&loc("de-DE"), 1234.56), "1.234,56");
    assert_eq!(format_decimal(&loc("fr-FR"), 1234.56), "1\u{202F}234,56");
    assert_eq!(format_decimal(&loc("en-IN"), 123456.78), "1,23,456.78");
    assert_eq!(format_decimal(&loc("en-IN"), 12345678.9), "1,23,45,678.9");
    assert_eq!(format_decimal(&loc("de-CH"), 1234.56), "1'234.56");
    assert_eq!(format_decimal(&loc("pl-PL"), 1234.56), "1\u{00A0}234,56");

    // Negative zero normalized to `"0"`
    assert_eq!(format_decimal(&loc("en-US"), -0.0), "0");
    assert_eq!(format_decimal(&loc("en-US"), f64::NAN), "NaN");
    assert_eq!(format_decimal(&loc("en-US"), f64::INFINITY), "∞");
    assert_eq!(format_decimal(&loc("en-US"), f64::NEG_INFINITY), "-∞");

    // Exact 64-bit integer formatting including i64::MIN
    assert_eq!(format_integer(&loc("en-US"), i64::MIN), "-9,223,372,036,854,775,808");

    // Rounding with full carry propagation (`999.995` -> `1,000.00`)
    let rounded = format_number(
        &loc("en-US"),
        999.995,
        NumberFormatOptions {
            use_grouping: true,
            min_fraction_digits: Some(2),
            max_fraction_digits: Some(2),
        },
    );
    assert_eq!(rounded, "1,000.00");

    // Grouping disabled
    let ungrouped = format_number(
        &loc("en-US"),
        123456.7,
        NumberFormatOptions {
            use_grouping: false,
            min_fraction_digits: Some(2),
            max_fraction_digits: Some(2),
        },
    );
    assert_eq!(ungrouped, "123456.70");
}

#[test]
fn currency_formatting_precision_placement_and_minor_units() {
    assert_eq!(format_currency(&loc("en-US"), 1234.5, Currency::Usd), "$1,234.50");
    assert_eq!(format_currency(&loc("en-GB"), 1234.56, Currency::Gbp), "£1,234.56");
    assert_eq!(format_currency(&loc("de-DE"), 1234.56, Currency::Eur), "1.234,56\u{00A0}€");
    assert_eq!(format_currency(&loc("fr-FR"), 1234.56, Currency::Eur), "1\u{202F}234,56\u{00A0}€");
    assert_eq!(format_currency(&loc("ja-JP"), 1234.0, Currency::Jpy), "¥1,234");
    assert_eq!(format_currency(&loc("de-CH"), 1234.56, Currency::Chf), "CHF\u{00A0}1'234.56");
    assert_eq!(format_currency(&loc("en-IN"), 123456.78, Currency::Inr), "₹1,23,456.78");
    assert_eq!(format_currency(&loc("en-US"), -42.5, Currency::Usd), "-$42.50");
    assert_eq!(format_currency(&loc("de-DE"), -42.5, Currency::Eur), "-42,50\u{00A0}€");

    // 3-decimal currency (KWD) & exact integer minor-unit formatting
    assert_eq!(format_currency_minor(&loc("en-US"), 123_456, Currency::Usd), "$1,234.56");
    assert_eq!(format_currency_minor(&loc("ja-JP"), 1_500, Currency::Jpy), "¥1,500");
    assert_eq!(format_currency_minor(&loc("en-US"), 12_345, Currency::Kwd), "KWD\u{00A0}12.345");
}

#[test]
fn calendar_date_names_weekday_and_first_day_of_week() {
    // Leap-year validation
    assert!(CalendarDate::new(2024, 2, 29).is_ok());
    assert!(CalendarDate::new(2000, 2, 29).is_ok());
    assert!(CalendarDate::new(1900, 2, 29).is_err());
    assert!(CalendarDate::new(2026, 2, 29).is_err());
    assert!(CalendarDate::new(2026, 13, 1).is_err());
    assert!(CalendarDate::new(2026, 4, 31).is_err());

    // Exact weekday calculation
    assert_eq!(CalendarDate::new(1970, 1, 1).unwrap().weekday(), Weekday::Thursday);
    assert_eq!(CalendarDate::new(2024, 2, 29).unwrap().weekday(), Weekday::Thursday);
    assert_eq!(CalendarDate::new(2026, 11, 25).unwrap().weekday(), Weekday::Wednesday);

    let d = CalendarDate::new(2026, 3, 5).unwrap();
    assert_eq!(format_date(&loc("en-US"), d, DateStyle::Short), "3/5/2026");
    assert_eq!(format_date(&loc("en-US"), d, DateStyle::Medium), "Mar 5, 2026");
    assert_eq!(format_date(&loc("en-US"), d, DateStyle::Long), "March 5, 2026");

    assert_eq!(format_date(&loc("en-GB"), d, DateStyle::Short), "05/03/2026");
    assert_eq!(format_date(&loc("de-DE"), d, DateStyle::Short), "05.03.2026");
    assert_eq!(format_date(&loc("de-DE"), d, DateStyle::Long), "5. März 2026");
    assert_eq!(format_date(&loc("fr-FR"), d, DateStyle::Long), "5 mars 2026");
    assert_eq!(format_date(&loc("ja-JP"), d, DateStyle::Long), "2026年3月5日");
    assert_eq!(format_date(&loc("ko-KR"), d, DateStyle::Long), "2026년 3월 5일");

    // Localized names
    assert_eq!(month_name(&loc("de-DE"), 3, NameWidth::Wide), "März");
    assert_eq!(weekday_name(&loc("fr-FR"), Weekday::Monday, NameWidth::Wide), "lundi");
    assert_eq!(weekday_name(&loc("ja-JP"), Weekday::Sunday, NameWidth::Abbreviated), "日");

    // First day of week (`Sunday`, `Monday`, `Saturday`) & ordered weekday grid
    assert_eq!(first_day_of_week(&loc("en-US")), Weekday::Sunday);
    assert_eq!(first_day_of_week(&loc("ja-JP")), Weekday::Sunday);
    assert_eq!(first_day_of_week(&loc("de-DE")), Weekday::Monday);
    assert_eq!(first_day_of_week(&loc("en-GB")), Weekday::Monday);
    assert_eq!(first_day_of_week(&loc("ar-EG")), Weekday::Saturday);
    assert_eq!(first_day_of_week(&loc("fa-IR")), Weekday::Saturday);

    assert_eq!(ordered_weekdays(&loc("de-DE"))[0], Weekday::Monday);
    assert_eq!(ordered_weekdays(&loc("de-DE"))[6], Weekday::Sunday);
    assert_eq!(ordered_weekdays(&loc("ar-EG"))[0], Weekday::Saturday);
}

#[test]
fn relative_time_formatting_with_cldr_plurals() {
    assert_eq!(format_relative_time(&loc("en-US"), -2, RelativeUnit::Hour), "2 hours ago");
    assert_eq!(format_relative_time(&loc("en-US"), -1, RelativeUnit::Hour), "1 hour ago");
    assert_eq!(format_relative_time(&loc("en-US"), 3, RelativeUnit::Day), "in 3 days");
    assert_eq!(format_relative_time(&loc("de-DE"), -2, RelativeUnit::Hour), "vor 2 Stunden");
    assert_eq!(format_relative_time(&loc("de-DE"), 3, RelativeUnit::Day), "in 3 Tagen");
    assert_eq!(format_relative_time(&loc("fr-FR"), -2, RelativeUnit::Hour), "il y a 2 heures");
    assert_eq!(format_relative_time(&loc("ja-JP"), -2, RelativeUnit::Hour), "2時間前");

    // Slavic CLDR pluralization inside relative time!
    assert_eq!(format_relative_time(&loc("pl-PL"), -1, RelativeUnit::Hour), "1 godzinę temu");
    assert_eq!(format_relative_time(&loc("pl-PL"), -2, RelativeUnit::Hour), "2 godziny temu");
    assert_eq!(format_relative_time(&loc("pl-PL"), -5, RelativeUnit::Hour), "5 godzin temu");

    assert_eq!(format_relative_time(&loc("ru-RU"), -1, RelativeUnit::Hour), "1 час назад");
    assert_eq!(format_relative_time(&loc("ru-RU"), -2, RelativeUnit::Hour), "2 часа назад");
    assert_eq!(format_relative_time(&loc("ru-RU"), -5, RelativeUnit::Hour), "5 часов назад");

    // Auto human-friendly strings
    assert_eq!(format_relative_time_auto(&loc("en-US"), 0, RelativeUnit::Day), "today");
    assert_eq!(format_relative_time_auto(&loc("en-US"), -1, RelativeUnit::Day), "yesterday");
    assert_eq!(format_relative_time_auto(&loc("en-US"), 1, RelativeUnit::Day), "tomorrow");
    assert_eq!(format_relative_time_auto(&loc("de-DE"), 0, RelativeUnit::Day), "heute");
    assert_eq!(format_relative_time_auto(&loc("de-DE"), 0, RelativeUnit::Second), "jetzt");
}

#[test]
fn pseudo_localization_wraps_accents_expands_and_preserves_icu() {
    let sample = "Settings and preferences panel";
    let pseudo = pseudo_localize(sample);
    assert!(pseudo.starts_with('[') && pseudo.ends_with(']'));

    // Accented lookalikes replace ASCII letters
    let inner = &pseudo[1..pseudo.len() - 1];
    assert!(
        inner.chars().all(|c| !c.is_ascii_alphabetic()),
        "all ASCII letters should be accented, got: {pseudo}"
    );

    // Expansion is within ~35-40% of visible length
    let orig_chars = sample.chars().count();
    let expanded_chars = inner.chars().count();
    let ratio = (expanded_chars as f64 - orig_chars as f64) / orig_chars as f64;
    assert!(
        (0.35..=0.42).contains(&ratio),
        "expected ~35-40% expansion, got {:.1}% ({orig_chars} -> {expanded_chars}): {pseudo}",
        ratio * 100.0
    );

    // Simple ICU placeholders remain intact
    let with_ph = pseudo_localize("Hello, {username}! Balance: {amount, number}");
    assert!(with_ph.contains("{username}"));
    assert!(with_ph.contains("{amount, number}"));
    assert!(parse_message(&with_ph).is_ok());

    // Complex ICU plural block remains valid and formattable!
    let plural_tpl = "{count, plural, =0 {No files} one {# file} other {# files}}";
    let pseudo_plural = pseudo_localize(plural_tpl);
    assert!(parse_message(&pseudo_plural).is_ok(), "pseudo plural must remain valid ICU: {pseudo_plural}");
    let rendered = format_message(&loc("en-US"), &pseudo_plural, &[("count", 3.into())]).unwrap();
    assert!(rendered.starts_with('[') && rendered.ends_with(']'));
    assert!(rendered.contains('3'), "plural `#` must still interpolate count: {rendered}");
}

#[test]
fn catalog_fallback_resolution_pseudo_mode_and_diagnostics() {
    let mut catalog = MessageCatalog::new(loc("en"));
    catalog
        .insert_bundle(
            loc("en"),
            [
                ("greeting", "Hello, {name}!"),
                ("items", "{count, plural, one {# item} other {# items}}"),
                ("only_en", "Fallback English copy"),
            ],
        )
        .unwrap();
    catalog
        .insert(loc("zh"), "greeting", "你好，{name}！")
        .unwrap();
    catalog
        .insert(loc("zh-Hans"), "items", "{count, plural, other {# 个项目}}")
        .unwrap();

    // Audit missing keys for zh-Hans-CN against default `en`
    assert_eq!(catalog.missing_keys(&loc("zh-Hans-CN")), vec!["only_en"]);

    let mut tr = Translator::with_backend(catalog, loc("zh-Hans-CN"), BuiltinI18nBackend);
    assert_eq!(tr.t_args("greeting", &[("name", "Ada".into())]), "你好，Ada！");
    assert_eq!(tr.t_args("items", &[("count", 5.into())]), "5 个项目");
    assert_eq!(tr.t("only_en"), "Fallback English copy");

    // Missing key records structured diagnostic with exact fallback chain
    assert_eq!(tr.t("does.not.exist"), "does.not.exist");
    let diags = tr.take_missing_diagnostics();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].key, "does.not.exist");
    assert_eq!(
        diags[0]
            .tried_locales
            .iter()
            .map(|l| l.canonical_tag())
            .collect::<Vec<_>>(),
        vec!["zh-Hans-CN", "zh-Hans", "zh", "en"]
    );
    assert!(tr.missing_diagnostics().is_empty());

    // Strict try_t surfaces MissingKey
    assert!(matches!(tr.try_t("nope", &()), Err(I18nError::MissingKey(_))));

    // Pseudo-localization toggle preserves placeholder value while accenting copy
    tr.set_locale(loc("en-US"));
    tr.set_pseudo_localize(true);
    let pseudo_greeting = tr.t_args("greeting", &[("name", "Ada".into())]);
    assert!(pseudo_greeting.starts_with('[') && pseudo_greeting.ends_with(']'));
    assert!(pseudo_greeting.contains("Ada"));
    assert!(!pseudo_greeting.contains("Hello"));

    // Translator delegates formatting methods cleanly through static backend
    assert_eq!(tr.format_currency(19.99, "USD"), "$19.99");
    assert_eq!(tr.first_day_of_week(), Weekday::Sunday);
}
