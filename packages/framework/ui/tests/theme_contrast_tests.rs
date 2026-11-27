//! WCAG 2.1 + APCA contrast contract for every generated [`Theme`]: all four polarities ×
//! Default / Docs / Studio brands, plus every Studio runtime accent override.
//!
//! Each role pair is checked on every surface it can appear on (translucent tokens are
//! composited first). All violations are collected so one run reports the full matrix.

use nve_abi::ColorRgba;
use nve_interop::traits::UiDensity;
use nve_interop::{ColorScheme, ContrastLevel};
use nve_ui::{
    apca_contrast, composite, relative_luminance, wcag_contrast, Accent, AppBrand, Appearance,
    accent_indicator, badge_colors, button_colors, error_text, selected_text, tab_colors,
    toast_tone, BadgeVariant, Breakpoint, ButtonVariant, ControlColors, Layer, Oklch, Polarity, Scale,
    Theme, ThemeConfig, ThemeMode, ToastVariant,
};

/// `(min WCAG ratio, min |APCA Lc|)` per role.
struct Bar {
    primary: (f64, f64),
    secondary: (f64, f64),
    tertiary: (f64, f64),
    readable: (f64, f64),
    non_text: f64,
}

const STANDARD: Bar = Bar {
    primary: (12.0, 90.0),
    secondary: (7.0, 70.0),
    tertiary: (4.5, 55.0),
    readable: (4.5, 55.0),
    non_text: 3.0,
};

const HIGH: Bar = Bar {
    primary: (15.0, 95.0),
    secondary: (10.0, 85.0),
    tertiary: (7.0, 70.0),
    readable: (7.0, 70.0),
    non_text: 4.5,
};

const STUDIO_ACCENTS: [(&str, Accent); 7] = [
    ("cyan", Accent::CYAN),
    ("violet", Accent::VIOLET),
    ("green", Accent::GREEN),
    ("amber", Accent::AMBER),
    ("blue", Accent::BLUE),
    ("indigo", Accent::INDIGO),
    ("red", Accent::RED),
];

fn all_themes() -> Vec<(String, Theme)> {
    let mut out = Vec::new();
    for p in Polarity::ALL {
        for b in AppBrand::ALL {
            out.push((format!("{p:?}/{b:?}"), Theme::preset(p, b)));
        }
        for (name, accent) in STUDIO_ACCENTS {
            let t = Theme::preset(p, AppBrand::Studio).with_accent(accent);
            out.push((format!("{p:?}/Studio+{name}"), t));
        }
    }
    out
}

struct Report(Vec<String>);

impl Report {
    fn text(&mut self, ctx: &str, fg: ColorRgba, bg: ColorRgba, (wcag, lc): (f64, f64)) {
        let fg = composite(fg, bg);
        let r = wcag_contrast(fg, bg);
        let a = apca_contrast(fg, bg).abs();
        if r < wcag || a < lc {
            self.0.push(format!("{ctx}: WCAG {r:.2} (min {wcag}) APCA {a:.1} (min {lc})"));
        }
    }

    fn non_text(&mut self, ctx: &str, fg: ColorRgba, bg: ColorRgba, min: f64) {
        let r = wcag_contrast(composite(fg, bg), bg);
        if r < min {
            self.0.push(format!("{ctx}: WCAG {r:.2} (min {min})"));
        }
    }

    fn check(&mut self, cond: bool, msg: String) {
        if !cond {
            self.0.push(msg);
        }
    }
}

