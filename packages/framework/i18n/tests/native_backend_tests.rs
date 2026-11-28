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

#[test]
fn native_backend_percent_fraction_min_max_currency_and_rounding_contracts() {
    let us = loc("en-US");
    let de = loc("de-DE");

    // Percent with `min = Some(2), max = None` must preserve fractional digits (`25.67%`, not `26.00%`).
    let pct_min2 = NumberFormatOptions::default()
        .with_style(NumberStyle::Percent)
        .with_fraction_digits(Some(2), None);
    let pct = format_number(&us, 0.2567, pct_min2);
    assert_eq!(pct, "25.67%", "Percent with min=2, max=None must not round to 0 digits");

    // Contradictory `min > max` aligns across all backends (`eff_max = max.max(min)`).
    let dec_contradict = NumberFormatOptions::default().with_fraction_digits(Some(3), Some(1));
    assert_eq!(format_number(&us, 1.2346, dec_contradict), "1.235");
    assert_eq!(format_number(&us, 1.2, dec_contradict), "1.200");

    let pct_contradict = NumberFormatOptions::default()
        .with_style(NumberStyle::Percent)
        .with_fraction_digits(Some(3), Some(1));
    assert_eq!(format_number(&us, 0.25678, pct_contradict), "25.678%");

    // Cross-locale currency formatting (falsifies removal of `kCFNumberFormatterCurrencyCode`).
    let us_eur = format_currency(&us, 1234.56, Currency::Eur);
    assert!(
        (us_eur.contains('€') || us_eur.contains("EUR"))
            && !us_eur.contains('$')
            && us_eur.contains("1,234.56"),
        "expected EUR symbol & US grouping for en-US + EUR, got: {us_eur}"
    );
    let us_jpy = format_currency(&us, 1234.0, Currency::Jpy);
    assert!(
        (us_jpy.contains('¥') || us_jpy.contains("JPY"))
            && !us_jpy.contains('$')
            && us_jpy.contains("1,234")
            && !us_jpy.contains('.'),
        "expected 0-decimal JPY in en-US, got: {us_jpy}"
    );
    let us_kwd = format_currency(&us, 12.345, Currency::Kwd);
    assert!(
        us_kwd.contains("KWD") && us_kwd.contains("12.345"),
        "expected 3-decimal KWD in en-US, got: {us_kwd}"
    );
    let de_usd = format_currency(&de, 1234.56, Currency::Usd);
    assert!(
        (de_usd.contains('$') || de_usd.contains("USD"))
            && !de_usd.contains('€')
            && de_usd.contains("1.234,56"),
        "expected USD symbol & German separators for de-DE + USD, got: {de_usd}"
    );

    // Exact-tie & directional rounding (falsifies removal of `kCFNumberFormatterRoundingMode`).
    let int_opts = NumberFormatOptions::default().with_fraction_digits(Some(0), Some(0));
    let half_even = int_opts.with_rounding_mode(RoundingMode::HalfEven);
    let half_away = int_opts.with_rounding_mode(RoundingMode::HalfAwayFromZero);
    let floor = int_opts.with_rounding_mode(RoundingMode::Floor);
    let ceil = int_opts.with_rounding_mode(RoundingMode::Ceiling);
    let trunc = int_opts.with_rounding_mode(RoundingMode::Truncate);

    assert_eq!(format_number(&us, 2.5, half_even), "2");
    assert_eq!(format_number(&us, 3.5, half_even), "4");
    assert_eq!(format_number(&us, 2.5, half_away), "3");
    assert_eq!(format_number(&us, 3.5, half_away), "4");
    assert_eq!(format_number(&us, -1.1, floor), "-2");
    assert_eq!(format_number(&us, 1.9, floor), "1");
    assert_eq!(format_number(&us, 1.1, ceil), "2");
    assert_eq!(format_number(&us, -1.9, ceil), "-1");
    assert_eq!(format_number(&us, 1.9, trunc), "1");
    assert_eq!(format_number(&us, -1.9, trunc), "-1");
}

