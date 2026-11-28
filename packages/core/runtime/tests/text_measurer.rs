//! Native text-measurer contracts through the C vtable and the Rust `SurfaceRuntime` API.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_abi::{
    NveRuntimeTextStyle, NveStrSlice, NVE_LINE_HEIGHT_PERCENT,
    NVE_LINE_HEIGHT_POINTS,
};
use nve_components::tables::{
    FontStyle, Layout, OverflowWrap, TextContent, Typography, WhiteSpace, WordBreak,
};
use nve_engine::Viewport;
use nve_layout::{TextMeasurer, TextStyle};
use nve_runtime::{SurfaceRuntime};
use nve_store::{Length, NodeId, NodeStore, ChangeSet, Rgba, Transaction, wire::WireEncoder};

mod common;
use common::*;

static MEASURE_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn fixed_measure(
    ctx: *mut c_void,
    text: NveStrSlice,
    style: *const NveRuntimeTextStyle,
    max_width: f32,
    out_width: *mut f32,
    out_height: *mut f32,
) {
    // SAFETY: `ctx` is the `f32` height the test registered; `style` and out slots are
    // runtime-owned for the call.
    unsafe {
        assert_eq!(text.as_str(), Ok("hello native text"));
        assert!((100..=900).contains(&(*style).font_weight));
        assert!(max_width.is_infinite() || max_width >= 0.0);
        MEASURE_CALLS.fetch_add(1, Ordering::Relaxed);
        *out_width = 40.0;
        *out_height = *ctx.cast::<f32>();
    }
}

unsafe extern "C" fn nan_measure(
    _ctx: *mut c_void,
    _text: NveStrSlice,
    _style: *const NveRuntimeTextStyle,
    _max_width: f32,
    out_width: *mut f32,
    out_height: *mut f32,
) {
    // SAFETY: Runtime-owned out slots.
    unsafe {
        *out_width = f32::NAN;
        *out_height = -5.0;
    }
}

#[test]
fn c_abi_text_measurer_drives_layout_sanitizes_and_restores_builtin() {
    let s = Surface::new();
    let (frame, text) = text_frame();
    s.submit(&frame);
    let builtin = s.rect_height(text);
    assert!(builtin > 0.0, "heuristic measurer sizes the text leaf");

    let mut native_height = 37.0f32;
    let ctx = (&raw mut native_height).cast::<c_void>();
    // SAFETY: Live handle; `ctx` outlives the measurer (cleared below).
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, Some(fixed_measure), ctx) }, OK);
    s.tick();
    assert!(MEASURE_CALLS.load(Ordering::Relaxed) > 0, "native measurer invoked during layout");
    assert_eq!(s.rect_height(text), 37.0);

    // Hostile sizes collapse to zero instead of poisoning layout with NaN / negatives.
    // SAFETY: Live handle.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, Some(nan_measure), std::ptr::null_mut()) }, OK);
    s.tick();
    assert_eq!(s.rect_height(text), 0.0);

    // SAFETY: Live handle; NULL restores the built-in measurer.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, None, std::ptr::null_mut()) }, OK);
    s.tick();
    assert_eq!(s.rect_height(text), builtin);

    // Writers are refused while a host holds the scene.
    let _ = s.acquire();
    // SAFETY: Live handle.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, None, std::ptr::null_mut()) }, BUSY);
    s.release();
    // SAFETY: Null handle is rejected.
    assert_eq!(unsafe { (s.api.set_text_measurer)(std::ptr::null_mut(), None, std::ptr::null_mut()) }, INVALID);
}

/// One host measurement: text, family, size, weight, style, letter spacing, line height
/// (value, unit), white-space, max lines, max width.
#[derive(Debug, Clone, PartialEq)]
struct Measured {
    text: String,
    family: String,
    size: f32,
    weight: u16,
    font_style: u8,
    letter_spacing: f32,
    line_height: (f32, u8),
    white_space: u8,
    word_break: u8,
    overflow_wrap: u8,
    max_lines: u32,
    max_width: f32,
}

unsafe extern "C" fn recording_measure(
    ctx: *mut c_void,
    text: NveStrSlice,
    style: *const NveRuntimeTextStyle,
    max_width: f32,
    out_width: *mut f32,
    out_height: *mut f32,
) {
    // SAFETY: `ctx` is the test's `Vec<Measured>`; `text`, `style` (and its family) and the out
    // slots are runtime-owned for the call.
    unsafe {
        let s = &*style;
        (*ctx.cast::<Vec<Measured>>()).push(Measured {
            text: text.as_str().expect("UTF-8 text").to_owned(),
            family: s.font_family.as_str().expect("UTF-8 family").to_owned(),
            size: s.font_size,
            weight: s.font_weight,
            font_style: s.font_style,
            letter_spacing: s.letter_spacing,
            line_height: (s.line_height, s.line_height_unit),
            white_space: s.white_space,
            word_break: s.word_break,
            overflow_wrap: s.overflow_wrap,
            max_lines: s.max_lines,
            max_width,
        });
        *out_width = 10.0;
        *out_height = 10.0;
    }
}

