//! Comprehensive tests for deterministic `BuiltinI18nBackend` formatting (number,
//! currency, calendar date/time, BCP-47 `-u-` overrides, first-day-of-week,
//! relative-time), pseudo-localization, and `MessageCatalog`/`Translator`.
//!
//! Exact golden strings are asserted against `BuiltinI18nBackend` explicitly so
//! these tests never flake across OS ICU data updates; OS-native Apple
//! `CoreFoundation` ICU coverage lives in `tests/native_backend_tests.rs`.

use nve_i18n::{
    BuiltinI18nBackend, CalendarDate, CalendarTime, Currency, DateStyle, I18nBackend, I18nError,
    LocaleId, MessageCatalog, NameWidth, NumberFormatOptions, NumberStyle, RelativeUnit,
    RoundingMode, TimeStyle, Translator, Weekday, format_currency_minor, format_message,
    format_relative_time_auto, month_name, ordered_weekdays, parse_message, pseudo_localize,
    weekday_name,
};

fn loc(tag: &str) -> LocaleId {
    LocaleId::parse(tag).expect("valid locale")
}

#[test]
fn builtin_number_formatting_across_locales_styles_rounding_and_nu_extensions() {
    let b = BuiltinI18nBackend;
    let def = NumberFormatOptions::default();

    assert_eq!(b.format_number(&loc("en-US"), 1234.56, def), "1,234.56");
    assert_eq!(b.format_number(&loc("de-DE"), 1234.56, def), "1.234,56");
    assert_eq!(b.format_number(&loc("fr-FR"), 1234.56, def), "1\u{202F}234,56");
    assert_eq!(b.format_number(&loc("en-IN"), 123456.78, def), "1,23,456.78");
    assert_eq!(b.format_number(&loc("en-IN"), 12345678.9, def), "1,23,45,678.9");
    assert_eq!(b.format_number(&loc("de-CH"), 1234.56, def), "1'234.56");
    assert_eq!(b.format_number(&loc("pl-PL"), 1234.56, def), "1\u{00A0}234,56");

    // Negative zero normalized to `"0"`
    assert_eq!(b.format_number(&loc("en-US"), -0.0, def), "0");
    assert_eq!(b.format_number(&loc("en-US"), f64::NAN, def), "NaN");
    assert_eq!(b.format_number(&loc("en-US"), f64::INFINITY, def), "∞");
    assert_eq!(b.format_number(&loc("en-US"), f64::NEG_INFINITY, def), "-∞");

    // Exact 64-bit integer formatting including i64::MIN
    assert_eq!(
        b.format_integer(&loc("en-US"), i64::MIN),
        "-9,223,372,036,854,775,808"
    );

    // Rounding with full carry propagation (`999.995` -> `1,000.00`)
    let rounded = b.format_number(
        &loc("en-US"),
        999.995,
        NumberFormatOptions {
            use_grouping: true,
            min_fraction_digits: Some(2),
            max_fraction_digits: Some(2),
            ..NumberFormatOptions::default()
        },
    );
    assert_eq!(rounded, "1,000.00");

    // HalfEven, Floor, Ceiling, Truncate rounding modes
    let half_even = NumberFormatOptions::default()
        .with_fraction_digits(Some(0), Some(0))
        .with_rounding_mode(RoundingMode::HalfEven);
    assert_eq!(b.format_number(&loc("en-US"), 2.5, half_even), "2");
    assert_eq!(b.format_number(&loc("en-US"), 3.5, half_even), "4");

    let floor = NumberFormatOptions::default()
        .with_fraction_digits(Some(0), Some(0))
        .with_rounding_mode(RoundingMode::Floor);
    assert_eq!(b.format_number(&loc("en-US"), 1.9, floor), "1");
    assert_eq!(b.format_number(&loc("en-US"), -1.1, floor), "-2");

    let ceil = NumberFormatOptions::default()
        .with_fraction_digits(Some(0), Some(0))
        .with_rounding_mode(RoundingMode::Ceiling);
    assert_eq!(b.format_number(&loc("en-US"), 1.1, ceil), "2");
    assert_eq!(b.format_number(&loc("en-US"), -1.9, ceil), "-1");

    let trunc = NumberFormatOptions::default()
        .with_fraction_digits(Some(0), Some(0))
        .with_rounding_mode(RoundingMode::Truncate);
    assert_eq!(b.format_number(&loc("en-US"), 1.9, trunc), "1");
    assert_eq!(b.format_number(&loc("en-US"), -1.9, trunc), "-1");

    // Percent and Scientific styles
    assert_eq!(
        b.format_number(
            &loc("en-US"),
            0.256,
            NumberFormatOptions::default().with_style(NumberStyle::Percent)
        ),
        "26%"
    );
    assert_eq!(
        b.format_number(
            &loc("en-US"),
            12345.6,
            NumberFormatOptions::default().with_style(NumberStyle::Scientific)
        ),
        "1.23456E4"
    );

    // Grouping disabled
    let ungrouped = b.format_number(
        &loc("en-US"),
        123456.7,
        NumberFormatOptions {
            use_grouping: false,
            min_fraction_digits: Some(2),
            max_fraction_digits: Some(2),
            ..NumberFormatOptions::default()
        },
    );
    assert_eq!(ungrouped, "123456.70");

    // `-u-nu-*` numbering system override on builtin backend
    assert_eq!(b.format_number(&loc("ar-EG-u-nu-arab"), 1234.56, def), "١٬٢٣٤٫٥٦");
    assert_eq!(b.format_integer(&loc("en-US-u-nu-thai"), 2026), "๒,๐๒๖");
    assert_eq!(b.format_integer(&loc("ja-JP-u-nu-hanidec"), 1234), "一,二三四");
}

