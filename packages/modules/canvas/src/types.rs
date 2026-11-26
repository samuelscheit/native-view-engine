//! Core types, constants, opcodes, sparkline generator, and error types for `NVE_DRAW1`.

use std::fmt;

use nve_abi::ColorRgba;

/// 9-byte ASCII magic header identifying a binary NVE vector draw-list stream.
pub const NVE_DRAW_MAGIC: [u8; 9] = *b"NVE_DRAW1";

/// Current binary format version of `NVE_DRAW1`.
pub const NVE_DRAW_VERSION: u16 = 1;

/// Fixed byte length of the `NVE_DRAW1` header (`9 + 2 + 4`).
pub const NVE_DRAW_HEADER_SIZE: usize = 15;

/// Strict upper bound on operations per draw list to guard against corrupt/hostile payloads.
pub const MAX_DRAW_OPS: usize = 65_536;

/// Strict upper bound on vertices inside a single `StrokePolyline` operation.
pub const MAX_POLYLINE_POINTS: usize = 16_384;

/// Compact prefix used when transporting an `NVE_DRAW1` binary buffer through UTF-8 RSX text props.
pub const DRAW_LIST_ATTR_PREFIX: &str = "nve-draw1;b64,";

/// Cross-platform image scaling mode for `ComponentKind::Image` (`resizeMode` prop).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ImageResizeMode {
    #[default]
    Cover = 0,
    Contain = 1,
    Stretch = 2,
    Center = 3,
}

impl ImageResizeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
            Self::Stretch => "stretch",
            Self::Center => "center",
        }
    }

    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Cover),
            1 => Some(Self::Contain),
            2 => Some(Self::Stretch),
            3 => Some(Self::Center),
            _ => None,
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("cover") {
            Some(Self::Cover)
        } else if s.eq_ignore_ascii_case("contain") {
            Some(Self::Contain)
        } else if s.eq_ignore_ascii_case("stretch") {
            Some(Self::Stretch)
        } else if s.eq_ignore_ascii_case("center") {
            Some(Self::Center)
        } else {
            None
        }
    }
}

impl fmt::Display for ImageResizeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DrawOpcode {
    Clear = 1,
    FillRect = 2,
    FillRoundedRect = 3,
    FillCircle = 4,
    StrokeLine = 5,
    StrokePolyline = 6,
    ClipRect = 7,
    RestoreClip = 8,
}

impl DrawOpcode {
    pub(super) fn from_u8(v: u8) -> Result<Self, DrawListError> {
        match v {
            1 => Ok(Self::Clear),
            2 => Ok(Self::FillRect),
            3 => Ok(Self::FillRoundedRect),
            4 => Ok(Self::FillCircle),
            5 => Ok(Self::StrokeLine),
            6 => Ok(Self::StrokePolyline),
            7 => Ok(Self::ClipRect),
            8 => Ok(Self::RestoreClip),
            other => Err(DrawListError::UnknownOpcode(other)),
        }
    }
}

/// Typed vector drawing operation encoded inside an `NVE_DRAW1` binary draw list.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasOp {
    Clear {
        color: ColorRgba,
    },
    FillRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: ColorRgba,
    },
    FillRoundedRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    },
    FillCircle {
        cx: f32,
        cy: f32,
        radius: f32,
        color: ColorRgba,
    },
    StrokeLine {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: ColorRgba,
    },
    StrokePolyline {
        points: Vec<(f32, f32)>,
        width: f32,
        color: ColorRgba,
    },
    ClipRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    RestoreClip,
}

