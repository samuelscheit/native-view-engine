//! Fixed-point decimal codec for style values (CSS lengths, opacity, flex factors, colors).
//!
//! `core::fmt` float printing (Grisu + Dragon4 fallback, ~11 KB of wasm) and `str::parse::<f32>`
//! (Eisel-Lemire + big-decimal fallback + a 10 KB power-of-five table) exist to round-trip
//! arbitrary doubles. Style values need neither, so every float that crosses a text boundary in
//! wasm-linked code goes through this module instead (docs/UPSTREAM.md, "Rust std on wasm32").
//!
//! # Formatting spec (`write_f32` / `write_f64`)
//!
//! This is the CSS number format of the whole engine; any other implementation (e.g. a JS
//! CommandBuffer interpreter) must reproduce it byte for byte:
//!
//! 1. `f32` inputs are widened to `f64` exactly; all arithmetic below is IEEE-754 binary64.
//! 2. A non-finite `v` (NaN, ±inf) prints `0`.
//! 3. `n = min(floor(|v| * 1000 + 0.5), 2^53)` — round half up on the magnitude to a multiple of
//!    1/1000 (one rounding of the product, then one of the sum, as in JS
//!    `Math.min(Math.floor(Math.abs(v) * 1000 + 0.5), 2 ** 53)`).
//! 4. `n == 0` prints `0` (so `-0.0` and `-0.0004` print `0`, never `-0`).
//! 5. Otherwise: `-` if `v < 0`, then the decimal digits of `n / 1000` (integer division, no
//!    leading zeros), then — only if `n % 1000 != 0` — `.` and the three digits of `n % 1000`
//!    with trailing zeros removed. Never an exponent, never a `+`.
//!
//! Examples: `12` → `12`, `0.5` → `0.5`, `-3.125` → `-3.125`, `1/3` → `0.333`, `0.9996` → `1`,
//! `128/255` → `0.502`, `1e300` → `9007199254740.992`.
//!
//! # Parsing spec (`parse_f32` / `parse_f64`)
//!
//! Grammar (ASCII only, no surrounding whitespace): `[+-]? (D+ ('.' D*)? | '.' D+) ([eE] [+-]? D+)?`
//! with `D = [0-9]`. Everything else — `inf`, `nan`, hex, `_`, units, a lone sign or dot — is
//! rejected with `None`, as is a result that overflows to ±inf. The sign is kept on zero (`-0`
//! parses to `-0.0`, like std).
//!
//! The first 19 significant digits are accumulated into a `u64` mantissa `m` and the decimal
//! exponent adjusted for the rest. When `m <= 2^53` and the exponent is within ±22 the result is a
//! single correctly rounded IEEE operation (`m * 10^e` or `m / 10^e`, both operands exact —
//! Clinger's fast path; exponents up to 37 are first folded into the mantissa while it stays
//! `<= 2^53`), i.e. **bit-identical to `str::parse::<f64>`**. `parse_f32` first tries
//! the same fast path in binary32 (`m <= 2^24`, exponent within ±10), which is bit-identical to
//! `str::parse::<f32>` and covers every style literal up to 7 significant digits; otherwise it
//! rounds the `f64` result. Outside the fast paths the result is within a few ULP of std.

/// Fractional digits kept by [`write_f64`] (1/1000 px is below any device pixel).
pub const FRACTION_DIGITS: u32 = 3;
/// Maximum finite scalar magnitude accepted across CSS parsers, typed style builders, and
/// native backends (`HeadlessBackend` / `SkiaBackend`), sealing range invariants at ingress.
pub const MAX_STYLE_SCALAR: f32 = 1e7;

/// Returns `true` iff `v` is finite and within `[-MAX_STYLE_SCALAR, MAX_STYLE_SCALAR]`.
#[inline]
pub const fn is_valid_style_scalar(v: f32) -> bool {
    v.is_finite() && v >= -MAX_STYLE_SCALAR && v <= MAX_STYLE_SCALAR
}
const SCALE: f64 = 1000.0;
/// Largest scaled magnitude: integers up to 2^53 are exact in binary64 (and in JS numbers).
const MAX_SCALED: f64 = 9_007_199_254_740_992.0;

/// Exact powers of ten representable in binary64 (`5^22 < 2^53`).
const POW10_F64: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];
/// Exact powers of ten representable in binary32 (`5^10 < 2^24`).
const POW10_F32: [f32; 11] = [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10];

/// Formats `v` in the fixed-point format of the module spec (`12`, `0.5`, `-3.125`) into a
/// caller-owned 24-byte stack buffer with zero heap allocation (max output length is 18 bytes).
pub fn format_f64(buf: &mut [u8; 24], v: f64) -> &str {
    if !v.is_finite() {
        return "0";
    }
    let scaled = (v.abs() * SCALE + 0.5).floor().min(MAX_SCALED) as u64;
    if scaled == 0 {
        return "0";
    }
    let mut len = 0usize;
    if v < 0.0 {
        buf[len] = b'-';
        len += 1;
    }
    let mut int_part = scaled / SCALE as u64;
    let int_start = len;
    loop {
        buf[len] = b'0' + (int_part % 10) as u8;
        len += 1;
        int_part /= 10;
        if int_part == 0 {
            break;
        }
    }
    buf[int_start..len].reverse();
    let mut frac = (scaled % SCALE as u64) as u32;
    if frac != 0 {
        buf[len] = b'.';
        len += 1;
        let mut div = SCALE as u32 / 10;
        while frac != 0 {
            buf[len] = b'0' + (frac / div) as u8;
            len += 1;
            frac %= div;
            div /= 10;
        }
    }
    core::str::from_utf8(&buf[..len]).unwrap_or("0")
}