#[test]
fn builtin_number_percent_fraction_min_max_overflow_subnormal_and_scientific_carry() {
    let b = BuiltinI18nBackend;
    let us = loc("en-US");
    let de = loc("de-DE");

    // Percent with `min_fraction_digits = Some(2), max = None`: must NOT round to 0 then pad!
    let pct_min2 = NumberFormatOptions::default()
        .with_style(NumberStyle::Percent)
        .with_fraction_digits(Some(2), None);
    assert_eq!(b.format_number(&us, 0.2567, pct_min2), "25.67%");
    assert_eq!(b.format_number(&us, 0.25, pct_min2), "25.00%");
    assert_eq!(b.format_number(&de, 0.2567, pct_min2), "25,67\u{00A0}%");

    // Contradictory `min > max` aligns by clamping `eff_max = max.max(min)` across all styles.
    let dec_contradict = NumberFormatOptions::default().with_fraction_digits(Some(3), Some(1));
    assert_eq!(b.format_number(&us, 1.2346, dec_contradict), "1.235");
    assert_eq!(b.format_number(&us, 1.2, dec_contradict), "1.200");

    let pct_contradict = NumberFormatOptions::default()
        .with_style(NumberStyle::Percent)
        .with_fraction_digits(Some(3), Some(1));
    assert_eq!(b.format_number(&us, 0.25678, pct_contradict), "25.678%");

    let sci_contradict = NumberFormatOptions::default()
        .with_style(NumberStyle::Scientific)
        .with_fraction_digits(Some(3), Some(1));
    assert_eq!(b.format_number(&us, 9.999, sci_contradict), "9.999E0");
    assert_eq!(b.format_number(&us, 1.2, sci_contradict), "1.200E0");

    // Percent multiplication overflow (`1e307 * 100.0 == INFINITY`) + special floats
    let pct_def = NumberFormatOptions::default().with_style(NumberStyle::Percent);
    assert_eq!(b.format_number(&us, 1e307, pct_def), "∞%");
    assert_eq!(b.format_number(&us, -1e307, pct_def), "-∞%");
    assert_eq!(b.format_number(&de, 1e307, pct_def), "∞\u{00A0}%");
    assert_eq!(b.format_number(&us, f64::INFINITY, pct_def), "∞%");
    assert_eq!(b.format_number(&us, f64::NEG_INFINITY, pct_def), "-∞%");
    assert_eq!(b.format_number(&us, f64::NAN, pct_def), "NaN");
    assert_eq!(b.format_number(&us, -0.0, pct_def), "0%");

    // Scientific subnormal (`5e-324` = smallest positive f64) & `f64::MIN_POSITIVE`
    let sci_def = NumberFormatOptions::default().with_style(NumberStyle::Scientific);
    let subnormal = f64::from_bits(1);
    assert_eq!(b.format_number(&us, subnormal, sci_def), "5E-324");
    assert_eq!(
        b.format_number(&us, subnormal, sci_def.with_fraction_digits(Some(2), Some(2))),
        "5.00E-324"
    );
    assert_eq!(
        b.format_number(&us, f64::MIN_POSITIVE, sci_def.with_fraction_digits(None, Some(2))),
        "2.23E-308"
    );

    // Scientific decade carry (`9.999` rounded to 2 fraction digits -> `1.00E1`, NOT `10.00E0`!)
    assert_eq!(
        b.format_number(&us, 9.999, sci_def.with_fraction_digits(Some(2), Some(2))),
        "1.00E1"
    );
    assert_eq!(
        b.format_number(&us, 9.999, sci_def.with_fraction_digits(None, Some(2))),
        "1E1"
    );
    assert_eq!(
        b.format_number(&us, -9.999, sci_def.with_fraction_digits(Some(2), Some(2))),
        "-1.00E1"
    );
    assert_eq!(
        b.format_number(&us, 0.9999, sci_def.with_fraction_digits(Some(2), Some(2))),
        "1.00E0"
    );
    assert_eq!(
        b.format_number(&us, 9.9, sci_def.with_fraction_digits(Some(0), Some(0))),
        "1E1"
    );
    assert_eq!(
        b.format_number(
            &us,
            9.999,
            sci_def
                .with_fraction_digits(Some(2), Some(2))
                .with_rounding_mode(RoundingMode::Truncate)
        ),
        "9.99E0"
    );

    // Scientific special floats
    assert_eq!(b.format_number(&us, -0.0, sci_def), "0E0");
    assert_eq!(b.format_number(&us, f64::NAN, sci_def), "NaN");
    assert_eq!(b.format_number(&us, f64::INFINITY, sci_def), "∞");
    assert_eq!(b.format_number(&us, f64::NEG_INFINITY, sci_def), "-∞");
}

