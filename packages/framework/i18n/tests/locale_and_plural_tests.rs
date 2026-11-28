//! Exhaustive BCP-47 locale parsing, RTL/icon-mirroring, fallback chains,
//! `PluralOperands`, and CLDR v45+ cardinal/ordinal vector tests.

use nve_i18n::{
    IconMirrorPolicy, LocaleId, PluralCategory, PluralKind, PluralOperands, TextDirection,
    UnicodeHourCycle, Weekday, plural_category, plural_category_for_int, should_mirror_icon,
};

fn loc(tag: &str) -> LocaleId {
    LocaleId::parse(tag).expect("valid test locale")
}

#[test]
fn bcp47_parsing_and_normalization() {
    let en = loc("en-us");
    assert_eq!(en.language(), "en");
    assert_eq!(en.script(), None);
    assert_eq!(en.region(), Some("US"));
    assert_eq!(en.canonical_tag(), "en-US");

    let zh = loc("ZH_hans_cn");
    assert_eq!(zh.language(), "zh");
    assert_eq!(zh.script(), Some("Hans"));
    assert_eq!(zh.region(), Some("CN"));
    assert_eq!(zh.canonical_tag(), "zh-Hans-CN");

    let es = loc("es-419");
    assert_eq!(es.language(), "es");
    assert_eq!(es.region(), Some("419"));
    assert_eq!(es.canonical_tag(), "es-419");

    let variant = loc("DE-ch-1901");
    assert_eq!(variant.language(), "de");
    assert_eq!(variant.region(), Some("CH"));
    assert_eq!(variant.variants(), &["1901"]);
    assert_eq!(variant.canonical_tag(), "de-CH-1901");

    for tag in [
        "ar-EG", "de-DE", "pl-PL", "ru-RU", "ja-JP", "pt-BR", "sr-Cyrl-RS", "zh-Hant-TW",
    ] {
        assert_eq!(loc(tag).canonical_tag(), tag);
    }
}

#[test]
fn bcp47_rejects_malformed_locales() {
    for bad in [
        "",
        "e",
        "engl",
        "123",
        "en-",
        "-US",
        "en--US",
        "en-12",
        "en-USA",
        "en-US-Hans",
        "en-Latn-Cyrl",
        "en-US-GB",
        "de-CH-1901-1901",
        "en US",
        "en-U$",
        "en-US-u",
        "en-US-u-",
        "en-US-u-fw",
        "en-US-u-fw-monday",
        "en-US-u-fw-mon-tue",
        "en-US-u-fw-mon-fw-sun",
        "en-US-u-hc-h25",
        "en-US-u-hc-12",
        "en-US-u-hc-h12-h23",
        "en-US-u-ca",
        "en-US-u-ca-gregorian",
        "en-US-u-ca-1901",
        "en-US-u-ca-gregory-1901",
        "en-US-u-ca-gregory-ca-japanese",
        "en-US-u-nu",
        "en-US-u-nu-ab",
        "en-US-u-nu-latn-arab",
        "en-US-u-nu-123",
        "en-US-u-zz-foo",
        "en-US-u-fw-mon-u-hc-h12",
        "en-US-x-private",
    ] {
        assert!(
            LocaleId::parse(bad).is_err(),
            "expected invalid locale error for '{bad}'"
        );
    }
}

