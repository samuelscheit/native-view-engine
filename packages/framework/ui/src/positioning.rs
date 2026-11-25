//! Floating UI-grade pure-Rust anchored overlay collision-flipping, viewport clamping,
//! auto-sizing, arrow center tracking, and submenu safe-polygon hover corridor solver.

const EPS: f32 = 1e-4;

/// Primary side relative to an anchor rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

impl Side {
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    #[inline]
    pub const fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Cross-axis alignment relative to an anchor rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alignment {
    Start,
    Center,
    End,
}

impl Alignment {
    #[inline]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Start => Self::End,
            Self::Center => Self::Center,
            Self::End => Self::Start,
        }
    }
}

/// All 12 anchored overlay placements (4 sides × 3 cross-axis alignments).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Placement {
    Top,
    TopStart,
    TopEnd,
    #[default]
    Bottom,
    BottomStart,
    BottomEnd,
    Left,
    LeftStart,
    LeftEnd,
    Right,
    RightStart,
    RightEnd,
}

impl Placement {
    /// All 12 placements in canonical order.
    pub const ALL: [Self; 12] = [
        Self::Top,
        Self::TopStart,
        Self::TopEnd,
        Self::Bottom,
        Self::BottomStart,
        Self::BottomEnd,
        Self::Left,
        Self::LeftStart,
        Self::LeftEnd,
        Self::Right,
        Self::RightStart,
        Self::RightEnd,
    ];

    #[inline]
    pub const fn side(self) -> Side {
        match self {
            Self::Top | Self::TopStart | Self::TopEnd => Side::Top,
            Self::Bottom | Self::BottomStart | Self::BottomEnd => Side::Bottom,
            Self::Left | Self::LeftStart | Self::LeftEnd => Side::Left,
            Self::Right | Self::RightStart | Self::RightEnd => Side::Right,
        }
    }

    #[inline]
    pub const fn alignment(self) -> Alignment {
        match self {
            Self::Top | Self::Bottom | Self::Left | Self::Right => Alignment::Center,
            Self::TopStart | Self::BottomStart | Self::LeftStart | Self::RightStart => {
                Alignment::Start
            }
            Self::TopEnd | Self::BottomEnd | Self::LeftEnd | Self::RightEnd => Alignment::End,
        }
    }

    #[inline]
    pub const fn from_side_align(side: Side, align: Alignment) -> Self {
        match (side, align) {
            (Side::Top, Alignment::Center) => Self::Top,
            (Side::Top, Alignment::Start) => Self::TopStart,
            (Side::Top, Alignment::End) => Self::TopEnd,
            (Side::Bottom, Alignment::Center) => Self::Bottom,
            (Side::Bottom, Alignment::Start) => Self::BottomStart,
            (Side::Bottom, Alignment::End) => Self::BottomEnd,
            (Side::Left, Alignment::Center) => Self::Left,
            (Side::Left, Alignment::Start) => Self::LeftStart,
            (Side::Left, Alignment::End) => Self::LeftEnd,
            (Side::Right, Alignment::Center) => Self::Right,
            (Side::Right, Alignment::Start) => Self::RightStart,
            (Side::Right, Alignment::End) => Self::RightEnd,
        }
    }

    /// Flips the primary side (`Bottom -> Top`, `RightStart -> LeftStart`) while keeping alignment.
    #[inline]
    pub const fn opposite(self) -> Self {
        Self::from_side_align(self.side().opposite(), self.alignment())
    }

    /// Flips cross-axis alignment (`Start <-> End`) while keeping the primary side.
    #[inline]
    pub const fn flip_alignment(self) -> Self {
        Self::from_side_align(self.side(), self.alignment().opposite())
    }

    /// Canonical kebab-case string representation (`"bottom-start"`, `"top"`, etc.).
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::TopStart => "top-start",
            Self::TopEnd => "top-end",
            Self::Bottom => "bottom",
            Self::BottomStart => "bottom-start",
            Self::BottomEnd => "bottom-end",
            Self::Left => "left",
            Self::LeftStart => "left-start",
            Self::LeftEnd => "left-end",
            Self::Right => "right",
            Self::RightStart => "right-start",
            Self::RightEnd => "right-end",
        }
    }
}

