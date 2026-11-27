//! CSS/HTML serialization of core value types. Core crates stay format-free; every keyword,
//! track list, color, gradient, shadow, and transform string is produced here.

use nve_abi::num::{write_f32, write_i64, write_u64};
use nve_abi::ColorRgba;
use nve_tables::interaction::CursorKind;
use nve_tables::layout_ext::{
    GridLine, GridSingleTrack, GridTrack, GridTrackPlacement, GridTrackUnit, LayoutDirection,
    PositionMode,
};
use nve_tables::paint::{normalize_stops, BackgroundFill, GradientStop};
use nve_tables::tables::{BackgroundFillView, BoxShadow, GridTracksView, ImageFit, InputKind};
use nve_tables::transform::Transform2D;

/// Canonical CSS / HTML keyword of a core enum (`cursor`, `object-fit`, `position`, `dir`,
/// `<input type>`).
pub trait Keyword: Copy {
    fn keyword(self) -> &'static str;
}

macro_rules! css_keywords {
    ($($ty:ident { $($variant:ident => $kw:literal),+ $(,)? })+) => {
        $(impl Keyword for $ty {
            #[inline]
            fn keyword(self) -> &'static str {
                match self {
                    $($ty::$variant => $kw),+
                }
            }
        })+
    };
}

css_keywords! {
    CursorKind {
        Auto => "auto", Default => "default", Pointer => "pointer", Text => "text",
        Grab => "grab", Grabbing => "grabbing", NotAllowed => "not-allowed",
        ColResize => "col-resize", RowResize => "row-resize", Move => "move",
        Crosshair => "crosshair", Progress => "progress", Wait => "wait",
    }
    ImageFit {
        Cover => "cover", Contain => "contain", Fill => "fill", None => "none",
        ScaleDown => "scale-down",
    }
    PositionMode { Relative => "relative", Absolute => "absolute", Sticky => "sticky" }
    LayoutDirection { Ltr => "ltr", Rtl => "rtl" }
    InputKind {
        Text => "text", Email => "email", Password => "password", Tel => "tel", Url => "url",
        Number => "number", Search => "search", Date => "date", Time => "time",
        Hidden => "hidden",
    }
}

/// Appends the canonical CSS value of `self`.
pub trait WriteCss {
    fn write_css(&self, out: &mut String);
}

#[inline]
fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// `<v>px`, or bare `0`.
pub fn write_px(v: f32, out: &mut String) {
    let start = out.len();
    write_f32(out, finite(v));
    if &out[start..] != "0" {
        out.push_str("px");
    }
}

fn write_pct(v: f32, out: &mut String) {
    let start = out.len();
    write_f32(out, finite(v));
    if &out[start..] != "0" {
        out.push('%');
    }
}

fn write_fr(v: f32, out: &mut String) {
    write_f32(out, finite(v).max(0.0));
    out.push_str("fr");
}

fn write_minmax(min: GridTrackUnit, max: GridTrackUnit, out: &mut String) {
    out.push_str("minmax(");
    min.write_css(out);
    out.push(',');
    max.write_css(out);
    out.push(')');
}

impl WriteCss for GridTrackUnit {
    fn write_css(&self, out: &mut String) {
        match *self {
            Self::Auto => out.push_str("auto"),
            Self::Px(v) => write_px(v.max(0.0), out),
            Self::Percent(v) => write_pct(v.max(0.0), out),
            Self::Fr(v) => write_fr(v, out),
            Self::MinContent => out.push_str("min-content"),
            Self::MaxContent => out.push_str("max-content"),
        }
    }
}

impl WriteCss for GridSingleTrack {
    fn write_css(&self, out: &mut String) {
        match *self {
            Self::Auto => out.push_str("auto"),
            Self::Px(v) => write_px(v.max(0.0), out),
            Self::Percent(v) => write_pct(v.max(0.0), out),
            Self::Fr(v) => write_fr(v, out),
            Self::MinMax(min, max) => write_minmax(min, max, out),
        }
    }
}

impl WriteCss for GridTrack {
    fn write_css(&self, out: &mut String) {
        match *self {
            Self::Auto => out.push_str("auto"),
            Self::Px(v) => write_px(v.max(0.0), out),
            Self::Percent(v) => write_pct(v.max(0.0), out),
            Self::Fr(v) => write_fr(v, out),
            Self::MinMax(min, max) => write_minmax(min, max, out),
            Self::Repeat(count, track) => {
                out.push_str("repeat(");
                write_u64(out, u64::from(count.max(1)));
                out.push(',');
                track.write_css(out);
                out.push(')');
            }
        }
    }
}