#[test]
fn bcp47_unicode_u_extensions_parse_canonicalize_roundtrip_and_parent_strip() {
    let full = loc("EN_us_U_NU_LATN_HC_H23_FW_MON_CA_JAPANESE");
    assert_eq!(full.language(), "en");
    assert_eq!(full.region(), Some("US"));
    assert!(full.has_extensions());
    assert_eq!(full.calendar(), Some("japanese"));
    assert_eq!(full.first_day_of_week_override(), Some(Weekday::Monday));
    assert_eq!(full.hour_cycle(), Some(UnicodeHourCycle::H23));
    assert_eq!(full.numbering_system(), Some("latn"));
    assert_eq!(
        full.canonical_tag(),
        "en-US-u-ca-japanese-fw-mon-hc-h23-nu-latn"
    );
    assert_eq!(full.to_string(), "en-US-u-ca-japanese-fw-mon-hc-h23-nu-latn");
    assert_eq!(loc(&full.to_string()), full);

    // Compound multi-subtag calendar (`islamic-umalqura`) followed by `-nu-arab`
    let islamic = loc("ar-SA-u-ca-ISLAMIC-UMALQURA-nu-ARAB");
    assert_eq!(islamic.calendar(), Some("islamic-umalqura"));
    assert_eq!(islamic.numbering_system(), Some("arab"));
    assert_eq!(islamic.canonical_tag(), "ar-SA-u-ca-islamic-umalqura-nu-arab");
    assert_eq!(loc(&islamic.canonical_tag()), islamic);

    // All 7 weekdays & 4 hour cycles
    for (tag, expected_fw) in [
        ("en-u-fw-sun", Weekday::Sunday),
        ("en-u-fw-mon", Weekday::Monday),
        ("en-u-fw-tue", Weekday::Tuesday),
        ("en-u-fw-wed", Weekday::Wednesday),
        ("en-u-fw-thu", Weekday::Thursday),
        ("en-u-fw-fri", Weekday::Friday),
        ("en-u-fw-sat", Weekday::Saturday),
    ] {
        let parsed = loc(tag);
        assert_eq!(parsed.first_day_of_week_override(), Some(expected_fw));
        assert_eq!(parsed.canonical_tag(), tag);
    }
    for (tag, expected_hc) in [
        ("de-DE-u-hc-h11", UnicodeHourCycle::H11),
        ("de-DE-u-hc-h12", UnicodeHourCycle::H12),
        ("de-DE-u-hc-h23", UnicodeHourCycle::H23),
        ("de-DE-u-hc-h24", UnicodeHourCycle::H24),
    ] {
        let parsed = loc(tag);
        assert_eq!(parsed.hour_cycle(), Some(expected_hc));
        assert_eq!(parsed.canonical_tag(), tag);
    }

    // Parent & fallback chain strip `-u-` extensions first so catalog bundles resolve seamlessly
    let ext_zh = loc("zh-Hans-CN-u-fw-mon-hc-h23");
    assert_eq!(ext_zh.parent(), Some(loc("zh-Hans-CN")));
    let available = vec![loc("zh-Hans"), loc("en-US")];
    assert_eq!(ext_zh.best_match(&available), Some(&loc("zh-Hans")));
}

#[test]
fn rtl_detection_and_selective_icon_mirroring() {
    // RTL primary languages
    for tag in ["ar", "ar-EG", "he", "he-IL", "fa-IR", "ur-PK", "ps", "yi", "dv"] {
        let l = loc(tag);
        assert!(l.is_rtl(), "{tag} must be RTL");
        assert_eq!(l.text_direction(), TextDirection::Rtl);
        assert!(l.should_mirror_icon(IconMirrorPolicy::Directional));
        assert!(!l.should_mirror_icon(IconMirrorPolicy::Fixed));
    }

    // RTL explicit scripts override otherwise LTR/ambiguous languages
    for tag in ["ks-Arab", "az-Arab-IR", "pa-Arab", "en-Hebr", "syr-Syrc", "dv-Thaa"] {
        let l = loc(tag);
        assert!(l.is_rtl(), "{tag} with RTL script must be RTL");
    }

    // Explicit LTR script overrides RTL primary language (e.g. romanized Arabic)
    let ar_latn = loc("ar-Latn");
    assert!(!ar_latn.is_rtl());
    assert_eq!(ar_latn.text_direction(), TextDirection::Ltr);
    assert!(!ar_latn.should_mirror_icon(IconMirrorPolicy::Directional));

    // Standard LTR locales never mirror either icon policy
    for tag in ["en-US", "de-DE", "zh-Hans-CN", "ja-JP", "pl-PL", "ru-RU"] {
        let l = loc(tag);
        assert!(!l.is_rtl());
        assert!(!l.should_mirror_icon(IconMirrorPolicy::Directional));
        assert!(!l.should_mirror_icon(IconMirrorPolicy::Fixed));
    }

    assert!(should_mirror_icon(TextDirection::Rtl, IconMirrorPolicy::Directional));
    assert!(!should_mirror_icon(TextDirection::Rtl, IconMirrorPolicy::Fixed));
    assert!(!should_mirror_icon(TextDirection::Ltr, IconMirrorPolicy::Directional));
    assert!(!should_mirror_icon(TextDirection::Ltr, IconMirrorPolicy::Fixed));
}

