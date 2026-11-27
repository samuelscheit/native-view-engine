//! `const fn` OKLCH ⇄ sRGB conversion, sRGB gamut mapping, and WCAG 2.1 / APCA contrast math.
//!
//! Everything here is `const` so `Theme` presets are baked at compile time with zero runtime
//! parsing; the same functions run at runtime for custom accent overrides. Transcendental
//! functions are implemented with range-reduced series (accurate to well below 1/255 per channel).

use nve_abi::ColorRgba;

/// Chroma ceiling for generated colors (the sRGB gamut peaks near C 0.32).
pub const MAX_CHROMA: f32 = 0.4;

const PI: f64 = core::f64::consts::PI;
const LN_2: f64 = core::f64::consts::LN_2;

/// Reduces `x` into `[-π, π]` (callers pre-reduce degrees, so `|x|` stays small; non-finite → 0).
const fn wrap_pi(x: f64) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let tau = 2.0 * PI;
    let k = ((x + PI) / tau) as i64;
    let mut r = x - k as f64 * tau;
    if r > PI {
        r -= tau;
    }
    if r < -PI {
        r += tau;
    }
    r
}

/// Taylor series sine on `[-π, π]`.
pub(crate) const fn sin(x: f64) -> f64 {
    let x = wrap_pi(x);
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    let mut n = 1;
    while n < 14 {
        term *= -x2 / ((2 * n) as f64 * (2 * n + 1) as f64);
        sum += term;
        n += 1;
    }
    sum
}

pub(crate) const fn cos(x: f64) -> f64 {
    sin(x + PI / 2.0)
}

/// `e^x` via `x = k·ln2 + r` reduction and a Taylor series on `|r| ≤ ln2/2`.
pub(crate) const fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x < -700.0 {
        return 0.0;
    }
    if x > 700.0 {
        return f64::INFINITY;
    }
    let k = if x >= 0.0 {
        (x / LN_2 + 0.5) as i64
    } else {
        (x / LN_2 - 0.5) as i64
    };
    let r = x - k as f64 * LN_2;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut n = 1;
    while n < 18 {
        term *= r / n as f64;
        sum += term;
        n += 1;
    }
    // Scale by 2^k through the exponent bits (|k| stays far inside the normal range here).
    let bits = ((k + 1023) as u64) << 52;
    sum * f64::from_bits(bits)
}

/// Natural log for `x > 0` via mantissa/exponent split and the `atanh` series. NaN stays NaN
/// (instead of reading the `0x7ff` exponent as a huge finite value).
pub(crate) const fn ln(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return f64::INFINITY;
    }
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let mantissa = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    let z = (mantissa - 1.0) / (mantissa + 1.0);
    let z2 = z * z;
    let mut term = z;
    let mut sum = z;
    let mut n = 1;
    while n < 16 {
        term *= z2;
        sum += term / (2 * n + 1) as f64;
        n += 1;
    }
    2.0 * sum + exponent as f64 * LN_2
}

/// `x^y` for finite `x ≥ 0`; non-finite or non-positive `x` → 0.
pub(crate) const fn powf(x: f64, y: f64) -> f64 {
    if !x.is_finite() || x <= 0.0 { 0.0 } else { exp(y * ln(x)) }
}

pub(crate) const fn cbrt(x: f64) -> f64 {
    if x == 0.0 {
        0.0
    } else if x < 0.0 {
        -powf(-x, 1.0 / 3.0)
    } else {
        powf(x, 1.0 / 3.0)
    }
}

const fn sqrt(x: f64) -> f64 {
    powf(x, 0.5)
}

/// `atan` via two half-angle reductions and a Taylor series on `|z| ≤ tan(π/16)`.
const fn atan(z: f64) -> f64 {
    if z < 0.0 {
        return -atan(-z);
    }
    if z > 1.0 {
        return PI / 2.0 - atan(1.0 / z);
    }
    let z1 = z / (1.0 + sqrt(1.0 + z * z));
    let z2 = z1 / (1.0 + sqrt(1.0 + z1 * z1));
    let sq = z2 * z2;
    let mut term = z2;
    let mut sum = z2;
    let mut n = 1;
    while n < 12 {
        term *= -sq;
        sum += term / (2 * n + 1) as f64;
        n += 1;
    }
    4.0 * sum
}

const fn atan2(y: f64, x: f64) -> f64 {
    if x > 0.0 {
        atan(y / x)
    } else if x < 0.0 {
        if y >= 0.0 { atan(y / x) + PI } else { atan(y / x) - PI }
    } else if y > 0.0 {
        PI / 2.0
    } else if y < 0.0 {
        -PI / 2.0
    } else {
        0.0
    }
}

/// A perceptual OKLCH color: lightness `l ∈ [0, 1]`, chroma `c ≥ 0`, hue `h` in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

