//! Integration tests for [`NativeI18nBackend`] exercising real OS ICU formatting
//! (`CoreFoundation.framework` on Apple macOS/iOS), including locales outside the
//! pure-Rust builtin tables, BCP-47 `-u-` overrides (`ca`, `fw`, `hc`, `nu`),
//! regional metadata (`CFLocaleCopyCurrent` / `CFLocaleGetValue`), and a
//! 10,000-iteration cache/eviction stress loop.

use nve_i18n::{
    CalendarDate, CalendarTime, Currency, DateStyle, I18nBackend, LocaleId, MeasurementSystem,
    NativeI18nBackend, NumberFormatOptions, NumberStyle, RoundingMode, TimeStyle, Weekday,
    first_day_of_week, format_currency, format_date, format_decimal, format_integer, format_number,
    format_time, locale_metadata, system_locale,
};

fn loc(tag: &str) -> LocaleId {
    LocaleId::parse(tag).expect("valid test locale")
}

#[test]
fn native_backend_formats_diverse_locales_and_unicode_extensions() {
    let d = CalendarDate::new(2026, 3, 5).unwrap();
    let t = CalendarTime::new(14, 30, 5).unwrap();

    // First day of week across territories + explicit `-u-fw-*` overrides
    assert_eq!(first_day_of_week(&loc("sv-SE")), Weekday::Monday);
    assert_eq!(first_day_of_week(&loc("th-TH")), Weekday::Sunday);
    assert_eq!(first_day_of_week(&loc("he-IL")), Weekday::Sunday);
    assert_eq!(first_day_of_week(&loc("uk-UA")), Weekday::Monday);
    assert_eq!(first_day_of_week(&loc("hi-IN")), Weekday::Sunday);
    assert_eq!(first_day_of_week(&loc("cs-CZ")), Weekday::Monday);
    assert_eq!(first_day_of_week(&loc("ar-EG")), Weekday::Saturday);
    assert_eq!(first_day_of_week(&loc("en-US")), Weekday::Sunday);
    assert_eq!(first_day_of_week(&loc("en-US-u-fw-mon")), Weekday::Monday);
    assert_eq!(first_day_of_week(&loc("en-US-u-fw-fri")), Weekday::Friday);
    assert_eq!(first_day_of_week(&loc("de-DE-u-fw-sun")), Weekday::Sunday);

    // Exact 64-bit integer precision + special floats
    assert_eq!(
        format_integer(&loc("en-US"), i64::MIN),
        "-9,223,372,036,854,775,808"
    );
    assert_eq!(format_decimal(&loc("en-US"), -0.0), "0");
    assert_eq!(format_decimal(&loc("en-US"), f64::NAN), "NaN");
    assert!(format_decimal(&loc("en-US"), f64::INFINITY).contains('∞'));
    assert!(format_decimal(&loc("en-US"), f64::NEG_INFINITY).contains("-∞"));

    // Number styles (Percent, Scientific, ungrouped, rounding)
    let pct = format_number(
        &loc("en-US"),
        0.256,
        NumberFormatOptions::default().with_style(NumberStyle::Percent),
    );
    assert!(pct.contains("26") && pct.contains('%'), "unexpected percent: {pct}");

    let sci = format_number(
        &loc("en-US"),
        12345.6,
        NumberFormatOptions::default().with_style(NumberStyle::Scientific),
    );
    assert!(
        sci.contains("1.23456") && (sci.contains("E4") || sci.contains("e4")),
        "unexpected scientific: {sci}"
    );

    let ungrouped = format_number(
        &loc("en-US"),
        123456.7,
        NumberFormatOptions::default()
            .with_grouping(false)
            .with_fraction_digits(Some(2), Some(2)),
    );
    assert_eq!(ungrouped, "123456.70");

    let rounded = format_number(
        &loc("en-US"),
        999.995,
        NumberFormatOptions::default()
            .with_fraction_digits(Some(2), Some(2))
            .with_rounding_mode(RoundingMode::HalfAwayFromZero),
    );
    assert_eq!(rounded, "1,000.00");

    // `-u-ca-japanese` & `-u-hc-h12` overrides
    let ja_jp = format_date(&loc("ja-JP-u-ca-japanese"), d, DateStyle::Long);
    assert!(
        ja_jp.contains("令和") && ja_jp.contains('8'),
        "expected Japanese Reiwa era date for ja-JP-u-ca-japanese, got: {ja_jp}"
    );

    let de_24 = format_time(&loc("de-DE"), t, TimeStyle::Short);
    let de_12 = format_time(&loc("de-DE-u-hc-h12"), t, TimeStyle::Short);
    assert!(de_24.contains("14:30"), "expected 24h time for de-DE, got: {de_24}");
    assert!(
        de_12.contains("2:30") && !de_12.contains("14:30"),
        "expected 12h time for de-DE-u-hc-h12, got: {de_12}"
    );

    let us_23 = format_time(&loc("en-US-u-hc-h23"), t, TimeStyle::Short);
    assert!(
        us_23.contains("14:30"),
        "expected 24h time for en-US-u-hc-h23, got: {us_23}"
    );

    // `-u-nu-*` numbering system overrides (specifically covers macOS ar-EG-u-nu-arab edge case!)
    let ar_arab = format_decimal(&loc("ar-EG-u-nu-arab"), 1234.56);
    assert!(
        ar_arab.contains('١') && ar_arab.contains('٢'),
        "expected Arabic-Indic digits for ar-EG-u-nu-arab, got: {ar_arab}"
    );
    let ar_latn = format_decimal(&loc("ar-EG-u-nu-latn"), 1234.56);
    assert!(
        ar_latn.contains('1') && ar_latn.contains("234"),
        "expected Latin digits for ar-EG-u-nu-latn, got: {ar_latn}"
    );

    // Currency formatting on native backend
    let usd = format_currency(&loc("en-US"), 1234.5, Currency::Usd);
    assert_eq!(usd, "$1,234.50");
    let jpy = format_currency(&loc("ja-JP"), 1234.0, Currency::Jpy);
    assert!(jpy.contains("1,234") && !jpy.contains('.'), "expected 0-decimal JPY, got: {jpy}");
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn apple_corefoundation_real_os_icu_locales_outside_builtin_tables() {
    let d = CalendarDate::new(2026, 3, 5).unwrap();

    // `sv-SE`: space grouping, comma decimal, ISO short date, Swedish month "mars"
    assert_eq!(format_date(&loc("sv-SE"), d, DateStyle::Short), "2026-03-05");
    assert!(
        format_date(&loc("sv-SE"), d, DateStyle::Long).contains("mars"),
        "expected Swedish month from OS ICU"
    );
    let sv_num = format_decimal(&loc("sv-SE"), 123456.78);
    assert!(sv_num.contains(",78") && sv_num.contains("123"), "sv-SE num: {sv_num}");

    // `th-TH`: OS ICU uses Thai Buddhist calendar (2569) and Thai month names (`มีนาคม`),
    // NOT present in the pure-Rust builtin calendar table!
    let th_long = format_date(&loc("th-TH"), d, DateStyle::Long);
    assert!(
        th_long.contains("มีนาคม") && th_long.contains("2569"),
        "expected real macOS ICU Thai Buddhist date, got: {th_long}"
    );
    let th_greg = format_date(&loc("th-TH-u-ca-gregory"), d, DateStyle::Long);
    assert!(
        th_greg.contains("มีนาคม") && th_greg.contains("2026"),
        "expected Thai Gregorian override date, got: {th_greg}"
    );

    // `he-IL`: OS ICU produces Hebrew month name (`מרץ`), absent from builtin table!
    let he_long = format_date(&loc("he-IL"), d, DateStyle::Long);
    assert!(
        he_long.contains("מרץ") && he_long.contains("2026"),
        "expected real macOS ICU Hebrew date, got: {he_long}"
    );

    // `uk-UA`: OS ICU produces Ukrainian genitive month (`березня`)!
    let uk_long = format_date(&loc("uk-UA"), d, DateStyle::Long);
    assert!(
        uk_long.contains("березня") && uk_long.contains("2026"),
        "expected real macOS ICU Ukrainian date, got: {uk_long}"
    );

    // `hi-IN`: OS ICU produces Indian lakh grouping (`1,23,456.78`) and Hindi Devanagari month (`मार्च`)!
    assert_eq!(format_decimal(&loc("hi-IN"), 123456.78), "1,23,456.78");
    let hi_long = format_date(&loc("hi-IN"), d, DateStyle::Long);
    assert!(
        hi_long.contains("मार्च") && hi_long.contains("2026"),
        "expected real macOS ICU Hindi date, got: {hi_long}"
    );

    // `cs-CZ`: OS ICU produces Czech genitive month (`března`), absent from builtin table!
    let cs_long = format_date(&loc("cs-CZ"), d, DateStyle::Long);
    assert!(
        cs_long.contains("března") && cs_long.contains("2026"),
        "expected real macOS ICU Czech date, got: {cs_long}"
    );

    // `CFLocaleCopyCurrent` & `CFLocaleGetValue` regional metadata
    assert!(
        system_locale().is_some(),
        "CFLocaleCopyCurrent should resolve a valid LocaleId on macOS"
    );

    let us_meta = locale_metadata(&loc("en-US"));
    assert_eq!(us_meta.decimal_separator, ".");
    assert_eq!(us_meta.grouping_separator, ",");
    assert_eq!(us_meta.currency_code.as_deref(), Some("USD"));
    assert_eq!(us_meta.currency_symbol.as_deref(), Some("$"));
    assert_eq!(us_meta.measurement_system, MeasurementSystem::Us);

    let de_meta = locale_metadata(&loc("de-DE"));
    assert_eq!(de_meta.decimal_separator, ",");
    assert_eq!(de_meta.grouping_separator, ".");
    assert_eq!(de_meta.currency_code.as_deref(), Some("EUR"));
    assert_eq!(de_meta.measurement_system, MeasurementSystem::Metric);

    let gb_meta = locale_metadata(&loc("en-GB"));
    assert_eq!(gb_meta.currency_code.as_deref(), Some("GBP"));
    assert_eq!(gb_meta.measurement_system, MeasurementSystem::Uk);
}

#[test]
fn native_backend_10000_iteration_stress_and_cache_eviction_loop() {
    // Include > 32 distinct locale/extension combinations to force repeated FIFO cache
    // evictions and `CFRelease` drops alongside hot-path reuse.
    let locales: Vec<LocaleId> = [
        "en-US", "en-GB", "de-DE", "de-CH", "fr-FR", "es-ES", "it-IT", "pt-BR",
        "ja-JP", "zh-Hans-CN", "ko-KR", "ar-EG", "sv-SE", "th-TH", "he-IL", "uk-UA",
        "hi-IN", "cs-CZ", "pl-PL", "ru-RU", "nl-NL", "da-DK", "fi-FI", "nb-NO",
        "tr-TR", "vi-VN", "id-ID", "el-GR", "hu-HU", "ro-RO", "sk-SK", "bg-BG",
        "ja-JP-u-ca-japanese", "en-US-u-fw-mon", "de-DE-u-hc-h12", "ar-EG-u-nu-arab",
        "ar-EG-u-nu-latn", "en-US-u-ca-buddhist-fw-fri-hc-h24-nu-latn", "th-TH-u-ca-gregory",
        "ja-JP-u-ca-japanese-nu-hanidec",
    ]
    .into_iter()
    .map(loc)
    .collect();

    let date = CalendarDate::new(2026, 3, 5).unwrap();
    let time = CalendarTime::new(14, 30, 5).unwrap();
    let opts = NumberFormatOptions::default();
    let mut scratch = String::with_capacity(64);

    for i in 0..10_000usize {
        let locale = &locales[i % locales.len()];
        scratch.clear();
        NativeI18nBackend.format_integer_into(locale, (i as i64) * 123 - 500_000, &mut scratch);
        assert!(!scratch.is_empty());

        scratch.clear();
        NativeI18nBackend.format_number_into(locale, (i as f64) * 0.37, opts, &mut scratch);
        assert!(!scratch.is_empty());

        scratch.clear();
        NativeI18nBackend.format_date_into(locale, date, DateStyle::Medium, &mut scratch);
        assert!(!scratch.is_empty());

        if i % 4 == 0 {
            scratch.clear();
            NativeI18nBackend.format_time_into(locale, time, TimeStyle::Short, &mut scratch);
            assert!(!scratch.is_empty());

            scratch.clear();
            NativeI18nBackend.format_currency_into(locale, 99.95, "EUR", &mut scratch);
            assert!(!scratch.is_empty());

            let _ = NativeI18nBackend.first_day_of_week(locale);
        }
    }
}
