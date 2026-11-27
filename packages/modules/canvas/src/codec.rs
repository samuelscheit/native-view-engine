//! Binary encoder (`DrawListBuilder`) and bounds-checked decoder (`DrawListDecoder`) for `NVE_DRAW1`.

use nve_abi::ColorRgba;

use super::base64::{decode_base64, encode_base64_into};
use super::types::{
    sanitize_finite, CanvasOp, DrawListError, DrawListHeader, DrawOpcode, DRAW_LIST_ATTR_PREFIX,
    MAX_DRAW_OPS, MAX_POLYLINE_POINTS, NVE_DRAW_HEADER_SIZE, NVE_DRAW_MAGIC, NVE_DRAW_VERSION,
};

/// Binary flat binary encoder producing `NVE_DRAW1` byte buffers directly into a single `Vec<u8>`.
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

    /// Encodes the finished `NVE_DRAW1` binary buffer as a prefixed base64 string for UTF-8 RSX
    /// attributes (`nve-draw1;b64,...`).
    pub fn to_attr_string(self) -> String {
        let bytes = self.finish();
        let mut out =
            String::with_capacity(DRAW_LIST_ATTR_PREFIX.len() + bytes.len().div_ceil(3) * 4);
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