/// 2D point in logical viewport coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// Axis-aligned rectangle in logical viewport coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    #[inline]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    #[inline]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    #[inline]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    #[inline]
    pub fn center_x(self) -> f32 {
        self.x + self.width * 0.5
    }

    #[inline]
    pub fn center_y(self) -> f32 {
        self.y + self.height * 0.5
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
    }

    #[inline]
    pub fn contains_point(self, p: Point) -> bool {
        self.is_valid()
            && p.is_finite()
            && p.x >= self.x
            && p.x <= self.right()
            && p.y >= self.y
            && p.y <= self.bottom()
    }
}

/// Logical dimensions of an overlay panel.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    #[inline]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Logical viewport dimensions (`[0, width] × [0, height]`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    #[inline]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Configuration for [`compute_overlay_position`].
#[derive(Debug, Clone, PartialEq)]
pub struct PositionOptions {
    pub placement: Placement,
    pub offset: f32,
    pub viewport_padding: f32,
    pub flip: bool,
    pub shift: bool,
    pub auto_size: bool,
    pub fallback_placements: Vec<Placement>,
}

impl Default for PositionOptions {
    fn default() -> Self {
        Self {
            placement: Placement::Bottom,
            offset: 8.0,
            viewport_padding: 8.0,
            flip: true,
            shift: true,
            auto_size: false,
            fallback_placements: Vec::new(),
        }
    }
}

impl PositionOptions {
    pub fn new(placement: Placement) -> Self {
        Self { placement, ..Self::default() }
    }

    pub fn with_offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    pub fn with_viewport_padding(mut self, padding: f32) -> Self {
        self.viewport_padding = padding;
        self
    }

    pub fn with_flip(mut self, flip: bool) -> Self {
        self.flip = flip;
        self
    }

    pub fn with_shift(mut self, shift: bool) -> Self {
        self.shift = shift;
        self
    }

    pub fn with_auto_size(mut self, auto_size: bool) -> Self {
        self.auto_size = auto_size;
        self
    }

    pub fn with_fallbacks(mut self, fallbacks: Vec<Placement>) -> Self {
        self.fallback_placements = fallbacks;
        self
    }
}

/// Resolved overlay coordinates, placement, arrow offset, and size constraints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedPosition {
    pub x: f32,
    pub y: f32,
    pub placement: Placement,
    pub arrow_x: Option<f32>,
    pub arrow_y: Option<f32>,
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,
    pub flipped: bool,
    pub shifted: bool,
}

