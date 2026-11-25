//! Strictly bounds-checked RFC 2083 PNG encoder & decoder with branch-free per-filter-type
//! scanline unfiltering loops.

use std::io::{Cursor, Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use super::png_rows::{expand_row_to_rgba, unfilter_scanline};
use super::{checked_rgba_len, DecodedImage, ImageDecodeError};

/// Maximum compressed IDAT payload accepted from a PNG stream (64 MiB).
const MAX_COMPRESSED_IDAT_BYTES: usize = 64 * 1024 * 1024;
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

const ADAM7_PASSES: [(u32, u32, u32, u32); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

const fn make_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const CRC32_TABLE: [u32; 256] = make_crc32_table();

fn crc32_ieee(chunk_type: &[u8], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in chunk_type.iter().chain(data.iter()) {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    !crc
}

/// Encodes a tight 24-bit RGB buffer (`width * height * 3` bytes) into a standard RFC 2083 PNG.
pub fn encode_rgb_png(width: u32, height: u32, rgb: &[u8]) -> std::io::Result<Vec<u8>> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(3))
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "PNG dimensions overflow")
        })?;
    if width == 0 || height == 0 || rgb.len() != expected {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "invalid RGB buffer length {} for {}x{} PNG",
                rgb.len(),
                width,
                height
            ),
        ));
    }
    let stride = (width as usize) * 3;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    for y in 0..height as usize {
        encoder.write_all(&[0u8])?;
        encoder.write_all(&rgb[y * stride..(y + 1) * stride])?;
    }
    let idat = encoder.finish()?;

    let mut out = Vec::with_capacity(PNG_SIGNATURE.len() + 57 + idat.len());
    out.extend_from_slice(&PNG_SIGNATURE);

    let push_chunk = |dst: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]| {
        dst.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        dst.extend_from_slice(tag);
        dst.extend_from_slice(payload);
        let crc = crc32_ieee(tag, payload);
        dst.extend_from_slice(&crc.to_be_bytes());
    };

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 2; // truecolor RGB
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &idat);
    push_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PngHeader {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bit_depth: u8,
    pub(crate) color_type: u8,
    pub(crate) interlace: u8,
}

impl PngHeader {
    fn channels(self) -> usize {
        match self.color_type {
            0 => 1, // Grayscale
            2 => 3, // Truecolor RGB
            3 => 1, // Indexed-color
            4 => 2, // Grayscale + alpha
            6 => 4, // Truecolor + alpha
            _ => 0,
        }
    }

    fn bits_per_pixel(self) -> usize {
        self.channels() * (self.bit_depth as usize)
    }

    fn filter_bpp(self) -> usize {
        self.bits_per_pixel().div_ceil(8).max(1)
    }

    fn scanline_bytes(self, pass_width: u32) -> Result<usize, ImageDecodeError> {
        let bits = (pass_width as usize)
            .checked_mul(self.bits_per_pixel())
            .ok_or_else(|| ImageDecodeError::InvalidPng("scanline size overflow".to_string()))?;
        Ok(bits.div_ceil(8))
    }
}

