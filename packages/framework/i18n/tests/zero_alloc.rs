//! Zero-allocation heap-guard suite for `nve-i18n` hot paths.
//!
//! Installs a per-thread `CountingAlloc` (`#[global_allocator]`) with an explicit
//! positive probe verifying that real `Box`/`Vec` allocations are observed and that
//! sibling threads never pollute the current thread's counter, then enforces strict
//! zero-allocation invariants on:
//! - BCP-47 locale parsing (simple tags and `-u-` Unicode extensions)
//! - Locale ancestry matching (`parent`, `best_match`)
//! - CLDR v45+ cardinal/ordinal plural selection
//! - `MessageCatalog` fallback lookup (`lookup` and `lookup_with_trace` hits)
//! - Pre-parsed ICU `MessagePattern::format_into` into a reusable buffer
//! - Warm cached `NativeI18nBackend` (`CoreFoundation` on Apple) & `BuiltinI18nBackend` formatting

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;
use std::hint::black_box;

use nve_i18n::{
    BuiltinI18nBackend, CalendarDate, CalendarTime, DateStyle, I18nBackend, LocaleId,
    MessageCatalog, NativeI18nBackend, NumberFormatOptions, PluralCategory, PluralKind,
    PluralOperands, TimeStyle, Weekday, parse_message, plural_category, plural_category_for_int,
};

struct CountingAlloc;

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
}

#[inline]
fn thread_alloc_count() -> usize {
    ALLOC_COUNT.with(Cell::get)
}

#[inline]
fn measure_allocs<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = thread_alloc_count();
    let out = f();
    let after = thread_alloc_count();
    (out, after.wrapping_sub(before))
}

// SAFETY: Every method forwards unchanged arguments to `System`, upholding `GlobalAlloc`.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn counting_alloc_positive_probe_and_thread_isolation() {
    let (boxed, box_allocs) = measure_allocs(|| black_box(Box::new([42u8; 64])));
    drop(boxed);
    assert!(
        box_allocs >= 1,
        "positive probe failed: Box allocation must increment CountingAlloc (got {box_allocs})"
    );

    let (vec_val, vec_allocs) = measure_allocs(|| black_box(vec![7u8; 128]));
    drop(vec_val);
    assert!(
        vec_allocs >= 1,
        "positive probe failed: Vec allocation must increment CountingAlloc (got {vec_allocs})"
    );

    // Confirm sibling thread allocations never leak into this thread's counter.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        for _ in 0..64 {
            let v = black_box(vec![1u8; 128]);
            drop(v);
        }
        done_tx.send(()).unwrap();
    });
    ready_rx.recv().unwrap();
    let start = thread_alloc_count();
    while done_rx.try_recv().is_err() {
        std::thread::yield_now();
    }
    let end = thread_alloc_count();
    worker.join().unwrap();
    assert_eq!(
        end.wrapping_sub(start),
        0,
        "background thread allocations must not pollute this thread's counter"
    );
}

#[test]
fn bcp47_locale_parsing_and_matching_are_zero_alloc() {
    let available = vec![
        LocaleId::parse("en-US").unwrap(),
        LocaleId::parse("zh-Hans").unwrap(),
        LocaleId::parse("de-DE").unwrap(),
    ];

    let ((en, zh, ext, matched), allocs) = measure_allocs(|| {
        let en = LocaleId::parse(black_box("en-us")).unwrap();
        let zh = LocaleId::parse(black_box("ZH_hans_cn")).unwrap();
        let ext =
            LocaleId::parse(black_box("ja-JP-u-ca-japanese-fw-mon-hc-h23-nu-latn")).unwrap();
        let matched = zh.best_match(black_box(&available)).is_some();
        let _parent = ext.parent();
        (en, zh, ext, matched)
    });

    assert_eq!(en.region(), Some("US"));
    assert_eq!(zh.script(), Some("Hans"));
    assert_eq!(ext.calendar(), Some("japanese"));
    assert!(matched);
    assert_eq!(
        allocs, 0,
        "BCP-47 parse (with/without -u- extensions), parent(), and best_match() must do 0 heap allocs, did {allocs}"
    );
}

#[test]
fn cldr_plural_selection_and_catalog_lookup_are_zero_alloc() {
    let en = LocaleId::parse("en-US").unwrap();
    let ar = LocaleId::parse("ar-EG").unwrap();
    let pl = LocaleId::parse("pl-PL").unwrap();
    let zh_cn = LocaleId::parse("zh-Hans-CN-u-fw-mon").unwrap();

    let mut catalog = MessageCatalog::new(LocaleId::parse("en").unwrap());
    catalog
        .insert_bundle(
            LocaleId::parse("en").unwrap(),
            [
                ("greeting", "Hello, {name}!"),
                ("items", "{count, plural, one {# item} other {# items}}"),
            ],
        )
        .unwrap();
    catalog
        .insert(LocaleId::parse("zh-Hans").unwrap(), "greeting", "你好，{name}！")
        .unwrap();

    let ((cats, hit_a, hit_b), allocs) = measure_allocs(|| {
        let c1 = plural_category_for_int(black_box(&en), PluralKind::Cardinal, 1);
        let c2 = plural_category_for_int(black_box(&ar), PluralKind::Cardinal, 15);
        let c3 = plural_category(
            black_box(&pl),
            PluralKind::Cardinal,
            PluralOperands::from_int(3),
        );
        let c4 = plural_category_for_int(black_box(&en), PluralKind::Ordinal, 23);
        let hit_a = catalog.lookup(black_box(&zh_cn), "greeting").map(|(_, t)| t);
        let hit_b = catalog
            .lookup_with_trace(black_box(&zh_cn), "items")
            .ok()
            .map(|(_, t)| t);
        ((c1, c2, c3, c4), hit_a, hit_b)
    });

    assert_eq!(
        cats,
        (
            PluralCategory::One,
            PluralCategory::Many,
            PluralCategory::Few,
            PluralCategory::Few
        )
    );
    assert_eq!(hit_a, Some("你好，{name}！"));
    assert_eq!(hit_b, Some("{count, plural, one {# item} other {# items}}"));
    assert_eq!(
        allocs, 0,
        "CLDR plural selection and MessageCatalog::lookup must perform 0 heap allocs, did {allocs}"
    );
}

