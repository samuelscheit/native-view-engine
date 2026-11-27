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
    Breakpoint, Layer, Polarity, Scale, Theme, ThemeConfig, ThemeMode,
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
        assert_eq!(n.step(12), t.text.primary);
    }
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
