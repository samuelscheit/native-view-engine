//! Zero-JSON compact binary vector draw-list IR (`NVE_DRAW1`) for `SkiaCanvas` + image resize modes.
//!
//! Layout of a binary `NVE_DRAW1` buffer (all multi-byte integers and IEEE-754 floats little-endian):
//! - `[0..9]`:   9-byte ASCII magic `b"NVE_DRAW1"`
//! - `[9..11]`:  `u16` format version (`NVE_DRAW_VERSION == 1`)
//! - `[11..15]`: `u32` op count (strictly capped at [`MAX_DRAW_OPS`])
//! - `[15..]`:   packed stream of tagged vector operations (`Clear`, `FillRect`, `FillRoundedRect`,
//!   `FillCircle`, `StrokeLine`, `StrokePolyline`, `ClipRect`, `RestoreClip`).
//!
//! Designed for zero-copy transport inside [`nve_abi::RawPropValue::Bytes`] (with an optional
//! compact ASCII base64 attribute wrapper for Dioxus `rsx!` string attributes).

use std::fmt;

use nve_abi::{ColorRgba, RawPropValue};

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
        match raw.trim().to_ascii_lowercase().as_str() {
            "cover" => Some(Self::Cover),
            "contain" => Some(Self::Contain),
            "stretch" => Some(Self::Stretch),
            "center" => Some(Self::Center),
            _ => None,
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
enum DrawOpcode {
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
    fn from_u8(v: u8) -> Result<Self, DrawListError> {
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
    /// cleanly under strict `DrawListDecoder` validation.
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

fn sanitize_finite(v: f32) -> f32 {
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

/// Strict error type returned by [`DrawListDecoder`] on invalid or corrupt buffers.
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
                write!(f, "NVE_DRAW1 polyline point count {count} exceeds limit {max}")
            }
            Self::UnknownOpcode(op) => write!(f, "unknown NVE_DRAW1 opcode {op}"),
            Self::NonFiniteFloat => write!(f, "non-finite f32 value in NVE_DRAW1 stream"),
            Self::TrailingBytes(n) => write!(f, "{n} unexpected trailing bytes in NVE_DRAW1 stream"),
            Self::InvalidBase64 => write!(f, "invalid base64 payload for NVE_DRAW1 attribute"),
            Self::UnsupportedPropValue => {
                write!(f, "expected RawPropValue::Bytes or base64 String for drawList")
            }
        }
    }
}

impl std::error::Error for DrawListError {}

/// Zero-JSON flat binary encoder producing `NVE_DRAW1` byte buffers directly into a single `Vec<u8>`.
#[derive(Debug, Clone)]
pub struct DrawListBuilder {
    buf: Vec<u8>,
    op_count: u32,
}

impl Default for DrawListBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DrawListBuilder {
    pub fn new() -> Self {
        Self::with_capacity(256)
    }