#[test]
fn message_format_into_and_warm_cached_formatting_are_zero_alloc() {
    let en = LocaleId::parse("en-US").unwrap();
    let de = LocaleId::parse("de-DE").unwrap();
    let ja_ext = LocaleId::parse("ja-JP-u-ca-japanese").unwrap();
    let de_hc = LocaleId::parse("de-DE-u-hc-h12").unwrap();
    let date = CalendarDate::new(2026, 3, 5).unwrap();
    let time = CalendarTime::new(14, 30, 5).unwrap();
    let opts = NumberFormatOptions::default();

    let pat = parse_message("Hello {name}: {count, plural, one {# file} other {# files}}").unwrap();
    let mut out = String::with_capacity(128);

    // Warm up thread-local native caches once before measuring steady-state allocations.
    for _ in 0..2 {
        out.clear();
        pat.format_into_with_backend(
            &BuiltinI18nBackend,
            &en,
            &[("name", "Ada".into()), ("count", 1234.into())],
            &mut out,
        )
        .unwrap();
        out.clear();
        pat.format_into(
            &en,
            &[("name", "Ada".into()), ("count", 1234.into())],
            &mut out,
        )
        .unwrap();
        out.clear();
        NativeI18nBackend.format_integer_into(&de, 987_654, &mut out);
        out.clear();
        NativeI18nBackend.format_number_into(&en, 1234.56, opts, &mut out);
        out.clear();
        NativeI18nBackend.format_currency_into(&en, 49.99, "USD", &mut out);
        out.clear();
        NativeI18nBackend.format_date_into(&ja_ext, date, DateStyle::Long, &mut out);
        out.clear();
        NativeI18nBackend.format_time_into(&de_hc, time, TimeStyle::Short, &mut out);
        let _ = NativeI18nBackend.first_day_of_week(&en);
    }

    // Builtin `format_into` with plural + `#` integer formatting -> 0 heap allocs.
    let ((), builtin_allocs) = measure_allocs(|| {
        out.clear();
        pat.format_into_with_backend(
            &BuiltinI18nBackend,
            black_box(&en),
            black_box(&[("name", "Ada".into()), ("count", 1234.into())]),
            &mut out,
        )
        .unwrap();
    });
    assert_eq!(out, "Hello Ada: 1,234 files");
    assert_eq!(
        builtin_allocs, 0,
        "Builtin MessagePattern::format_into must perform 0 heap allocs, did {builtin_allocs}"
    );

    // Unconditional zero-alloc checks on ALL platforms (`pat.format_into` with `Int`,
    // `format_integer_into`, `first_day_of_week` including `-u-fw-*` short-circuit).
    let en_fw = LocaleId::parse("en-US-u-fw-mon").unwrap();
    let ((first_dow, override_dow), uncond_allocs) = measure_allocs(|| {
        out.clear();
        pat.format_into(
            black_box(&en),
            black_box(&[("name", "Ada".into()), ("count", 1234.into())]),
            &mut out,
        )
        .unwrap();
        out.clear();
        NativeI18nBackend.format_integer_into(black_box(&de), 987_654, &mut out);
        let d1 = NativeI18nBackend.first_day_of_week(black_box(&en));
        let d2 = NativeI18nBackend.first_day_of_week(black_box(&en_fw));
        (d1, d2)
    });
    assert_eq!(first_dow, Weekday::Sunday);
    assert_eq!(override_dow, Weekday::Monday);
    assert_eq!(
        uncond_allocs, 0,
        "NativeI18nBackend message Int format_into, format_integer_into, and first_day_of_week must perform 0 Rust heap allocs on all platforms, did {uncond_allocs}"
    );

    // Apple-only zero-Rust-heap-alloc checks for warm cached `CFNumberFormatter` /
    // `CFDateFormatter` formatting paths (non-Apple builtin fallback currently formats
    // via intermediate Rust `String`s).
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        let ((), apple_allocs) = measure_allocs(|| {
            out.clear();
            NativeI18nBackend.format_number_into(black_box(&en), 1234.56, opts, &mut out);
            out.clear();
            NativeI18nBackend.format_currency_into(black_box(&en), 49.99, "USD", &mut out);
            out.clear();
            NativeI18nBackend.format_date_into(black_box(&ja_ext), date, DateStyle::Long, &mut out);
            out.clear();
            NativeI18nBackend.format_time_into(black_box(&de_hc), time, TimeStyle::Short, &mut out);
        });
        assert_eq!(
            apple_allocs, 0,
            "Warm cached NativeI18nBackend CoreFoundation formatting must perform 0 Rust heap allocs on Apple, did {apple_allocs}"
        );
    }
}
