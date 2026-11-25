//! Canonical zero-dependency, 100% safe-Rust **XXH64** (xxHash64) implementation.
//!
//! Uses the 4-lane `u64` accumulator (`PRIME64_1..5`) for high instruction-level parallelism
//! across 32-byte stripes with zero `unsafe` blocks and zero external crate dependencies.

use core::hash::Hasher;

pub const XXH64_PRIME1: u64 = 0x9E37_79B1_85EB_CA87;
pub const XXH64_PRIME2: u64 = 0xC2B2_AE3D_27D4_EB4F;
pub const XXH64_PRIME3: u64 = 0x1656_67B1_9E37_79F9;
pub const XXH64_PRIME4: u64 = 0x85EB_CA77_C2B2_AE63;
pub const XXH64_PRIME5: u64 = 0x27D4_EB2F_1656_67C5;

#[inline(always)]
fn read_u64_le(bytes: &[u8]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

#[inline(always)]
fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[inline(always)]
const fn round(acc: u64, lane: u64) -> u64 {
    acc.wrapping_add(lane.wrapping_mul(XXH64_PRIME2))
        .rotate_left(31)
        .wrapping_mul(XXH64_PRIME1)
}

#[inline(always)]
const fn merge_round(acc: u64, val: u64) -> u64 {
    (acc ^ round(0, val))
        .wrapping_mul(XXH64_PRIME1)
        .wrapping_add(XXH64_PRIME4)
}

#[inline(always)]
const fn avalanche(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(XXH64_PRIME2);
    h ^= h >> 29;
    h = h.wrapping_mul(XXH64_PRIME3);
    h ^= h >> 32;
    h
}

#[inline]
fn finalize(mut h: u64, mut rem: &[u8]) -> u64 {
    while rem.len() >= 8 {
        let k1 = round(0, read_u64_le(rem));
        h ^= k1;
        h = h
            .rotate_left(27)
            .wrapping_mul(XXH64_PRIME1)
            .wrapping_add(XXH64_PRIME4);
        rem = &rem[8..];
    }
    if rem.len() >= 4 {
        h ^= (read_u32_le(rem) as u64).wrapping_mul(XXH64_PRIME1);
        h = h
            .rotate_left(23)
            .wrapping_mul(XXH64_PRIME2)
            .wrapping_add(XXH64_PRIME3);
        rem = &rem[4..];
    }
    for &b in rem {
        h ^= (b as u64).wrapping_mul(XXH64_PRIME5);
        h = h.rotate_left(11).wrapping_mul(XXH64_PRIME1);
    }
    avalanche(h)
}

/// Computes the canonical 64-bit **XXH64** digest over `bytes` with `seed`.
#[inline]
pub fn xxh64(bytes: &[u8], seed: u64) -> u64 {
    let (chunks, rem) = bytes.as_chunks::<32>();
    let h64 = if !chunks.is_empty() {
        let mut v1 = seed.wrapping_add(XXH64_PRIME1).wrapping_add(XXH64_PRIME2);
        let mut v2 = seed.wrapping_add(XXH64_PRIME2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(XXH64_PRIME1);

        for chunk in chunks {
            v1 = round(v1, read_u64_le(&chunk[0..8]));
            v2 = round(v2, read_u64_le(&chunk[8..16]));
            v3 = round(v3, read_u64_le(&chunk[16..24]));
            v4 = round(v4, read_u64_le(&chunk[24..32]));
        }

        let mut h = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        h = merge_round(h, v1);
        h = merge_round(h, v2);
        h = merge_round(h, v3);
        h = merge_round(h, v4);
        h.wrapping_add(bytes.len() as u64)
    } else {
        seed.wrapping_add(XXH64_PRIME5)
            .wrapping_add(bytes.len() as u64)
    };

    finalize(h64, rem)
}

/// Streaming canonical **XXH64** hasher producing byte-for-byte identical digests to [`xxh64`].
#[derive(Debug, Clone)]
pub struct Xxh64Hasher {
    total_len: u64,
    large_len: bool,
    seed: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    v4: u64,
    buf: [u8; 32],
    buf_len: usize,
}

impl Xxh64Hasher {
    /// Creates a new streaming XXH64 hasher initialized with `seed`.
    #[inline]
    pub const fn new(seed: u64) -> Self {
        Self {
            total_len: 0,
            large_len: false,
            seed,
            v1: seed.wrapping_add(XXH64_PRIME1).wrapping_add(XXH64_PRIME2),
            v2: seed.wrapping_add(XXH64_PRIME2),
            v3: seed,
            v4: seed.wrapping_sub(XXH64_PRIME1),
            buf: [0u8; 32],
            buf_len: 0,
        }
    }

    /// Feeds additional `bytes` into the streaming XXH64 state.
    #[inline]
    pub fn update(&mut self, mut bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.total_len = self.total_len.wrapping_add(bytes.len() as u64);
        if self.total_len >= 32 {
            self.large_len = true;
        }

        if self.buf_len + bytes.len() < 32 {
            self.buf[self.buf_len..self.buf_len + bytes.len()].copy_from_slice(bytes);
            self.buf_len += bytes.len();
            return;
        }

        let mut v1 = self.v1;
        let mut v2 = self.v2;
        let mut v3 = self.v3;
        let mut v4 = self.v4;

        if self.buf_len > 0 {
            let take = 32 - self.buf_len;
            self.buf[self.buf_len..32].copy_from_slice(&bytes[..take]);
            v1 = round(v1, read_u64_le(&self.buf[0..8]));
            v2 = round(v2, read_u64_le(&self.buf[8..16]));
            v3 = round(v3, read_u64_le(&self.buf[16..24]));
            v4 = round(v4, read_u64_le(&self.buf[24..32]));
            bytes = &bytes[take..];
            self.buf_len = 0;
        }

        let (chunks, rem) = bytes.as_chunks::<32>();
        for chunk in chunks {
            v1 = round(v1, read_u64_le(&chunk[0..8]));
            v2 = round(v2, read_u64_le(&chunk[8..16]));
            v3 = round(v3, read_u64_le(&chunk[16..24]));
            v4 = round(v4, read_u64_le(&chunk[24..32]));
        }

        self.v1 = v1;
        self.v2 = v2;
        self.v3 = v3;
        self.v4 = v4;
        if !rem.is_empty() {
            self.buf[..rem.len()].copy_from_slice(rem);
            self.buf_len = rem.len();
        }
    }

    /// Finalizes and returns the 64-bit XXH64 digest without mutating the hasher state.
    #[inline]
    pub fn finish(&self) -> u64 {
        let mut h64 = if self.large_len {
            let mut h = self
                .v1
                .rotate_left(1)
                .wrapping_add(self.v2.rotate_left(7))
                .wrapping_add(self.v3.rotate_left(12))
                .wrapping_add(self.v4.rotate_left(18));
            h = merge_round(h, self.v1);
            h = merge_round(h, self.v2);
            h = merge_round(h, self.v3);
            h = merge_round(h, self.v4);
            h
        } else {
            self.seed.wrapping_add(XXH64_PRIME5)
        };
        h64 = h64.wrapping_add(self.total_len);
        finalize(h64, &self.buf[..self.buf_len])
    }
}

impl Default for Xxh64Hasher {
    #[inline]
    fn default() -> Self {
        Self::new(0)
    }
}

impl Hasher for Xxh64Hasher {
    #[inline]
    fn finish(&self) -> u64 {
        Xxh64Hasher::finish(self)
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.update(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cyan_sanity_buf(len: usize) -> Vec<u8> {
        const PRIME32_1: u64 = 0x9E37_79B1;
        let mut byte_gen = PRIME32_1;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push((byte_gen >> 56) as u8);
            byte_gen = byte_gen.wrapping_mul(XXH64_PRIME1);
        }
        out
    }

    #[test]
    fn xxh64_matches_official_reference_vectors_seed_zero() {
        assert_eq!(xxh64(b"", 0), 0xEF46_DB37_51D8_E999);
        assert_eq!(xxh64(b"a", 0), 0xD24E_C4F1_A98C_6E5B);
        assert_eq!(xxh64(b"abc", 0), 0x44BC_2CF5_AD77_0999);
        assert_eq!(xxh64(b"NVE_SNAP", 0), 0x3C2A_9102_3F0E_7727);

        let seq_1: Vec<u8> = (0..1u8).collect();
        let seq_4: Vec<u8> = (0..4u8).collect();
        let seq_14: Vec<u8> = (0..14u8).collect();
        let seq_31: Vec<u8> = (0..31u8).collect();
        let seq_32: Vec<u8> = (0..32u8).collect();
        let seq_33: Vec<u8> = (0..33u8).collect();
        let seq_64: Vec<u8> = (0..64u8).collect();
        let seq_222: Vec<u8> = (0..222usize).map(|i| i as u8).collect();
        let seq_1024: Vec<u8> = (0..1024usize).map(|i| (i * 31 + 7) as u8).collect();

        assert_eq!(xxh64(&seq_1, 0), 0xE934_A84A_DB05_2768);
        assert_eq!(xxh64(&seq_4, 0), 0xFFCE_D860_4453_CC1E);
        assert_eq!(xxh64(&seq_14, 0), 0x5CDA_8B69_BBFC_1D45);
        assert_eq!(xxh64(&seq_31, 0), 0xC346_D2B5_9B4D_8EE1);
        assert_eq!(xxh64(&seq_32, 0), 0xCBF5_9C51_16FF_32B4);
        assert_eq!(xxh64(&seq_33, 0), 0x0C53_5D1A_CAFB_8EAD);
        assert_eq!(xxh64(&seq_64, 0), 0xF7C6_7301_DB67_13F0);
        assert_eq!(xxh64(&seq_222, 0), 0x88FF_0DAD_A4BF_A877);
        assert_eq!(xxh64(&seq_1024, 0), 0x149A_A449_72CD_AE00);

        // Official Cyan4973 xxHash sanity buffer vectors.
        assert_eq!(xxh64(&cyan_sanity_buf(1), 0), 0xE934_A84A_DB05_2768);
        assert_eq!(xxh64(&cyan_sanity_buf(4), 0), 0xE244_DDBC_F81E_29E4);
        assert_eq!(xxh64(&cyan_sanity_buf(14), 0), 0xB89B_3598_E0BD_0A0A);
        assert_eq!(xxh64(&cyan_sanity_buf(222), 0), 0x06CC_5BD9_30BC_EC3A);
        assert_eq!(xxh64(&cyan_sanity_buf(1024), 0), 0x2E30_7F2F_E063_991A);
    }

    #[test]
    fn xxh64_matches_official_seeded_vectors() {
        const SEED_PRIME32: u64 = 0x9E37_79B1;
        // Official reference vectors verified against `libxxhash` (`XXH64`) with seed = PRIME32_1.
        assert_eq!(xxh64(b"", SEED_PRIME32), 0xAC75_FDA2_929B_17EF);
        assert_eq!(xxh64(b"a", SEED_PRIME32), 0x393D_A8B7_8992_279B);
        assert_eq!(xxh64(b"abc", SEED_PRIME32), 0x1318_DF30_094A_85FD);
        assert_eq!(
            xxh64(&cyan_sanity_buf(1), SEED_PRIME32),
            0x5014_6076_43A9_B4C3
        );
        assert_eq!(
            xxh64(&cyan_sanity_buf(4), SEED_PRIME32),
            0x9C2A_6616_4D52_EBC2
        );
        assert_eq!(
            xxh64(&cyan_sanity_buf(14), SEED_PRIME32),
            0x9BB5_720D_90B3_D7F1
        );
        assert_eq!(
            xxh64(&cyan_sanity_buf(32), SEED_PRIME32),
            0x134F_5CE3_D4DA_1311
        );
        assert_eq!(
            xxh64(&cyan_sanity_buf(222), SEED_PRIME32),
            0x0B10_5469_DF89_AF66
        );
        assert_eq!(
            xxh64(&cyan_sanity_buf(1024), SEED_PRIME32),
            0x5C94_E947_9691_DD09
        );

        // Official reference vectors verified against `libxxhash` (`XXH64`) with
        // non-zero 64-bit seed `XXH64_PRIME1 = 0x9E37_79B1_85EB_CA87`.
        const SEED64: u64 = XXH64_PRIME1;
        assert_eq!(xxh64(b"", SEED64), 0x6EC6_D05F_61C7_E7A7);
        assert_eq!(xxh64(b"a", SEED64), 0x727C_10E0_D238_E188);
        assert_eq!(xxh64(b"abc", SEED64), 0xA7CB_2AAC_405E_36C7);
        assert_eq!(xxh64(&cyan_sanity_buf(1), SEED64), 0x6050_8B0C_ED72_C717);
        assert_eq!(xxh64(&cyan_sanity_buf(4), SEED64), 0x1F46_865B_24B2_F3A9);
        assert_eq!(xxh64(&cyan_sanity_buf(14), SEED64), 0x31DC_D1D4_E2E7_D0FE);
        assert_eq!(xxh64(&cyan_sanity_buf(32), SEED64), 0x6D02_42B0_B7E2_78ED);
        assert_eq!(xxh64(&cyan_sanity_buf(222), SEED64), 0x0AC0_52E0_3CF9_7089);
        assert_eq!(
            xxh64(&cyan_sanity_buf(1024), SEED64),
            0xF464_C3F6_9A39_421C
        );
    }

    #[test]
    fn xxh64_streaming_matches_oneshot_across_all_chunk_splits() {
        let data: Vec<u8> = (0..1024usize).map(|i| (i * 31 + 7) as u8).collect();
        for &seed in &[0u64, XXH64_PRIME1] {
            for len in [0usize, 1, 3, 4, 7, 8, 15, 16, 24, 31, 32, 33, 63, 64, 65, 129, 255, 513, 1024]
            {
                let slice = &data[..len];
                let expected = xxh64(slice, seed);

                for &step in &[1usize, 3, 7, 13, 24, 31, 32, 33, 64, 100] {
                    let mut hasher = Xxh64Hasher::new(seed);
                    for chunk in slice.chunks(step) {
                        hasher.update(chunk);
                    }
                    assert_eq!(
                        hasher.finish(),
                        expected,
                        "mismatch for len={len}, step={step}, seed={seed:#x}"
                    );
                }

                // Specifically test NVE_SNAP / NVERELOC header split (first 24 bytes, then tail).
                if len >= 32 {
                    let mut hasher = Xxh64Hasher::new(seed);
                    hasher.update(&slice[..24]);
                    hasher.update(&slice[24..]);
                    assert_eq!(hasher.finish(), expected);
                }
            }
        }
    }
}