impl Oklch {
    pub const fn new(l: f32, c: f32, h: f32) -> Self {
        Self { l, c, h }
    }

    /// Maps into the sRGB gamut by bisecting chroma (hue and lightness are preserved), then
    /// quantizes to 8-bit sRGB.
    pub const fn to_srgb(self) -> ColorRgba {
        // Non-finite inputs collapse to their neutral value instead of poisoning the encode.
        let l = if self.l.is_nan() { 0.0 } else { (self.l as f64).clamp(0.0, 1.0) };
        // Reduce degrees first: `h · π/180` for huge finite hues would overflow the radian wrap.
        let h = if self.h.is_finite() { (self.h % 360.0) as f64 * PI / 180.0 } else { 0.0 };
        let (ca, sa) = (cos(h), sin(h));
        // sRGB peaks near C 0.32; capping keeps the 18-step bisection precise for any input.
        // The gamut collapses to a point at L = 0 (black) and L = 1 (white).
        let mut c = if self.c > 0.0 && l > 0.0 && l < 1.0 { (self.c as f64).min(MAX_CHROMA as f64) } else { 0.0 };
        if !in_gamut(oklab_to_linear(l, c * ca, c * sa)) {
            let (mut lo, mut hi) = (0.0, c);
            let mut i = 0;
            while i < 18 {
                let mid = (lo + hi) * 0.5;
                if in_gamut(oklab_to_linear(l, mid * ca, mid * sa)) {
                    lo = mid;
                } else {
                    hi = mid;
                }
                i += 1;
            }
            c = lo;
        }
        let [r, g, b] = oklab_to_linear(l, c * ca, c * sa);
        ColorRgba::rgb(encode_channel(r), encode_channel(g), encode_channel(b))
    }

    /// Converts an 8-bit sRGB color into OKLCH (alpha is ignored).
    pub const fn from_srgb(color: ColorRgba) -> Self {
        let r = decode_channel(color.r);
        let g = decode_channel(color.g);
        let b = decode_channel(color.b);
        let l_ = cbrt(0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b);
        let m_ = cbrt(0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b);
        let s_ = cbrt(0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b);
        let l = 0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_;
        let a = 1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_;
        let bb = 0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_;
        let c = sqrt(a * a + bb * bb);
        let mut h = atan2(bb, a) * 180.0 / PI;
        if h < 0.0 {
            h += 360.0;
        }
        Self { l: l as f32, c: c as f32, h: h as f32 }
    }
}

const fn oklab_to_linear(l: f64, a: f64, b: f64) -> [f64; 3] {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701_0 * s3,
    ]
}

const fn in_gamut(rgb: [f64; 3]) -> bool {
    const EPS: f64 = 1e-4;
    rgb[0] >= -EPS
        && rgb[0] <= 1.0 + EPS
        && rgb[1] >= -EPS
        && rgb[1] <= 1.0 + EPS
        && rgb[2] >= -EPS
        && rgb[2] <= 1.0 + EPS
}

/// Linear-light channel → 8-bit sRGB (IEC 61966-2-1 transfer function).
const fn encode_channel(v: f64) -> u8 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let e = if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * powf(v, 1.0 / 2.4) - 0.055
    };
    (e * 255.0 + 0.5) as u8
}

/// 8-bit sRGB → linear-light channel.
const fn decode_channel(c: u8) -> f64 {
    let v = c as f64 / 255.0;
    if v <= 0.040_45 {
        v / 12.92
    } else {
        powf((v + 0.055) / 1.055, 2.4)
    }
}

/// Replaces the alpha channel.
pub const fn with_alpha(color: ColorRgba, alpha: u8) -> ColorRgba {
    ColorRgba::rgba(color.r, color.g, color.b, alpha)
}

/// Source-over composites `fg` onto an opaque `bg` in 8-bit sRGB space (browser semantics).
pub const fn composite(fg: ColorRgba, bg: ColorRgba) -> ColorRgba {
    const fn mix(f: u8, b: u8, a: u32) -> u8 {
        ((f as u32 * a + b as u32 * (255 - a) + 127) / 255) as u8
    }
    let a = fg.a as u32;
    ColorRgba::rgb(mix(fg.r, bg.r, a), mix(fg.g, bg.g, a), mix(fg.b, bg.b, a))
}

/// WCAG 2.1 relative luminance of an opaque sRGB color.
pub const fn relative_luminance(c: ColorRgba) -> f64 {
    0.2126 * decode_channel(c.r) + 0.7152 * decode_channel(c.g) + 0.0722 * decode_channel(c.b)
}

