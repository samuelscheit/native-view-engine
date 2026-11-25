//! Physical-pixel clip stack with interior-span fast path for rounded-rect clips.

use nve_engine::LayoutRect;

/// Signed distance to a rounded rectangle centered at `(cx, cy)` with half-size `(hx, hy)`
/// and corner radius `r`. Negative inside, positive outside.
///
/// Uses an interior/edge branch so pixels outside the four `r x r` corner quadrants avoid `sqrt`.
#[inline]
pub(crate) fn rounded_rect_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    if qx > 0.0 && qy > 0.0 {
        (qx * qx + qy * qy).sqrt() - r
    } else {
        qx.max(qy) - r
    }
}

/// Active physical-pixel clip stack supporting nested rectangular and rounded-rect clips.
#[derive(Debug, Clone)]
pub(crate) struct ClipStack {
    buf_w: i32,
    buf_h: i32,
    entries: Vec<ClipEntry>,
    bounds: [i32; 4], // [x0, y0, x1, y1]
    rounded_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct ClipEntry {
    cx: f32,
    cy: f32,
    hx: f32,
    hy: f32,
    radius: f32,
    /// Precomputed `hx - radius` and `hy - radius` for fast inner-inset rejection.
    inner_hx: f32,
    inner_hy: f32,
    bounds: [i32; 4],
}

impl ClipStack {
    pub(crate) fn new(buf_w: u32, buf_h: u32) -> Self {
        let w = buf_w as i32;
        let h = buf_h as i32;
        Self {
            buf_w: w,
            buf_h: h,
            entries: Vec::with_capacity(8),
            bounds: [0, 0, w, h],
            rounded_count: 0,
        }
    }

    pub(crate) fn push(&mut self, rect: LayoutRect, radius: f32) {
        let x0 = (rect.x.floor() as i32)
            .max(self.bounds[0])
            .clamp(0, self.buf_w);
        let y0 = (rect.y.floor() as i32)
            .max(self.bounds[1])
            .clamp(0, self.buf_h);
        let x1 = ((rect.x + rect.width).ceil() as i32)
            .min(self.bounds[2])
            .clamp(0, self.buf_w);
        let y1 = ((rect.y + rect.height).ceil() as i32)
            .min(self.bounds[3])
            .clamp(0, self.buf_h);
        let hx = (rect.width * 0.5).max(0.0);
        let hy = (rect.height * 0.5).max(0.0);
        let r = radius.clamp(0.0, hx.min(hy));
        if r > 0.01 {
            self.rounded_count += 1;
        }
        let next_bounds = if x0 < x1 && y0 < y1 {
            [x0, y0, x1, y1]
        } else {
            [0, 0, 0, 0]
        };
        self.entries.push(ClipEntry {
            cx: rect.x + hx,
            cy: rect.y + hy,
            hx,
            hy,
            radius: r,
            inner_hx: hx - r,
            inner_hy: hy - r,
            bounds: next_bounds,
        });
        self.bounds = next_bounds;
    }

    pub(crate) fn pop(&mut self) {
        if let Some(popped) = self.entries.pop() {
            if popped.radius > 0.01 {
                self.rounded_count = self.rounded_count.saturating_sub(1);
            }
            self.bounds = self
                .entries
                .last()
                .map_or([0, 0, self.buf_w, self.buf_h], |e| e.bounds);
        }
    }

    #[inline]
    pub(crate) fn bounds(&self) -> [i32; 4] {
        self.bounds
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.bounds[0] >= self.bounds[2] || self.bounds[1] >= self.bounds[3]
    }

    #[inline]
    pub(crate) fn has_rounded(&self) -> bool {
        self.rounded_count > 0
    }

    /// Evaluates exact antialiased rounded-clip coverage in `0.0..=1.0` at `(fx, fy)`.
    ///
    /// Pixels strictly inside the inner corner inset (or outside the outer `+0.5px` envelope)
    /// bypass `sqrt` completely.
    #[inline]
    pub(crate) fn rounded_coverage(&self, fx: f32, fy: f32) -> f32 {
        if self.rounded_count == 0 {
            return 1.0;
        }
        let mut cov = 1.0f32;
        for entry in &self.entries {
            if entry.radius <= 0.01 {
                continue;
            }
            let ax = (fx - entry.cx).abs();
            let ay = (fy - entry.cy).abs();
            // Outside the +0.5px AA envelope: coverage is identically 0.0 (no sqrt).
            if ax >= entry.hx + 0.5 || ay >= entry.hy + 0.5 {
                return 0.0;
            }
            let qx = ax - entry.inner_hx;
            let qy = ay - entry.inner_hy;
            // Strictly inside the inner corner inset or straight interior band: coverage == 1.0 (no sqrt).
            if (qx <= 0.0 || qy <= 0.0) && ax <= entry.hx - 0.5 && ay <= entry.hy - 0.5 {
                continue;
            }
            let sdf = if qx > 0.0 && qy > 0.0 {
                let r_in = (entry.radius - 0.5).max(0.0);
                let dist_sq = qx * qx + qy * qy;
                if entry.radius >= 0.5 && dist_sq <= r_in * r_in {
                    continue;
                }
                let r_out = entry.radius + 0.5;
                if dist_sq >= r_out * r_out {
                    return 0.0;
                }
                dist_sq.sqrt() - entry.radius
            } else {
                qx.max(qy) - entry.radius
            };
            let c = (0.5 - sdf).clamp(0.0, 1.0);
            if c <= 0.0 {
                return 0.0;
            }
            cov = cov.min(c);
        }
        cov
    }

    /// Returns a guaranteed-coverage-`1.0` horizontal span `[span_l, span_r)` inside `[min_x, max_x)`
    /// for scanline `py` across all active rounded clips, enabling fast `blend_span` fills with
    /// zero per-pixel `sqrt` inside the interior of rounded containers.
    #[inline]
    pub(crate) fn interior_span_for_row(&self, py: i32, min_x: i32, max_x: i32) -> (i32, i32) {
        if min_x >= max_x {
            return (max_x, max_x);
        }
        if self.rounded_count == 0 {
            return (min_x, max_x);
        }
        let fy = py as f32 + 0.5;
        let mut l = min_x;
        let mut r = max_x;
        for entry in &self.entries {
            if entry.radius <= 0.01 {
                continue;
            }
            let ay = (fy - entry.cy).abs();
            if ay > entry.hy - 0.5 {
                return (max_x, max_x);
            }
            let raw_half_x = if ay <= entry.inner_hy {
                entry.hx - 0.5
            } else {
                entry.inner_hx.min(entry.hx - 0.5)
            };
            // Subtract a tiny epsilon to protect against float boundary rounding at non-integer rects.
            let safe_half_x = raw_half_x - 1e-4;
            if safe_half_x <= 0.0 {
                return (max_x, max_x);
            }
            let entry_l = (entry.cx - safe_half_x - 0.5).ceil() as i32;
            let entry_r = (entry.cx + safe_half_x - 0.5).floor() as i32 + 1;
            l = l.max(entry_l);
            r = r.min(entry_r);
            if l >= r {
                return (max_x, max_x);
            }
        }
        (l, r)
    }
}
