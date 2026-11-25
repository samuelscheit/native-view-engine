// Codec B: NVE1 CommandBuffer Changed-Op Delta (Zero-Copy Boundary Scanner)

use std::collections::HashMap;

use nve_abi::{NVE_ABI_MAGIC, NVE_ABI_VERSION};

use super::wire::{fnv1a_64, read_varint, write_varint, SyncDeltaError, MAX_DECODED_FRAME_BYTES};
use super::xor_rle::{decode_xor_rle_delta_into, encode_xor_rle_delta_into};

/// NVE CommandBuffer fixed header length (`nve_abi::codec::HEADER_SIZE`).
const NVE_CMD_HEADER_SIZE: usize = 24;

#[derive(Debug, Clone, Copy)]
struct RawCmdHeader {
    flags: u16,
    surface_id: u32,
    frame_id: u64,
    op_count: u32,
}

/// Scans an `NVE1` `CommandBuffer` in a single zero-copy pass and returns its header plus the
/// byte range `(start, end)` of every encoded op. Returns `None` if `bytes` is not a valid `NVE1`
/// command buffer.
fn scan_nve_command_slices(bytes: &[u8]) -> Option<(RawCmdHeader, Vec<(usize, usize)>)> {
    if bytes.len() < NVE_CMD_HEADER_SIZE {
        return None;
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    if magic != NVE_ABI_MAGIC {
        return None;
    }
    let version = u16::from_le_bytes(bytes[4..6].try_into().ok()?);
    if version != NVE_ABI_VERSION {
        return None;
    }
    let flags = u16::from_le_bytes(bytes[6..8].try_into().ok()?);
    let surface_id = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    let frame_id = u64::from_le_bytes(bytes[12..20].try_into().ok()?);
    let op_count = u32::from_le_bytes(bytes[20..24].try_into().ok()?);

    let mut pos = NVE_CMD_HEADER_SIZE;
    let mut ranges = Vec::with_capacity((op_count as usize).min(4096));

    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let end = pos.checked_add(n)?;
        if end > bytes.len() {
            return None;
        }
        let s = &bytes[*pos..end];
        *pos = end;
        Some(s)
    };
    let read_u16 = |pos: &mut usize| -> Option<u16> {
        Some(u16::from_le_bytes(take(pos, 2)?.try_into().ok()?))
    };
    let read_u32 = |pos: &mut usize| -> Option<u32> {
        Some(u32::from_le_bytes(take(pos, 4)?.try_into().ok()?))
    };

    for _ in 0..op_count {
        let start = pos;
        let op = take(&mut pos, 1)?[0];
        match op {
            // CreateNode: u32 node_id + u16 kind + u16 name_len + name_bytes
            1 => {
                take(&mut pos, 6)?;
                let name_len = usize::from(read_u16(&mut pos)?);
                take(&mut pos, name_len)?;
            }
            // RemoveNode: u32 node_id
            2 => {
                take(&mut pos, 4)?;
            }
            // SetChildren: u32 parent_id + u32 count + count*u32
            3 => {
                take(&mut pos, 4)?;
                let count = read_u32(&mut pos)? as usize;
                take(&mut pos, count.checked_mul(4)?)?;
            }
            // SetText: u32 node_id + u32 len + text_bytes
            4 => {
                take(&mut pos, 4)?;
                let len = read_u32(&mut pos)? as usize;
                take(&mut pos, len)?;
            }
            // SetLayout: u32 node_id + 6*(u8+f32=5) + 3*u8 + 3*f32 + 2*(4*f32) = 4+30+3+12+32 = 81
            5 => {
                take(&mut pos, 81)?;
            }
            // SetStyle: u32 node_id + 3*u32 + 4*f32 + i32 = 4+12+16+4 = 36
            6 => {
                take(&mut pos, 36)?;
            }
            // SetRawProp: u32 node_id + u16 key_len + key + u8 val_tag + val
            7 => {
                take(&mut pos, 4)?;
                let key_len = usize::from(read_u16(&mut pos)?);
                take(&mut pos, key_len)?;
                let vtag = take(&mut pos, 1)?[0];
                match vtag {
                    0 => {}
                    1 => {
                        take(&mut pos, 1)?;
                    }
                    2 | 3 => {
                        take(&mut pos, 8)?;
                    }
                    4 | 5 => {
                        let len = read_u32(&mut pos)? as usize;
                        take(&mut pos, len)?;
                    }
                    _ => return None,
                }
            }
            // SetEventMask: u32 node_id + u64 mask
            8 => {
                take(&mut pos, 12)?;
            }
            // DispatchCommand: u32 node_id + u16 name_len + name + u32 payload_len + payload
            9 => {
                take(&mut pos, 4)?;
                let nlen = usize::from(read_u16(&mut pos)?);
                take(&mut pos, nlen)?;
                let plen = read_u32(&mut pos)? as usize;
                take(&mut pos, plen)?;
            }
            // Commit: u8 sync_mount
            10 => {
                take(&mut pos, 1)?;
            }
            _ => return None,
        }
        ranges.push((start, pos));
    }

    if pos != bytes.len() {
        return None;
    }

    Some((
        RawCmdHeader {
            flags,
            surface_id,
            frame_id,
            op_count,
        },
        ranges,
    ))
}

