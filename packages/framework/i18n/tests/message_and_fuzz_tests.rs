//! Exhaustive ICU MessageFormat tests, custom `I18nBackend` static-dispatch tests,
//! error/recursion-bound tests, and zero-panic adversarial fuzz tests.

use nve_i18n::{
    BuiltinI18nBackend, CalendarDate, DateStyle, I18nBackend, I18nError, LocaleId,
    MAX_NESTING_DEPTH, MessageArgs, MessageValue, NumberFormatOptions, PluralCategory, PluralKind,
    PluralOperands, RelativeUnit, Weekday, format_message, format_message_into,
    format_message_with_backend, parse_message, pseudo_localize,
};

fn loc(tag: &str) -> LocaleId {
    LocaleId::parse(tag).expect("valid locale")
}

#[test]
fn icu_simple_placeholders_and_number_formatting() {
    let en = loc("en-US");
    let de = loc("de-DE");

    assert_eq!(
        format_message(&en, "Hello, {name}!", &[("name", MessageValue::Str("Ada"))]).unwrap(),
        "Hello, Ada!"
    );

    let mut buf = String::from("Prefix: ");
    format_message_into(
        &de,
        "Total { amount , number } ({count, number, integer})",
        &MessageArgs::new().with("amount", 1234.56).with("count", 10000),
        &mut buf,
    )
    .unwrap();
    assert_eq!(buf, "Prefix: Total 1.234,56 (10.000)");
}

#[test]
fn icu_plural_exact_offset_and_locale_number_replacement() {
    let en = loc("en-US");
    let de = loc("de-DE");
    let tpl = "{count, plural, =0 {No items} one {# item} other {# items}}";

    assert_eq!(format_message(&en, tpl, &[("count", 0.into())]).unwrap(), "No items");
    assert_eq!(format_message(&en, tpl, &[("count", 1.into())]).unwrap(), "1 item");
    assert_eq!(format_message(&en, tpl, &[("count", 1234.into())]).unwrap(), "1,234 items");
    assert_eq!(format_message(&de, tpl, &[("count", 1234.into())]).unwrap(), "1.234 items");

    // Decimal `"1.0"` has `v = 1` -> `other` in English, NOT `one`!
    assert_eq!(
        format_message(&en, tpl, &[("count", MessageValue::DecimalStr("1.0"))]).unwrap(),
        "1.0 items"
    );

    // Offset semantics: `=0`/`=1` match un-offset value; `#` and category use `count - offset`.
    let offset_tpl = "{count, plural, offset:1 =0 {Nobody} =1 {Just you} one {You and # other} other {You and # others}}";
    assert_eq!(format_message(&en, offset_tpl, &[("count", 0.into())]).unwrap(), "Nobody");
    assert_eq!(format_message(&en, offset_tpl, &[("count", 1.into())]).unwrap(), "Just you");
    assert_eq!(format_message(&en, offset_tpl, &[("count", 2.into())]).unwrap(), "You and 1 other");
    assert_eq!(format_message(&en, offset_tpl, &[("count", 5.into())]).unwrap(), "You and 4 others");
}

#[test]
fn icu_plural_arabic_six_categories_and_polish_russian() {
    // Use BuiltinI18nBackend for exact deterministic Western digits on `ar-EG`, and
    // verify `ar-EG-u-nu-latn` produces Western digits on the native OS backend as well.
    let ar = loc("ar-EG");
    let ar_latn = loc("ar-EG-u-nu-latn");
    let ar_tpl = "{n, plural, zero {لا عناصر} one {عنصر واحد} two {عنصران} few {# عناصر} many {# عنصرًا} other {# عنصر}}";
    assert_eq!(
        format_message_with_backend(&BuiltinI18nBackend, &ar, ar_tpl, &[("n", 0.into())]).unwrap(),
        "لا عناصر"
    );
    assert_eq!(
        format_message_with_backend(&BuiltinI18nBackend, &ar, ar_tpl, &[("n", 1.into())]).unwrap(),
        "عنصر واحد"
    );
    assert_eq!(
        format_message_with_backend(&BuiltinI18nBackend, &ar, ar_tpl, &[("n", 2.into())]).unwrap(),
        "عنصران"
    );
    assert_eq!(
        format_message_with_backend(&BuiltinI18nBackend, &ar, ar_tpl, &[("n", 5.into())]).unwrap(),
        "5 عناصر"
    );
    assert_eq!(
        format_message_with_backend(&BuiltinI18nBackend, &ar, ar_tpl, &[("n", 15.into())]).unwrap(),
        "15 عنصرًا"
    );
    assert_eq!(
        format_message_with_backend(&BuiltinI18nBackend, &ar, ar_tpl, &[("n", 100.into())])
            .unwrap(),
        "100 عنصر"
    );
    assert_eq!(format_message(&ar_latn, ar_tpl, &[("n", 5.into())]).unwrap(), "5 عناصر");

    let pl = loc("pl-PL");
    let pl_tpl = "{n, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}";
    assert_eq!(format_message(&pl, pl_tpl, &[("n", 1.into())]).unwrap(), "1 plik");
    assert_eq!(format_message(&pl, pl_tpl, &[("n", 3.into())]).unwrap(), "3 pliki");
    assert_eq!(format_message(&pl, pl_tpl, &[("n", 12.into())]).unwrap(), "12 plików");
    assert_eq!(
        format_message(&pl, pl_tpl, &[("n", MessageValue::DecimalStr("1.5"))]).unwrap(),
        "1,5 pliku"
    );
}

