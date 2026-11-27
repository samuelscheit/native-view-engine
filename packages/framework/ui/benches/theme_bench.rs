//! Criterion benchmarks for the unified OKLCH `Theme`.
//!
//! - `theme/preset_lookup`: compile-time preset table index (Light/Dark/HC × brand).
//! - `theme/resolve_system`: `ThemeConfig::resolve` following the OS appearance.
//! - `theme/with_accent_runtime`: runtime OKLCH accent re-derivation (Studio per-workspace accent).
//! - `theme/build_full_runtime`: every token generated at runtime (what the `const` presets avoid).
//! - `theme/hex_bridge_write`: `Hex` formatting into a reused buffer (Phase-0 `style:` bridge).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::fmt::Write as _;
    use std::hint::black_box;

    use criterion::Criterion;
    use nve_ui::{Accent, AppBrand, Appearance, ColorHex, Polarity, Theme, ThemeConfig};

    pub fn bench(c: &mut Criterion) {
        let mut g = c.benchmark_group("theme");
        g.bench_function("preset_lookup", |b| {
            b.iter(|| Theme::preset(black_box(Polarity::HighContrastDark), black_box(AppBrand::Docs)))
        });
        g.bench_function("resolve_system", |b| {
            let cfg = ThemeConfig::new(AppBrand::Studio);
            b.iter(|| black_box(cfg).resolve(black_box(Appearance::DARK)))
        });
        g.bench_function("with_accent_runtime", |b| {
            let base = Theme::preset(Polarity::Dark, AppBrand::Studio);
            b.iter(|| black_box(base).with_accent(black_box(Accent::VIOLET)))
        });
        g.bench_function("build_full_runtime", |b| {
            b.iter(|| Theme::build(black_box(Polarity::Light), black_box(AppBrand::Docs)))
        });
        g.bench_function("hex_bridge_write", |b| {
            let theme = Theme::DARK;
            let mut buf = String::with_capacity(64);
            b.iter(|| {
                buf.clear();
                let _ = write!(buf, "{}", black_box(theme.border.subtle).hex());
                buf.len()
            })
        });
        g.finish();
    }
}

#[cfg(not(target_arch = "wasm32"))]
criterion::criterion_group!(benches, native::bench);
#[cfg(not(target_arch = "wasm32"))]
criterion::criterion_main!(benches);