/// Every component variant's actual `(fg, bg)` pairing, through the same pure helpers the
/// components paint with (catches wrong token pairings that raw-token checks cannot).
#[test]
fn every_component_variant_pairing_is_readable_in_every_preset() {
    let mut rep = Report(Vec::new());
    for (name, t) in all_themes() {
        let bar = if t.is_high_contrast() { &HIGH } else { &STANDARD };
        let s = t.surface;
        let mut control = |label: String, c: ControlColors, on: ColorRgba| {
            let bg = composite(c.bg, on);
            rep.text(&label, c.fg, bg, bar.readable);
        };
        for (sn, on) in [("base", s.base), ("raised", s.raised), ("overlay", s.overlay)] {
            for v in [
                ButtonVariant::Primary,
                ButtonVariant::Secondary,
                ButtonVariant::Ghost,
                ButtonVariant::Destructive,
                ButtonVariant::Outline,
            ] {
                for focused in [false, true] {
                    let c = button_colors(&t, v, focused, false);
                    control(format!("{name} Button::{v:?} focused={focused}@{sn}"), c, on);
                }
            }
            for v in [
                BadgeVariant::Neutral,
                BadgeVariant::Accent,
                BadgeVariant::Success,
                BadgeVariant::Warning,
                BadgeVariant::Danger,
            ] {
                control(format!("{name} Badge::{v:?}@{sn}"), badge_colors(&t, v), on);
            }
            for (selected, focused) in [(false, false), (false, true), (true, false), (true, true)] {
                let c = tab_colors(&t, selected, focused);
                control(format!("{name} Tab sel={selected} focus={focused}@{sn}"), c, on);
            }
        }
        for (sn, bg) in [("canvas", s.canvas), ("base", s.base), ("raised", s.raised), ("overlay", s.overlay)] {
            rep.text(&format!("{name} error_text/{sn}"), error_text(&t), bg, bar.readable);
            rep.text(&format!("{name} selected_text/{sn}"), selected_text(&t), bg, bar.readable);
            rep.non_text(&format!("{name} accent_indicator/{sn}"), accent_indicator(&t), bg, bar.non_text);
        }
        for v in [ToastVariant::Info, ToastVariant::Success, ToastVariant::Warning, ToastVariant::Danger] {
            let edge = toast_tone(&t, v).text;
            rep.non_text(&format!("{name} Toast::{v:?} edge/overlay"), edge, s.overlay, bar.non_text);
        }
        rep.check(toast_tone(&t, ToastVariant::Info) == t.status.info, format!("{name} Toast::Info uses status.info"));
    }
    assert!(rep.0.is_empty(), "{} component contrast violations:\n{}", rep.0.len(), rep.0.join("\n"));
}