#[inline]
fn sanitize_non_neg(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

#[inline]
fn sanitize_finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

fn push_unique(vec: &mut Vec<Placement>, p: Placement) {
    if !vec.contains(&p) {
        vec.push(p);
    }
}

#[derive(Clone, Copy)]
struct SolverCtx {
    anchor: Rect,
    overlay: Size,
    offset: f32,
    vp_w: f32,
    vp_h: f32,
    pad_x: f32,
    pad_y: f32,
}

impl SolverCtx {
    fn raw_coords(self, size: Size, placement: Placement) -> (f32, f32) {
        let x = match placement.side() {
            Side::Left => self.anchor.x - self.offset - size.width,
            Side::Right => self.anchor.right() + self.offset,
            Side::Top | Side::Bottom => match placement.alignment() {
                Alignment::Start => self.anchor.x,
                Alignment::Center => self.anchor.x + (self.anchor.width - size.width) * 0.5,
                Alignment::End => self.anchor.right() - size.width,
            },
        };
        let y = match placement.side() {
            Side::Top => self.anchor.y - self.offset - size.height,
            Side::Bottom => self.anchor.bottom() + self.offset,
            Side::Left | Side::Right => match placement.alignment() {
                Alignment::Start => self.anchor.y,
                Alignment::Center => self.anchor.y + (self.anchor.height - size.height) * 0.5,
                Alignment::End => self.anchor.bottom() - size.height,
            },
        };
        (x, y)
    }

    fn available_main_space(self, side: Side) -> f32 {
        match side {
            Side::Top => (self.anchor.y - self.offset - self.pad_y).max(0.0),
            Side::Bottom => ((self.vp_h - self.pad_y) - (self.anchor.bottom() + self.offset)).max(0.0),
            Side::Left => (self.anchor.x - self.offset - self.pad_x).max(0.0),
            Side::Right => ((self.vp_w - self.pad_x) - (self.anchor.right() + self.offset)).max(0.0),
        }
    }

    fn overflows(self, placement: Placement) -> (f32, f32) {
        let (x, y) = self.raw_coords(self.overlay, placement);
        let left_ov = (self.pad_x - x).max(0.0);
        let right_ov = ((x + self.overlay.width) - (self.vp_w - self.pad_x)).max(0.0);
        let top_ov = (self.pad_y - y).max(0.0);
        let bottom_ov = ((y + self.overlay.height) - (self.vp_h - self.pad_y)).max(0.0);
        let main_ov = match placement.side() {
            Side::Top => top_ov,
            Side::Bottom => bottom_ov,
            Side::Left => left_ov,
            Side::Right => right_ov,
        };
        let cross_ov = if placement.side().is_vertical() {
            left_ov + right_ov
        } else {
            top_ov + bottom_ov
        };
        (main_ov, cross_ov)
    }

    fn select_placement(self, primary: &[Placement], cross: &[Placement], shift: bool) -> Placement {
        // 1. Exact fit in primary tier (normal fit, single-axis flip, or corner double-flip).
        for &c in primary {
            let (m, x) = self.overflows(c);
            if m <= EPS && x <= EPS {
                return c;
            }
        }
        // 2. Main-axis fit in primary tier when cross-axis shifting is enabled.
        if shift {
            for &c in primary {
                if self.overflows(c).0 <= EPS {
                    return c;
                }
            }
        }
        // 3. Cross-axis tier: exact fit, then main-axis fit when shifting is enabled.
        for &c in cross {
            let (m, x) = self.overflows(c);
            if m <= EPS && x <= EPS {
                return c;
            }
        }
        if shift {
            for &c in cross {
                if self.overflows(c).0 <= EPS {
                    return c;
                }
            }
        }
        // 4. Tiny viewport fallback: pick candidate with maximum available main-axis space.
        let mut best = primary[0];
        let mut best_space = self.available_main_space(best.side());
        let mut best_cross = self.overflows(best).1;
        for &c in primary.iter().chain(cross.iter()).skip(1) {
            let space = self.available_main_space(c.side());
            let cross_ov = self.overflows(c).1;
            if space > best_space + EPS
                || ((space - best_space).abs() <= EPS && cross_ov + EPS < best_cross)
            {
                best = c;
                best_space = space;
                best_cross = cross_ov;
            }
        }
        best
    }

    fn pick_placement(self, options: &PositionOptions) -> Placement {
        let req = options.placement;
        if !options.flip {
            return req;
        }
        if !options.fallback_placements.is_empty() {
            let mut candidates = Vec::with_capacity(1 + options.fallback_placements.len());
            candidates.push(req);
            for &fb in &options.fallback_placements {
                push_unique(&mut candidates, fb);
            }
            return self.select_placement(&candidates, &[], options.shift);
        }
        let mut primary = Vec::with_capacity(4);
        push_unique(&mut primary, req);
        push_unique(&mut primary, req.opposite());
        push_unique(&mut primary, req.flip_alignment());
        push_unique(&mut primary, req.opposite().flip_alignment());

        let cross_sides = if req.side().is_vertical() {
            [Side::Right, Side::Left]
        } else {
            [Side::Bottom, Side::Top]
        };
        let mut cross = Vec::with_capacity(4);
        for side in cross_sides {
            push_unique(&mut cross, Placement::from_side_align(side, req.alignment()));
            push_unique(
                &mut cross,
                Placement::from_side_align(side, req.alignment().opposite()),
            );
        }
        self.select_placement(&primary, &cross, options.shift)
    }
}

/// Computes the collision-flipped, viewport-clamped, optionally auto-sized position and arrow
/// offset for an overlay anchored to `anchor` within `viewport`.
pub fn compute_overlay_position(
    anchor: Rect,
    overlay: Size,
    viewport: Viewport,
    options: &PositionOptions,
) -> ComputedPosition {
    let vp_w = sanitize_non_neg(viewport.width);
    let vp_h = sanitize_non_neg(viewport.height);
    let raw_pad = sanitize_non_neg(options.viewport_padding);
    let ctx = SolverCtx {
        anchor: Rect::new(
            sanitize_finite(anchor.x),
            sanitize_finite(anchor.y),
            sanitize_non_neg(anchor.width),
            sanitize_non_neg(anchor.height),
        ),
        overlay: Size::new(
            sanitize_non_neg(overlay.width),
            sanitize_non_neg(overlay.height),
        ),
        offset: sanitize_finite(options.offset),
        vp_w,
        vp_h,
        pad_x: raw_pad.min(vp_w * 0.5),
        pad_y: raw_pad.min(vp_h * 0.5),
    };

    let chosen = ctx.pick_placement(options);
    let flipped = chosen != options.placement;

    let (max_width, max_height, eff_size) = if options.auto_size {
        let avail_main = ctx.available_main_space(chosen.side());
        let avail_w = if chosen.side().is_vertical() {
            (ctx.vp_w - 2.0 * ctx.pad_x).max(0.0)
        } else {
            avail_main
        };
        let avail_h = if chosen.side().is_vertical() {
            avail_main
        } else {
            (ctx.vp_h - 2.0 * ctx.pad_y).max(0.0)
        };
        (
            Some(avail_w),
            Some(avail_h),
            Size::new(ctx.overlay.width.min(avail_w), ctx.overlay.height.min(avail_h)),
        )
    } else {
        (None, None, ctx.overlay)
    };

    let (mut x, mut y) = ctx.raw_coords(eff_size, chosen);
    let mut shifted = false;

    if options.shift {
        if chosen.side().is_vertical() {
            let max_x = (ctx.vp_w - ctx.pad_x - eff_size.width).max(ctx.pad_x);
            let clamped = x.clamp(ctx.pad_x, max_x);
            if (clamped - x).abs() > EPS {
                x = clamped;
                shifted = true;
            }
        } else {
            let max_y = (ctx.vp_h - ctx.pad_y - eff_size.height).max(ctx.pad_y);
            let clamped = y.clamp(ctx.pad_y, max_y);
            if (clamped - y).abs() > EPS {
                y = clamped;
                shifted = true;
            }
        }
    }

    let (arrow_x, arrow_y) = if chosen.side().is_vertical() {
        (Some((ctx.anchor.center_x() - x).clamp(0.0, eff_size.width)), None)
    } else {
        (None, Some((ctx.anchor.center_y() - y).clamp(0.0, eff_size.height)))
    };

    ComputedPosition {
        x,
        y,
        placement: chosen,
        arrow_x,
        arrow_y,
        max_width,
        max_height,
        flipped,
        shifted,
    }
}

#[inline]
fn cross_2d(a: Point, b: Point, p: Point) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

#[inline]
fn point_in_triangle(p: Point, a: Point, b: Point, c: Point) -> bool {
    let (d1, d2, d3) = (cross_2d(a, b, p), cross_2d(b, c, p), cross_2d(c, a, p));
    !((d1 < -EPS || d2 < -EPS || d3 < -EPS) && (d1 > EPS || d2 > EPS || d3 > EPS))
}

/// Safe-triangle hover corridor test for cascading submenus.
///
/// Returns `true` when `cursor` lies inside `submenu_rect` or within the safe triangle formed by
/// `prev_cursor` and the two extreme silhouette corners of `submenu_rect` as seen from
/// `prev_cursor`, allowing diagonal mouse movement toward a submenu without dismissing it.
pub fn point_in_safe_polygon(cursor: Point, prev_cursor: Point, submenu_rect: Rect) -> bool {
    if !submenu_rect.is_valid() || !cursor.is_finite() || !prev_cursor.is_finite() {
        return false;
    }
    if submenu_rect.contains_point(cursor) {
        return true;
    }
    if submenu_rect.contains_point(prev_cursor) {
        return false;
    }

    let tl = Point::new(submenu_rect.x, submenu_rect.y);
    let tr = Point::new(submenu_rect.right(), submenu_rect.y);
    let br = Point::new(submenu_rect.right(), submenu_rect.bottom());
    let bl = Point::new(submenu_rect.x, submenu_rect.bottom());

    let (c1, c2) = if prev_cursor.x < submenu_rect.x {
        if prev_cursor.y < submenu_rect.y {
            (tr, bl)
        } else if prev_cursor.y > submenu_rect.bottom() {
            (tl, br)
        } else {
            (tl, bl)
        }
    } else if prev_cursor.x > submenu_rect.right() {
        if prev_cursor.y < submenu_rect.y {
            (tl, br)
        } else if prev_cursor.y > submenu_rect.bottom() {
            (tr, bl)
        } else {
            (tr, br)
        }
    } else if prev_cursor.y < submenu_rect.y {
        (tl, tr)
    } else {
        (bl, br)
    };

    point_in_triangle(cursor, prev_cursor, c1, c2)
}