/// The host measures each text leaf with its fully resolved, inherited typography: a parent
/// column sets family, size, style, letter spacing, a unit-less line height, `nowrap`, and
/// `max_lines`; one leaf inherits all of it, its sibling overrides some fields.
#[test]
fn c_abi_text_measurer_receives_the_resolved_inherited_typography() {
    let s = Surface::new();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let column = tx.create();
    let inherits = tx.create();
    let overrides = tx.create();
    tx.edit::<Typography>(column)
        .font_family("Consolas, monospace")
        .font_size(13.0)
        .font_style(FontStyle::Italic)
        .letter_spacing(1.5)
        .line_height(Length::percent(150.0))
        .white_space(WhiteSpace::NoWrap)
        .max_lines(2)
        .set(
            Typography::SET_FONT_FAMILY
                | Typography::SET_FONT_SIZE
                | Typography::SET_FONT_STYLE
                | Typography::SET_LETTER_SPACING
                | Typography::SET_LINE_HEIGHT
                | Typography::SET_WHITE_SPACE
                | Typography::SET_MAX_LINES,
        );
    tx.edit::<TextContent>(inherits).text("inherits");
    tx.edit::<TextContent>(overrides).text("overrides");
    tx.edit::<Typography>(overrides)
        .font_weight(700)
        .line_height(Length::points(22.0))
        .white_space(WhiteSpace::Normal)
        .set(Typography::SET_FONT_WEIGHT | Typography::SET_LINE_HEIGHT | Typography::SET_WHITE_SPACE);
    tx.edit::<Layout>(overrides).width(Length::points(120.0));
    tx.set_children(column, &[inherits, overrides]);
    tx.set_children(NodeId::ROOT, &[column]);
    tx.commit_into(&mut changes);

    let mut seen: Vec<Measured> = Vec::new();
    let ctx = (&raw mut seen).cast::<c_void>();
    // SAFETY: Live handle; `seen` outlives the measurer (cleared below).
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, Some(recording_measure), ctx) }, OK);
    s.submit(&encode(&store, &changes, &mut WireEncoder::default()));
    // SAFETY: Live handle.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, None, std::ptr::null_mut()) }, OK);

    let of = |text: &str| {
        let calls: Vec<&Measured> = seen.iter().filter(|m| m.text == text).collect();
        assert!(!calls.is_empty(), "`{text}` was measured");
        calls
    };
    let expected_inherits = Measured {
        text: "inherits".into(),
        family: "Consolas, monospace".into(),
        size: 13.0,
        weight: 400,
        font_style: FontStyle::Italic as u8,
        letter_spacing: 1.5,
        line_height: (150.0, NVE_LINE_HEIGHT_PERCENT),
        white_space: WhiteSpace::NoWrap as u8,
        word_break: 0,
        overflow_wrap: 0,
        max_lines: 2,
        max_width: f32::INFINITY, // `nowrap` never wraps, whatever the probe
    };
    for m in of("inherits") {
        assert_eq!(*m, expected_inherits);
    }
    for m in of("overrides") {
        assert_eq!(
            Measured { max_width: 0.0, ..m.clone() },
            Measured {
                text: "overrides".into(),
                weight: 700,
                line_height: (22.0, NVE_LINE_HEIGHT_POINTS),
                white_space: WhiteSpace::Normal as u8,
                max_width: 0.0,
                ..expected_inherits.clone()
            }
        );
    }
    assert!(of("overrides").iter().any(|m| m.max_width == 120.0), "wrapping text gets its width");
}

