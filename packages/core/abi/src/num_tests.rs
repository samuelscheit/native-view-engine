use super::*;

fn fmt(v: f64) -> String {
    f64_to_string(v)
}

/// Deterministic xorshift64* so property tests need no `rand` dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Spec reference built on std: exact decimal rounding to 3 places, trailing zeros trimmed,
/// `-0` collapsed. Agrees with [`write_f64`] whenever `|v| * 1000` is not within float noise of
/// a `.5` tie (std rounds the exact binary value, the spec rounds the binary64 product).
fn std_reference(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".to_string() } else { s }
}

#[test]
fn formats_spec_examples_and_edges() {
    let cases: &[(f64, &str)] = &[
        (0.0, "0"),
        (-0.0, "0"),
        (12.0, "12"),
        (0.5, "0.5"),
        (-3.125, "-3.125"),
        (1.0 / 3.0, "0.333"),
        (-2.0 / 3.0, "-0.667"),
        (0.9996, "1"),
        (-0.9996, "-1"),
        (0.0004, "0"),
        (-0.0004, "0"),
        (0.0005, "0.001"),
        (0.05, "0.05"),
        (0.1, "0.1"),
        (100.25, "100.25"),
        (1e-300, "0"),
        (f64::MIN_POSITIVE, "0"),
        (f64::NAN, "0"),
        (-f64::NAN, "0"),
        (f64::INFINITY, "0"),
        (f64::NEG_INFINITY, "0"),
        (1e300, "9007199254740.992"),
        (-1e300, "-9007199254740.992"),
        (9_007_199_254_740.0, "9007199254740"),
        (128.0 / 255.0, "0.502"),
        (255.0 / 255.0, "1"),
        (1_000_000.0, "1000000"),
        (10.0, "10"),
        (0.01, "0.01"),
        (0.001, "0.001"),
        (0.0101, "0.01"),
        (-0.0015, "-0.002"),
        (999.9995, "1000"),
    ];
    for &(v, want) in cases {
        assert_eq!(fmt(v), want, "{v:e}");
    }
    let mut s = String::from("x=");
    write_f32(&mut s, 0.1f32);
    assert_eq!(s, "x=0.1", "f32 widening must not leak binary noise");
    s.clear();
    write_f32(&mut s, -0.0f32);
    assert_eq!(s, "0");
}

#[test]
fn integers_cover_full_range() {
    let mut s = String::new();
    write_i64(&mut s, i64::MIN);
    s.push(' ');
    write_i64(&mut s, i64::MAX);
    s.push(' ');
    write_u64(&mut s, u64::MAX);
    s.push(' ');
    write_u64(&mut s, 0);
    s.push(' ');
    write_i64(&mut s, -7);
    assert_eq!(s, "-9223372036854775808 9223372036854775807 18446744073709551615 0 -7");
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for _ in 0..20_000 {
        let n = rng.next() as i64 >> rng.below(64);
        let mut s = String::new();
        write_i64(&mut s, n);
        assert_eq!(s, n.to_string());
    }
}

#[test]
fn format_matches_std_rounding_away_from_ties() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut checked = 0;
    for _ in 0..200_000 {
        let mag = 10f64.powi(rng.below(14) as i32 - 5);
        let v = (rng.next() as f64 / u64::MAX as f64 - 0.5) * 2.0 * mag;
        let frac = (v.abs() * 1000.0).fract();
        if (frac - 0.5).abs() < 1e-6 {
            continue;
        }
        assert_eq!(fmt(v), std_reference(v), "{v:e}");
        checked += 1;
    }
    assert!(checked > 190_000);
}

#[test]
fn format_never_emits_exponent_sign_or_trailing_zero() {
    let mut rng = Rng(7);
    for _ in 0..100_000 {
        let v = f64::from_bits(rng.next());
        let s = fmt(v);
        assert!(!s.contains(['e', 'E', '+', 'n', 'i']), "{v:e} -> {s}");
        assert!(s != "-0" && !s.ends_with('.'), "{v:e} -> {s}");
        if let Some((_, frac)) = s.split_once('.') {
            assert!(!frac.is_empty() && frac.len() <= 3 && !frac.ends_with('0'), "{s}");
        }
        assert!(parse_f64(&s).is_some(), "formatted output must parse: {s}");
    }
}

