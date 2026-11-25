// Codec A: XOR + Zero-Run RLE Delta

use super::wire::{read_varint, write_varint, SyncDeltaError, MAX_DECODED_FRAME_BYTES};

/// Encodes `next` as a compact XOR + zero-run RLE delta relative to `base`.
///
/// Token encoding:
/// - `0x01..=0x7F`: `tag` unchanged bytes (`base[i] ^ next[i] == 0`)
/// - `0x00` + varint `n`: extended run of `n` unchanged bytes
/// - `0x81..=0xFF`: `tag - 0x80` XOR literal bytes
/// - `0x80` + varint `n` + `n` bytes: extended run of `n` XOR literal bytes
pub fn encode_xor_rle_delta(base: &[u8], next: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    encode_xor_rle_delta_into(base, next, &mut out);
    out
}

/// Zero-allocation-reuse version of [`encode_xor_rle_delta`].
pub fn encode_xor_rle_delta_into(base: &[u8], next: &[u8], out: &mut Vec<u8>) {
    out.clear();
    write_varint(next.len() as u64, out);

    let len = next.len();
    let mut i = 0usize;

    while i < len {
        // Count unchanged run where (base_byte ^ next[i]) == 0.
        let zero_start = i;
        while i < len {
            let b = base.get(i).copied().unwrap_or(0);
            if (b ^ next[i]) != 0 {
                break;
            }
            i += 1;
        }
        let zero_run = i - zero_start;
        if zero_run > 0 {
            if zero_run <= 127 {
                out.push(zero_run as u8);
            } else {
                out.push(0x00);
                write_varint(zero_run as u64, out);
            }
        }

        if i >= len {
            break;
        }

        // Count changed run; stop early if we encounter >= 2 consecutive zero-XOR bytes so
        // RLE stays maximally compact.
        let lit_start = i;
        while i < len {
            let b0 = base.get(i).copied().unwrap_or(0);
            if (b0 ^ next[i]) == 0 {
                let next_also_zero = i + 1 < len
                    && (base.get(i + 1).copied().unwrap_or(0) ^ next[i + 1]) == 0;
                if next_also_zero || i + 1 == len {
                    break;
                }
            }
            i += 1;
        }
        let lit_len = i - lit_start;
        if lit_len <= 127 {
            out.push(0x80 | (lit_len as u8));
        } else {
            out.push(0x80);
            write_varint(lit_len as u64, out);
        }
        for (offset, &nb) in next[lit_start..i].iter().enumerate() {
            let b = base.get(lit_start + offset).copied().unwrap_or(0);
            out.push(b ^ nb);
        }
    }
}

/// Applies an XOR+RLE delta payload onto `base`, writing the reconstructed frame into `out`.
pub fn decode_xor_rle_delta_into(
    base: &[u8],
    delta_payload: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), SyncDeltaError> {
    let mut pos = 0usize;
    let target_len_u64 = read_varint(delta_payload, &mut pos)?;
    let target_len = usize::try_from(target_len_u64)
        .map_err(|_| SyncDeltaError::FrameTooLarge(usize::MAX))?;
    if target_len > MAX_DECODED_FRAME_BYTES {
        return Err(SyncDeltaError::FrameTooLarge(target_len));
    }

    out.clear();
    out.reserve(target_len);

    while out.len() < target_len {
        let Some(&tag) = delta_payload.get(pos) else {
            return Err(SyncDeltaError::Truncated);
        };
        pos += 1;

        if (tag & 0x80) == 0 {
            let run_len = if tag == 0 {
                usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("zero-run length overflow"))?
            } else {
                usize::from(tag)
            };
            if run_len == 0 || out.len().saturating_add(run_len) > target_len {
                return Err(SyncDeltaError::CorruptPayload("zero-run exceeds target length"));
            }
            let start = out.len();
            let end = start + run_len;
            let from_base_end = end.min(base.len());
            if start < from_base_end {
                out.extend_from_slice(&base[start..from_base_end]);
            }
            let pad_zeros = end.saturating_sub(from_base_end.max(start));
            if pad_zeros > 0 {
                out.resize(out.len() + pad_zeros, 0);
            }
        } else {
            let lit_len = if tag == 0x80 {
                usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("literal length overflow"))?
            } else {
                usize::from(tag & 0x7F)
            };
            if lit_len == 0 || out.len().saturating_add(lit_len) > target_len {
                return Err(SyncDeltaError::CorruptPayload("literal run exceeds target length"));
            }
            if delta_payload.len().saturating_sub(pos) < lit_len {
                return Err(SyncDeltaError::Truncated);
            }
            let base_offset = out.len();
            let xor_bytes = &delta_payload[pos..pos + lit_len];
            pos += lit_len;
            for (k, &xb) in xor_bytes.iter().enumerate() {
                let b = base.get(base_offset + k).copied().unwrap_or(0);
                out.push(b ^ xb);
            }
        }
    }

    if pos != delta_payload.len() {
        return Err(SyncDeltaError::CorruptPayload("trailing bytes after xor-rle delta"));
    }
    Ok(())
}

/// Decodes an XOR+RLE delta payload onto `base`, returning the reconstructed bytes.
pub fn decode_xor_rle_delta(base: &[u8], delta_payload: &[u8]) -> Result<Vec<u8>, SyncDeltaError> {
    let mut out = Vec::new();
    decode_xor_rle_delta_into(base, delta_payload, &mut out)?;
    Ok(out)
}
