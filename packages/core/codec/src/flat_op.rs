//! Generic schema descriptors for flat 4-byte-aligned (`u32`-word) operation-stream resources.
//!
//! A flat word-op stream uses the little-endian wire layout:
//! ```text
//! [op_count: u32]
//!   for _ in 0..op_count:
//!     [opcode: u32] [field_0 ...] [field_1 ...] ...
//! ```
//! Each scalar field occupies one little-endian 32-bit word (`u32`, `ColorRgba` packed as
//! `r | (g << 8) | (b << 16) | (a << 24)`, or IEEE-754 `f32`). Variable-length
//! [`FlatOpFieldType::Point2Slice`] fields encode `[count: u32, (x: f32, y: f32) * count]` and
//! must appear as the final field of their operation.

/// Validation predicate enforced on decoded IEEE-754 `f32` words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FloatConstraint {
    /// Rejects `NaN`, `+Inf`, and `-Inf`.
    Finite,
    /// Rejects `NaN`, `+Inf`, `-Inf`, and strictly negative values (`< 0.0`).
    /// IEEE-754 `-0.0 >= 0.0` is `true` across Rust, C++, Java, Swift, Dart, and JS/TS.
    NonNegative,
}

impl FloatConstraint {
    /// Returns `true` when `value` satisfies this constraint.
    #[inline]
    pub const fn check(self, value: f32) -> bool {
        match self {
            Self::Finite => value.is_finite(),
            Self::NonNegative => value.is_finite() && value >= 0.0,
        }
    }
}

/// Wire type and validation constraint for one field inside a [`FlatOpDesc`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlatOpFieldType {
    /// Single little-endian `u32` word.
    U32,
    /// Single little-endian `u32` RGBA8888 word (`r | (g << 8) | (b << 16) | (a << 24)`).
    ColorRgba,
    /// Single IEEE-754 little-endian `f32` word subject to [`FloatConstraint`].
    F32(FloatConstraint),
    /// Variable-length `(x, y)` `f32` vertex slice: `[count: u32, (x: f32, y: f32) * count]`.
    Point2Slice {
        /// Strict upper bound on vertex count per operation.
        max_len: u32,
        /// Constant identifier emitted in generated host decoders (e.g. `"MAX_POLYLINE_POINTS"`).
        max_const_name: &'static str,
        /// Float constraint applied to every decoded `(x, y)` coordinate.
        coord_constraint: FloatConstraint,
    },
}

/// Named payload field inside a [`FlatOpDesc`], ordered in exact wire sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlatOpFieldDesc {
    /// `snake_case` field name (e.g. `"x0"`, `"radius"`, `"color"`, `"points"`).
    pub name: &'static str,
    /// Wire type and validation constraint.
    pub ty: FlatOpFieldType,
}

/// Single operation variant inside a [`FlatOpStreamDesc`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlatOpDesc {
    /// `PascalCase` operation name (e.g. `"FillRoundedRect"`, `"StrokePolyline"`).
    pub name: &'static str,
    /// Stable little-endian `u32` opcode word (`>= 1`).
    pub opcode: u32,
    /// Ordered payload fields immediately following the opcode word.
    pub fields: &'static [FlatOpFieldDesc],
}

impl FlatOpDesc {
    /// Number of fixed `u32` payload words preceding any trailing [`FlatOpFieldType::Point2Slice`]
    /// vertices (including the slice's `u32` vertex count word itself).
    #[inline]
    pub const fn fixed_words(&self) -> usize {
        self.fields.len()
    }

    /// Returns `true` if this operation ends with a variable-length [`FlatOpFieldType::Point2Slice`].
    #[inline]
    pub const fn has_point_slice(&self) -> bool {
        if self.fields.is_empty() {
            return false;
        }
        matches!(
            self.fields[self.fields.len() - 1].ty,
            FlatOpFieldType::Point2Slice { .. }
        )
    }
}