#[test]
fn fallback_chains_and_best_match() {
    let zh_hans_cn = loc("zh-Hans-CN");
    let en = loc("en");
    let chain: Vec<String> = zh_hans_cn
        .fallback_chain(Some(&en))
        .into_iter()
        .map(|l| l.canonical_tag())
        .collect();
    assert_eq!(chain, vec!["zh-Hans-CN", "zh-Hans", "zh", "en"]);

    // Deduplication when default shares ancestry
    let en_gb = loc("en-GB");
    let en_us = loc("en-US");
    let dedup: Vec<String> = en_gb
        .fallback_chain(Some(&en_us))
        .into_iter()
        .map(|l| l.canonical_tag())
        .collect();
    assert_eq!(dedup, vec!["en-GB", "en", "en-US"]);

    let available = vec![loc("en-US"), loc("zh-Hans"), loc("de-DE")];
    assert_eq!(zh_hans_cn.best_match(&available), Some(&loc("zh-Hans")));
    assert_eq!(loc("de-AT").best_match(&available), Some(&loc("de-DE")));
    assert_eq!(loc("fr-FR").best_match(&available), None);
}

#[test]
fn plural_operands_exact_integer_and_decimal_semantics() {
    let int_one = PluralOperands::parse("1").unwrap();
    assert_eq!((int_one.i, int_one.v, int_one.w, int_one.f, int_one.t), (1, 0, 0, 0, 0));

    let dec_one_zero = PluralOperands::parse("1.0").unwrap();
    assert_eq!(
        (dec_one_zero.i, dec_one_zero.v, dec_one_zero.w, dec_one_zero.f, dec_one_zero.t),
        (1, 1, 0, 0, 0)
    );

    let dec_1_20 = PluralOperands::parse("1.20").unwrap();
    assert_eq!(
        (dec_1_20.i, dec_1_20.v, dec_1_20.w, dec_1_20.f, dec_1_20.t),
        (1, 2, 1, 20, 2)
    );

    let dec_1_05 = PluralOperands::parse("-1.05").unwrap();
    assert_eq!(
        (dec_1_05.i, dec_1_05.v, dec_1_05.w, dec_1_05.f, dec_1_05.t),
        (1, 2, 2, 5, 5)
    );

    // i64::MIN does not overflow
    let min_op = PluralOperands::from_int(i64::MIN);
    assert_eq!(min_op.i, i64::MIN.unsigned_abs());
    assert_eq!(min_op.v, 0);

    // Compact exponent `1c6`
    let compact = PluralOperands::parse("1c6").unwrap();
    assert_eq!(compact.i, 1);
    assert_eq!(compact.e, 6);

    // Invalid inputs
    assert!(PluralOperands::parse("").is_err());
    assert!(PluralOperands::parse(".").is_err());
    assert!(PluralOperands::parse("1.").is_err());
    assert!(PluralOperands::parse("1.2.3").is_err());
    assert!(PluralOperands::from_f64(f64::NAN).is_err());
    assert!(PluralOperands::from_f64(f64::INFINITY).is_err());
}

#[test]
fn cldr_cardinal_english_german_dutch_swedish_spanish_italian_portuguese() {
    for tag in ["en-US", "de-DE", "nl-NL", "sv-SE"] {
        let l = loc(tag);
        assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, 0), PluralCategory::Other);
        assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, 1), PluralCategory::One);
        assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, 2), PluralCategory::Other);
        // `1.0` has `v = 1` -> Other!
        assert_eq!(
            plural_category(&l, PluralKind::Cardinal, PluralOperands::parse("1.0").unwrap()),
            PluralCategory::Other,
            "1.0 must be Other in {tag}"
        );
    }

    // Spanish: `1` and `1.0` are `One`, `1_000_000` is `Many`
    let es = loc("es-ES");
    assert_eq!(plural_category_for_int(&es, PluralKind::Cardinal, 1), PluralCategory::One);
    assert_eq!(
        plural_category(&es, PluralKind::Cardinal, PluralOperands::parse("1.0").unwrap()),
        PluralCategory::One
    );
    assert_eq!(plural_category_for_int(&es, PluralKind::Cardinal, 1_000_000), PluralCategory::Many);
    assert_eq!(plural_category_for_int(&es, PluralKind::Cardinal, 2), PluralCategory::Other);

    // Italian: `1` is `One`, `1.0` is `Other`, `1_000_000` is `Many`
    let it = loc("it-IT");
    assert_eq!(plural_category_for_int(&it, PluralKind::Cardinal, 1), PluralCategory::One);
    assert_eq!(
        plural_category(&it, PluralKind::Cardinal, PluralOperands::parse("1.0").unwrap()),
        PluralCategory::Other
    );
    assert_eq!(plural_category_for_int(&it, PluralKind::Cardinal, 1_000_000), PluralCategory::Many);

    // Portuguese: `pt-BR` has 0 and 1 as `One`; `pt-PT` has only 1 (`v=0`) as `One`
    let pt_br = loc("pt-BR");
    let pt_pt = loc("pt-PT");
    assert_eq!(plural_category_for_int(&pt_br, PluralKind::Cardinal, 0), PluralCategory::One);
    assert_eq!(plural_category_for_int(&pt_br, PluralKind::Cardinal, 1), PluralCategory::One);
    assert_eq!(plural_category_for_int(&pt_br, PluralKind::Cardinal, 1_000_000), PluralCategory::Many);
    assert_eq!(plural_category_for_int(&pt_pt, PluralKind::Cardinal, 0), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&pt_pt, PluralKind::Cardinal, 1), PluralCategory::One);
}