/// WCAG 2.1 contrast ratio (`1.0 ..= 21.0`) between two opaque colors.
pub const fn wcag_contrast(a: ColorRgba, b: ColorRgba) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// APCA (SAPC 0.0.98G-4g) lightness contrast `Lc` of `text` on `bg`; positive for dark-on-light,
/// negative for light-on-dark. Use `abs()` against the APCA readability thresholds.
pub const fn apca_contrast(text: ColorRgba, bg: ColorRgba) -> f64 {
    const fn screen_y(c: ColorRgba) -> f64 {
        const fn ch(v: u8) -> f64 {
            powf(v as f64 / 255.0, 2.4)
        }
        let y = 0.212_672_9 * ch(c.r) + 0.715_152_2 * ch(c.g) + 0.072_175 * ch(c.b);
        if y < 0.022 { y + powf(0.022 - y, 1.414) } else { y }
    }
    let (yt, yb) = (screen_y(text), screen_y(bg));
    let diff = yb - yt;
    if (if diff < 0.0 { -diff } else { diff }) < 0.000_5 {
        return 0.0;
    }
    let out = if yb > yt {
        let sapc = (powf(yb, 0.56) - powf(yt, 0.57)) * 1.14;
        if sapc < 0.1 { 0.0 } else { sapc - 0.027 }
    } else {
        let sapc = (powf(yb, 0.65) - powf(yt, 0.62)) * 1.14;
        if sapc > -0.1 { 0.0 } else { sapc + 0.027 }
    };
    out * 100.0
}

/// Picks whichever of `light` / `dark` foreground has the higher WCAG contrast on `fill`.
pub const fn best_foreground(fill: ColorRgba, light: ColorRgba, dark: ColorRgba) -> ColorRgba {
    if wcag_contrast(light, fill) >= wcag_contrast(dark, fill) { light } else { dark }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcendental_math_matches_std() {
        for i in -40..=40 {
            let x = i as f64 * 0.37;
            assert!((sin(x) - x.sin()).abs() < 1e-9, "sin {x}");
            assert!((cos(x) - x.cos()).abs() < 1e-9, "cos {x}");
            assert!((exp(x / 4.0) - (x / 4.0).exp()).abs() < 1e-9 * (x / 4.0).exp().max(1.0));
            assert!((atan2(x, 1.3) - x.atan2(1.3)).abs() < 1e-9, "atan2 {x}");
            assert!((atan2(-1.1, x) - (-1.1f64).atan2(x)).abs() < 1e-9, "atan2 x {x}");
        }
        for x in [1e-6, 0.003, 0.5, 1.0, 2.0, 7.5, 1234.5] {
            assert!((ln(x) - f64::ln(x)).abs() < 1e-10, "ln {x}");
            assert!((cbrt(x) - x.cbrt()).abs() < 1e-9 * x.cbrt().max(1.0), "cbrt {x}");
        }
    }

    #[test]
    fn oklch_round_trips_known_tailwind_values() {
        // Tailwind v4 publishes its palette in OKLCH: zinc-950 = oklch(14.1% 0.005 285.823).
        assert_eq!(Oklch::new(0.141, 0.005, 285.823).to_srgb(), ColorRgba::rgb(0x09, 0x09, 0x0b));
        // zinc-900 = oklch(21% 0.006 285.885) => #18181b.
        assert_eq!(Oklch::new(0.21, 0.006, 285.885).to_srgb(), ColorRgba::rgb(0x18, 0x18, 0x1b));
        for c in [
            ColorRgba::rgb(0x38, 0xbd, 0xf8),
            ColorRgba::rgb(0xa7, 0x8b, 0xfa),
            ColorRgba::rgb(0xdc, 0x26, 0x26),
            ColorRgba::WHITE,
            ColorRgba::BLACK,
        ] {
            assert_eq!(Oklch::from_srgb(c).to_srgb(), c);
        }
    }

    #[test]
    fn contrast_formulas_match_reference_values() {
        assert!((wcag_contrast(ColorRgba::BLACK, ColorRgba::WHITE) - 21.0).abs() < 1e-9);
        // APCA reference: #000 on #fff = Lc 106.04; #fff on #000 = Lc -107.88.
        assert!((apca_contrast(ColorRgba::BLACK, ColorRgba::WHITE) - 106.04).abs() < 0.1);
        assert!((apca_contrast(ColorRgba::WHITE, ColorRgba::BLACK) + 107.88).abs() < 0.1);
        // #888 on #fff = Lc 63.06 (APCA demo tool).
        let grey = ColorRgba::rgb(0x88, 0x88, 0x88);
        assert!((apca_contrast(grey, ColorRgba::WHITE) - 63.06).abs() < 0.2);
        assert_eq!(
            composite(ColorRgba::rgba(255, 255, 255, 128), ColorRgba::BLACK),
            ColorRgba::rgb(128, 128, 128)
        );
    }
}
