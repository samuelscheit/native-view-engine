//! Binary encoder (`DrawListBuilder`) and bounds-checked decoder (`DrawListDecoder`) for `NVE_DRAW1`.

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use nve_abi::ColorRgba;
use nve_codec::{ByteReader, ByteWriter, CodecError};

use super::types::{
    sanitize_finite, CanvasOp, DrawListError, DrawListHeader, DrawOpcode, DRAW_LIST_ATTR_PREFIX,
    MAX_DRAW_OPS, MAX_POLYLINE_POINTS, NVE_DRAW_HEADER_SIZE, NVE_DRAW_MAGIC, NVE_DRAW_VERSION,
};

#[inline]
fn map_err(err: CodecError) -> DrawListError {
    match err {
        CodecError::BadMagic => DrawListError::BadMagic,
        CodecError::TrailingBytes(n) => DrawListError::TrailingBytes(n),
        CodecError::BadEnum { raw, .. } => DrawListError::UnknownOpcode(raw as u8),
        _ => DrawListError::Truncated,
    }
}

/// Binary flat encoder producing `NVE_DRAW1` byte buffers directly into a single `ByteWriter`.
#[derive(Debug, Clone)]
pub struct DrawListBuilder {
    writer: ByteWriter,
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
        let mut writer = ByteWriter::with_capacity(capacity_bytes.max(NVE_DRAW_HEADER_SIZE));
        writer.write_raw(&NVE_DRAW_MAGIC);
        writer.write_u16(NVE_DRAW_VERSION);
        writer.write_u32(0);
        debug_assert_eq!(writer.len(), NVE_DRAW_HEADER_SIZE);
        Self {
            writer,
            op_count: 0,
        }
    }

    pub fn op_count(&self) -> u32 {
        self.op_count
    }

    pub fn is_empty(&self) -> bool {
        self.op_count == 0
    }

    fn write_f32(&mut self, v: f32) {
        self.writer.write_f32(sanitize_finite(v));
    }

    fn write_color(&mut self, c: ColorRgba) {
        self.writer.write_u32(c.to_u32());
    }

    pub fn clear(&mut self, color: ColorRgba) -> &mut Self {
        self.writer.write_u8(DrawOpcode::Clear as u8);
        self.write_color(color);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) -> &mut Self {
        self.writer.write_u8(DrawOpcode::FillRect as u8);
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
        self.writer.write_u8(DrawOpcode::FillRoundedRect as u8);
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
        self.writer.write_u8(DrawOpcode::FillCircle as u8);
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
        self.writer.write_u8(DrawOpcode::StrokeLine as u8);
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
        self.writer.write_u8(DrawOpcode::StrokePolyline as u8);
        self.write_f32(width);
        self.write_color(color);
        self.writer.write_u32(clamped.len() as u32);
        for &(px, py) in clamped {
            self.write_f32(px);
            self.write_f32(py);
        }
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        self.writer.write_u8(DrawOpcode::ClipRect as u8);
        self.write_f32(x);
        self.write_f32(y);
        self.write_f32(w);
        self.write_f32(h);
        self.op_count = self.op_count.saturating_add(1);
        self
    }

    pub fn restore_clip(&mut self) -> &mut Self {
        self.writer.write_u8(DrawOpcode::RestoreClip as u8);
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
        self.writer
            .patch_u32(11, self.op_count)
            .expect("NVE_DRAW1 header always occupies 15 bytes");
        self.writer.into_bytes()
    }

    /// Encodes the finished `NVE_DRAW1` binary buffer as a prefixed base64 string for UTF-8 RSX
    /// attributes (`nve-draw1;b64,...`).
    pub fn to_attr_string(self) -> String {
        let bytes = self.finish();
        let mut out =
            String::with_capacity(DRAW_LIST_ATTR_PREFIX.len() + bytes.len().div_ceil(3) * 4);
        out.push_str(DRAW_LIST_ATTR_PREFIX);
        BASE64_STANDARD.encode_string(&bytes, &mut out);
        out
    }
}

/// Strict bounds-checked decoder for `NVE_DRAW1` binary vector draw-list streams.
pub struct DrawListDecoder;

impl DrawListDecoder {
    /// Parses and validates only the fixed 15-byte `NVE_DRAW1` header.
    pub fn decode_header(bytes: &[u8]) -> Result<DrawListHeader, DrawListError> {
        let mut r = ByteReader::new(bytes);
        Self::decode_header_from(&mut r)
    }