    pub fn with_capacity(capacity_bytes: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity_bytes.max(NVE_DRAW_HEADER_SIZE));
        buf.extend_from_slice(&NVE_DRAW_MAGIC);
        buf.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        debug_assert_eq!(buf.len(), NVE_DRAW_HEADER_SIZE);
        Self { buf, op_count: 0 }
    }

    pub fn op_count(&self) -> u32 {
        self.op_count
    }

    pub fn is_empty(&self) -> bool {
        self.op_count == 0
    }

    fn write_f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&sanitize_finite(v).to_le_bytes());
    }

    fn write_color(&mut self, c: ColorRgba) {
        self.buf.extend_from_slice(&c.to_u32().to_le_bytes());
    }

    pub fn clear(&mut self, color: ColorRgba) -> &mut Self {
        self.buf.push(DrawOpcode::Clear as u8);
        self.write_color(color);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) -> &mut Self {
        self.buf.push(DrawOpcode::FillRect as u8);
        self.write_f32(x);
        self.write_f32(y);
        self.write_f32(w);
        self.write_f32(h);
        self.write_color(color);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    ) -> &mut Self {
        self.buf.push(DrawOpcode::FillRoundedRect as u8);
        self.write_f32(x);
        self.write_f32(y);
        self.write_f32(w);
        self.write_f32(h);
        self.write_f32(radius);
        self.write_color(color);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) -> &mut Self {
        self.buf.push(DrawOpcode::FillCircle as u8);
        self.write_f32(cx);
        self.write_f32(cy);
        self.write_f32(radius);
        self.write_color(color);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn stroke_line(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: ColorRgba,
    ) -> &mut Self {
        self.buf.push(DrawOpcode::StrokeLine as u8);
        self.write_f32(x0);
        self.write_f32(y0);
        self.write_f32(x1);
        self.write_f32(y1);
        self.write_f32(width);
        self.write_color(color);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn stroke_polyline(
        &mut self,
        points: &[(f32, f32)],
        width: f32,
        color: ColorRgba,
    ) -> &mut Self {
        let clamped = &points[..points.len().min(MAX_POLYLINE_POINTS)];
        self.buf.push(DrawOpcode::StrokePolyline as u8);
        self.write_f32(width);
        self.write_color(color);
        self.buf
            .extend_from_slice(&(clamped.len() as u32).to_le_bytes());
        for &(px, py) in clamped {
            self.write_f32(px);
            self.write_f32(py);
        }
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        self.buf.push(DrawOpcode::ClipRect as u8);
        self.write_f32(x);
        self.write_f32(y);
        self.write_f32(w);
        self.write_f32(h);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn restore_clip(&mut self) -> &mut Self {
        self.buf.push(DrawOpcode::RestoreClip as u8);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn push_op(&mut self, op: &CanvasOp) -> &mut Self {
        match op {
            CanvasOp::Clear { color } => self.clear(*color),
            CanvasOp::FillRect { x, y, w, h, color } => self.fill_rect(*x, *y, *w, *h, *color),
            CanvasOp::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => self.fill_rounded_rect(*x, *y, *w, *h, *radius, *color),
            CanvasOp::FillCircle {
                cx,
                cy,
                radius,
                color,
            } => self.fill_circle(*cx, *cy, *radius, *color),
            CanvasOp::StrokeLine {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
            } => self.stroke_line(*x0, *y0, *x1, *y1, *width, *color),
            CanvasOp::StrokePolyline {
                points,
                width,
                color,
            } => self.stroke_polyline(points, *width, *color),
            CanvasOp::ClipRect { x, y, w, h } => self.clip_rect(*x, *y, *w, *h),
            CanvasOp::RestoreClip => self.restore_clip(),
        }
    }

    pub fn extend_ops(&mut self, ops: &[CanvasOp]) -> &mut Self {
        for op in ops {
            self.push_op(op);
        }
        self
    }

    /// Appends a complete telemetry sparkline directly into this draw-list builder.
    pub fn sparkline(
        &mut self,
        samples: &[f32],
        width: f32,
        height: f32,
        stroke_color: ColorRgba,
        background_color: ColorRgba,
    ) -> &mut Self {
        let ops = CanvasOp::sparkline(samples, width, height, stroke_color, background_color);
        self.extend_ops(&ops)
    }

    /// Patches the header op count in-place and returns the owned binary `NVE_DRAW1` byte buffer.
    pub fn finish(mut self) -> Vec<u8> {
        self.buf[11..15].copy_from_slice(&self.op_count.to_le_bytes());
        self.buf
    }

    /// Consumes the builder directly into a zero-JSON [`RawPropValue::Bytes`] property value.
    pub fn to_raw_prop(self) -> RawPropValue {
        RawPropValue::Bytes(self.finish())
    }

    /// Encodes the finished `NVE_DRAW1` binary buffer as a prefixed base64 string for UTF-8 RSX
    /// attributes (`nve-draw1;b64,...`).
    pub fn to_attr_string(self) -> String {
        let bytes = self.finish();
        let mut out = String::with_capacity(DRAW_LIST_ATTR_PREFIX.len() + bytes.len().div_ceil(3) * 4);
        out.push_str(DRAW_LIST_ATTR_PREFIX);
        encode_base64_into(&bytes, &mut out);
        out
    }
}

/// Strict bounds-checked decoder for `NVE_DRAW1` binary vector draw-list streams.
pub struct DrawListDecoder;

impl DrawListDecoder {
    /// Parses and validates only the fixed 15-byte `NVE_DRAW1` header.
    pub fn decode_header(bytes: &[u8]) -> Result<DrawListHeader, DrawListError> {
        if bytes.len() < NVE_DRAW_HEADER_SIZE {
            return Err(DrawListError::Truncated);
        }
        if bytes[0..9] != NVE_DRAW_MAGIC {
            return Err(DrawListError::BadMagic);
        }
        let version = u16::from_le_bytes([bytes[9], bytes[10]]);
        if version != NVE_DRAW_VERSION {
            return Err(DrawListError::UnsupportedVersion(version));
        }
        let op_count = u32::from_le_bytes([bytes[11], bytes[12], bytes[13], bytes[14]]);
        if (op_count as usize) > MAX_DRAW_OPS {
            return Err(DrawListError::OpCountExceedsLimit {
                count: op_count,
                max: MAX_DRAW_OPS,
            });
        }
        Ok(DrawListHeader { version, op_count })
    }

    /// Strictly decodes an entire `NVE_DRAW1` binary buffer into typed [`CanvasOp`]s.
    ///
    /// Never panics on arbitrary, truncated, or adversarial inputs.
    pub fn decode(bytes: &[u8]) -> Result<Vec<CanvasOp>, DrawListError> {
        let header = Self::decode_header(bytes)?;
        let mut pos = NVE_DRAW_HEADER_SIZE;
        let op_count = header.op_count as usize;
        // Every operation occupies at least 1 byte (its opcode tag), so reject lying op counts
        // before reserving heap memory.
        if bytes.len() - pos < op_count {
            return Err(DrawListError::Truncated);
        }

        let mut ops = Vec::with_capacity(op_count.min(1024));
        for _ in 0..op_count {
            let tag = read_u8(bytes, &mut pos)?;
            let op = match DrawOpcode::from_u8(tag)? {
                DrawOpcode::Clear => CanvasOp::Clear {
                    color: read_color(bytes, &mut pos)?,
                },
                DrawOpcode::FillRect => CanvasOp::FillRect {
                    x: read_f32(bytes, &mut pos)?,
                    y: read_f32(bytes, &mut pos)?,
                    w: read_f32(bytes, &mut pos)?,
                    h: read_f32(bytes, &mut pos)?,
                    color: read_color(bytes, &mut pos)?,
                },
                DrawOpcode::FillRoundedRect => CanvasOp::FillRoundedRect {
                    x: read_f32(bytes, &mut pos)?,
                    y: read_f32(bytes, &mut pos)?,
                    w: read_f32(bytes, &mut pos)?,
                    h: read_f32(bytes, &mut pos)?,
                    radius: read_f32(bytes, &mut pos)?,
                    color: read_color(bytes, &mut pos)?,
                },
                DrawOpcode::FillCircle => CanvasOp::FillCircle {
                    cx: read_f32(bytes, &mut pos)?,
                    cy: read_f32(bytes, &mut pos)?,
                    radius: read_f32(bytes, &mut pos)?,
                    color: read_color(bytes, &mut pos)?,
                },
                DrawOpcode::StrokeLine => CanvasOp::StrokeLine {
                    x0: read_f32(bytes, &mut pos)?,
                    y0: read_f32(bytes, &mut pos)?,
                    x1: read_f32(bytes, &mut pos)?,
                    y1: read_f32(bytes, &mut pos)?,
                    width: read_f32(bytes, &mut pos)?,
                    color: read_color(bytes, &mut pos)?,
                },
                DrawOpcode::StrokePolyline => {
                    let width = read_f32(bytes, &mut pos)?;
                    let color = read_color(bytes, &mut pos)?;
                    let pt_count = read_u32(bytes, &mut pos)?;
                    if (pt_count as usize) > MAX_POLYLINE_POINTS {
                        return Err(DrawListError::PolylinePointCountExceedsLimit {
                            count: pt_count,
                            max: MAX_POLYLINE_POINTS,
                        });
                    }
                    let needed_bytes = (pt_count as usize)
                        .checked_mul(8)
                        .ok_or(DrawListError::Truncated)?;
                    if bytes.len() - pos < needed_bytes {
                        return Err(DrawListError::Truncated);
                    }
                    let mut points = Vec::with_capacity(pt_count as usize);
                    for _ in 0..pt_count {
                        let px = read_f32(bytes, &mut pos)?;
                        let py = read_f32(bytes, &mut pos)?;
                        points.push((px, py));
                    }
                    CanvasOp::StrokePolyline {
                        points,
                        width,
                        color,
                    }
                }
                DrawOpcode::ClipRect => CanvasOp::ClipRect {
                    x: read_f32(bytes, &mut pos)?,
                    y: read_f32(bytes, &mut pos)?,
                    w: read_f32(bytes, &mut pos)?,
                    h: read_f32(bytes, &mut pos)?,
                },
                DrawOpcode::RestoreClip => CanvasOp::RestoreClip,
            };
            ops.push(op);
        }

        if pos != bytes.len() {
            return Err(DrawListError::TrailingBytes(bytes.len() - pos));
        }
        Ok(ops)
    }

    /// Extracts raw `NVE_DRAW1` bytes from a prefixed or bare base64 attribute string.
    pub fn decode_attr_bytes(raw: &str) -> Result<Vec<u8>, DrawListError> {
        let b64 = raw.strip_prefix(DRAW_LIST_ATTR_PREFIX).unwrap_or(raw);
        let bytes = decode_base64(b64)?;
        let _ = Self::decode_header(&bytes)?;
        Ok(bytes)
    }

    /// Decodes a `drawList` [`RawPropValue`] accepting either direct binary `RawPropValue::Bytes`
    /// or a base64-encoded `RawPropValue::String`.
    pub fn decode_raw_prop(prop: &RawPropValue) -> Result<Vec<CanvasOp>, DrawListError> {
        match prop {
            RawPropValue::Bytes(bytes) => Self::decode(bytes),
            RawPropValue::String(s) => {
                let bytes = Self::decode_attr_bytes(s)?;
                Self::decode(&bytes)
            }
            _ => Err(DrawListError::UnsupportedPropValue),
        }
    }
}

fn take<'a>(bytes: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], DrawListError> {
    if bytes.len().saturating_sub(*pos) < n {
        return Err(DrawListError::Truncated);
    }
    let slice = &bytes[*pos..*pos + n];
    *pos += n;
    Ok(slice)
}

fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, DrawListError> {
    Ok(take(bytes, pos, 1)?[0])
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, DrawListError> {
    let s = take(bytes, pos, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_f32(bytes: &[u8], pos: &mut usize) -> Result<f32, DrawListError> {
    let s = take(bytes, pos, 4)?;
    let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    if !v.is_finite() {
        return Err(DrawListError::NonFiniteFloat);
    }
    Ok(v)
}

fn read_color(bytes: &[u8], pos: &mut usize) -> Result<ColorRgba, DrawListError> {
    Ok(ColorRgba::from_u32(read_u32(bytes, pos)?))
}

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64_into(input: &[u8], out: &mut String) {
    let mut chunks = input.chunks_exact(3);
    for chunk in &mut chunks {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[(n & 0x3f) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        1 => {
            let n = (rem[0] as u32) << 16;
            out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
}

fn decode_b64_digit(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, DrawListError> {
    let raw = input.as_bytes();
    if raw.is_empty() || !raw.len().is_multiple_of(4) {
        return Err(DrawListError::InvalidBase64);
    }
    let mut out = Vec::with_capacity((raw.len() / 4) * 3);
    for (idx, chunk) in raw.chunks_exact(4).enumerate() {
        let is_last = (idx + 1) * 4 == raw.len();
        let a = decode_b64_digit(chunk[0]).ok_or(DrawListError::InvalidBase64)?;
        let b = decode_b64_digit(chunk[1]).ok_or(DrawListError::InvalidBase64)?;
        if chunk[2] == b'=' {
            if !is_last || chunk[3] != b'=' {
                return Err(DrawListError::InvalidBase64);
            }
            out.push((a << 2) | (b >> 4));
        } else {
            let c = decode_b64_digit(chunk[2]).ok_or(DrawListError::InvalidBase64)?;
            out.push((a << 2) | (b >> 4));
            if chunk[3] == b'=' {
                if !is_last {
                    return Err(DrawListError::InvalidBase64);
                }
                out.push((b << 4) | (c >> 2));
            } else {
                let d = decode_b64_digit(chunk[3]).ok_or(DrawListError::InvalidBase64)?;
                out.push((b << 4) | (c >> 2));
                out.push((c << 6) | d);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_every_canvas_op_and_raw_prop_formats() {
        let mut builder = DrawListBuilder::new();
        builder
            .clear(ColorRgba::rgba(11, 17, 32, 255))
            .clip_rect(4.0, 4.0, 200.0, 80.0)
            .fill_rect(8.0, 12.0, 40.0, 24.0, ColorRgba::rgb(30, 41, 59))
            .fill_rounded_rect(10.0, 10.0, 180.0, 60.0, 12.0, ColorRgba::rgb(15, 23, 42))
            .stroke_line(12.0, 40.0, 188.0, 40.0, 1.5, ColorRgba::rgba(148, 163, 184, 128))
            .stroke_polyline(
                &[(12.0, 50.0), (60.0, 32.0), (120.0, 42.0), (188.0, 18.0)],
                2.5,
                ColorRgba::rgb(56, 189, 248),
            )
            .fill_circle(188.0, 18.0, 4.0, ColorRgba::rgb(34, 197, 94))
            .restore_clip();

        assert_eq!(builder.op_count(), 8);
        let attr = builder.clone().to_attr_string();
        let raw_prop = builder.clone().to_raw_prop();
        let bytes = builder.finish();

        assert!(bytes.starts_with(&NVE_DRAW_MAGIC));
        let header = DrawListDecoder::decode_header(&bytes).expect("valid header");
        assert_eq!(header.version, NVE_DRAW_VERSION);
        assert_eq!(header.op_count, 8);

        let decoded = DrawListDecoder::decode(&bytes).expect("decode bytes");
        assert_eq!(decoded.len(), 8);
        assert_eq!(
            decoded[0],
            CanvasOp::Clear {
                color: ColorRgba::rgba(11, 17, 32, 255)
            }
        );
        assert_eq!(
            decoded[1],
            CanvasOp::ClipRect {
                x: 4.0,
                y: 4.0,
                w: 200.0,
                h: 80.0
            }
        );
        assert_eq!(decoded[7], CanvasOp::RestoreClip);

        // Both RawPropValue::Bytes and prefixed base64 String decode identically.
        assert_eq!(DrawListDecoder::decode_raw_prop(&raw_prop).unwrap(), decoded);
        assert_eq!(
            DrawListDecoder::decode_raw_prop(&RawPropValue::String(attr.clone())).unwrap(),
            decoded
        );
        assert_eq!(DrawListDecoder::decode_attr_bytes(&attr).unwrap(), bytes);
    }

    #[test]
    fn empty_draw_list_and_image_resize_mode_roundtrip() {
        let empty = DrawListBuilder::new().finish();
        assert_eq!(empty.len(), NVE_DRAW_HEADER_SIZE);
        assert!(DrawListDecoder::decode(&empty).unwrap().is_empty());

        for mode in [
            ImageResizeMode::Cover,
            ImageResizeMode::Contain,
            ImageResizeMode::Stretch,
            ImageResizeMode::Center,
        ] {
            assert_eq!(ImageResizeMode::parse(mode.as_str()), Some(mode));
            assert_eq!(
                ImageResizeMode::parse(&mode.as_str().to_ascii_uppercase()),
                Some(mode)
            );
            assert_eq!(ImageResizeMode::from_u8(mode.to_u8()), Some(mode));
        }
        assert_eq!(ImageResizeMode::parse("invalid"), None);
        assert_eq!(ImageResizeMode::from_u8(99), None);
    }

    #[test]
    fn sparkline_handles_normal_empty_single_flat_and_non_finite_samples() {
        let stroke = ColorRgba::rgb(56, 189, 248);
        let bg = ColorRgba::rgb(17, 24, 39);

        let cases: &[&[f32]] = &[
            &[],
            &[42.0],
            &[10.0, 10.0, 10.0, 10.0],
            &[120.0, 240.0, 180.0, 310.0, 195.0, 160.0],
            &[f32::NAN, 12.0, f32::INFINITY, -5.0, f32::NEG_INFINITY, 25.0],
        ];
        for samples in cases {
            let mut b = DrawListBuilder::new();
            b.sparkline(samples, 260.0, 72.0, stroke, bg);
            let encoded = b.finish();
            let ops = DrawListDecoder::decode(&encoded).expect("sparkline decodes cleanly");
            assert!(ops.len() >= 6);
            assert_eq!(ops.last(), Some(&CanvasOp::RestoreClip));
        }

        // Normal multi-point sparkline contains a StrokePolyline with matching vertex count.
        let ops = CanvasOp::sparkline(&[10.0, 20.0, 15.0, 30.0], 200.0, 60.0, stroke, bg);
        let polyline = ops.iter().find_map(|op| match op {
            CanvasOp::StrokePolyline { points, .. } => Some(points),
            _ => None,
        });
        assert_eq!(polyline.map(Vec::len), Some(4));
    }

    #[test]
    fn decoder_rejects_every_truncation_offset_and_corrupt_headers() {
        let mut b = DrawListBuilder::new();
        b.sparkline(
            &[110.0, 165.0, 140.0, 290.0, 210.0],
            240.0,
            64.0,
            ColorRgba::rgb(167, 139, 250),
            ColorRgba::rgb(15, 23, 42),
        );
        let valid = b.finish();

        // Truncating at EVERY single byte offset before the end must return Truncated (zero panics).
        for len in 0..valid.len() {
            let err = DrawListDecoder::decode(&valid[..len]).unwrap_err();
            assert_eq!(err, DrawListError::Truncated, "failed at len={len}");
        }

        // Trailing garbage byte rejected.
        let mut trailing = valid.clone();
        trailing.push(0xAA);
        assert_eq!(
            DrawListDecoder::decode(&trailing),
            Err(DrawListError::TrailingBytes(1))
        );

        // Bad magic rejected.
        let mut bad_magic = valid.clone();
        bad_magic[0] = b'X';
        assert_eq!(DrawListDecoder::decode(&bad_magic), Err(DrawListError::BadMagic));

        // Unsupported version rejected.
        let mut bad_ver = valid.clone();
        bad_ver[9..11].copy_from_slice(&99u16.to_le_bytes());
        assert_eq!(
            DrawListDecoder::decode(&bad_ver),
            Err(DrawListError::UnsupportedVersion(99))
        );

        // Huge op_count rejected without allocating.
        let mut huge_ops = valid[..NVE_DRAW_HEADER_SIZE].to_vec();
        huge_ops[11..15].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            DrawListDecoder::decode(&huge_ops),
            Err(DrawListError::OpCountExceedsLimit {
                count: u32::MAX,
                max: MAX_DRAW_OPS
            })
        );

        // Lying op_count within MAX_DRAW_OPS but larger than remaining bytes rejected before OOM.
        let mut lying_ops = valid[..NVE_DRAW_HEADER_SIZE].to_vec();
        lying_ops[11..15].copy_from_slice(&(MAX_DRAW_OPS as u32).to_le_bytes());
        assert_eq!(
            DrawListDecoder::decode(&lying_ops),
            Err(DrawListError::Truncated)
        );

        // Huge polyline point count rejected without allocating.
        let mut huge_poly = Vec::new();
        huge_poly.extend_from_slice(&NVE_DRAW_MAGIC);
        huge_poly.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
        huge_poly.extend_from_slice(&1u32.to_le_bytes());
        huge_poly.push(DrawOpcode::StrokePolyline as u8);
        huge_poly.extend_from_slice(&2.0f32.to_le_bytes());
        huge_poly.extend_from_slice(&ColorRgba::WHITE.to_u32().to_le_bytes());
        huge_poly.extend_from_slice(&((MAX_POLYLINE_POINTS as u32) + 1).to_le_bytes());
        assert_eq!(
            DrawListDecoder::decode(&huge_poly),
            Err(DrawListError::PolylinePointCountExceedsLimit {
                count: (MAX_POLYLINE_POINTS as u32) + 1,
                max: MAX_POLYLINE_POINTS
            })
        );

        // Unknown opcode rejected.
        let mut bad_op = Vec::new();
        bad_op.extend_from_slice(&NVE_DRAW_MAGIC);
        bad_op.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
        bad_op.extend_from_slice(&1u32.to_le_bytes());
        bad_op.push(0xFE);
        assert_eq!(
            DrawListDecoder::decode(&bad_op),
            Err(DrawListError::UnknownOpcode(0xFE))
        );

        // Non-finite float (NaN / Inf) rejected.
        for bad_float in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut buf = Vec::new();
            buf.extend_from_slice(&NVE_DRAW_MAGIC);
            buf.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
            buf.extend_from_slice(&1u32.to_le_bytes());
            buf.push(DrawOpcode::FillCircle as u8);
            buf.extend_from_slice(&bad_float.to_le_bytes());
            buf.extend_from_slice(&10.0f32.to_le_bytes());
            buf.extend_from_slice(&5.0f32.to_le_bytes());
            buf.extend_from_slice(&ColorRgba::WHITE.to_u32().to_le_bytes());
            assert_eq!(
                DrawListDecoder::decode(&buf),
                Err(DrawListError::NonFiniteFloat)
            );
        }

        // Invalid base64 / unsupported prop values rejected cleanly.
        assert_eq!(
            DrawListDecoder::decode_attr_bytes("nve-draw1;b64,!!!"),
            Err(DrawListError::InvalidBase64)
        );
        assert_eq!(
            DrawListDecoder::decode_raw_prop(&RawPropValue::Bool(true)),
            Err(DrawListError::UnsupportedPropValue)
        );
    }

    #[test]
    fn fuzz_arbitrary_and_mutated_draw_list_buffers_never_panic() {
        let mut base = DrawListBuilder::new();
        base.sparkline(
            &[10.0, 40.0, 22.0, 55.0],
            180.0,
            60.0,
            ColorRgba::rgb(34, 197, 94),
            ColorRgba::rgb(17, 24, 39),
        );
        let seed_buf = base.finish();

        // Deterministic xorshift64 fuzzing across 2,000 random & bit-flipped inputs.
        let mut rng: u64 = 0x5EED_CAFE_BABE_1234;
        let next_u64 = |state: &mut u64| -> u64 {
            let mut x = *state;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *state = x;
            x
        };

        for iter in 0..2_000usize {
            let mut buf = if iter % 2 == 0 {
                // Mutate a valid header/body buffer.
                let mut m = seed_buf.clone();
                let flips = (next_u64(&mut rng) % 4 + 1) as usize;
                for _ in 0..flips {
                    let pos = (next_u64(&mut rng) as usize) % m.len();
                    m[pos] ^= (next_u64(&mut rng) & 0xFF) as u8;
                }
                let cut = (next_u64(&mut rng) as usize) % (m.len() + 1);
                m.truncate(cut);
                m
            } else {
                // Valid magic + version + random trailing payload.
                let len = (next_u64(&mut rng) % 96) as usize;
                let mut m = Vec::with_capacity(NVE_DRAW_HEADER_SIZE + len);
                m.extend_from_slice(&NVE_DRAW_MAGIC);
                m.extend_from_slice(&NVE_DRAW_VERSION.to_le_bytes());
                for _ in 0..(4 + len) {
                    m.push((next_u64(&mut rng) & 0xFF) as u8);
                }
                m
            };

            let _ = DrawListDecoder::decode(&buf);
            // Also fuzz valid header with random raw bytes reversed.
            buf.reverse();
            let _ = DrawListDecoder::decode(&buf);
        }
    }
}