/// Every measure-affecting field edited on an ancestor alone re-measures the inheriting leaf
/// with the new value (also a same-length family the string arena replaces in place); a
/// paint-only `color` edit on the ancestor measures nothing.
#[test]
fn ancestor_typography_edits_remeasure_descendants_and_color_does_not() {
    type Edit = fn(&mut Transaction<'_>, NodeId);
    let s = Surface::new();
    let mut store = NodeStore::new();
    let mut enc = WireEncoder::default();
    let column = {
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        let [column, wrapper, leaf] = [(); 3].map(|_| tx.create());
        // Every field explicit (at its default) so later edits need no `set` bookkeeping.
        tx.edit::<Typography>(column).font_family("Arial").set(
            Typography::SET_FONT_FAMILY
                | Typography::SET_LINE_HEIGHT
                | Typography::SET_LETTER_SPACING
                | Typography::SET_WHITE_SPACE
                | Typography::SET_MAX_LINES
                | Typography::SET_FONT_STYLE
                | Typography::SET_WORD_BREAK
                | Typography::SET_OVERFLOW_WRAP
                | Typography::SET_COLOR,
        );
        tx.edit::<TextContent>(leaf).text("leaf");
        tx.set_children(NodeId::ROOT, &[column]);
        tx.set_children(column, &[wrapper]);
        tx.set_children(wrapper, &[leaf]);
        tx.commit_into(&mut changes);
        s.submit(&encode(&store, &changes, &mut enc));
        column
    };
    let seen: *mut Vec<Measured> = Box::into_raw(Box::default());
    // SAFETY: Live handle; `seen` outlives the measurer (cleared below, then freed).
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, Some(recording_measure), seen.cast()) }, OK);
    let mut frame = |store: &mut NodeStore, edit: &dyn Fn(&mut Transaction<'_>)| -> Vec<Measured> {
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        edit(&mut tx);
        tx.commit_into(&mut changes);
        // SAFETY: The runtime only touches `seen` inside `submit` / `tick` (same thread).
        unsafe { (*seen).clear() };
        s.submit(&encode(store, &changes, &mut enc));
        // SAFETY: As above.
        unsafe { std::mem::take(&mut *seen) }
    };
    let edits: [(&str, Edit, fn(&Measured) -> bool); 8] = [
        ("line_height", |tx, c| {
            tx.edit::<Typography>(c).line_height(Length::points(30.0));
        }, |m| m.line_height == (30.0, NVE_LINE_HEIGHT_POINTS)),
        ("letter_spacing", |tx, c| {
            tx.edit::<Typography>(c).letter_spacing(2.5);
        }, |m| m.letter_spacing == 2.5),
        ("white_space", |tx, c| {
            tx.edit::<Typography>(c).white_space(WhiteSpace::PreWrap);
        }, |m| m.white_space == WhiteSpace::PreWrap as u8),
        ("max_lines", |tx, c| {
            tx.edit::<Typography>(c).max_lines(4);
        }, |m| m.max_lines == 4),
        ("font_style", |tx, c| {
            tx.edit::<Typography>(c).font_style(FontStyle::Italic);
        }, |m| m.font_style == FontStyle::Italic as u8),
        ("word_break", |tx, c| {
            tx.edit::<Typography>(c).word_break(WordBreak::BreakAll);
        }, |m| m.word_break == WordBreak::BreakAll as u8),
        ("overflow_wrap", |tx, c| {
            tx.edit::<Typography>(c).overflow_wrap(OverflowWrap::Anywhere);
        }, |m| m.overflow_wrap == OverflowWrap::Anywhere as u8),
        // Same length as "Arial": replaced in place under the same arena span.
        ("same-length font_family", |tx, c| {
            tx.edit::<Typography>(c).font_family("Inter");
        }, |m| m.family == "Inter"),
    ];
    for (what, edit, check) in edits {
        let measured = frame(&mut store, &|tx| edit(tx, column));
        assert!(!measured.is_empty(), "{what} re-measures the inheriting leaf");
        assert!(measured.iter().all(|m| m.text == "leaf" && check(m)), "{what}: {measured:?}");
    }
    let measured = frame(&mut store, &|tx| {
        tx.edit::<Typography>(column).color(Rgba::WHITE);
    });
    assert!(measured.is_empty(), "inherited color never re-measures: {measured:?}");

    // SAFETY: Live handle; restoring the built-in measurer releases `seen`.
    assert_eq!(unsafe { (s.api.set_text_measurer)(s.h, None, std::ptr::null_mut()) }, OK);
    // SAFETY: Allocated above; no longer referenced by the runtime.
    drop(unsafe { Box::from_raw(seen) });
}

struct HalfMeasurer;

impl TextMeasurer for HalfMeasurer {
    fn measure(&self, _text: &str, _style: &TextStyle<&str>, _max_width: Option<f32>) -> (f32, f32) {
        (10.0, 12.0)
    }
}

#[test]
fn rust_text_measurer_swaps_and_resets() {
    let mut rt = SurfaceRuntime::new(1, Viewport::default());
    let (frame, text) = text_frame();
    rt.submit_nve2(&frame).unwrap();
    rt.tick(0.0).unwrap();
    rt.set_text_measurer(Some(Arc::new(HalfMeasurer))).unwrap();
    rt.tick(0.0).unwrap();
    let height = |rt: &SurfaceRuntime| {
        let g = rt.acquire_scene().unwrap();
        // SAFETY: Reader lock held by `g`; index within `node_capacity`.
        unsafe { (*g.view().rects.add(text.index())).height }
    };
    assert_eq!(height(&rt), 12.0);
    rt.set_text_measurer(None).unwrap();
    rt.tick(0.0).unwrap();
    assert_ne!(height(&rt), 12.0);
}
