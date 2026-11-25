use nve_abi::hash::HashMap;

use nve_abi::SurfaceId;
use serde::{Deserialize, Serialize};

use crate::error::InteropError;

/// C-compatible box constraints negotiated across Flutter `RenderBox`, React Native Yoga,
/// SwiftUI `ProposedViewSize`/`sizeThatFits`, and Jetpack Compose `MeasurePolicy`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SizeConstraints {
    pub min_w: f32,
    pub min_h: f32,
    pub max_w: f32,
    pub max_h: f32,
}

impl SizeConstraints {
    pub const UNBOUNDED: Self = Self {
        min_w: 0.0,
        min_h: 0.0,
        max_w: f32::INFINITY,
        max_h: f32::INFINITY,
    };

    pub const fn new(min_w: f32, min_h: f32, max_w: f32, max_h: f32) -> Self {
        Self {
            min_w,
            min_h,
            max_w,
            max_h,
        }
    }

    pub const fn tight(width: f32, height: f32) -> Self {
        Self {
            min_w: width,
            min_h: height,
            max_w: width,
            max_h: height,
        }
    }

    pub const fn loose(max_w: f32, max_h: f32) -> Self {
        Self {
            min_w: 0.0,
            min_h: 0.0,
            max_w,
            max_h,
        }
    }

    pub fn validate(self) -> Result<Self, InteropError> {
        if self.min_w.is_nan()
            || self.min_h.is_nan()
            || self.max_w.is_nan()
            || self.max_h.is_nan()
        {
            return Err(InteropError::InvalidConstraints("constraints contain NaN".into()));
        }
        if self.min_w < 0.0 || self.min_h < 0.0 || self.max_w < 0.0 || self.max_h < 0.0 {
            return Err(InteropError::InvalidConstraints(
                "constraints must be non-negative".into(),
            ));
        }
        if self.min_w > self.max_w {
            return Err(InteropError::InvalidConstraints(format!(
                "min_w ({}) exceeds max_w ({})",
                self.min_w, self.max_w
            )));
        }
        if self.min_h > self.max_h {
            return Err(InteropError::InvalidConstraints(format!(
                "min_h ({}) exceeds max_h ({})",
                self.min_h, self.max_h
            )));
        }
        Ok(self)
    }

    pub fn has_bounded_width(self) -> bool {
        self.max_w.is_finite()
    }

    pub fn has_bounded_height(self) -> bool {
        self.max_h.is_finite()
    }

    pub fn is_tight(self) -> bool {
        self.min_w == self.max_w && self.min_h == self.max_h
    }

    /// Clamps a measured size strictly within `[min, max]` bounds.
    pub fn clamp(self, size: IntrinsicSize) -> IntrinsicSize {
        let w = if size.width.is_finite() {
            size.width.clamp(self.min_w, self.max_w)
        } else {
            self.min_w
        };
        let h = if size.height.is_finite() {
            size.height.clamp(self.min_h, self.max_h)
        } else {
            self.min_h
        };
        IntrinsicSize {
            width: w,
            height: h,
        }
    }

    /// Shrinks constraints by horizontal/vertical insets without underflowing below zero.
    pub fn deflate(self, horizontal: f32, vertical: f32) -> Self {
        let dh = horizontal.max(0.0);
        let dv = vertical.max(0.0);
        let min_w = (self.min_w - dh).max(0.0);
        let min_h = (self.min_h - dv).max(0.0);
        let max_w = if self.max_w.is_finite() {
            (self.max_w - dh).max(min_w)
        } else {
            f32::INFINITY
        };
        let max_h = if self.max_h.is_finite() {
            (self.max_h - dv).max(min_h)
        } else {
            f32::INFINITY
        };
        Self {
            min_w,
            min_h,
            max_w,
            max_h,
        }
    }

    fn cache_key(self) -> (u32, u32, u32, u32) {
        (
            self.min_w.to_bits(),
            self.min_h.to_bits(),
            self.max_w.to_bits(),
            self.max_h.to_bits(),
        )
    }
}

/// C-compatible intrinsic size reported by an embedded NVE or foreign-framework surface.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct IntrinsicSize {
    pub width: f32,
    pub height: f32,
}

impl IntrinsicSize {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };

    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn is_finite(self) -> bool {
        self.width.is_finite() && self.height.is_finite()
    }

    /// Rounds dimensions up to the nearest physical pixel boundary for `scale_factor`.
    pub fn snap_to_pixel_grid(self, scale_factor: f32) -> Self {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return self;
        }
        Self {
            width: (self.width * scale_factor).ceil() / scale_factor,
            height: (self.height * scale_factor).ceil() / scale_factor,
        }
    }
}