const CMD_DELTA_COPY_RUN: u8 = 1;
const CMD_DELTA_LITERAL_OP: u8 = 2;
const CMD_DELTA_XOR_OP: u8 = 3;

/// Extracts a stable `(opcode, node_id)` identity key from a raw encoded `NVE1` command slice.
///
/// Used by [`encode_command_op_delta`] so a modified `SetStyle`, `SetLayout`, or `SetText` on the
/// same node can be encoded as a tiny per-op XOR+RLE patch instead of re-sending the entire op.
#[inline]
fn raw_op_identity(op_slice: &[u8]) -> Option<(u8, u32)> {
    let opcode = *op_slice.first()?;
    if opcode == 10 {
        // Commit has no node_id.
        return Some((opcode, 0));
    }
    if op_slice.len() < 5 {
        return None;
    }
    let node_id = u32::from_le_bytes(op_slice[1..5].try_into().ok()?);
    Some((opcode, node_id))
}

/// Encodes an op-level delta between two valid `NVE1` `CommandBuffer`s.
///
/// - Identical op runs collapse into compact `CopyRun(prev_idx, count)` tokens (3–5 bytes).
/// - Modified ops targeting the same `(opcode, node_id)` emit a per-op `XorOp(prev_idx, xor_rle)`
///   whenever smaller than the raw op (shrinking `SetStyle`/`SetLayout`/`SetText` edits by 3–5x!).
/// - New ops emit only their raw slice via `LiteralOp`.
///
/// Returns `None` if either buffer is not a valid `NVE1` `CommandBuffer`.
pub fn encode_command_op_delta(base: &[u8], next: &[u8]) -> Option<Vec<u8>> {
    let (_, prev_ranges) = scan_nve_command_slices(base)?;
    let (next_hdr, next_ranges) = scan_nve_command_slices(next)?;

    // Index previous op slices by exact FNV-1a hash AND by `(opcode, node_id)` for per-op XOR patching.
    let mut prev_exact: HashMap<u64, Vec<usize>> = HashMap::with_capacity(prev_ranges.len());
    let mut prev_by_node: HashMap<(u8, u32), usize> = HashMap::with_capacity(prev_ranges.len());
    for (idx, &(s, e)) in prev_ranges.iter().enumerate() {
        let slice = &base[s..e];
        let h = fnv1a_64(slice);
        let entry = prev_exact.entry(h).or_default();
        if entry.len() < 4 {
            entry.push(idx);
        }
        if let Some(key) = raw_op_identity(slice) {
            prev_by_node.insert(key, idx);
        }
    }

    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&next_hdr.flags.to_le_bytes());
    write_varint(u64::from(next_hdr.surface_id), &mut out);
    write_varint(next_hdr.frame_id, &mut out);
    write_varint(u64::from(next_hdr.op_count), &mut out);

    let mut scratch_xor = Vec::with_capacity(64);
    let mut scratch_token = Vec::with_capacity(64);

    let mut ni = 0usize;
    let mut expected_prev = 0usize;
    let n_next = next_ranges.len();
    let n_prev = prev_ranges.len();

    while ni < n_next {
        let (ns, ne) = next_ranges[ni];
        let nslice = &next[ns..ne];

        // Fast path: check `expected_prev` first for sequential runs (O(1) no hash lookup).
        let matched_prev = if expected_prev < n_prev {
            let (ps, pe) = prev_ranges[expected_prev];
            if &base[ps..pe] == nslice {
                Some(expected_prev)
            } else {
                None
            }
        } else {
            None
        }
        .or_else(|| {
            let h = fnv1a_64(nslice);
            prev_exact.get(&h).and_then(|cands| {
                cands.iter().copied().find(|&pidx| {
                    let (ps, pe) = prev_ranges[pidx];
                    &base[ps..pe] == nslice
                })
            })
        });

        if let Some(pstart) = matched_prev {
            let mut run = 1usize;
            while ni + run < n_next && pstart + run < n_prev {
                let (ns2, ne2) = next_ranges[ni + run];
                let (ps2, pe2) = prev_ranges[pstart + run];
                if next[ns2..ne2] != base[ps2..pe2] {
                    break;
                }
                run += 1;
            }
            out.push(CMD_DELTA_COPY_RUN);
            write_varint(pstart as u64, &mut out);
            write_varint(run as u64, &mut out);
            ni += run;
            expected_prev = pstart + run;
        } else {
            // Try per-op XOR+RLE against the previous op on the same `(opcode, node_id)`.
            let mut emitted_xor = false;
            if let Some(key) = raw_op_identity(nslice) {
                if let Some(&pidx) = prev_by_node.get(&key) {
                    let (ps, pe) = prev_ranges[pidx];
                    encode_xor_rle_delta_into(&base[ps..pe], nslice, &mut scratch_xor);
                    scratch_token.clear();
                    scratch_token.push(CMD_DELTA_XOR_OP);
                    write_varint(pidx as u64, &mut scratch_token);
                    write_varint(scratch_xor.len() as u64, &mut scratch_token);
                    scratch_token.extend_from_slice(&scratch_xor);

                    // Compare against LiteralOp size: 1 + varint(len) + len
                    let lit_overhead = 1 + if nslice.len() < 128 { 1 } else { 2 } + nslice.len();
                    if scratch_token.len() < lit_overhead {
                        out.extend_from_slice(&scratch_token);
                        emitted_xor = true;
                        expected_prev = pidx + 1;
                    }
                }
            }
            if !emitted_xor {
                out.push(CMD_DELTA_LITERAL_OP);
                write_varint(nslice.len() as u64, &mut out);
                out.extend_from_slice(nslice);
                expected_prev = expected_prev.saturating_add(1);
            }
            ni += 1;
        }
    }

    Some(out)
}

