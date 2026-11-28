//! Shared `<select>` base-value parity table (test / probe fixture, not runtime API): the
//! headless parser (unit test) and live Chrome hydration (`issues-probes` E2E) must both derive
//! `expected` as the SSR base value of each case. Overflow/trailing-garbage `size` expectations
//! were verified against Chrome.

/// One `<select>` markup case with the `select.value` browsers derive from it.
#[derive(Debug, Clone, Copy)]
pub struct SelectBaseCase {
    /// Attributes inside `<select …>`.
    pub attrs: &'static str,
    pub options: &'static str,
    pub expected: &'static str,
}

/// Every case's markup, hydrated headlessly and in live Chrome, yields `expected`.
pub const SELECT_BASE_CASES: &[SelectBaseCase] = &[
    SelectBaseCase {
        attrs: "",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "a",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option value=\"a\">A</option><option value=\"b\" selected>B</option>",
        expected: "b",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option value=\"a\" selected>A</option><option value=\"b\" selected>B</option>",
        expected: "b",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option value=\"a\" disabled>A</option><option value=\"b\">B</option>",
        expected: "b",
    },
    SelectBaseCase {
        attrs: "",
        options: "<optgroup label=\"g\" disabled><option value=\"a\">A</option></optgroup><option value=\"b\">B</option>",
        expected: "b",
    },
    SelectBaseCase {
        attrs: "",
        options: "<optgroup label=\"g\"><option value=\"a\">A</option><option value=\"b\" selected>B</option></optgroup>",
        expected: "b",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option value=\"a\">A</option><option value=\"b\" selected disabled>B</option>",
        expected: "b",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option disabled value=\"a\">A</option><optgroup disabled label=\"g\"><option value=\"b\">B</option></optgroup>",
        expected: "",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option>  Hello \n  World </option><option>x</option>",
        expected: "Hello World",
    },
    SelectBaseCase {
        attrs: "multiple",
        options: "<option value=\"a\" selected>A</option><option value=\"b\" selected>B</option>",
        expected: "a",
    },
    SelectBaseCase {
        attrs: "multiple",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "",
    },
    SelectBaseCase {
        attrs: "size=\"3\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "",
    },
    SelectBaseCase {
        attrs: "size=\"0\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "a",
    },
    SelectBaseCase {
        attrs: "size=\"1\"",
        options: "<option value=\"a\" disabled>A</option><option value=\"b\">B</option>",
        expected: "b",
    },
    // `size` follows the HTML non-negative integer rules: trailing garbage ignored.
    SelectBaseCase {
        attrs: "size=\"3px\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "",
    },
    SelectBaseCase {
        attrs: "size=\" 3 x\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "",
    },
    SelectBaseCase {
        attrs: "size=\"+3\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "",
    },
    SelectBaseCase {
        attrs: "size=\"2147483648\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "",
    },
    // `u32` overflow, negative, non-digit, and fractional-below-2 sizes are display size 1.
    SelectBaseCase {
        attrs: "size=\"4294967296\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "a",
    },
    SelectBaseCase {
        attrs: "size=\"-3\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "a",
    },
    SelectBaseCase {
        attrs: "size=\"-0\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "a",
    },
    SelectBaseCase {
        attrs: "size=\"0x3\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "a",
    },
    SelectBaseCase {
        attrs: "size=\"1.9\"",
        options: "<option value=\"a\">A</option><option value=\"b\">B</option>",
        expected: "a",
    },
    SelectBaseCase {
        attrs: "size=\"3\"",
        options: "<option value=\"a\">A</option><option value=\"b\" selected>B</option>",
        expected: "b",
    },
    // Option text spans element descendants and excludes `<script>`; empty markup is `""`.
    SelectBaseCase {
        attrs: "",
        options: "<option>A<b> B </b> C</option>",
        expected: "A B C",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option>x<script>y</script></option>",
        expected: "x",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option></option><option value=\"b\">B</option>",
        expected: "",
    },
    SelectBaseCase {
        attrs: "",
        options: "<option value=\"\">Label</option>",
        expected: "",
    },
    SelectBaseCase {
        attrs: "",
        options: "",
        expected: "",
    },
];
