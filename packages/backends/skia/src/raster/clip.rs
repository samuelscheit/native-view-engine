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

/// Signed distance and outward unit normal `(sdf, nx, ny)` to a rounded rectangle centered at
/// `(cx, cy)` with half-size `(hx, hy)` and corner radius `r`. Used by transform-aware software
/// shadows to normalize local SDF distances into exact device pixels under non-uniform scales.
#[inline]
pub(crate) fn rounded_rect_sdf_with_normal(
    px: f32,
    py: f32,
    cx: f32,
    cy: f32,
    hx: f32,
    hy: f32,
    r: f32,
) -> (f32, f32, f32) {
    let dx = px - cx;
    let dy = py - cy;
    let sx = if dx >= 0.0 { 1.0 } else { -1.0 };
    let sy = if dy >= 0.0 { 1.0 } else { -1.0 };
    let qx = dx.abs() - (hx - r);
    let qy = dy.abs() - (hy - r);
    if qx > 0.0 && qy > 0.0 {
        let len = (qx * qx + qy * qy).sqrt();
        // At the exact corner-arc center (`len <= 1e-6`) or overflow (`!len.is_finite()`), emit
        // a zero normal so `device_sdf_from_local` scales the fallback distance by `m_scale`.
        let inv = if len > 1e-6 && len.is_finite() {
            1.0 / len
        } else {
            0.0
        };
        (len - r, sx * qx * inv, sy * qy * inv)
    } else if qx > qy {
        (qx - r, sx, 0.0)
    } else {
        (qy - r, 0.0, sy)
    }
}

const INLINE_CLIP_DEPTH: usize = 16;

/// Active physical-pixel clip stack supporting nested rectangular and rounded-rect clips.
/// Uses an inline stack up to depth 16 for zero heap allocations on typical UI frames.
#[derive(Debug, Clone)]
pub(crate) struct ClipStack {
    buf_w: i32,
    buf_h: i32,
    inline: [ClipEntry; INLINE_CLIP_DEPTH],
    len: usize,
    overflow: Vec<ClipEntry>,
    bounds: [i32; 4], // [x0, y0, x1, y1]
    rounded_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
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
            inline: [ClipEntry::default(); INLINE_CLIP_DEPTH],
            len: 0,
            overflow: Vec::new(),
            bounds: [0, 0, w, h],
            rounded_count: 0,
        }
    }

    #[inline]
    fn inline_slice(&self) -> &[ClipEntry] {
        &self.inline[..self.len.min(INLINE_CLIP_DEPTH)]
    }

    #[inline]
    fn last_entry(&self) -> Option<&ClipEntry> {
        if self.len == 0 {
            None
        } else if self.len <= INLINE_CLIP_DEPTH {
            Some(&self.inline[self.len - 1])
        } else {
            self.overflow.last()
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
        let entry = ClipEntry {
            cx: rect.x + hx,
            cy: rect.y + hy,
            hx,
            hy,
            radius: r,
            inner_hx: hx - r,
            inner_hy: hy - r,
            bounds: next_bounds,
        };
        if self.len < INLINE_CLIP_DEPTH {
            self.inline[self.len] = entry;
        } else {
            self.overflow.push(entry);
        }
        self.len += 1;
        self.bounds = next_bounds;
    }

    pub(crate) fn pop(&mut self) {
        if self.len == 0 {
            return;
        }
        self.len -= 1;
        let popped = if self.len < INLINE_CLIP_DEPTH {
            Some(self.inline[self.len])
        } else {
            self.overflow.pop()
        };
        if let Some(popped) = popped {
            if popped.radius > 0.01 {
                self.rounded_count = self.rounded_count.saturating_sub(1);
            }
            self.bounds = self
                .last_entry()
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
        for entry in self.inline_slice().iter().chain(self.overflow.iter()) {
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
        for entry in self.inline_slice().iter().chain(self.overflow.iter()) {
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