/// Applies a `CommandOpDelta` payload onto a base `NVE1` `CommandBuffer`, reconstructing the exact
/// target `CommandBuffer` byte-for-byte.
pub fn decode_command_op_delta(base: &[u8], delta_payload: &[u8]) -> Result<Vec<u8>, SyncDeltaError> {
    let (_, prev_ranges) = scan_nve_command_slices(base)
        .ok_or(SyncDeltaError::CorruptPayload("base frame is not a valid NVE1 CommandBuffer"))?;

    let mut pos = 0usize;
    if delta_payload.len() < 2 {
        return Err(SyncDeltaError::Truncated);
    }
    let flags = u16::from_le_bytes(delta_payload[0..2].try_into().unwrap());
    pos += 2;

    let surface_id = u32::try_from(read_varint(delta_payload, &mut pos)?)
        .map_err(|_| SyncDeltaError::CorruptPayload("surface_id overflow"))?;
    let frame_id = read_varint(delta_payload, &mut pos)?;
    let op_count = u32::try_from(read_varint(delta_payload, &mut pos)?)
        .map_err(|_| SyncDeltaError::CorruptPayload("op_count overflow"))?;

    let mut out = Vec::with_capacity(base.len().min(64 * 1024).max(NVE_CMD_HEADER_SIZE));
    out.extend_from_slice(&NVE_ABI_MAGIC.to_le_bytes());
    out.extend_from_slice(&NVE_ABI_VERSION.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&surface_id.to_le_bytes());
    out.extend_from_slice(&frame_id.to_le_bytes());
    out.extend_from_slice(&op_count.to_le_bytes());

    let mut xor_scratch = Vec::with_capacity(64);
    let mut emitted_ops = 0u32;
    while pos < delta_payload.len() {
        let tag = delta_payload[pos];
        pos += 1;
        match tag {
            CMD_DELTA_COPY_RUN => {
                let start_idx = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("copy_run start overflow"))?;
                let count = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("copy_run count overflow"))?;
                if count == 0 || start_idx.saturating_add(count) > prev_ranges.len() {
                    return Err(SyncDeltaError::CorruptPayload("copy_run out of bounds"));
                }
                for &(ps, pe) in &prev_ranges[start_idx..start_idx + count] {
                    if out.len().saturating_add(pe - ps) > MAX_DECODED_FRAME_BYTES {
                        return Err(SyncDeltaError::FrameTooLarge(out.len() + (pe - ps)));
                    }
                    out.extend_from_slice(&base[ps..pe]);
                    emitted_ops = emitted_ops.saturating_add(1);
                }
            }
            CMD_DELTA_LITERAL_OP => {
                let len = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("literal op len overflow"))?;
                if len == 0 || delta_payload.len().saturating_sub(pos) < len {
                    return Err(SyncDeltaError::Truncated);
                }
                if out.len().saturating_add(len) > MAX_DECODED_FRAME_BYTES {
                    return Err(SyncDeltaError::FrameTooLarge(out.len() + len));
                }
                out.extend_from_slice(&delta_payload[pos..pos + len]);
                pos += len;
                emitted_ops = emitted_ops.saturating_add(1);
            }
            CMD_DELTA_XOR_OP => {
                let prev_idx = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("xor_op prev_idx overflow"))?;
                let &(ps, pe) = prev_ranges
                    .get(prev_idx)
                    .ok_or(SyncDeltaError::CorruptPayload("xor_op prev_idx out of bounds"))?;
                let xor_len = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("xor_op len overflow"))?;
                if xor_len == 0 || delta_payload.len().saturating_sub(pos) < xor_len {
                    return Err(SyncDeltaError::Truncated);
                }
                decode_xor_rle_delta_into(
                    &base[ps..pe],
                    &delta_payload[pos..pos + xor_len],
                    &mut xor_scratch,
                )?;
                pos += xor_len;
                if out.len().saturating_add(xor_scratch.len()) > MAX_DECODED_FRAME_BYTES {
                    return Err(SyncDeltaError::FrameTooLarge(out.len() + xor_scratch.len()));
                }
                out.extend_from_slice(&xor_scratch);
                emitted_ops = emitted_ops.saturating_add(1);
            }
            _ => {
                return Err(SyncDeltaError::CorruptPayload("unknown command delta op tag"));
            }
        }
    }

    if emitted_ops != op_count {
        return Err(SyncDeltaError::CorruptPayload("reconstructed op_count mismatch"));
    }
    Ok(out)
}