/// Optional balanced push/pop scope discipline enforced by generated stream decoders
/// (e.g. `ClipRect` / `RestoreClip` capped at `MAX_CLIP_DEPTH = 64`).
///
/// Every generated decoder enforces identical semantics:
/// - Pushing while `depth < max_depth` increments `depth` BEFORE invoking the push callback
///   (so throwing callbacks still unwind cleanly).
/// - Pushing while `depth >= max_depth` skips the push callback and increments `overflow`.
/// - Popping while `overflow > 0` decrements `overflow` without invoking the pop callback.
/// - Popping while `overflow == 0 && depth > 0` decrements `depth` and invokes the pop callback.
/// - Popping while `overflow == 0 && depth == 0` is a no-op.
/// - On normal stream end or callback error/exception unwind, all remaining `depth` scopes are
///   closed by invoking the pop callback `depth` times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlatOpScopeDesc {
    /// Constant identifier for the depth cap (e.g. `"MAX_CLIP_DEPTH"`).
    pub limit_name: &'static str,
    /// Opcode that pushes a scope (e.g. `7` for `ClipRect`).
    pub push_opcode: u32,
    /// Opcode that pops a scope (e.g. `8` for `RestoreClip`; must have zero fields).
    pub pop_opcode: u32,
    /// Maximum live scope depth passed to the sink (e.g. `64`).
    pub max_depth: u32,
}

/// Complete schema descriptor for one flat 4-byte-aligned word-op stream resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlatOpStreamDesc {
    /// `PascalCase` stream name (e.g. `"DrawList"` -> `NveDrawList` / `nve_draw_list`).
    pub name: &'static str,
    /// Canonical resource kind string hashed via XXH64 (`Key64::of(kind_name)`).
    pub kind_name: &'static str,
    /// Constant identifier for the op-count upper bound (e.g. `"MAX_DRAW_OPS"`).
    pub max_ops_const_name: &'static str,
    /// Strict upper bound on operations per stream.
    pub max_ops: u32,
    /// Optional balanced push/pop scope pair.
    pub scope: Option<FlatOpScopeDesc>,
    /// All valid operation variants in canonical wire order.
    pub ops: &'static [FlatOpDesc],
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OP: FlatOpDesc = FlatOpDesc {
        name: "StrokePolyline",
        opcode: 6,
        fields: &[
            FlatOpFieldDesc {
                name: "width",
                ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
            },
            FlatOpFieldDesc {
                name: "color",
                ty: FlatOpFieldType::ColorRgba,
            },
            FlatOpFieldDesc {
                name: "points",
                ty: FlatOpFieldType::Point2Slice {
                    max_len: 16_384,
                    max_const_name: "MAX_POLYLINE_POINTS",
                    coord_constraint: FloatConstraint::Finite,
                },
            },
        ],
    };

    #[test]
    fn float_constraints_handle_edge_values() {
        assert!(FloatConstraint::Finite.check(0.0));
        assert!(FloatConstraint::Finite.check(-0.0));
        assert!(FloatConstraint::Finite.check(-42.5));
        assert!(!FloatConstraint::Finite.check(f32::NAN));
        assert!(!FloatConstraint::Finite.check(f32::INFINITY));
        assert!(!FloatConstraint::Finite.check(f32::NEG_INFINITY));

        assert!(FloatConstraint::NonNegative.check(0.0));
        assert!(FloatConstraint::NonNegative.check(-0.0));
        assert!(FloatConstraint::NonNegative.check(1.25));
        assert!(!FloatConstraint::NonNegative.check(-0.0001));
        assert!(!FloatConstraint::NonNegative.check(f32::NAN));
        assert!(!FloatConstraint::NonNegative.check(f32::INFINITY));
        assert!(!FloatConstraint::NonNegative.check(f32::NEG_INFINITY));
    }

    #[test]
    fn flat_op_desc_reports_word_shape() {
        assert_eq!(SAMPLE_OP.fixed_words(), 3);
        assert!(SAMPLE_OP.has_point_slice());

        const EMPTY_OP: FlatOpDesc = FlatOpDesc {
            name: "RestoreClip",
            opcode: 8,
            fields: &[],
        };
        assert_eq!(EMPTY_OP.fixed_words(), 0);
        assert!(!EMPTY_OP.has_point_slice());
    }
}