#[test]
fn every_role_pair_meets_wcag_and_apca_in_every_preset() {
    let mut rep = Report(Vec::new());
    for (name, t) in all_themes() {
        let bar = if t.is_high_contrast() { &HIGH } else { &STANDARD };
        let s = t.surface;
        let surfaces = [
            ("canvas", s.canvas),
            ("base", s.base),
            ("raised", s.raised),
            ("overlay", s.overlay),
            ("sunken", s.sunken),
            ("subtle", s.subtle),
        ];
        for (sn, bg) in surfaces {
            rep.text(&format!("{name} text.primary/{sn}"), t.text.primary, bg, bar.primary);
            rep.text(&format!("{name} text.secondary/{sn}"), t.text.secondary, bg, bar.secondary);
            rep.text(&format!("{name} text.tertiary/{sn}"), t.text.tertiary, bg, bar.tertiary);
            rep.text(&format!("{name} text.link/{sn}"), t.text.link, bg, bar.readable);
            rep.non_text(&format!("{name} border.focus/{sn}"), t.border.focus, bg, bar.non_text);
        }
        for (sn, bg) in [("base", s.base), ("raised", s.raised), ("overlay", s.overlay)] {
            rep.non_text(&format!("{name} border.strong/{sn}"), t.border.strong, bg, bar.non_text);
            for (ln, layer) in [("hover", t.state.hover), ("active", t.state.active), ("selected", t.state.selected)] {
                let bg = composite(layer, bg);
                rep.text(&format!("{name} text.primary/{ln}@{sn}"), t.text.primary, bg, bar.secondary);
                rep.text(&format!("{name} text.secondary/{ln}@{sn}"), t.text.secondary, bg, bar.readable);
            }
        }
        if t.is_high_contrast() {
            rep.non_text(&format!("{name} border.default/base"), t.border.default, s.base, 3.0);
        } else {
            rep.non_text(&format!("{name} border.default/base"), t.border.default, s.base, 1.15);
            rep.non_text(&format!("{name} border.subtle/base"), t.border.subtle, s.base, 1.08);
        }
        rep.text(&format!("{name} text.on_accent/accent"), t.text.on_accent, t.status.accent.solid, bar.readable);
        let st = t.status;
        for (tn, tone) in [("accent", st.accent), ("success", st.success), ("warning", st.warning), ("danger", st.danger), ("info", st.info)] {
            rep.text(&format!("{name} {tn}.on_solid/solid"), tone.on_solid, tone.solid, bar.readable);
            rep.text(&format!("{name} {tn}.text/subtle"), tone.text, tone.subtle, bar.readable);
            for (sn, bg) in [("canvas", s.canvas), ("base", s.base), ("overlay", s.overlay)] {
                rep.text(&format!("{name} {tn}.text/{sn}"), tone.text, bg, bar.readable);
            }
            rep.check(tone.subtle != s.base, format!("{name} {tn}.subtle indistinct from base"));
        }
        let y = t.syntax;
        for (yn, c) in [
            ("plain", y.plain), ("keyword", y.keyword), ("type", y.type_name), ("string", y.string),
            ("comment", y.comment), ("number", y.number), ("function", y.function),
            ("macro", y.macro_name), ("attribute", y.attribute), ("punctuation", y.punctuation),
            ("diff_add", y.diff_add), ("diff_del", y.diff_del),
        ] {
            for (sn, bg) in [("sunken", s.sunken), ("base", s.base)] {
                rep.text(&format!("{name} syntax.{yn}/{sn}"), c, bg, bar.readable);
            }
        }
    }
    assert!(rep.0.is_empty(), "{} contrast violations:\n{}", rep.0.len(), rep.0.join("\n"));
}

#[test]
fn surface_elevation_and_state_layers_are_ordered() {
    let lum = relative_luminance;
    for p in Polarity::ALL {
        let t = Theme::preset(p, AppBrand::Default);
        let s = t.surface;
        if p == Polarity::Light {
            // Fixed inverted light elevation: tinted canvas under crisp white surfaces.
            assert!(lum(s.canvas) < lum(s.base), "{p:?}: canvas must sit below base");
            assert_eq!(s.base, ColorRgba::WHITE);
            assert_eq!(s.raised, ColorRgba::WHITE);
            assert_eq!(s.overlay, ColorRgba::WHITE);
            assert!(lum(s.sunken) < lum(s.base));
            assert_ne!(t.elevation.overlay, "none");
        }
        if p == Polarity::Dark {
            assert_eq!(s.canvas, ColorRgba::rgb(0x09, 0x09, 0x0b), "graphite canvas");
            assert_eq!(s.raised, ColorRgba::rgb(0x18, 0x18, 0x1b), "graphite raised");
            assert!(lum(s.canvas) < lum(s.base) && lum(s.base) < lum(s.raised));
            assert!(lum(s.raised) < lum(s.overlay));
            assert!(t.border.subtle.a < 0xff && t.border.default.a < 0xff, "translucent hairlines");
        }
        if t.is_high_contrast() {
            assert_eq!(t.elevation.overlay, "none", "{p:?}: high contrast uses borders, not shadows");
        }
        for bg in [s.base, s.raised] {
            let (base_y, hover_y, active_y) = (lum(bg), lum(composite(t.state.hover, bg)), lum(composite(t.state.active, bg)));
            assert!((hover_y - base_y).abs() > 1e-3, "{p:?}: hover must be visible");
            assert!((active_y - base_y).abs() > (hover_y - base_y).abs(), "{p:?}: active stronger than hover");
        }
        assert!(t.scrim.a > 0x40);
        let n = Scale::neutral(p);
        assert_eq!(n.step::<12>(), t.text.primary);
    }
}