#[test]
fn format_parse_format_is_idempotent() {
    let mut rng = Rng(42);
    for _ in 0..100_000 {
        let v = f32::from_bits(rng.next() as u32);
        let a = {
            let mut s = String::new();
            write_f32(&mut s, v);
            s
        };
        let back = parse_f32(&a).expect(&a);
        if v.is_finite() && v.abs() < 1e9 {
            assert!((f64::from(back) - f64::from(v)).abs() <= 0.0005 + f64::from(v.abs()) * 1e-7, "{v:e} {a}");
        }
        let mut b = String::new();
        write_f32(&mut b, back);
        if f64::from(v.abs()) < 1e4 {
            assert_eq!(a, b, "{v:e}");
        }
    }
}

#[test]
fn parses_plain_decimals_bit_identical_to_std() {
    for s in [
        "0", "-0", "+0", "-0.0", "1", "-1", "+2.5", ".5", "5.", "-.5", "0.1", "16", "100.0",
        "-0.75", "1e3", "2.5E-2", "1E+2", "123456.789", "0.000001", "00012", "9999999",
        "16777216", "16777217", "0.1234567", "1e10", "1e-10", "0.0000000001", "7e22", "12e30",
        "9007199254740992", "-9007199254740.992", "0.3", "1.1", "2.675", "1e-22",
    ] {
        let ours = parse_f32(s).unwrap();
        let std = s.parse::<f32>().unwrap();
        assert_eq!(ours.to_bits(), std.to_bits(), "f32 {s}");
        let ours = parse_f64(s).unwrap();
        let std = s.parse::<f64>().unwrap();
        assert_eq!(ours.to_bits(), std.to_bits(), "f64 {s}");
    }
    // Outside the f64 fast path (mantissa > 2^53 or |exp| > 22 after folding): f32 still
    // bit-identical, f64 within the documented few-ULP bound.
    for s in [
        "3.4028235e38", "-3.4028235e38", "1.17549435e-38", "1e-45", "1.5e-45",
        "12345678901234567890123", "0.1234567890123456789012", "4.9e-324", "1e-30",
    ] {
        let ours = parse_f32(s).unwrap();
        let std = s.parse::<f32>().unwrap();
        assert_eq!(ours.to_bits(), std.to_bits(), "f32 {s}");
        let ours = parse_f64(s).unwrap().to_bits() as i64;
        let std = s.parse::<f64>().unwrap().to_bits() as i64;
        assert!((ours - std).unsigned_abs() <= 4, "f64 {s}");
    }
}

#[test]
fn rejects_malformed_and_non_finite_input() {
    for s in [
        "", "-", "+", ".", "-.", "+.", "e5", ".e5", "1e", "1e+", "1e-", "1e+-2", "1ee2", "inf",
        "-inf", "infinity", "NaN", "nan", "1px", "12%", " 1", "1 ", "\t1", "0x10", "1.2.3",
        "--1", "+-1", "1_000", "1,5", "١", "1e3.5", "1.5e", "3.5e38", "1e39", "-1e39", "1e400",
        "1e99999999999", "\u{0}", "1\u{0}",
    ] {
        assert_eq!(parse_f32(s), None, "f32 {s:?}");
        if !matches!(s, "3.5e38" | "1e39" | "-1e39") {
            assert_eq!(parse_f64(s), None, "f64 {s:?}");
        }
    }
    assert_eq!(parse_f64("1e-99999999999").map(f64::to_bits), Some(0));
    assert_eq!(parse_f64("-1e-400").map(f64::to_bits), Some((-0.0f64).to_bits()));
}