#[test]
fn builtin_currency_formatting_precision_placement_and_minor_units() {
    let b = BuiltinI18nBackend;
    assert_eq!(b.format_currency(&loc("en-US"), 1234.5, "USD"), "$1,234.50");
    assert_eq!(b.format_currency(&loc("en-GB"), 1234.56, "GBP"), "£1,234.56");
    assert_eq!(b.format_currency(&loc("de-DE"), 1234.56, "EUR"), "1.234,56\u{00A0}€");
    assert_eq!(
        b.format_currency(&loc("fr-FR"), 1234.56, "EUR"),
        "1\u{202F}234,56\u{00A0}€"
    );
    assert_eq!(b.format_currency(&loc("ja-JP"), 1234.0, "JPY"), "¥1,234");
    assert_eq!(b.format_currency(&loc("de-CH"), 1234.56, "CHF"), "CHF\u{00A0}1'234.56");
    assert_eq!(b.format_currency(&loc("en-IN"), 123456.78, "INR"), "₹1,23,456.78");
    assert_eq!(b.format_currency(&loc("en-US"), -42.5, "USD"), "-$42.50");
    assert_eq!(b.format_currency(&loc("de-DE"), -42.5, "EUR"), "-42,50\u{00A0}€");

    // 3-decimal currency (KWD) & exact integer minor-unit formatting
    assert_eq!(format_currency_minor(&loc("en-US"), 123_456, Currency::Usd), "$1,234.56");
    assert_eq!(format_currency_minor(&loc("ja-JP"), 1_500, Currency::Jpy), "¥1,500");
    assert_eq!(
        format_currency_minor(&loc("en-US"), 12_345, Currency::Kwd),
        "KWD\u{00A0}12.345"
    );
}

