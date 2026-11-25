//! Branch-free per-filter-type PNG scanline unfiltering and RGBA8888 row expansion.

use super::png::PngHeader;
use super::ImageDecodeError;

/// Unfilters one PNG scanline using per-filter-type row loops with zero per-byte `i >= bpp`
/// branching in the inner body.
pub(crate) fn unfilter_scanline(
    filter: u8,
    src: &[u8],
    prev: &[u8],
    dst: &mut [u8],
    bpp: usize,
) -> Result<(), ImageDecodeError> {
    let len = src.len();
    debug_assert_eq!(prev.len(), len);
    debug_assert_eq!(dst.len(), len);

    match filter {
        0 => unfilter_row_none(src, dst),
        1 => unfilter_row_sub(src, dst, bpp),
        2 => unfilter_row_up(src, prev, dst),
        3 => unfilter_row_average(src, prev, dst, bpp),
        4 => unfilter_row_paeth(src, prev, dst, bpp),
        other => {
            return Err(ImageDecodeError::InvalidPng(format!(
                "unknown PNG scanline filter {other}"
            )));
        }
    }
    Ok(())
}

#[inline]
fn unfilter_row_none(src: &[u8], dst: &mut [u8]) {
    dst.copy_from_slice(src);
}

#[inline]
fn unfilter_row_sub(src: &[u8], dst: &mut [u8], bpp: usize) {
    let head = bpp.min(src.len());
    dst[..head].copy_from_slice(&src[..head]);
    for i in head..src.len() {
        dst[i] = src[i].wrapping_add(dst[i - bpp]);
    }
}

#[inline]
fn unfilter_row_up(src: &[u8], prev: &[u8], dst: &mut [u8]) {
    for ((d, &s), &p) in dst.iter_mut().zip(src.iter()).zip(prev.iter()) {
        *d = s.wrapping_add(p);
    }
}

#[inline]
fn unfilter_row_average(src: &[u8], prev: &[u8], dst: &mut [u8], bpp: usize) {
    let head = bpp.min(src.len());
    for ((d, &s), &p) in dst[..head]
        .iter_mut()
        .zip(&src[..head])
        .zip(&prev[..head])
    {
        *d = s.wrapping_add(p >> 1);
    }
    for i in head..src.len() {
        let avg = ((dst[i - bpp] as u16 + prev[i] as u16) >> 1) as u8;
        dst[i] = src[i].wrapping_add(avg);
    }
}

#[inline]
fn unfilter_row_paeth(src: &[u8], prev: &[u8], dst: &mut [u8], bpp: usize) {
    let head = bpp.min(src.len());
    // For i < bpp, left (a) = 0 and upper-left (c) = 0, so Paeth(0, b, 0) == b identically.
    for ((d, &s), &p) in dst[..head]
        .iter_mut()
        .zip(&src[..head])
        .zip(&prev[..head])
    {
        *d = s.wrapping_add(p);
    }
    for i in head..src.len() {
        let a = dst[i - bpp];
        let b = prev[i];
        let c = prev[i - bpp];
        dst[i] = src[i].wrapping_add(paeth_predictor(a, b, c));
    }
}

#[inline]
pub(crate) fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

#[inline]
fn read_packed_sample(row: &[u8], bit_depth: u8, index: usize) -> Result<u8, ImageDecodeError> {
    let sample = match bit_depth {
        1 => {
            let b = *row
                .get(index >> 3)
                .ok_or_else(|| ImageDecodeError::InvalidPng("scanline read out of bounds".to_string()))?;
            (b >> (7 - (index & 7))) & 1
        }
        2 => {
            let b = *row
                .get(index >> 2)
                .ok_or_else(|| ImageDecodeError::InvalidPng("scanline read out of bounds".to_string()))?;
            (b >> (6 - ((index & 3) * 2))) & 3
        }
        4 => {
            let b = *row
                .get(index >> 1)
                .ok_or_else(|| ImageDecodeError::InvalidPng("scanline read out of bounds".to_string()))?;
            (b >> (4 - ((index & 1) * 4))) & 15
        }
        8 => *row
            .get(index)
            .ok_or_else(|| ImageDecodeError::InvalidPng("scanline read out of bounds".to_string()))?,
        _ => 0,
    };
    Ok(sample)
}

#[inline]
fn read_row_slice(row: &[u8], px: usize, stride: usize) -> Result<&[u8], ImageDecodeError> {
    let start = px
        .checked_mul(stride)
        .ok_or_else(|| ImageDecodeError::InvalidPng("scanline pixel offset overflow".to_string()))?;
    let end = start
        .checked_add(stride)
        .ok_or_else(|| ImageDecodeError::InvalidPng("scanline pixel offset overflow".to_string()))?;
    row.get(start..end)
        .ok_or_else(|| ImageDecodeError::InvalidPng("scanline read out of bounds".to_string()))
}