#[test]
fn icu_selectordinal_and_select_and_nested_blocks() {
    let en = loc("en-US");
    let ord = "Finished {pos, selectordinal, one {#st} two {#nd} few {#rd} other {#th}}!";
    assert_eq!(format_message(&en, ord, &[("pos", 1.into())]).unwrap(), "Finished 1st!");
    assert_eq!(format_message(&en, ord, &[("pos", 2.into())]).unwrap(), "Finished 2nd!");
    assert_eq!(format_message(&en, ord, &[("pos", 3.into())]).unwrap(), "Finished 3rd!");
    assert_eq!(format_message(&en, ord, &[("pos", 11.into())]).unwrap(), "Finished 11th!");
    assert_eq!(format_message(&en, ord, &[("pos", 22.into())]).unwrap(), "Finished 22nd!");
    assert_eq!(format_message(&en, ord, &[("pos", 103.into())]).unwrap(), "Finished 103rd!");

    let nested = "{gender, select, female {{count, plural, =0 {She has no cats} one {She has # cat} other {She has # cats}}} male {{count, plural, one {He has # cat} other {He has # cats}}} other {{count, plural, one {They have # cat} other {They have # cats}}}}";
    assert_eq!(
        format_message(&en, nested, &MessageArgs::new().with("gender", "female").with("count", 0))
            .unwrap(),
        "She has no cats"
    );
    assert_eq!(
        format_message(&en, nested, &MessageArgs::new().with("gender", "male").with("count", 1))
            .unwrap(),
        "He has 1 cat"
    );
    assert_eq!(
        format_message(&en, nested, &MessageArgs::new().with("gender", "nb").with("count", 4))
            .unwrap(),
        "They have 4 cats"
    );
}

#[test]
fn icu_apostrophe_escaping_and_pound_scoping() {
    let en = loc("en-US");
    // Natural contractions remain literal
    assert_eq!(
        format_message(&en, "It's {name}'s book.", &[("name", "Ada".into())]).unwrap(),
        "It's Ada's book."
    );
    // Double apostrophe `''` -> `'`
    assert_eq!(
        format_message(&en, "Use ''single'' quotes", &()).unwrap(),
        "Use 'single' quotes"
    );
    // Quoted braces `'{'` and quoted `#` inside plural
    assert_eq!(
        format_message(&en, "Template '{name}' stays literal", &()).unwrap(),
        "Template {name} stays literal"
    );
    assert_eq!(
        format_message(
            &en,
            "{n, plural, one {'#'# item} other {'#'# items}}",
            &[("n", 3.into())]
        )
        .unwrap(),
        "#3 items"
    );
    // `#` inside a `select` nested inside a `plural` is literal `#` per ICU spec
    let scoped = "{n, plural, other {{kind, select, other {tag #}}}}";
    assert_eq!(
        format_message(&en, scoped, &MessageArgs::new().with("n", 5).with("kind", "x")).unwrap(),
        "tag #"
    );
}