#[test]
fn cldr_cardinal_french_cldr45() {
    let fr = loc("fr-FR");
    assert_eq!(plural_category_for_int(&fr, PluralKind::Cardinal, 0), PluralCategory::One);
    assert_eq!(plural_category_for_int(&fr, PluralKind::Cardinal, 1), PluralCategory::One);
    assert_eq!(
        plural_category(&fr, PluralKind::Cardinal, PluralOperands::parse("1.5").unwrap()),
        PluralCategory::One
    );
    assert_eq!(plural_category_for_int(&fr, PluralKind::Cardinal, 2), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&fr, PluralKind::Cardinal, 1_000_000), PluralCategory::Many);
    assert_eq!(plural_category_for_int(&fr, PluralKind::Cardinal, 2_000_000), PluralCategory::Many);
    assert_eq!(
        plural_category(&fr, PluralKind::Cardinal, PluralOperands::parse("1000000.0").unwrap()),
        PluralCategory::Other
    );
    assert_eq!(
        plural_category(&fr, PluralKind::Cardinal, PluralOperands::parse("1c6").unwrap()),
        PluralCategory::Many
    );
}

#[test]
fn cldr_cardinal_arabic_all_six_categories() {
    let ar = loc("ar-EG");
    assert_eq!(plural_category_for_int(&ar, PluralKind::Cardinal, 0), PluralCategory::Zero);
    assert_eq!(plural_category_for_int(&ar, PluralKind::Cardinal, 1), PluralCategory::One);
    assert_eq!(plural_category_for_int(&ar, PluralKind::Cardinal, 2), PluralCategory::Two);

    for n in [3, 4, 10, 103, 110, 1003] {
        assert_eq!(plural_category_for_int(&ar, PluralKind::Cardinal, n), PluralCategory::Few, "ar {n}");
    }
    for n in [11, 25, 99, 111, 199, 1011] {
        assert_eq!(plural_category_for_int(&ar, PluralKind::Cardinal, n), PluralCategory::Many, "ar {n}");
    }
    for n in [100, 101, 102, 200, 1000] {
        assert_eq!(plural_category_for_int(&ar, PluralKind::Cardinal, n), PluralCategory::Other, "ar {n}");
    }
    assert_eq!(
        plural_category(&ar, PluralKind::Cardinal, PluralOperands::parse("0.5").unwrap()),
        PluralCategory::Other
    );
}