/// Space-separated `grid-template-*` track list.
pub fn write_grid_tracks(tracks: impl IntoIterator<Item = GridTrack>, out: &mut String) {
    for (i, track) in tracks.into_iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        track.write_css(out);
    }
}

impl WriteCss for GridTracksView<'_> {
    fn write_css(&self, out: &mut String) {
        write_grid_tracks(self.iter(), out);
    }
}

impl WriteCss for GridLine {
    fn write_css(&self, out: &mut String) {
        match *self {
            Self::Auto => out.push_str("auto"),
            Self::Line(n) => write_i64(out, i64::from(n)),
            Self::Span(n) => {
                out.push_str("span ");
                write_u64(out, u64::from(n.max(1)));
            }
        }
    }
}

impl WriteCss for GridTrackPlacement {
    fn write_css(&self, out: &mut String) {
        match (self.start, self.end) {
            (GridLine::Auto, GridLine::Auto) => out.push_str("auto"),
            (start, GridLine::Auto) => start.write_css(out),
            (start, end) => {
                start.write_css(out);
                out.push('/');
                end.write_css(out);
            }
        }
    }
}

/// Compact, injection-free CSS color: `#rrggbb`, `transparent`, or `rgba(r,g,b,a)`.
impl WriteCss for ColorRgba {
    fn write_css(&self, out: &mut String) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let c = *self;
        if c.a == 255 {
            out.push('#');
            for ch in [c.r, c.g, c.b] {
                out.push(HEX[usize::from(ch >> 4)] as char);
                out.push(HEX[usize::from(ch & 15)] as char);
            }
        } else if c.a == 0 {
            out.push_str("transparent");
        } else {
            out.push_str("rgba(");
            for ch in [c.r, c.g, c.b] {
                write_u64(out, u64::from(ch));
                out.push(',');
            }
            write_f32(out, f32::from(c.a) / 255.0);
            out.push(')');
        }
    }
}

/// `<color> <offset>%` stop list (leading commas); sorts unnormalized lists, pads 0/1 stops.
fn write_stops<I>(stops: I, out: &mut String)
where
    I: ExactSizeIterator<Item = GradientStop> + Clone,
{
    let write_one = |s: GradientStop, out: &mut String| {
        out.push(',');
        s.color.write_css(out);
        out.push(' ');
        write_f32(out, s.offset.clamp(0.0, 1.0) * 100.0);
        out.push('%');
    };
    match stops.len() {
        0 => out.push_str(",transparent 0%,transparent 100%"),
        1 => {
            if let Some(s) = stops.clone().next() {
                write_one(GradientStop { offset: 0.0, ..s }, out);
                write_one(GradientStop { offset: 1.0, ..s }, out);
            }
        }
        _ => {
            let mut prev = 0.0f32;
            let sorted = stops.clone().all(|s| {
                let ok = s.offset.is_finite() && (prev..=1.0).contains(&s.offset);
                prev = s.offset;
                ok
            });
            if sorted {
                stops.for_each(|s| write_one(s, out));
            } else {
                normalize_stops(stops.collect()).into_iter().for_each(|s| write_one(s, out));
            }
        }
    }
}

fn write_linear<I>(angle_deg: f32, stops: I, out: &mut String)
where
    I: ExactSizeIterator<Item = GradientStop> + Clone,
{
    out.push_str("linear-gradient(");
    write_f32(out, if angle_deg.is_finite() { angle_deg } else { 180.0 });
    out.push_str("deg");
    write_stops(stops, out);
    out.push(')');
}

fn write_radial<I>(center: (f32, f32), radius: f32, stops: I, out: &mut String)
where
    I: ExactSizeIterator<Item = GradientStop> + Clone,
{
    let unit_pct = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) * 100.0 } else { 50.0 };
    let r_pct = if radius.is_finite() { radius.max(0.0) * 50.0 } else { 50.0 };
    out.push_str("radial-gradient(");
    write_f32(out, r_pct);
    out.push_str("% ");
    write_f32(out, r_pct);
    out.push_str("% at ");
    write_f32(out, unit_pct(center.0));
    out.push_str("% ");
    write_f32(out, unit_pct(center.1));
    out.push('%');
    write_stops(stops, out);
    out.push(')');
}

/// Gradient (or solid color) CSS value of a background fill.
impl WriteCss for BackgroundFill {
    fn write_css(&self, out: &mut String) {
        match self {
            Self::Solid(c) => c.write_css(out),
            Self::Linear(lg) => write_linear(lg.angle_deg, lg.stops.iter().copied(), out),
            Self::Radial(rg) => write_radial(
                (rg.center_x, rg.center_y),
                rg.radius,
                rg.stops.iter().copied(),
                out,
            ),
        }
    }
}