#[test]
fn icu_strict_error_reporting_and_recursion_limit() {
    let en = loc("en-US");

    assert!(matches!(parse_message("{unclosed"), Err(I18nError::UnmatchedOpenBrace { .. })));
    assert!(matches!(parse_message("stray }"), Err(I18nError::UnmatchedCloseBrace { .. })));
    assert!(matches!(parse_message("'{unterminated"), Err(I18nError::UnterminatedQuote { .. })));
    assert!(matches!(parse_message("{}"), Err(I18nError::EmptyArgumentName { .. })));
    assert!(matches!(
        parse_message("{x, bogus, other {a}}"),
        Err(I18nError::InvalidArgumentType { .. })
    ));
    assert!(matches!(
        parse_message("{n, plural, one {a}}"),
        Err(I18nError::MissingOtherBranch { .. })
    ));
    assert!(matches!(
        parse_message("{n, plural, bogus {a} other {b}}"),
        Err(I18nError::InvalidPluralSelector { .. })
    ));
    assert!(matches!(
        parse_message("{n, plural, one {a} one {b} other {c}}"),
        Err(I18nError::DuplicateSelector { .. })
    ));
    assert!(matches!(
        parse_message("{n, plural, offset:abc other {a}}"),
        Err(I18nError::InvalidOffset { .. })
    ));
    assert!(matches!(
        format_message(&en, "Hi {name}", &()),
        Err(I18nError::MissingArgument { .. })
    ));
    assert!(matches!(
        format_message(&en, "{n, plural, other {#}}", &[("n", "not-a-num".into())]),
        Err(I18nError::InvalidArgumentValue { .. })
    ));

    // Nesting deeper than MAX_NESTING_DEPTH must fail cleanly with MaxDepthExceeded
    let mut deep = String::new();
    for _ in 0..=MAX_NESTING_DEPTH {
        deep.push_str("{a, select, other {");
    }
    deep.push_str("leaf");
    for _ in 0..=MAX_NESTING_DEPTH {
        deep.push_str("}}");
    }
    assert!(matches!(
        parse_message(&deep),
        Err(I18nError::MaxDepthExceeded { .. })
    ));
}

#[test]
fn custom_i18n_backend_static_dispatch_is_used_by_icu_message() {
    struct MockBackend;
    impl I18nBackend for MockBackend {
        fn plural_category(
            &self,
            _locale: &LocaleId,
            _kind: PluralKind,
            _operands: PluralOperands,
        ) -> PluralCategory {
            PluralCategory::Few
        }
        fn format_number(&self, _locale: &LocaleId, value: f64, _opts: NumberFormatOptions) -> String {
            format!("NUM({value})")
        }
        fn format_integer(&self, _locale: &LocaleId, value: i64) -> String {
            format!("INT({value})")
        }
        fn format_currency(&self, _locale: &LocaleId, amount: f64, currency: &str) -> String {
            format!("CUR({currency}:{amount})")
        }
        fn format_date(&self, _locale: &LocaleId, date: CalendarDate, _style: DateStyle) -> String {
            format!("DATE({}-{}-{})", date.year, date.month, date.day)
        }
        fn format_relative_time(&self, _locale: &LocaleId, value: i64, unit: RelativeUnit) -> String {
            format!("REL({value}:{unit:?})")
        }
        fn first_day_of_week(&self, _locale: &LocaleId) -> Weekday {
            Weekday::Wednesday
        }
    }

    let en = loc("en-US");
    let res = format_message_with_backend(
        &MockBackend,
        &en,
        "{n, plural, few {FEW #} other {OTHER #}}",
        &[("n", 99.into())],
    )
    .unwrap();
    assert_eq!(res, "FEW INT(99)");
}

/// Deterministic XorShift64 fuzzing of parser, formatter, locale parser,
/// plural operands, and pseudo-localizer: zero panics on 10,000+ adversarial inputs.
#[test]
fn fuzz_zero_panics_on_adversarial_and_truncated_templates() {
    let en = loc("en-US");
    let alphabet: &[&str] = &[
        "{", "}", ",", "'", "''", "#", "=", ":", " ", "\n", "a", "n", "0", "1", "plural",
        "select", "selectordinal", "number", "offset:", "one", "other", "few", "€", "م", "日",
        "🦀",
    ];

    let mut state: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let mut next_u64 = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    for _ in 0..4_000 {
        let token_count = (next_u64() % 28) as usize;
        let mut s = String::new();
        for _ in 0..token_count {
            let idx = (next_u64() as usize) % alphabet.len();
            s.push_str(alphabet[idx]);
        }

        if let Ok(pat) = parse_message(&s) {
            let _ = pat.format(&en, &MessageArgs::new().with("a", "x").with("n", 3));
        }
        let _ = pseudo_localize(&s);
        let _ = LocaleId::parse(&s);
        let _ = PluralOperands::parse(&s);
    }

    // Truncate every valid complex template at every UTF-8 char boundary
    let seeds = [
        "{count, plural, offset:1 =0 {No items} one {'#'# item} other {# items for {name}}}",
        "{gender, select, female {{pos, selectordinal, one {#st} two {#nd} few {#rd} other {#th}}} other {It's '{ok}'}}",
        "Привет {name}, у вас {n, plural, one {# файл} few {# файла} many {# файлов} other {# файла}}",
    ];
    for seed in seeds {
        for (byte_idx, _) in seed.char_indices() {
            let prefix = &seed[..byte_idx];
            let _ = parse_message(prefix);
            let _ = pseudo_localize(prefix);
        }
    }
}