    fn decode_header_from(r: &mut ByteReader<'_>) -> Result<DrawListHeader, DrawListError> {
        r.ensure_remaining(NVE_DRAW_HEADER_SIZE).map_err(map_err)?;
        r.expect_magic(&NVE_DRAW_MAGIC).map_err(map_err)?;
        let version = r.read_u16().map_err(map_err)?;
        if version != NVE_DRAW_VERSION {
            return Err(DrawListError::UnsupportedVersion(version));
        }
        let op_count = r.read_u32().map_err(map_err)?;
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
        let mut r = ByteReader::new(bytes);
        let header = Self::decode_header_from(&mut r)?;
        let op_count = header.op_count as usize;
        // Every operation occupies at least 1 byte (its opcode tag), so reject lying op counts
        // before reserving heap memory.
        if r.remaining() < op_count {
            return Err(DrawListError::Truncated);
        }

        let mut ops = Vec::with_capacity(op_count.min(1024));
        for _ in 0..op_count {
            let tag = r.read_u8().map_err(map_err)?;
            let op = match DrawOpcode::from_repr(tag).ok_or(DrawListError::UnknownOpcode(tag))? {
                DrawOpcode::Clear => CanvasOp::Clear {
                    color: read_color(&mut r)?,
                },
                DrawOpcode::FillRect => CanvasOp::FillRect {
                    x: read_finite_f32(&mut r)?,
                    y: read_finite_f32(&mut r)?,
                    w: read_finite_f32(&mut r)?,
                    h: read_finite_f32(&mut r)?,
                    color: read_color(&mut r)?,
                },
                DrawOpcode::FillRoundedRect => CanvasOp::FillRoundedRect {
                    x: read_finite_f32(&mut r)?,
                    y: read_finite_f32(&mut r)?,
                    w: read_finite_f32(&mut r)?,
                    h: read_finite_f32(&mut r)?,
                    radius: read_finite_f32(&mut r)?,
                    color: read_color(&mut r)?,
                },
                DrawOpcode::FillCircle => CanvasOp::FillCircle {
                    cx: read_finite_f32(&mut r)?,
                    cy: read_finite_f32(&mut r)?,
                    radius: read_finite_f32(&mut r)?,
                    color: read_color(&mut r)?,
                },
                DrawOpcode::StrokeLine => CanvasOp::StrokeLine {
                    x0: read_finite_f32(&mut r)?,
                    y0: read_finite_f32(&mut r)?,
                    x1: read_finite_f32(&mut r)?,
                    y1: read_finite_f32(&mut r)?,
                    width: read_finite_f32(&mut r)?,
                    color: read_color(&mut r)?,
                },
                DrawOpcode::StrokePolyline => {
                    let width = read_finite_f32(&mut r)?;
                    let color = read_color(&mut r)?;
                    let pt_count = r.read_u32().map_err(map_err)?;
                    if (pt_count as usize) > MAX_POLYLINE_POINTS {
                        return Err(DrawListError::PolylinePointCountExceedsLimit {
                            count: pt_count,
                            max: MAX_POLYLINE_POINTS,
                        });
                    }
                    let needed_bytes = (pt_count as usize)
                        .checked_mul(8)
                        .ok_or(DrawListError::Truncated)?;
                    r.ensure_remaining(needed_bytes).map_err(map_err)?;
                    let mut points = Vec::with_capacity(pt_count as usize);
                    for _ in 0..pt_count {
                        let px = read_finite_f32(&mut r)?;
                        let py = read_finite_f32(&mut r)?;
                        points.push((px, py));
                    }
                    CanvasOp::StrokePolyline {
                        points,
                        width,
                        color,
                    }
                }
                DrawOpcode::ClipRect => CanvasOp::ClipRect {
                    x: read_finite_f32(&mut r)?,
                    y: read_finite_f32(&mut r)?,
                    w: read_finite_f32(&mut r)?,
                    h: read_finite_f32(&mut r)?,
                },
                DrawOpcode::RestoreClip => CanvasOp::RestoreClip,
            };
            ops.push(op);
        }

        r.finish().map_err(map_err)?;
        Ok(ops)
    }

    /// Extracts raw `NVE_DRAW1` bytes from a prefixed or bare base64 attribute string.
    pub fn decode_attr_bytes(raw: &str) -> Result<Vec<u8>, DrawListError> {
        let b64 = raw.strip_prefix(DRAW_LIST_ATTR_PREFIX).unwrap_or(raw);
        if b64.is_empty() {
            return Err(DrawListError::InvalidBase64);
        }
        let bytes = BASE64_STANDARD
            .decode(b64)
            .map_err(|_| DrawListError::InvalidBase64)?;
        let _ = Self::decode_header(&bytes)?;
        Ok(bytes)
    }
}

#[inline]
fn read_finite_f32(r: &mut ByteReader<'_>) -> Result<f32, DrawListError> {
    let v = r.read_f32_raw().map_err(map_err)?;
    if !v.is_finite() {
        return Err(DrawListError::NonFiniteFloat);
    }
    Ok(v)
}

#[inline]
fn read_color(r: &mut ByteReader<'_>) -> Result<ColorRgba, DrawListError> {
    Ok(ColorRgba::from_u32(r.read_u32().map_err(map_err)?))
}