/// Checks an accent's every accent-bound role in all four polarities.
fn check_accent(rep: &mut Report, name: &str, accent: Accent) {
    for p in Polarity::ALL {
        let t = Theme::preset(p, AppBrand::Default).with_accent(accent);
        let bar = if t.is_high_contrast() { &HIGH } else { &STANDARD };
        let tone = t.status.accent;
        rep.text(&format!("{name} {p:?} on_solid/solid"), tone.on_solid, tone.solid, bar.readable);
        rep.text(&format!("{name} {p:?} text/subtle"), tone.text, tone.subtle, bar.readable);
        for (sn, bg) in [("canvas", t.surface.canvas), ("base", t.surface.base), ("overlay", t.surface.overlay)] {
            rep.text(&format!("{name} {p:?} link/{sn}"), t.text.link, bg, bar.readable);
            rep.non_text(&format!("{name} {p:?} focus/{sn}"), t.border.focus, bg, bar.non_text);
        }
    }
}

/// Any runtime brand color (`Accent::from_srgb`) yields a contrast-safe accent: real Tailwind-500
/// brand colors, sRGB primaries, and a full hue × chroma × lightness sweep.
#[test]
fn every_runtime_accent_is_contrast_safe_in_every_polarity() {
    let mut rep = Report(Vec::new());
    let srgb = [
        0xef4444, 0xf97316, 0xf59e0b, 0xeab308, 0x84cc16, 0x22c55e, 0x10b981, 0x14b8a6, 0x06b6d4,
        0x0ea5e9, 0x3b82f6, 0x6366f1, 0x8b5cf6, 0xa855f7, 0xd946ef, 0xec4899, 0xf43f5e, 0xff00ff,
        0x00ff00, 0x0000ff, 0xffff00, 0xff0000, 0x000000, 0xffffff, 0x808080,
    ];
    for hex in srgb {
        let c = ColorRgba::rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
        check_accent(&mut rep, &format!("#{hex:06x}"), Accent::from_srgb(c));
    }
    for hue in (0..360).step_by(15) {
        for chroma in [0.0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.37] {
            for li in 0..=14 {
                let l = 0.25 + li as f32 * 0.05;
                check_accent(&mut rep, &format!("oklch({l:.2} {chroma} {hue})"), Accent::new(hue as f32, chroma, l));
            }
        }
    }
    assert!(rep.0.is_empty(), "{} accent violations:\n{}", rep.0.len(), rep.0.join("\n"));
}

/// Non-finite and out-of-range inputs never panic, poison the encode, or collapse saturated
/// colors to gray.
#[test]
fn oklch_and_accent_inputs_are_sanitized() {
    let saturated = Oklch::new(0.63, 0.4, 29.0).to_srgb();
    for c in [0.4, 100.0, 1e6, f32::INFINITY] {
        assert_eq!(Oklch::new(0.63, c, 29.0).to_srgb(), saturated, "chroma {c} caps to the gamut edge");
    }
    assert!(saturated.r > 200 && saturated.g < 80, "saturated red stays red: {saturated:?}");
    let gray = Oklch::new(0.63, 0.0, 29.0).to_srgb();
    assert_eq!(Oklch::new(0.63, f32::NAN, 29.0).to_srgb(), gray);
    assert_eq!(Oklch::new(f32::NAN, 0.1, 29.0).to_srgb(), ColorRgba::BLACK);
    assert_eq!(Oklch::new(0.63, 0.1, f32::NAN).to_srgb(), Oklch::new(0.63, 0.1, 0.0).to_srgb());
    assert_eq!(Oklch::new(2.0, 0.1, 29.0).to_srgb(), ColorRgba::WHITE);

    for (h, c, l) in [(f32::NAN, f32::NAN, f32::NAN), (f32::INFINITY, -1.0, -3.0), (-30.0, 9.0, 7.0)] {
        let a = Accent::new(h, c, l);
        assert!(a.hue.is_finite() && (0.0..360.0).contains(&a.hue), "{a:?}");
        assert!((0.0..=0.4).contains(&a.chroma) && (0.0..=1.0).contains(&a.solid_l), "{a:?}");
        let mut rep = Report(Vec::new());
        check_accent(&mut rep, &format!("{a:?}"), a);
        assert!(rep.0.is_empty(), "{}", rep.0.join("\n"));
    }
    assert_eq!(Accent::new(-30.0, 0.1, 0.5).hue, 330.0);
}