#[inline]
fn scale_subbyte_gray(v: u8, bit_depth: u8) -> u8 {
    match bit_depth {
        1 => v * 255,
        2 => v * 85,
        4 => v * 17,
        _ => v,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn expand_row_to_rgba(
    hdr: PngHeader,
    row: &[u8],
    pass_w: u32,
    palette: Option<&[[u8; 3]]>,
    trns: Option<&[u8]>,
    x_start: u32,
    x_step: u32,
    row_dst_base: usize,
    out_rgba: &mut [u8],
) -> Result<(), ImageDecodeError> {
    let trns_gray16 = trns
        .filter(|t| t.len() >= 2)
        .map(|t| u16::from_be_bytes([t[0], t[1]]));
    let trns_rgb16 = trns.filter(|t| t.len() >= 6).map(|t| {
        (
            u16::from_be_bytes([t[0], t[1]]),
            u16::from_be_bytes([t[2], t[3]]),
            u16::from_be_bytes([t[4], t[5]]),
        )
    });

    for px in 0..pass_w as usize {
        let (r, g, b, a) = match (hdr.color_type, hdr.bit_depth) {
            (0, d @ (1 | 2 | 4 | 8)) => {
                let raw = read_packed_sample(row, d, px)?;
                let g8 = scale_subbyte_gray(raw, d);
                let alpha = if trns_gray16 == Some(raw as u16) { 0 } else { 255 };
                (g8, g8, g8, alpha)
            }
            (0, 16) => {
                let s = read_row_slice(row, px, 2)?;
                let raw = u16::from_be_bytes([s[0], s[1]]);
                let g8 = (raw >> 8) as u8;
                let alpha = if trns_gray16 == Some(raw) { 0 } else { 255 };
                (g8, g8, g8, alpha)
            }
            (2, 8) => {
                let s = read_row_slice(row, px, 3)?;
                let (r, g, b) = (s[0], s[1], s[2]);
                let alpha = if trns_rgb16 == Some((r as u16, g as u16, b as u16)) {
                    0
                } else {
                    255
                };
                (r, g, b, alpha)
            }
            (2, 16) => {
                let s = read_row_slice(row, px, 6)?;
                let r16 = u16::from_be_bytes([s[0], s[1]]);
                let g16 = u16::from_be_bytes([s[2], s[3]]);
                let b16 = u16::from_be_bytes([s[4], s[5]]);
                let alpha = if trns_rgb16 == Some((r16, g16, b16)) { 0 } else { 255 };
                ((r16 >> 8) as u8, (g16 >> 8) as u8, (b16 >> 8) as u8, alpha)
            }
            (3, d @ (1 | 2 | 4 | 8)) => {
                let idx = read_packed_sample(row, d, px)? as usize;
                let pal = palette.ok_or_else(|| {
                    ImageDecodeError::InvalidPng("missing PLTE for indexed PNG".to_string())
                })?;
                let rgb = *pal.get(idx).ok_or_else(|| {
                    ImageDecodeError::InvalidPng(format!("palette index {idx} out of bounds"))
                })?;
                let alpha = trns.and_then(|t| t.get(idx).copied()).unwrap_or(255);
                (rgb[0], rgb[1], rgb[2], alpha)
            }
            (4, 8) => {
                let s = read_row_slice(row, px, 2)?;
                (s[0], s[0], s[0], s[1])
            }
            (4, 16) => {
                let s = read_row_slice(row, px, 4)?;
                (s[0], s[0], s[0], s[2])
            }
            (6, 8) => {
                let s = read_row_slice(row, px, 4)?;
                (s[0], s[1], s[2], s[3])
            }
            (6, 16) => {
                let s = read_row_slice(row, px, 8)?;
                (s[0], s[2], s[4], s[6])
            }
            _ => {
                return Err(ImageDecodeError::InvalidPng(
                    "unsupported color_type/bit_depth combination".to_string(),
                ));
            }
        };

        let px_u32 = u32::try_from(px)
            .map_err(|_| ImageDecodeError::InvalidPng("Adam7 X coordinate overflow".to_string()))?;
        let dst_x = px_u32
            .checked_mul(x_step)
            .and_then(|dx| x_start.checked_add(dx))
            .ok_or_else(|| ImageDecodeError::InvalidPng("Adam7 X coordinate overflow".to_string()))?;
        if dst_x >= hdr.width {
            return Err(ImageDecodeError::InvalidPng(
                "Adam7 X coordinate out of bounds".to_string(),
            ));
        }
        let out_idx = row_dst_base
            .checked_add(dst_x as usize)
            .and_then(|p| p.checked_mul(4))
            .ok_or_else(|| ImageDecodeError::InvalidPng("RGBA buffer index overflow".to_string()))?;
        let out_end = out_idx
            .checked_add(4)
            .ok_or_else(|| ImageDecodeError::InvalidPng("RGBA buffer index overflow".to_string()))?;
        let dst_px = out_rgba.get_mut(out_idx..out_end).ok_or_else(|| {
            ImageDecodeError::InvalidPng("RGBA pixel write out of bounds".to_string())
        })?;
        dst_px[0] = r;
        dst_px[1] = g;
        dst_px[2] = b;
        dst_px[3] = a;
    }

    Ok(())
}