/// Strictly bounds-checked RFC 2083 PNG decoder producing straight-alpha RGBA8888 pixels.
///
/// Never panics on truncated, corrupted, or adversarial PNG payloads.
pub fn decode_png(bytes: &[u8]) -> Result<DecodedImage, ImageDecodeError> {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..8] != PNG_SIGNATURE {
        return Err(ImageDecodeError::InvalidPng("bad PNG signature".to_string()));
    }

    let mut pos = 8usize;
    let mut header: Option<PngHeader> = None;
    let mut palette: Option<Vec<[u8; 3]>> = None;
    let mut trns: Option<Vec<u8>> = None;
    let mut idat: Vec<u8> = Vec::new();
    let mut saw_iend = false;

    while pos < bytes.len() {
        if bytes.len() - pos < 12 {
            return Err(ImageDecodeError::InvalidPng(
                "truncated PNG chunk header".to_string(),
            ));
        }
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let chunk_type = &bytes[pos + 4..pos + 8];
        pos += 8;

        let chunk_total = len
            .checked_add(4)
            .ok_or_else(|| ImageDecodeError::InvalidPng("PNG chunk length overflow".to_string()))?;
        if bytes.len() - pos < chunk_total {
            return Err(ImageDecodeError::InvalidPng(
                "truncated PNG chunk payload".to_string(),
            ));
        }
        let data_end = pos
            .checked_add(len)
            .ok_or_else(|| ImageDecodeError::InvalidPng("PNG chunk offset overflow".to_string()))?;
        let crc_end = pos
            .checked_add(chunk_total)
            .ok_or_else(|| ImageDecodeError::InvalidPng("PNG chunk offset overflow".to_string()))?;
        let data = bytes
            .get(pos..data_end)
            .ok_or_else(|| ImageDecodeError::InvalidPng("truncated PNG chunk payload".to_string()))?;
        let crc_bytes = bytes
            .get(data_end..crc_end)
            .ok_or_else(|| ImageDecodeError::InvalidPng("truncated PNG chunk CRC".to_string()))?;
        let expected_crc =
            u32::from_be_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);
        pos = crc_end;

        let actual_crc = crc32_ieee(chunk_type, data);
        if actual_crc != expected_crc {
            return Err(ImageDecodeError::InvalidPng(format!(
                "CRC mismatch in chunk {:?}",
                std::str::from_utf8(chunk_type).unwrap_or("????")
            )));
        }

        match chunk_type {
            b"IHDR" => {
                if header.is_some() || data.len() != 13 {
                    return Err(ImageDecodeError::InvalidPng("invalid IHDR chunk".to_string()));
                }
                let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                let _ = checked_rgba_len(width, height)?;
                let bit_depth = data[8];
                let color_type = data[9];
                let compression = data[10];
                let filter = data[11];
                let interlace = data[12];

                if compression != 0 || filter != 0 || interlace > 1 {
                    return Err(ImageDecodeError::InvalidPng(
                        "unsupported PNG compression/filter/interlace method".to_string(),
                    ));
                }
                let valid_depth = match color_type {
                    0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
                    2 | 4 | 6 => matches!(bit_depth, 8 | 16),
                    3 => matches!(bit_depth, 1 | 2 | 4 | 8),
                    _ => false,
                };
                if !valid_depth {
                    return Err(ImageDecodeError::InvalidPng(format!(
                        "invalid PNG color_type={color_type} bit_depth={bit_depth}"
                    )));
                }
                header = Some(PngHeader {
                    width,
                    height,
                    bit_depth,
                    color_type,
                    interlace,
                });
            }
            b"PLTE" => {
                if header.is_none()
                    || data.is_empty()
                    || !data.len().is_multiple_of(3)
                    || data.len() > 256 * 3
                {
                    return Err(ImageDecodeError::InvalidPng("invalid PLTE chunk".to_string()));
                }
                let mut pal = Vec::with_capacity(data.len() / 3);
                for &rgb in data.as_chunks::<3>().0 {
                    pal.push(rgb);
                }
                palette = Some(pal);
            }
            b"tRNS" => {
                if header.is_none() {
                    return Err(ImageDecodeError::InvalidPng(
                        "tRNS chunk before IHDR".to_string(),
                    ));
                }
                trns = Some(data.to_vec());
            }
            b"IDAT" => {
                if header.is_none() {
                    return Err(ImageDecodeError::InvalidPng(
                        "IDAT chunk before IHDR".to_string(),
                    ));
                }
                if idat.len().saturating_add(data.len()) > MAX_COMPRESSED_IDAT_BYTES {
                    return Err(ImageDecodeError::InvalidPng(
                        "IDAT payload exceeds safety limit".to_string(),
                    ));
                }
                idat.extend_from_slice(data);
            }
            b"IEND" => {
                saw_iend = true;
                break;
            }
            _ => {
                if header.is_none() {
                    return Err(ImageDecodeError::InvalidPng(
                        "missing IHDR as first chunk".to_string(),
                    ));
                }
            }
        }
    }

    let hdr = header.ok_or_else(|| ImageDecodeError::InvalidPng("missing IHDR".to_string()))?;
    if !saw_iend || idat.is_empty() {
        return Err(ImageDecodeError::InvalidPng(
            "missing IDAT or IEND chunk".to_string(),
        ));
    }
    if hdr.color_type == 3 && palette.is_none() {
        return Err(ImageDecodeError::InvalidPng(
            "indexed PNG missing PLTE chunk".to_string(),
        ));
    }

    let expected_raw = expected_decompressed_size(hdr)?;
    let mut inflated = Vec::with_capacity(expected_raw.min(1024 * 1024));
    let mut decoder = ZlibDecoder::new(Cursor::new(&idat)).take((expected_raw as u64) + 1);
    decoder
        .read_to_end(&mut inflated)
        .map_err(|e| ImageDecodeError::InvalidPng(format!("zlib inflate error: {e}")))?;
    if inflated.len() < expected_raw {
        return Err(ImageDecodeError::InvalidPng(format!(
            "inflated PNG data truncated: expected {expected_raw} bytes, got {}",
            inflated.len()
        )));
    }

    let out_len = checked_rgba_len(hdr.width, hdr.height)?;
    let mut rgba = vec![0u8; out_len];

    if hdr.interlace == 0 {
        decode_pass(
            hdr,
            &inflated[..expected_raw],
            palette.as_deref(),
            trns.as_deref(),
            0,
            0,
            1,
            1,
            hdr.width,
            hdr.height,
            &mut rgba,
        )?;
    } else {
        let mut cursor = 0usize;
        for &(xs, ys, xstep, ystep) in &ADAM7_PASSES {
            if xs >= hdr.width || ys >= hdr.height {
                continue;
            }
            let pw = (hdr.width - xs).div_ceil(xstep);
            let ph = (hdr.height - ys).div_ceil(ystep);
            let row_bytes = hdr.scanline_bytes(pw)?;
            let stride = row_bytes
                .checked_add(1)
                .ok_or_else(|| ImageDecodeError::InvalidPng("pass stride overflow".to_string()))?;
            let pass_bytes = (ph as usize)
                .checked_mul(stride)
                .ok_or_else(|| ImageDecodeError::InvalidPng("pass size overflow".to_string()))?;
            let end = cursor
                .checked_add(pass_bytes)
                .ok_or_else(|| ImageDecodeError::InvalidPng("Adam7 cursor overflow".to_string()))?;
            let slice = inflated
                .get(cursor..end)
                .ok_or_else(|| ImageDecodeError::InvalidPng("truncated Adam7 pass".to_string()))?;
            decode_pass(
                hdr,
                slice,
                palette.as_deref(),
                trns.as_deref(),
                xs,
                ys,
                xstep,
                ystep,
                pw,
                ph,
                &mut rgba,
            )?;
            cursor = end;
        }
    }

    Ok(DecodedImage {
        width: hdr.width,
        height: hdr.height,
        rgba,
    })
}

