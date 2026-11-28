//! Synthetic large `rsx!` source shared by the `rsx_diff` tests and `benches/rsx_diff_bench.rs`.

use std::fmt::Write as _;

/// Source with at least `min_lines` lines of realistic components: typed attributes of every value
/// form, static text, control flow, event handlers and interpolated text.
pub fn synthetic_rsx_source(min_lines: usize) -> String {
    let mut src = String::from("use nve_dioxus::prelude::*;\n\n");
    let mut lines = 2usize;
    let mut i = 0usize;
    while lines < min_lines {
        let before = src.len();
        let _ = write!(
            src,
            r##"/// Card number {i}.
#[component]
pub fn Card{i}(mut count: Signal<i32>, label: String) -> Element {{
    let total = count() * {i};
    rsx! {{
        view {{
            test_id: "card-{i}",
            padding: 8.0,
            opacity: 0.9,
            nve_style: TypedStyle::new()
                .gap(4.0)
                .width_pct(100.0)
                .bg(ColorRgba::rgb(15, 23, 42))
                .flex_direction(FlexDirection::Row),
            text {{
                font_size: 14.0,
                nve_style: TypedStyle::new().color(ColorRgba::rgba(248, 250, 252, 255)),
                "Card title {i}"
            }}
            if total > 3 {{
                text {{ "Many items" }}
            }}
            for n in 0..3 {{
                text {{ key: "{{n}}", "{{label}} {{n}}" }}
            }}
            pressable {{
                test_id: "card-{i}-cta",
                onpress: move |_| count += 1,
                border_radius: 6.0,
                nve_style: TypedStyle::new().cursor(CursorKind::Pointer),
                "Press {i}"
            }}
        }}
    }}
}}

"##
        );
        lines += src[before..].bytes().filter(|&b| b == b'\n').count();
        i += 1;
    }
    src
}

/// Replaces the last occurrence of `from` with `to` (the worst case for the differ: it must scan
/// the whole file before reaching the change).
pub fn replace_last(src: &str, from: &str, to: &str) -> String {
    let mut out = src.to_owned();
    if let Some(at) = src.rfind(from) {
        out.replace_range(at..at + from.len(), to);
    }
    out
}