impl WriteCss for BackgroundFillView<'_> {
    fn write_css(&self, out: &mut String) {
        match *self {
            Self::Solid(c) => c.write_css(out),
            Self::Linear { angle_deg, stops } => write_linear(angle_deg, stops.iter(), out),
            Self::Radial { center_x, center_y, radius, stops } => {
                write_radial((center_x, center_y), radius, stops.iter(), out);
            }
        }
    }
}

/// `background-*` declaration(s) of `fill`. With `clear_image_on_solid` (state/`@media`
/// overrides), a solid fill also resets any base gradient (`background-image:none`).
pub fn write_background_decl(fill: &impl BackgroundValue, clear_image_on_solid: bool, out: &mut String) {
    match fill.solid() {
        Some(c) if clear_image_on_solid => {
            out.push_str("background-image:none;background-color:");
            c.write_css(out);
            out.push(';');
        }
        Some(c) if c.a > 0 => {
            out.push_str("background-color:");
            c.write_css(out);
            out.push(';');
        }
        Some(_) => {}
        None => {
            out.push_str("background-image:");
            fill.write_css(out);
            out.push(';');
        }
    }
}

/// Owned or borrowed background fill.
pub trait BackgroundValue: WriteCss {
    fn solid(&self) -> Option<ColorRgba>;
}

impl BackgroundValue for BackgroundFill {
    fn solid(&self) -> Option<ColorRgba> {
        match self {
            Self::Solid(c) => Some(*c),
            _ => None,
        }
    }
}

impl BackgroundValue for BackgroundFillView<'_> {
    fn solid(&self) -> Option<ColorRgba> {
        match self {
            Self::Solid(c) => Some(*c),
            _ => None,
        }
    }
}

impl WriteCss for BoxShadow {
    fn write_css(&self, out: &mut String) {
        let px = |v: f32, out: &mut String| {
            if !v.is_finite() || v == 0.0 {
                out.push('0');
            } else {
                write_f32(out, v);
                out.push_str("px");
            }
        };
        if self.inset {
            out.push_str("inset ");
        }
        px(self.offset_x, out);
        out.push(' ');
        px(self.offset_y, out);
        if self.blur_radius != 0.0 || self.spread_radius != 0.0 {
            out.push(' ');
            px(self.blur_radius.max(0.0), out);
            if self.spread_radius != 0.0 {
                out.push(' ');
                px(self.spread_radius, out);
            }
        }
        out.push(' ');
        ColorRgba::from_u32(self.color.0).write_css(out);
    }
}

/// Comma-separated `box-shadow` value (`none` when empty).
pub fn write_box_shadows(shadows: impl IntoIterator<Item = BoxShadow>, out: &mut String) {
    let start = out.len();
    for (i, s) in shadows.into_iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        s.write_css(out);
    }
    if out.len() == start {
        out.push_str("none");
    }
}

/// Canonical `transform:` (and `transform-origin:`) declarations of `tf`. With
/// `emit_identity_as_none` (sparse overrides), identity emits `transform:none;` to reset a base.
pub fn write_transform_decls(tf: &Transform2D, emit_identity_as_none: bool, out: &mut String) {
    const EPS: f32 = 1e-6;
    let or = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
    let (tx, ty) = (or(tf.translate_x, 0.0), or(tf.translate_y, 0.0));
    let (sx, sy) = (or(tf.scale_x, 1.0), or(tf.scale_y, 1.0));
    let deg = or(tf.rotate_deg, 0.0);
    let has_t = tx.abs() > EPS || ty.abs() > EPS;
    let has_s = (sx - 1.0).abs() > EPS || (sy - 1.0).abs() > EPS;
    let has_r = deg.abs() > EPS;

    if has_t || has_s || has_r {
        out.push_str("transform:");
        let mut sep = "";
        if has_t {
            out.push_str("translate(");
            write_px(tx, out);
            out.push(',');
            write_px(ty, out);
            out.push(')');
            sep = " ";
        }
        if has_s {
            out.push_str(sep);
            out.push_str("scale(");
            write_f32(out, sx);
            if (sx - sy).abs() > EPS {
                out.push(',');
                write_f32(out, sy);
            }
            out.push(')');
            sep = " ";
        }
        if has_r {
            out.push_str(sep);
            out.push_str("rotate(");
            write_f32(out, deg);
            out.push_str("deg)");
        }
        out.push(';');
    } else if emit_identity_as_none {
        out.push_str("transform:none;");
    }

    if tf.has_custom_origin() {
        let pct = |v: f32| or(v, 0.5).clamp(0.0, 1.0) * 100.0;
        out.push_str("transform-origin:");
        write_f32(out, pct(tf.origin_x));
        out.push_str("% ");
        write_f32(out, pct(tf.origin_y));
        out.push_str("%;");
    }
}