#[test]
fn cldr_cardinal_polish_russian_ukrainian_czech_slovak_welsh_and_other_only() {
    // Polish
    let pl = loc("pl-PL");
    assert_eq!(plural_category_for_int(&pl, PluralKind::Cardinal, 1), PluralCategory::One);
    for n in [2, 3, 4, 22, 23, 24, 102] {
        assert_eq!(plural_category_for_int(&pl, PluralKind::Cardinal, n), PluralCategory::Few, "pl {n}");
    }
    for n in [0, 5, 10, 11, 12, 13, 14, 21, 25, 100, 112] {
        assert_eq!(plural_category_for_int(&pl, PluralKind::Cardinal, n), PluralCategory::Many, "pl {n}");
    }
    for s in ["0.5", "1.0", "1.5", "2.0"] {
        assert_eq!(
            plural_category(&pl, PluralKind::Cardinal, PluralOperands::parse(s).unwrap()),
            PluralCategory::Other,
            "pl fractional {s}"
        );
    }

    // Russian & Ukrainian
    for tag in ["ru-RU", "uk-UA"] {
        let l = loc(tag);
        for n in [1, 21, 31, 101] {
            assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, n), PluralCategory::One, "{tag} {n}");
        }
        for n in [2, 3, 4, 22, 23, 24, 102] {
            assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, n), PluralCategory::Few, "{tag} {n}");
        }
        for n in [0, 5, 11, 12, 13, 14, 20, 25, 111, 114] {
            assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, n), PluralCategory::Many, "{tag} {n}");
        }
        assert_eq!(
            plural_category(&l, PluralKind::Cardinal, PluralOperands::parse("1.5").unwrap()),
            PluralCategory::Other
        );
    }

    // Czech & Slovak
    for tag in ["cs-CZ", "sk-SK"] {
        let l = loc(tag);
        assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, 1), PluralCategory::One);
        assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, 2), PluralCategory::Few);
        assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, 4), PluralCategory::Few);
        assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, 0), PluralCategory::Other);
        assert_eq!(plural_category_for_int(&l, PluralKind::Cardinal, 5), PluralCategory::Other);
        assert_eq!(
            plural_category(&l, PluralKind::Cardinal, PluralOperands::parse("1.5").unwrap()),
            PluralCategory::Many
        );
    }

    // Welsh: all 6 cardinal categories
    let cy = loc("cy-GB");
    assert_eq!(plural_category_for_int(&cy, PluralKind::Cardinal, 0), PluralCategory::Zero);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Cardinal, 1), PluralCategory::One);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Cardinal, 2), PluralCategory::Two);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Cardinal, 3), PluralCategory::Few);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Cardinal, 6), PluralCategory::Many);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Cardinal, 4), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Cardinal, 5), PluralCategory::Other);

    // Other-only family
    for tag in ["ja-JP", "zh-Hans-CN", "ko-KR", "tr-TR", "vi-VN"] {
        let l = loc(tag);
        for n in [0, 1, 2, 3, 5, 11, 100] {
            assert_eq!(
                plural_category_for_int(&l, PluralKind::Cardinal, n),
                PluralCategory::Other,
                "{tag} {n}"
            );
        }
    }
}

#[test]
fn cldr_ordinal_rules() {
    let en = loc("en-US");
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 1), PluralCategory::One);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 2), PluralCategory::Two);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 3), PluralCategory::Few);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 4), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 11), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 12), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 13), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 21), PluralCategory::One);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 22), PluralCategory::Two);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 23), PluralCategory::Few);
    assert_eq!(plural_category_for_int(&en, PluralKind::Ordinal, 111), PluralCategory::Other);

    let sv = loc("sv-SE");
    assert_eq!(plural_category_for_int(&sv, PluralKind::Ordinal, 1), PluralCategory::One);
    assert_eq!(plural_category_for_int(&sv, PluralKind::Ordinal, 2), PluralCategory::One);
    assert_eq!(plural_category_for_int(&sv, PluralKind::Ordinal, 3), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&sv, PluralKind::Ordinal, 11), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&sv, PluralKind::Ordinal, 12), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&sv, PluralKind::Ordinal, 21), PluralCategory::One);

    let fr = loc("fr-FR");
    assert_eq!(plural_category_for_int(&fr, PluralKind::Ordinal, 1), PluralCategory::One);
    assert_eq!(plural_category_for_int(&fr, PluralKind::Ordinal, 2), PluralCategory::Other);

    let it = loc("it-IT");
    for n in [8, 11, 80, 800] {
        assert_eq!(plural_category_for_int(&it, PluralKind::Ordinal, n), PluralCategory::Many);
    }
    assert_eq!(plural_category_for_int(&it, PluralKind::Ordinal, 1), PluralCategory::Other);

    let uk = loc("uk-UA");
    assert_eq!(plural_category_for_int(&uk, PluralKind::Ordinal, 3), PluralCategory::Few);
    assert_eq!(plural_category_for_int(&uk, PluralKind::Ordinal, 13), PluralCategory::Other);
    assert_eq!(plural_category_for_int(&uk, PluralKind::Ordinal, 23), PluralCategory::Few);

    // Welsh: all 6 ordinal categories
    let cy = loc("cy-GB");
    assert_eq!(plural_category_for_int(&cy, PluralKind::Ordinal, 0), PluralCategory::Zero);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Ordinal, 1), PluralCategory::One);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Ordinal, 2), PluralCategory::Two);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Ordinal, 3), PluralCategory::Few);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Ordinal, 5), PluralCategory::Many);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Ordinal, 8), PluralCategory::Zero);
    assert_eq!(plural_category_for_int(&cy, PluralKind::Ordinal, 10), PluralCategory::Other);
}