#[test]
fn native_backend_dates_times_long_strings_uncached_locale_and_worker_thread_tls() {
    let us = loc("en-US");
    let de = loc("de-DE");

    // Pre-1970, epoch, leap-day, and pre-1582 proleptic Gregorian dates
    let epoch = CalendarDate::new(1970, 1, 1).unwrap();
    let pre_epoch = CalendarDate::new(1969, 12, 31).unwrap();
    let leap = CalendarDate::new(2024, 2, 29).unwrap();
    let pre_1582 = CalendarDate::new(1500, 3, 1).unwrap();

    let epoch_str = format_date(&us, epoch, DateStyle::Long);
    assert!(
        epoch_str.contains("January") && epoch_str.contains("1") && epoch_str.contains("1970"),
        "unexpected epoch date: {epoch_str}"
    );
    let pre_epoch_str = format_date(&us, pre_epoch, DateStyle::Long);
    assert!(
        pre_epoch_str.contains("December")
            && pre_epoch_str.contains("31")
            && pre_epoch_str.contains("1969"),
        "unexpected pre-1970 date: {pre_epoch_str}"
    );
    let leap_str = format_date(&us, leap, DateStyle::Long);
    assert!(
        leap_str.contains("February") && leap_str.contains("29") && leap_str.contains("2024"),
        "unexpected leap-day date: {leap_str}"
    );
    let pre_1582_str = format_date(&us, pre_1582, DateStyle::Long);
    assert!(
        pre_1582_str.contains("March") && pre_1582_str.contains("1") && pre_1582_str.contains("1500"),
        "expected proleptic Gregorian date for 1500-03-01 (no Julian cutover shift), got: {pre_1582_str}"
    );

    // `TimeStyle::Medium`, `-u-hc-h11`, `-u-hc-h24`, and midnight edge cases
    let afternoon = CalendarTime::new(14, 30, 5).unwrap();
    let midnight = CalendarTime::new(0, 5, 9).unwrap();
    let de_med = format_time(&de, afternoon, TimeStyle::Medium);
    assert!(
        de_med.contains("14:30:05"),
        "expected seconds in de-DE TimeStyle::Medium, got: {de_med}"
    );
    let us_med = format_time(&us, afternoon, TimeStyle::Medium);
    assert!(
        us_med.contains("2:30:05"),
        "expected seconds in en-US TimeStyle::Medium, got: {us_med}"
    );
    let h11_midnight = format_time(&loc("en-US-u-hc-h11"), midnight, TimeStyle::Short);
    assert!(
        h11_midnight.contains("0:05") && !h11_midnight.contains("12:05"),
        "expected 0:05 for -u-hc-h11 midnight, got: {h11_midnight}"
    );
    let h24_midnight = format_time(&loc("en-US-u-hc-h24"), midnight, TimeStyle::Short);
    assert!(
        h24_midnight.contains("24:05"),
        "expected 24:05 for -u-hc-h24 midnight, got: {h24_midnight}"
    );
    let h23_med = format_time(&loc("en-US-u-hc-h23"), midnight, TimeStyle::Medium);
    assert!(
        h23_med.contains("00:05:09"),
        "expected 00:05:09 for -u-hc-h23 Medium midnight, got: {h23_med}"
    );

    // `append_cf_string` >256-byte heap fallback (grouped `1e300` exceeds 400 UTF-8 bytes)
    let huge = format_number(&us, 1e300, NumberFormatOptions::default());
    assert!(
        huge.len() > 350 && huge.starts_with("1,000,000") && huge.ends_with(",000"),
        "expected >256-byte grouped 1e300 string, got len={} prefix={:?}",
        huge.len(),
        &huge[..huge.len().min(32)]
    );

    // `create_uncached_locale` path (>96-byte ICU locale key with valid BCP-47 variants)
    let long_loc = loc(
        "en-US-1901-valencia-biske-boont-heploc-hepburn-nedis-njiva-osojs-rozaj-solba-fonipa-fonupa-alalc97-scotland",
    );
    assert!(long_loc.canonical_tag().len() > 96);
    assert_eq!(format_decimal(&long_loc, 1234.56), "1,234.56");
    assert_eq!(format_integer(&long_loc, 987_654), "987,654");
    assert_eq!(format_currency(&long_loc, 42.5, Currency::Usd), "$42.50");
    assert!(format_date(&long_loc, leap, DateStyle::Long).contains("2024"));
    assert!(format_time(&long_loc, afternoon, TimeStyle::Medium).contains("30:05"));
    assert_eq!(first_day_of_week(&long_loc), Weekday::Sunday);
    assert_eq!(locale_metadata(&long_loc).decimal_separator, ".");

    // Worker-thread TLS creation & clean thread-exit teardown
    let handle = std::thread::spawn(move || {
        let ja = loc("ja-JP-u-ca-japanese");
        let num = format_decimal(&loc("de-DE"), 9876.54);
        let cur = format_currency(&loc("en-US"), 12.34, Currency::Eur);
        let dt = format_date(&ja, CalendarDate::new(2026, 3, 5).unwrap(), DateStyle::Long);
        let tm = format_time(&loc("de-DE"), afternoon, TimeStyle::Medium);
        let dow = first_day_of_week(&loc("sv-SE"));
        let meta = locale_metadata(&loc("en-GB"));
        (num, cur, dt, tm, dow, meta.measurement_system)
    });
    let (num, cur, dt, tm, dow, ms) = handle.join().expect("worker thread must exit cleanly");
    assert!(num.contains("9.876,54"), "worker num: {num}");
    assert!(cur.contains("12.34"), "worker cur: {cur}");
    assert!(dt.contains("令和") && dt.contains('8'), "worker date: {dt}");
    assert!(tm.contains("14:30:05"), "worker time: {tm}");
    assert_eq!(dow, Weekday::Monday);
    assert_eq!(ms, MeasurementSystem::Uk);
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