/// Formats `n` in decimal into a caller-owned 24-byte stack buffer with zero heap allocation.
pub fn format_u64(buf: &mut [u8; 24], mut n: u64) -> &str {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("0")
}

/// Formats `n` in decimal with a leading `-` when negative into a caller-owned 24-byte stack buffer.
pub fn format_i64(buf: &mut [u8; 24], n: i64) -> &str {
    let mut v = n.unsigned_abs();
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if n < 0 {
        i -= 1;
        buf[i] = b'-';
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("0")
}

/// Appends `v` in the fixed-point format of the module spec (`12`, `0.5`, `-3.125`).
pub fn write_f64(out: &mut String, v: f64) {
    let mut buf = [0u8; 24];
    out.push_str(format_f64(&mut buf, v));
}

/// [`write_f64`] for an `f32` (widened exactly).
#[inline]
pub fn write_f32(out: &mut String, v: f32) {
    write_f64(out, f64::from(v));
}

/// Appends the decimal digits of `n` without going through `core::fmt`.
pub fn write_u64(out: &mut String, n: u64) {
    let mut buf = [0u8; 24];
    out.push_str(format_u64(&mut buf, n));
}

/// Appends `n` in decimal with a leading `-` when negative.
pub fn write_i64(out: &mut String, n: i64) {
    let mut buf = [0u8; 24];
    out.push_str(format_i64(&mut buf, n));
}

/// Returns `v` formatted per the module spec (allocates; prefer [`write_f64`] into a scratch buffer).
pub fn f64_to_string(v: f64) -> String {
    let mut s = String::new();
    write_f64(&mut s, v);
    s
}

/// A decimal literal split into sign, a ≤ 19-digit mantissa and a base-10 exponent.
struct Decimal {
    neg: bool,
    mant: u64,
    exp: i32,
}

fn scan_decimal(s: &str) -> Option<Decimal> {
    let b = s.as_bytes();
    let (neg, mut i) = match b.first() {
        Some(b'-') => (true, 1),
        Some(b'+') => (false, 1),
        _ => (false, 0),
    };
    let (mut mant, mut exp, mut sig, mut seen, mut frac) = (0u64, 0i32, 0u32, false, false);
    while i < b.len() {
        match b[i] {
            c @ b'0'..=b'9' => {
                seen = true;
                if sig < 19 {
                    mant = mant * 10 + u64::from(c - b'0');
                    if mant != 0 {
                        sig += 1;
                    }
                    if frac {
                        exp -= 1;
                    }
                } else if !frac {
                    exp += 1;
                }
            }
            b'.' if !frac => frac = true,
            _ => break,
        }
        i += 1;
    }
    if !seen {
        return None;
    }
    if i < b.len() {
        if b[i] | 0x20 != b'e' {
            return None;
        }
        i += 1;
        let eneg = match b.get(i) {
            Some(b'-') => {
                i += 1;
                true
            }
            Some(b'+') => {
                i += 1;
                false
            }
            _ => false,
        };
        if i == b.len() {
            return None;
        }
        let mut e: i32 = 0;
        for &c in &b[i..] {
            if !c.is_ascii_digit() {
                return None;
            }
            e = e.saturating_mul(10).saturating_add(i32::from(c - b'0'));
        }
        exp = exp.saturating_add(if eneg { -e } else { e });
    }
    Some(Decimal { neg, mant, exp })
}

fn decimal_to_f64(d: &Decimal) -> f64 {
    let m = d.mant as f64;
    // Disguised fast path: `12e30` = `12_000_000_000e22`, exact while the shifted mantissa fits.
    let shifted = (22..22 + 16)
        .contains(&d.exp)
        .then(|| d.mant.checked_mul(10u64.pow((d.exp - 22) as u32)))
        .flatten()
        .filter(|&s| s <= 1 << 53);
    let v = if d.mant == 0 {
        0.0
    } else if d.mant <= 1 << 53 && d.exp.unsigned_abs() <= 22 {
        let p = POW10_F64[d.exp.unsigned_abs() as usize];
        if d.exp < 0 { m / p } else { m * p }
    } else if let Some(s) = shifted {
        s as f64 * POW10_F64[22]
    } else {
        // Slow path: chunked exact powers; within a few ULP, saturates to 0 / inf.
        let (mut v, mut e) = (m, d.exp);
        while e > 0 && v.is_finite() {
            let k = e.min(22);
            v *= POW10_F64[k as usize];
            e -= k;
        }
        while e < 0 && v != 0.0 {
            let k = (-e).min(22);
            v /= POW10_F64[k as usize];
            e += k;
        }
        v
    };
    if d.neg { -v } else { v }
}

/// Parses a plain decimal per the module spec; bit-identical to `str::parse::<f64>` on the fast path.
pub fn parse_f64(s: &str) -> Option<f64> {
    let v = decimal_to_f64(&scan_decimal(s)?);
    v.is_finite().then_some(v)
}

/// Parses a plain decimal per the module spec; bit-identical to `str::parse::<f32>` for every
/// literal with ≤ 7 significant digits and a decimal exponent within ±10.
pub fn parse_f32(s: &str) -> Option<f32> {
    let d = scan_decimal(s)?;
    let v = if d.mant <= 1 << 24 && d.exp.unsigned_abs() <= 10 {
        let m = d.mant as f32;
        let p = POW10_F32[d.exp.unsigned_abs() as usize];
        let v = if d.exp < 0 { m / p } else { m * p };
        if d.neg { -v } else { v }
    } else {
        decimal_to_f64(&d) as f32
    };
    v.is_finite().then_some(v)
}

#[cfg(test)]
#[path = "num_tests.rs"]
mod tests;