fn random_literal(rng: &mut Rng, max_digits: u64, max_exp: u64) -> String {
    let digits = 1 + rng.below(max_digits);
    let mut lit = String::new();
    if rng.below(2) == 0 {
        lit.push('-');
    }
    let mut ds = String::new();
    for _ in 0..digits {
        ds.push((b'0' + rng.below(10) as u8) as char);
    }
    match rng.below(3) {
        0 => lit.push_str(&ds),
        1 => {
            let dot = rng.below(digits + 1) as usize;
            lit.push_str(&ds[..dot]);
            lit.push('.');
            lit.push_str(&ds[dot..]);
            if dot == 0 && ds.is_empty() {
                lit.push('0');
            }
        }
        _ => {
            lit.push_str(&ds);
            lit.push('e');
            let e = rng.below(2 * max_exp + 1) as i64 - max_exp as i64;
            lit.push_str(&e.to_string());
        }
    }
    lit
}

#[test]
fn f32_fast_path_is_bit_identical_to_std() {
    let mut rng = Rng(0xDEAD_BEEF);
    for _ in 0..300_000 {
        let lit = random_literal(&mut rng, 7, 3);
        let std = lit.parse::<f32>().unwrap();
        assert_eq!(parse_f32(&lit).unwrap().to_bits(), std.to_bits(), "{lit}");
    }
}

#[test]
fn f64_fast_path_is_bit_identical_to_std() {
    let mut rng = Rng(0xFEED_FACE);
    for _ in 0..300_000 {
        let lit = random_literal(&mut rng, 15, 7);
        let std = lit.parse::<f64>().unwrap();
        assert_eq!(parse_f64(&lit).unwrap().to_bits(), std.to_bits(), "{lit}");
    }
}

#[test]
fn f32_from_long_literals_matches_std() {
    // 8–15 significant digits take the f64 route then round to f32; check it never disagrees with
    // std's direct correctly rounded f32 parse on realistic literals.
    let mut rng = Rng(0xC0FF_EE00);
    for _ in 0..300_000 {
        let lit = random_literal(&mut rng, 15, 7);
        let std = lit.parse::<f32>().unwrap();
        assert_eq!(parse_f32(&lit).unwrap().to_bits(), std.to_bits(), "{lit}");
    }
}

#[test]
fn slow_path_stays_within_a_few_ulp() {
    let mut rng = Rng(0xABCD_EF01);
    for _ in 0..100_000 {
        let lit = random_literal(&mut rng, 25, 250);
        let std = lit.parse::<f64>().unwrap();
        let Some(ours) = parse_f64(&lit) else {
            assert!(std.is_infinite() || std.abs() > 1e300, "{lit} -> std {std:e}");
            continue;
        };
        if std == 0.0 || !std.is_normal() {
            assert!(ours.abs() <= f64::MIN_POSITIVE * 4.0, "{lit}");
            continue;
        }
        let ulps = (ours.to_bits() as i64 - std.to_bits() as i64).unsigned_abs();
        assert!(ulps <= 16, "{lit}: {ours:e} vs {std:e} ({ulps} ulp)");
    }
}

#[test]
fn stack_formatters_reuse_dirty_buffer_across_extremes() {
    let mut buf = [0xFFu8; 24];
    assert_eq!(format_i64(&mut buf, i64::MIN), "-9223372036854775808");
    buf.fill(0xFF);
    assert_eq!(format_i64(&mut buf, i64::MAX), "9223372036854775807");
    buf.fill(0xFF);
    assert_eq!(format_u64(&mut buf, u64::MAX), "18446744073709551615");
    buf.fill(0xFF);
    assert_eq!(format_u64(&mut buf, 0), "0");
    for (v, want) in [
        (f64::NAN, "0"),
        (f64::INFINITY, "0"),
        (f64::NEG_INFINITY, "0"),
        (-0.0, "0"),
        (1e300, "9007199254740.992"),
        (-1e300, "-9007199254740.992"),
        (-3.125, "-3.125"),
    ] {
        buf.fill(0xFF);
        assert_eq!(format_f64(&mut buf, v), want, "{v:e}");
    }
}
