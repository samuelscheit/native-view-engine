//! Studio style resolution, run by every view on every render: the precomputed theme table
//! lookup (accent × polarity, then density and radius), the X-Ray overlay, layout metrics, and the
//! hashed author tone.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nve_studio_workspace::theme::{LayoutMetrics, XRayPalette, author_tone, studio_theme};
use nve_studio_workspace::{AccentTheme, StudioConfig};
use nve_system_env::{ColorScheme, ContrastLevel};
use nve_ui::Appearance;

fn bench_studio_style(c: &mut Criterion) {
    let config = StudioConfig {
        xray: true,
        ..StudioConfig::default()
    };
    let os = Appearance {
        scheme: ColorScheme::Dark,
        contrast: ContrastLevel::Standard,
    };
    let theme = studio_theme(&config, AccentTheme::Violet, os);

    c.bench_function("studio_theme_lookup", |b| {
        b.iter(|| {
            studio_theme(
                black_box(&config),
                black_box(AccentTheme::Violet),
                black_box(os),
            )
        })
    });
    c.bench_function("xray_palette_for_theme", |b| {
        b.iter(|| XRayPalette::for_theme(black_box(&theme)))
    });
    c.bench_function("layout_metrics_from_studio", |b| {
        b.iter(|| LayoutMetrics::from_studio(black_box(&config)))
    });
    c.bench_function("author_tone", |b| {
        b.iter(|| author_tone(black_box(&theme), black_box("Ada Lovelace")))
    });
    c.bench_function("use_studio_style_inputs_xray", |b| {
        b.iter(|| {
            let t = studio_theme(black_box(&config), AccentTheme::Violet, os);
            (
                XRayPalette::for_theme(&t),
                LayoutMetrics::from_studio(&config),
            )
        })
    });
}

criterion_group!(benches, bench_studio_style);
criterion_main!(benches);
