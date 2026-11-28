//! Canonical `nve-codec` schema descriptor ([`DRAW_LIST_CODEC_DESC`]) and shared balanced
//! clip-stack tracker ([`ClipDepthTracker`]) for flat 4-byte-aligned `nve.res.draw_list` streams.

use nve_codec::{
    FlatOpDesc, FlatOpFieldDesc, FlatOpFieldType, FlatOpScopeDesc, FlatOpStreamDesc,
    FloatConstraint,
};

use crate::{MAX_CLIP_DEPTH, MAX_DRAW_OPS, MAX_POLYLINE_POINTS};

pub(crate) const OP_CLEAR: u32 = 1;
pub(crate) const OP_FILL_RECT: u32 = 2;
pub(crate) const OP_FILL_ROUNDED_RECT: u32 = 3;
pub(crate) const OP_FILL_CIRCLE: u32 = 4;
pub(crate) const OP_STROKE_LINE: u32 = 5;
pub(crate) const OP_STROKE_POLYLINE: u32 = 6;
pub(crate) const OP_CLIP_RECT: u32 = 7;
pub(crate) const OP_RESTORE_CLIP: u32 = 8;

/// Canonical `nve-codec` schema descriptor for `nve.res.draw_list` (`FlatResource`), consumed by
/// `nve-cli codegen abi` to emit zero-allocation decoders across C++, Swift, Java, Dart, and TS.
pub const DRAW_LIST_CODEC_DESC: FlatOpStreamDesc = FlatOpStreamDesc {
    name: "DrawList",
    kind_name: "nve.res.draw_list",
    max_ops_const_name: "MAX_DRAW_OPS",
    max_ops: MAX_DRAW_OPS as u32,
    scope: Some(FlatOpScopeDesc {
        limit_name: "MAX_CLIP_DEPTH",
        push_opcode: OP_CLIP_RECT,
        pop_opcode: OP_RESTORE_CLIP,
        max_depth: MAX_CLIP_DEPTH as u32,
    }),
    ops: &[
        FlatOpDesc {
            name: "Clear",
            opcode: OP_CLEAR,
            fields: &[FlatOpFieldDesc {
                name: "color",
                ty: FlatOpFieldType::ColorRgba,
            }],
        },
        FlatOpDesc {
            name: "FillRect",
            opcode: OP_FILL_RECT,
            fields: &[
                FlatOpFieldDesc {
                    name: "x",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "y",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "w",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "h",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "color",
                    ty: FlatOpFieldType::ColorRgba,
                },
            ],
        },
        FlatOpDesc {
            name: "FillRoundedRect",
            opcode: OP_FILL_ROUNDED_RECT,
            fields: &[
                FlatOpFieldDesc {
                    name: "x",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "y",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "w",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "h",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "radius",
                    ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
                },
                FlatOpFieldDesc {
                    name: "color",
                    ty: FlatOpFieldType::ColorRgba,
                },
            ],
        },
        FlatOpDesc {
            name: "FillCircle",
            opcode: OP_FILL_CIRCLE,
            fields: &[
                FlatOpFieldDesc {
                    name: "cx",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "cy",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "radius",
                    ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
                },
                FlatOpFieldDesc {
                    name: "color",
                    ty: FlatOpFieldType::ColorRgba,
                },
            ],
        },
        FlatOpDesc {
            name: "StrokeLine",
            opcode: OP_STROKE_LINE,
            fields: &[
                FlatOpFieldDesc {
                    name: "x0",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "y0",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "x1",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "y1",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "width",
                    ty: FlatOpFieldType::F32(FloatConstraint::NonNegative),
                },
                FlatOpFieldDesc {
                    name: "color",
                    ty: FlatOpFieldType::ColorRgba,
                },
            ],
        },
        FlatOpDesc {
            name: "StrokePolyline",
            opcode: OP_STROKE_POLYLINE,
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
                        max_len: MAX_POLYLINE_POINTS as u32,
                        max_const_name: "MAX_POLYLINE_POINTS",
                        coord_constraint: FloatConstraint::Finite,
                    },
                },
            ],
        },
        FlatOpDesc {
            name: "ClipRect",
            opcode: OP_CLIP_RECT,
            fields: &[
                FlatOpFieldDesc {
                    name: "x",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "y",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "w",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
                FlatOpFieldDesc {
                    name: "h",
                    ty: FlatOpFieldType::F32(FloatConstraint::Finite),
                },
            ],
        },
        FlatOpDesc {
            name: "RestoreClip",
            opcode: OP_RESTORE_CLIP,
            fields: &[],
        },
    ],
};

/// Shared balanced clip-stack depth tracker enforcing [`MAX_CLIP_DEPTH`] (`64`) identically
/// across Rust renderers (`nve-canvas-web`, `nve-canvas-skia`) and generated host decoders.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClipDepthTracker {
    depth: usize,
    overflow: usize,
}

impl ClipDepthTracker {
    #[inline]
    pub const fn new() -> Self {
        Self {
            depth: 0,
            overflow: 0,
        }
    }

    #[inline]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    #[inline]
    pub const fn overflow(&self) -> usize {
        self.overflow
    }

    /// Records a `ClipRect`. Returns `true` when `depth < MAX_CLIP_DEPTH` (caller should push a
    /// live clip), or `false` when absorbed into the overflow depth counter.
    #[inline]
    pub fn push(&mut self) -> bool {
        if self.depth < MAX_CLIP_DEPTH {
            self.depth += 1;
            true
        } else {
            self.overflow = self.overflow.saturating_add(1);
            false
        }
    }

    /// Records a `RestoreClip`. Returns `true` when `overflow == 0 && depth > 0` (caller should
    /// pop a live clip), or `false` when absorbed by overflow or an empty clip stack.
    #[inline]
    pub fn pop(&mut self) -> bool {
        if self.overflow > 0 {
            self.overflow -= 1;
            false
        } else if self.depth > 0 {
            self.depth -= 1;
            true
        } else {
            false
        }
    }

    /// Drains all remaining live clips on stream end/unwind and returns how many restores to emit.
    #[inline]
    pub fn drain_open(&mut self) -> usize {
        let open = self.depth;
        self.depth = 0;
        self.overflow = 0;
        open
    }
}