#[test]
fn presets_are_compile_time_tables_and_brands_share_one_base_scale() {
    for p in Polarity::ALL {
        let docs = Theme::preset(p, AppBrand::Docs);
        let studio = Theme::preset(p, AppBrand::Studio);
        assert_eq!(docs, Theme::build(p, AppBrand::Docs));
        assert_eq!(docs.surface, studio.surface, "{p:?}: shared neutral base scale");
        assert_eq!(docs.text.primary, studio.text.primary);
        assert_ne!(docs.status.accent.solid, studio.status.accent.solid, "{p:?}: distinct brand accents");
        let violet = studio.with_accent(Accent::VIOLET);
        assert_ne!(violet.border.focus, studio.border.focus);
        assert_eq!(violet.text.link, violet.status.accent.text);
        assert_eq!(violet.with_accent(Accent::CYAN), studio);
    }
}

#[test]
fn theme_config_follows_os_by_default_and_honors_explicit_overrides() {
    let docs = ThemeConfig::new(AppBrand::Docs);
    assert_eq!(docs.mode, ThemeMode::System);
    assert!(docs.resolve(Appearance::DARK).is_dark());
    assert!(!docs.resolve(Appearance::LIGHT).is_dark());
    let os_hc = Appearance { scheme: ColorScheme::Dark, contrast: ContrastLevel::High };
    assert_eq!(docs.resolve(os_hc).polarity, Polarity::HighContrastDark);

    let forced = ThemeConfig { mode: ThemeMode::Light, contrast: Some(ContrastLevel::Standard), ..docs };
    assert_eq!(forced.resolve(os_hc).polarity, Polarity::Light);

    let studio = ThemeConfig {
        accent: Some(Accent::AMBER),
        density: UiDensity::Compact,
        ..ThemeConfig::new(AppBrand::Studio)
    };
    let t = studio.resolve(Appearance::DARK);
    assert_eq!(t.status.accent, Theme::preset(Polarity::Dark, AppBrand::Studio).with_accent(Accent::AMBER).status.accent);
    assert!(t.spacing.md < Theme::DARK.spacing.md);
}

#[test]
fn layout_tokens_are_consistent() {
    assert!(Layer::Popover.z() < Layer::Modal.z() && Layer::Modal.z() < Layer::Tooltip.z());
    assert_eq!(Breakpoint::from_width(599.0), Breakpoint::Compact);
    assert_eq!(Breakpoint::from_width(600.0), Breakpoint::Medium);
    assert_eq!(Breakpoint::from_width(1024.0), Breakpoint::Expanded);
    assert_eq!(Breakpoint::Expanded.min_width(), 1024.0);
    let ty = Theme::LIGHT.typography;
    assert_eq!((ty.docs_body.size, ty.docs_body.line_height), (16.0, 1.65));
    assert_eq!((ty.ui.size, ty.label.size), (13.0, 12.0));
    assert_eq!([ty.display_lg.size, ty.display_md.size, ty.display_sm.size], [48.0, 36.0, 28.0]);
    assert!(ty.display_lg.tracking < 0.0);
    let m = Theme::LIGHT.motion;
    assert!((110..=150).contains(&m.fast_ms) && (110..=150).contains(&m.base_ms));
    assert_eq!(m.reduced().base_ms, 0);
}