#[test]
fn builtin_calendar_date_time_names_weekday_and_unicode_overrides() {
    let b = BuiltinI18nBackend;

    // Leap-year & time validation
    assert!(CalendarDate::new(2024, 2, 29).is_ok());
    assert!(CalendarDate::new(2000, 2, 29).is_ok());
    assert!(CalendarDate::new(1900, 2, 29).is_err());
    assert!(CalendarDate::new(2026, 2, 29).is_err());
    assert!(CalendarDate::new(2026, 13, 1).is_err());
    assert!(CalendarDate::new(2026, 4, 31).is_err());
    assert!(CalendarTime::new(23, 59, 59).is_ok());
    assert!(CalendarTime::new(24, 0, 0).is_err());
    assert!(CalendarTime::new(12, 60, 0).is_err());

    // Exact weekday calculation
    assert_eq!(CalendarDate::new(1970, 1, 1).unwrap().weekday(), Weekday::Thursday);
    assert_eq!(CalendarDate::new(2024, 2, 29).unwrap().weekday(), Weekday::Thursday);
    assert_eq!(CalendarDate::new(2026, 11, 25).unwrap().weekday(), Weekday::Wednesday);

    let d = CalendarDate::new(2026, 3, 5).unwrap();
    assert_eq!(b.format_date(&loc("en-US"), d, DateStyle::Short), "3/5/2026");
    assert_eq!(b.format_date(&loc("en-US"), d, DateStyle::Medium), "Mar 5, 2026");
    assert_eq!(b.format_date(&loc("en-US"), d, DateStyle::Long), "March 5, 2026");

    assert_eq!(b.format_date(&loc("en-GB"), d, DateStyle::Short), "05/03/2026");
    assert_eq!(b.format_date(&loc("de-DE"), d, DateStyle::Short), "05.03.2026");
    assert_eq!(b.format_date(&loc("de-DE"), d, DateStyle::Long), "5. März 2026");
    assert_eq!(b.format_date(&loc("fr-FR"), d, DateStyle::Long), "5 mars 2026");
    assert_eq!(b.format_date(&loc("ja-JP"), d, DateStyle::Long), "2026年3月5日");
    assert_eq!(b.format_date(&loc("ko-KR"), d, DateStyle::Long), "2026년 3월 5일");

    // `-u-ca-*` calendar overrides on builtin backend
    assert_eq!(b.format_date(&loc("ja-JP-u-ca-japanese"), d, DateStyle::Long), "令和8年3月5日");
    assert_eq!(b.format_date(&loc("ja-JP-u-ca-japanese"), d, DateStyle::Short), "R8/03/05");
    assert_eq!(b.format_date(&loc("en-US-u-ca-iso8601"), d, DateStyle::Short), "2026-03-05");
    assert_eq!(b.format_date(&loc("en-US-u-ca-buddhist"), d, DateStyle::Long), "March 5, 2569 BE");

    // `-u-hc-*` hour-cycle overrides on builtin backend
    let t = CalendarTime::new(14, 30, 5).unwrap();
    let midnight = CalendarTime::new(0, 5, 0).unwrap();
    assert_eq!(b.format_time(&loc("en-US"), t, TimeStyle::Short), "2:30 PM");
    assert_eq!(b.format_time(&loc("en-US-u-hc-h23"), t, TimeStyle::Short), "14:30");
    assert_eq!(b.format_time(&loc("de-DE"), t, TimeStyle::Short), "14:30");
    assert_eq!(b.format_time(&loc("de-DE-u-hc-h12"), t, TimeStyle::Short), "2:30 PM");
    assert_eq!(b.format_time(&loc("en-US-u-hc-h11"), midnight, TimeStyle::Short), "0:05 AM");
    assert_eq!(b.format_time(&loc("en-US-u-hc-h24"), midnight, TimeStyle::Short), "24:05");

    // Localized names
    assert_eq!(month_name(&loc("de-DE"), 3, NameWidth::Wide), "März");
    assert_eq!(weekday_name(&loc("fr-FR"), Weekday::Monday, NameWidth::Wide), "lundi");
    assert_eq!(weekday_name(&loc("ja-JP"), Weekday::Sunday, NameWidth::Abbreviated), "日");

    // First day of week (`Sunday`, `Monday`, `Saturday`, plus `-u-fw-*` overrides)
    assert_eq!(b.first_day_of_week(&loc("en-US")), Weekday::Sunday);
    assert_eq!(b.first_day_of_week(&loc("en-US-u-fw-mon")), Weekday::Monday);
    assert_eq!(b.first_day_of_week(&loc("de-DE-u-fw-sun")), Weekday::Sunday);
    assert_eq!(b.first_day_of_week(&loc("ja-JP")), Weekday::Sunday);
    assert_eq!(b.first_day_of_week(&loc("de-DE")), Weekday::Monday);
    assert_eq!(b.first_day_of_week(&loc("en-GB")), Weekday::Monday);
    assert_eq!(b.first_day_of_week(&loc("ar-EG")), Weekday::Saturday);
    assert_eq!(b.first_day_of_week(&loc("fa-IR")), Weekday::Saturday);

    assert_eq!(ordered_weekdays(&loc("de-DE"))[0], Weekday::Monday);
    assert_eq!(ordered_weekdays(&loc("de-DE"))[6], Weekday::Sunday);
    assert_eq!(ordered_weekdays(&loc("ar-EG"))[0], Weekday::Saturday);
    assert_eq!(ordered_weekdays(&loc("en-US-u-fw-mon"))[0], Weekday::Monday);
}