/// Optional typographic baseline metrics for inline alignment in SwiftUI/Compose/Flutter rows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct BaselineMetrics {
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
}

/// Full layout proposal sent from a host framework into a measurable surface.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutProposal {
    pub constraints: SizeConstraints,
    pub scale_factor: f32,
    pub snap_to_pixels: bool,
}

impl LayoutProposal {
    pub fn new(constraints: SizeConstraints, scale_factor: f32) -> Self {
        Self {
            constraints,
            scale_factor,
            snap_to_pixels: true,
        }
    }

    /// Adapts a SwiftUI `ProposedViewSize` (`Option<f32>` width/height) into box constraints.
    pub fn from_swiftui_proposal(
        proposed_width: Option<f32>,
        proposed_height: Option<f32>,
        scale_factor: f32,
    ) -> Self {
        Self::new(
            SizeConstraints::loose(
                proposed_width.unwrap_or(f32::INFINITY),
                proposed_height.unwrap_or(f32::INFINITY),
            ),
            scale_factor,
        )
    }
}

/// Output of a layout negotiation pass.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutResult {
    pub size: IntrinsicSize,
    pub unclipped_size: IntrinsicSize,
    pub baseline: BaselineMetrics,
    pub clipped: bool,
}

/// Trait implemented by any NVE or bridged foreign surface that can report its
/// intrinsic dimensions to Flutter `RenderBox`, RN Yoga, SwiftUI `sizeThatFits`,
/// and Jetpack Compose `MeasurePolicy`.
pub trait MeasurableSurface {
    fn measure(&self, constraints: SizeConstraints) -> IntrinsicSize;

    fn baseline_metrics(&self, _size: IntrinsicSize) -> BaselineMetrics {
        BaselineMetrics::default()
    }

    fn min_intrinsic_width(&self, height: f32) -> f32 {
        self.measure(SizeConstraints::new(0.0, 0.0, 0.0, height))
            .width
    }

    fn max_intrinsic_width(&self, height: f32) -> f32 {
        self.measure(SizeConstraints::new(0.0, 0.0, f32::INFINITY, height))
            .width
    }

    fn min_intrinsic_height(&self, width: f32) -> f32 {
        self.measure(SizeConstraints::new(width, 0.0, width, f32::INFINITY))
            .height
    }

    fn negotiate(&self, proposal: LayoutProposal) -> Result<LayoutResult, InteropError> {
        let constraints = proposal.constraints.validate()?;
        let raw = self.measure(constraints);
        let mut clamped = constraints.clamp(raw);
        let clipped = (raw.width - clamped.width).abs() > f32::EPSILON
            || (raw.height - clamped.height).abs() > f32::EPSILON;
        if proposal.snap_to_pixels {
            clamped = constraints.clamp(clamped.snap_to_pixel_grid(proposal.scale_factor));
        }
        let baseline = self.baseline_metrics(clamped);
        Ok(LayoutResult {
            size: clamped,
            unclipped_size: raw,
            baseline,
            clipped,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheEntryKey {
    surface_id: SurfaceId,
    constraints: (u32, u32, u32, u32),
    scale_bits: u32,
    snap: bool,
}

/// Memoizing layout negotiation coordinator that avoids redundant cross-FFI measurement
/// passes when host layout engines (Yoga, Flutter RenderBox, Compose) probe constraints repeatedly.
#[derive(Debug, Default)]
pub struct LayoutNegotiator {
    cache: HashMap<CacheEntryKey, LayoutResult>,
    hit_count: u64,
    miss_count: u64,
}

impl LayoutNegotiator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn negotiate_surface<S: MeasurableSurface + ?Sized>(
        &mut self,
        surface_id: SurfaceId,
        surface: &S,
        proposal: LayoutProposal,
    ) -> Result<LayoutResult, InteropError> {
        let constraints = proposal.constraints.validate()?;
        let key = CacheEntryKey {
            surface_id,
            constraints: constraints.cache_key(),
            scale_bits: proposal.scale_factor.to_bits(),
            snap: proposal.snap_to_pixels,
        };
        if let Some(&cached) = self.cache.get(&key) {
            self.hit_count += 1;
            return Ok(cached);
        }
        self.miss_count += 1;
        let result = surface.negotiate(proposal)?;
        self.cache.insert(key, result);
        Ok(result)
    }

    /// Invalidates all cached layout results for `surface_id` (e.g. after `set_props` or hot-reload).
    pub fn invalidate_surface(&mut self, surface_id: SurfaceId) {
        self.cache.retain(|k, _| k.surface_id != surface_id);
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn cache_hits(&self) -> u64 {
        self.hit_count
    }

    pub fn cache_misses(&self) -> u64 {
        self.miss_count
    }
}