fn expected_decompressed_size(hdr: PngHeader) -> Result<usize, ImageDecodeError> {
    if hdr.interlace == 0 {
        let row_bytes = hdr.scanline_bytes(hdr.width)?;
        let stride = row_bytes.checked_add(1).ok_or_else(|| {
            ImageDecodeError::InvalidPng("decompressed size overflow".to_string())
        })?;
        return (hdr.height as usize)
            .checked_mul(stride)
            .ok_or_else(|| ImageDecodeError::InvalidPng("decompressed size overflow".to_string()));
    }
    let mut total = 0usize;
    for &(xs, ys, xstep, ystep) in &ADAM7_PASSES {
        if xs >= hdr.width || ys >= hdr.height {
            continue;
        }
        let pw = (hdr.width - xs).div_ceil(xstep);
        let ph = (hdr.height - ys).div_ceil(ystep);
        let row_bytes = hdr.scanline_bytes(pw)?;
        let stride = row_bytes.checked_add(1).ok_or_else(|| {
            ImageDecodeError::InvalidPng("decompressed size overflow".to_string())
        })?;
        let pass_bytes = (ph as usize)
            .checked_mul(stride)
            .ok_or_else(|| ImageDecodeError::InvalidPng("decompressed size overflow".to_string()))?;
        total = total
            .checked_add(pass_bytes)
            .ok_or_else(|| ImageDecodeError::InvalidPng("decompressed size overflow".to_string()))?;
    }
    Ok(total)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_pass(
    hdr: PngHeader,
    pass_data: &[u8],
    palette: Option<&[[u8; 3]]>,
    trns: Option<&[u8]>,
    x_start: u32,
    y_start: u32,
    x_step: u32,
    y_step: u32,
    pass_w: u32,
    pass_h: u32,
    out_rgba: &mut [u8],
) -> Result<(), ImageDecodeError> {
    let row_bytes = hdr.scanline_bytes(pass_w)?;
    let stride = row_bytes
        .checked_add(1)
        .ok_or_else(|| ImageDecodeError::InvalidPng("scanline stride overflow".to_string()))?;
    let bpp = hdr.filter_bpp();
    let mut prev_row = vec![0u8; row_bytes];
    let mut cur_row = vec![0u8; row_bytes];

    for py in 0..pass_h {
        let src_off = (py as usize)
            .checked_mul(stride)
            .ok_or_else(|| ImageDecodeError::InvalidPng("scanline offset overflow".to_string()))?;
        let &filter = pass_data
            .get(src_off)
            .ok_or_else(|| ImageDecodeError::InvalidPng("truncated PNG scanline".to_string()))?;
        let row_start = src_off
            .checked_add(1)
            .ok_or_else(|| ImageDecodeError::InvalidPng("scanline offset overflow".to_string()))?;
        let row_end = row_start
            .checked_add(row_bytes)
            .ok_or_else(|| ImageDecodeError::InvalidPng("scanline offset overflow".to_string()))?;
        let filtered = pass_data
            .get(row_start..row_end)
            .ok_or_else(|| ImageDecodeError::InvalidPng("truncated PNG scanline".to_string()))?;
        unfilter_scanline(filter, filtered, &prev_row, &mut cur_row, bpp)?;

        let dst_y = py
            .checked_mul(y_step)
            .and_then(|dy| y_start.checked_add(dy))
            .ok_or_else(|| ImageDecodeError::InvalidPng("Adam7 Y coordinate overflow".to_string()))?;
        if dst_y >= hdr.height {
            return Err(ImageDecodeError::InvalidPng(
                "Adam7 Y coordinate out of bounds".to_string(),
            ));
        }
        let row_dst_base = (dst_y as usize)
            .checked_mul(hdr.width as usize)
            .ok_or_else(|| ImageDecodeError::InvalidPng("pixel row offset overflow".to_string()))?;
        expand_row_to_rgba(
            hdr,
            &cur_row,
            pass_w,
            palette,
            trns,
            x_start,
            x_step,
            row_dst_base,
            out_rgba,
        )?;

        std::mem::swap(&mut prev_row, &mut cur_row);
    }
    Ok(())
}