#[test]
fn relative_time_formatting_with_cldr_plurals() {
    let b = BuiltinI18nBackend;
    assert_eq!(b.format_relative_time(&loc("en-US"), -2, RelativeUnit::Hour), "2 hours ago");
    assert_eq!(b.format_relative_time(&loc("en-US"), -1, RelativeUnit::Hour), "1 hour ago");
    assert_eq!(b.format_relative_time(&loc("en-US"), 3, RelativeUnit::Day), "in 3 days");
    assert_eq!(b.format_relative_time(&loc("de-DE"), -2, RelativeUnit::Hour), "vor 2 Stunden");
    assert_eq!(b.format_relative_time(&loc("de-DE"), 3, RelativeUnit::Day), "in 3 Tagen");
    assert_eq!(b.format_relative_time(&loc("fr-FR"), -2, RelativeUnit::Hour), "il y a 2 heures");
    assert_eq!(b.format_relative_time(&loc("ja-JP"), -2, RelativeUnit::Hour), "2時間前");

    // Slavic CLDR pluralization inside relative time!
    assert_eq!(b.format_relative_time(&loc("pl-PL"), -1, RelativeUnit::Hour), "1 godzinę temu");
    assert_eq!(b.format_relative_time(&loc("pl-PL"), -2, RelativeUnit::Hour), "2 godziny temu");
    assert_eq!(b.format_relative_time(&loc("pl-PL"), -5, RelativeUnit::Hour), "5 godzin temu");

    assert_eq!(b.format_relative_time(&loc("ru-RU"), -1, RelativeUnit::Hour), "1 час назад");
    assert_eq!(b.format_relative_time(&loc("ru-RU"), -2, RelativeUnit::Hour), "2 часа назад");
    assert_eq!(b.format_relative_time(&loc("ru-RU"), -5, RelativeUnit::Hour), "5 часов назад");

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

    let inner = &pseudo[1..pseudo.len() - 1];
    assert!(
        inner.chars().all(|c| !c.is_ascii_alphabetic()),
        "all ASCII letters should be accented, got: {pseudo}"
    );

    let orig_chars = sample.chars().count();
    let expanded_chars = inner.chars().count();
    let ratio = (expanded_chars as f64 - orig_chars as f64) / orig_chars as f64;
    assert!(
        (0.35..=0.42).contains(&ratio),
        "expected ~35-40% expansion, got {:.1}% ({orig_chars} -> {expanded_chars}): {pseudo}",
        ratio * 100.0
    );

    let with_ph = pseudo_localize("Hello, {username}! Balance: {amount, number}");
    assert!(with_ph.contains("{username}"));
    assert!(with_ph.contains("{amount, number}"));
    assert!(parse_message(&with_ph).is_ok());

    let plural_tpl = "{count, plural, =0 {No files} one {# file} other {# files}}";
    let pseudo_plural = pseudo_localize(plural_tpl);
    assert!(
        parse_message(&pseudo_plural).is_ok(),
        "pseudo plural must remain valid ICU: {pseudo_plural}"
    );
    let rendered = format_message(&loc("en-US"), &pseudo_plural, &[("count", 3.into())]).unwrap();
    assert!(rendered.starts_with('[') && rendered.ends_with(']'));
    assert!(
        rendered.contains('3'),
        "plural `#` must still interpolate count: {rendered}"
    );
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

    let mut tr = Translator::with_backend(catalog, loc("zh-Hans-CN-u-fw-mon"), BuiltinI18nBackend);
    assert_eq!(tr.t_args("greeting", &[("name", "Ada".into())]), "你好，Ada！");
    assert_eq!(tr.t_args("items", &[("count", 5.into())]), "5 个项目");
    assert_eq!(tr.t("only_en"), "Fallback English copy");
    assert_eq!(tr.first_day_of_week(), Weekday::Monday);

    // Missing key records structured diagnostic with exact fallback chain (stripping `-u-` first)
    tr.set_locale(loc("zh-Hans-CN"));
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

    assert!(matches!(tr.try_t("nope", &()), Err(I18nError::MissingKey(_))));

    tr.set_locale(loc("en-US"));
    tr.set_pseudo_localize(true);
    let pseudo_greeting = tr.t_args("greeting", &[("name", "Ada".into())]);
    assert!(pseudo_greeting.starts_with('[') && pseudo_greeting.ends_with(']'));
    assert!(pseudo_greeting.contains("Ada"));
    assert!(!pseudo_greeting.contains("Hello"));

    assert_eq!(tr.format_currency(19.99, "USD"), "$19.99");
    assert_eq!(tr.first_day_of_week(), Weekday::Sunday);
}
