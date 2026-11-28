//! Criterion micro-benchmarks for `nve-i18n`:
//! - BCP-47 locale parsing (simple, script+region, `-u-` Unicode extensions) & `best_match`
//! - CLDR v45+ cardinal/ordinal plural selection
//! - ICU MessageFormat parsing & zero-alloc `format_into`
//! - Native OS (`NativeI18nBackend` / Apple `CoreFoundation`) vs pure-Rust (`BuiltinI18nBackend`)
//!   number, integer, currency, date, time, and `first_day_of_week` formatting

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nve_i18n::{
    BuiltinI18nBackend, CalendarDate, CalendarTime, DateStyle, I18nBackend, LocaleId,
    NativeI18nBackend, NumberFormatOptions, PluralKind, PluralOperands, TimeStyle, parse_message,
    plural_category,
};

fn loc(tag: &str) -> LocaleId {
    LocaleId::parse(tag).expect("valid benchmark locale")
}

fn bench_locale_and_plural(c: &mut Criterion) {
    let mut g = c.benchmark_group("i18n_locale_plural");

    g.bench_function("locale_parse_simple_en_us", |b| {
        b.iter(|| black_box(LocaleId::parse(black_box("en-US")).expect("valid locale")));
    });

    g.bench_function("locale_parse_u_extensions", |b| {
        b.iter(|| {
            black_box(
                LocaleId::parse(black_box("ja-JP-u-ca-japanese-fw-mon-hc-h23-nu-latn"))
                    .expect("valid locale"),
            )
        });
    });

    let available = vec![loc("en-US"), loc("zh-Hans"), loc("de-DE"), loc("ja-JP")];
    let query = loc("zh-Hans-CN-u-fw-mon");
    g.bench_function("locale_best_match", |b| {
        b.iter(|| black_box(query.best_match(black_box(&available))));
    });

    let ar = loc("ar-EG");
    let pl = loc("pl-PL");
    let op_ar = PluralOperands::from_int(15);
    let op_pl = PluralOperands::from_int(23);
    g.bench_function("plural_select_ar_and_pl", |b| {
        b.iter(|| {
            let a = plural_category(black_box(&ar), PluralKind::Cardinal, black_box(op_ar));
            let p = plural_category(black_box(&pl), PluralKind::Cardinal, black_box(op_pl));
            black_box((a, p))
        });
    });

    g.finish();
}

fn bench_message_format(c: &mut Criterion) {
    let mut g = c.benchmark_group("i18n_message");
    let tpl = "Hello {name}, you have {count, plural, =0 {no messages} one {# message} other {# messages}}.";
    let en = loc("en-US");
    let pat = parse_message(tpl).expect("valid ICU template");

    g.bench_function("parse_icu_plural_template", |b| {
        b.iter(|| black_box(parse_message(black_box(tpl)).expect("valid template")));
    });

    let mut out = String::with_capacity(128);
    g.bench_function("format_into_builtin_preparsed", |b| {
        b.iter(|| {
            out.clear();
            pat.format_into_with_backend(
                &BuiltinI18nBackend,
                black_box(&en),
                black_box(&[("name", "Ada".into()), ("count", 1234.into())]),
                &mut out,
            )
            .expect("format builtin message");
            black_box(out.len())
        });
    });

    g.bench_function("format_into_native_preparsed", |b| {
        b.iter(|| {
            out.clear();
            pat.format_into(
                black_box(&en),
                black_box(&[("name", "Ada".into()), ("count", 1234.into())]),
                &mut out,
            )
            .expect("format native message");
            black_box(out.len())
        });
    });

    g.finish();
}

fn bench_native_vs_builtin_formatting(c: &mut Criterion) {
    let mut g = c.benchmark_group("i18n_native_vs_builtin");
    let de = loc("de-DE");
    let ja_ca = loc("ja-JP-u-ca-japanese");
    let de_hc = loc("de-DE-u-hc-h12");
    let date = CalendarDate::new(2026, 3, 5).expect("valid date");
    let time = CalendarTime::new(14, 30, 5).expect("valid time");
    let opts = NumberFormatOptions::default();

    let mut out = String::with_capacity(64);
    g.bench_function("number_builtin_de_de", |b| {
        b.iter(|| {
            out.clear();
            BuiltinI18nBackend.format_number_into(
                black_box(&de),
                black_box(123_456.78),
                opts,
                &mut out,
            );
            black_box(out.len())
        });
    });

    g.bench_function("number_native_de_de", |b| {
        b.iter(|| {
            out.clear();
            NativeI18nBackend.format_number_into(
                black_box(&de),
                black_box(123_456.78),
                opts,
                &mut out,
            );
            black_box(out.len())
        });
    });

    g.bench_function("date_builtin_ja_japanese_cal", |b| {
        b.iter(|| {
            out.clear();
            BuiltinI18nBackend.format_date_into(
                black_box(&ja_ca),
                black_box(date),
                DateStyle::Long,
                &mut out,
            );
            black_box(out.len())
        });
    });

    g.bench_function("date_native_ja_japanese_cal", |b| {
        b.iter(|| {
            out.clear();
            NativeI18nBackend.format_date_into(
                black_box(&ja_ca),
                black_box(date),
                DateStyle::Long,
                &mut out,
            );
            black_box(out.len())
        });
    });

    g.bench_function("time_native_de_hc_h12", |b| {
        b.iter(|| {
            out.clear();
            NativeI18nBackend.format_time_into(
                black_box(&de_hc),
                black_box(time),
                TimeStyle::Short,
                &mut out,
            );
            black_box(out.len())
        });
    });

    g.bench_function("first_day_of_week_native_cached", |b| {
        b.iter(|| black_box(NativeI18nBackend.first_day_of_week(black_box(&de))));
    });

    g.finish();
}

criterion_group!(
    benches,
    bench_locale_and_plural,
    bench_message_format,
    bench_native_vs_builtin_formatting
);
criterion_main!(benches);