impl CanvasOp {
    /// Generates a crisp telemetry sparkline sequence (background rounded frame, clip region,
    /// subtle horizontal reference guides, vector polyline curve, and highlighted terminal dot).
    ///
    /// Non-finite sample values are safely sanitized so the resulting stream always decodes
    /// cleanly under strict [`super::DrawListDecoder`] validation.
    pub fn sparkline(
        samples: &[f32],
        width: f32,
        height: f32,
        stroke_color: ColorRgba,
        background_color: ColorRgba,
    ) -> Vec<CanvasOp> {
        let w = sanitize_positive(width, 16.0);
        let h = sanitize_positive(height, 16.0);
        let pad_x = (w * 0.04).clamp(4.0, 12.0);
        let pad_y = (h * 0.14).clamp(4.0, 12.0);
        let plot_w = (w - pad_x * 2.0).max(1.0);
        let plot_h = (h - pad_y * 2.0).max(1.0);

        let grid_color = ColorRgba::rgba(148, 163, 184, 52);
        let baseline_color = ColorRgba::rgba(148, 163, 184, 90);

        let mut ops = Vec::with_capacity(12);
        ops.push(CanvasOp::Clear {
            color: ColorRgba::TRANSPARENT,
        });
        ops.push(CanvasOp::FillRoundedRect {
            x: 0.0,
            y: 0.0,
            w,
            h,
            radius: 8.0_f32.min(h * 0.25),
            color: background_color,
        });
        ops.push(CanvasOp::ClipRect {
            x: 1.0,
            y: 1.0,
            w: (w - 2.0).max(1.0),
            h: (h - 2.0).max(1.0),
        });

        // Subtle horizontal telemetry guide rails (top / mid / bottom).
        let mid_y = pad_y + plot_h * 0.5;
        ops.push(CanvasOp::StrokeLine {
            x0: pad_x,
            y0: pad_y,
            x1: pad_x + plot_w,
            y1: pad_y,
            width: 1.0,
            color: grid_color,
        });
        ops.push(CanvasOp::StrokeLine {
            x0: pad_x,
            y0: mid_y,
            x1: pad_x + plot_w,
            y1: mid_y,
            width: 1.0,
            color: grid_color,
        });
        ops.push(CanvasOp::StrokeLine {
            x0: pad_x,
            y0: pad_y + plot_h,
            x1: pad_x + plot_w,
            y1: pad_y + plot_h,
            width: 1.0,
            color: baseline_color,
        });

        let clean: Vec<f32> = samples
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .take(MAX_POLYLINE_POINTS)
            .collect();

        if clean.is_empty() {
            ops.push(CanvasOp::RestoreClip);
            return ops;
        }

        if clean.len() == 1 {
            let cy = pad_y + plot_h * 0.5;
            ops.push(CanvasOp::StrokeLine {
                x0: pad_x,
                y0: cy,
                x1: pad_x + plot_w,
                y1: cy,
                width: 2.0,
                color: stroke_color,
            });
            ops.push(CanvasOp::FillCircle {
                cx: pad_x + plot_w,
                cy,
                radius: 3.5,
                color: stroke_color,
            });
            ops.push(CanvasOp::RestoreClip);
            return ops;
        }

        let mut min_v = f32::INFINITY;
        let mut max_v = f32::NEG_INFINITY;
        for &v in &clean {
            if v < min_v {
                min_v = v;
            }
            if v > max_v {
                max_v = v;
            }
        }
        let span = max_v - min_v;
        let denom = (clean.len() - 1) as f32;

        let mut points = Vec::with_capacity(clean.len());
        for (idx, &v) in clean.iter().enumerate() {
            let t = idx as f32 / denom;
            let norm = if span.abs() <= f32::EPSILON {
                0.5
            } else {
                ((v - min_v) / span).clamp(0.0, 1.0)
            };
            let px = pad_x + t * plot_w;
            let py = pad_y + (1.0 - norm) * plot_h;
            points.push((px, py));
        }

        // Subtle highlight bar under the latest telemetry reading.
        if let Some(&(last_x, last_y)) = points.last() {
            let bar_w = (plot_w / clean.len() as f32).clamp(3.0, 10.0);
            let bar_h = (pad_y + plot_h - last_y).max(2.0);
            ops.push(CanvasOp::FillRect {
                x: (last_x - bar_w * 0.5).max(pad_x),
                y: last_y,
                w: bar_w,
                h: bar_h,
                color: ColorRgba::rgba(stroke_color.r, stroke_color.g, stroke_color.b, 48),
            });
            ops.push(CanvasOp::StrokePolyline {
                points: points.clone(),
                width: 2.0,
                color: stroke_color,
            });
            ops.push(CanvasOp::FillCircle {
                cx: last_x,
                cy: last_y,
                radius: 3.5,
                color: stroke_color,
            });
        }

        ops.push(CanvasOp::RestoreClip);
        ops
    }
}

fn sanitize_positive(v: f32, fallback: f32) -> f32 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        fallback
    }
}

pub(super) fn sanitize_finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Decoded header of an `NVE_DRAW1` binary draw-list buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawListHeader {
    pub version: u16,
    pub op_count: u32,
}

/// Strict error type returned by [`super::DrawListDecoder`] on invalid or corrupt buffers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawListError {
    Truncated,
    BadMagic,
    UnsupportedVersion(u16),
    OpCountExceedsLimit { count: u32, max: usize },
    PolylinePointCountExceedsLimit { count: u32, max: usize },
    UnknownOpcode(u8),
    NonFiniteFloat,
    TrailingBytes(usize),
    InvalidBase64,
    UnsupportedPropValue,
}

impl fmt::Display for DrawListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "NVE_DRAW1 buffer truncated"),
            Self::BadMagic => write!(f, "NVE_DRAW1 magic header mismatch"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NVE_DRAW1 version {v}"),
            Self::OpCountExceedsLimit { count, max } => {
                write!(f, "NVE_DRAW1 op count {count} exceeds limit {max}")
            }
            Self::PolylinePointCountExceedsLimit { count, max } => {
                write!(
                    f,
                    "NVE_DRAW1 polyline point count {count} exceeds limit {max}"
                )
            }
            Self::UnknownOpcode(op) => write!(f, "unknown NVE_DRAW1 opcode {op}"),
            Self::NonFiniteFloat => write!(f, "non-finite f32 value in NVE_DRAW1 stream"),
            Self::TrailingBytes(n) => {
                write!(f, "{n} unexpected trailing bytes in NVE_DRAW1 stream")
            }
            Self::InvalidBase64 => write!(f, "invalid base64 payload for NVE_DRAW1 attribute"),
            Self::UnsupportedPropValue => {
                write!(f, "expected RawPropValue::Bytes or base64 String for drawList")
            }
        }
    }
}

impl std::error::Error for DrawListError {}
