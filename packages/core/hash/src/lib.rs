//! Canonical zero-dependency, `no_std`, 100% safe **XXH64** for every NVE crate and host.
//!
//! - [`xxh64`] is a `const fn`: table keys, schema hashes, interface ids and static string keys are
//!   baked at compile time (`const KEY: Key64 = Key64::of("com.acme.map_view");`) and resolve to
//!   dense runtime slots once.
//! - [`Xxh64Hasher`] streams the same digest for runtime data (wire trailers, resource dedup,
//!   persisted snapshots).
//! - [`Key64`] splits into `(lo, hi)` `u32` halves so Dart-web / JavaScript hosts (53-bit safe
//!   integers) compare keys without `BigInt`. Generated host mirrors live in
//!   `nve_hash.generated.{h,hpp,dart,ts,swift,java}` (`nve codegen abi`).
#![no_std]

use core::hash::Hasher;

pub const PRIME1: u64 = 0x9E37_79B1_85EB_CA87;
pub const PRIME2: u64 = 0xC2B2_AE3D_27D4_EB4F;
pub const PRIME3: u64 = 0x1656_67B1_9E37_79F9;
pub const PRIME4: u64 = 0x85EB_CA77_C2B2_AE63;
pub const PRIME5: u64 = 0x27D4_EB2F_1656_67C5;

#[inline(always)]
const fn read_u64(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([
        b[i],
        b[i + 1],
        b[i + 2],
        b[i + 3],
        b[i + 4],
        b[i + 5],
        b[i + 6],
        b[i + 7],
    ])
}

#[inline(always)]
const fn read_u32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

#[inline(always)]
const fn round(acc: u64, lane: u64) -> u64 {
    acc.wrapping_add(lane.wrapping_mul(PRIME2))
        .rotate_left(31)
        .wrapping_mul(PRIME1)
}

#[inline(always)]
const fn merge_round(acc: u64, val: u64) -> u64 {
    (acc ^ round(0, val))
        .wrapping_mul(PRIME1)
        .wrapping_add(PRIME4)
}

#[inline(always)]
const fn converge(v: [u64; 4]) -> u64 {
    let h = v[0]
        .rotate_left(1)
        .wrapping_add(v[1].rotate_left(7))
        .wrapping_add(v[2].rotate_left(12))
        .wrapping_add(v[3].rotate_left(18));
    merge_round(
        merge_round(merge_round(merge_round(h, v[0]), v[1]), v[2]),
        v[3],
    )
}

#[inline(always)]
const fn init_lanes(seed: u64) -> [u64; 4] {
    [
        seed.wrapping_add(PRIME1).wrapping_add(PRIME2),
        seed.wrapping_add(PRIME2),
        seed,
        seed.wrapping_sub(PRIME1),
    ]
}

#[inline(always)]
const fn stripe(mut v: [u64; 4], b: &[u8], i: usize) -> [u64; 4] {
    v[0] = round(v[0], read_u64(b, i));
    v[1] = round(v[1], read_u64(b, i + 8));
    v[2] = round(v[2], read_u64(b, i + 16));
    v[3] = round(v[3], read_u64(b, i + 24));
    v
}

/// Consumes the `< 32` byte tail `b[i..end]` and avalanches.
#[inline(always)]
const fn finalize(mut h: u64, b: &[u8], mut i: usize, end: usize) -> u64 {
    while i + 8 <= end {
        h ^= round(0, read_u64(b, i));
        h = h.rotate_left(27).wrapping_mul(PRIME1).wrapping_add(PRIME4);
        i += 8;
    }
    if i + 4 <= end {
        h ^= (read_u32(b, i) as u64).wrapping_mul(PRIME1);
        h = h.rotate_left(23).wrapping_mul(PRIME2).wrapping_add(PRIME3);
        i += 4;
    }
    while i < end {
        h ^= (b[i] as u64).wrapping_mul(PRIME5);
        h = h.rotate_left(11).wrapping_mul(PRIME1);
        i += 1;
    }
    h ^= h >> 33;
    h = h.wrapping_mul(PRIME2);
    h ^= h >> 29;
    h = h.wrapping_mul(PRIME3);
    h ^ (h >> 32)
}

/// Canonical 64-bit **XXH64** digest of `bytes` with `seed` (Cyan4973 reference compatible).
#[inline]
pub const fn xxh64(bytes: &[u8], seed: u64) -> u64 {
    let len = bytes.len();
    let mut i = 0;
    let h = if len >= 32 {
        let mut v = init_lanes(seed);
        while i + 32 <= len {
            v = stripe(v, bytes, i);
            i += 32;
        }
        converge(v)
    } else {
        seed.wrapping_add(PRIME5)
    };
    finalize(h.wrapping_add(len as u64), bytes, i, len)
}

/// Compile-time identity of a UTF-8 name: `xxh64(name, 0)`.
#[inline]
pub const fn const_id(name: &str) -> u64 {
    xxh64(name.as_bytes(), 0)
}

/// 64-bit compile-time identity key (table keys, schema hashes, interface ids, static keys).
///
/// Crosses every ABI as two `u32` halves so 53-bit JavaScript / Dart-web integers stay exact.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Key64(pub u64);

impl Key64 {
    /// Key of a reverse-DNS or plain name, e.g. `Key64::of("nve.visual")`.
    #[inline]
    pub const fn of(name: &str) -> Self {
        Self(const_id(name))
    }

    #[inline]
    pub const fn from_halves(lo: u32, hi: u32) -> Self {
        Self((hi as u64) << 32 | lo as u64)
    }

    #[inline]
    pub const fn lo(self) -> u32 {
        self.0 as u32
    }

    #[inline]
    pub const fn hi(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

/// Streaming **XXH64** producing digests identical to [`xxh64`] for any chunking.
#[derive(Debug, Clone)]
pub struct Xxh64Hasher {
    total_len: u64,
    seed: u64,
    lanes: [u64; 4],
    buf: [u8; 32],
    buf_len: usize,
}

impl Xxh64Hasher {
    #[inline]
    pub const fn new(seed: u64) -> Self {
        Self {
            total_len: 0,
            seed,
            lanes: init_lanes(seed),
            buf: [0; 32],
            buf_len: 0,
        }
    }

    /// Feeds `bytes` into the digest. `const`, so compile-time schema hashes stream like runtime ones.
    pub const fn update(&mut self, bytes: &[u8]) {
        self.total_len = self.total_len.wrapping_add(bytes.len() as u64);
        let mut i = 0;
        if self.buf_len > 0 {
            while self.buf_len < 32 && i < bytes.len() {
                self.buf[self.buf_len] = bytes[i];
                self.buf_len += 1;
                i += 1;
            }
            if self.buf_len < 32 {
                return;
            }
            self.lanes = stripe(self.lanes, &self.buf, 0);
            self.buf_len = 0;
        }
        while i + 32 <= bytes.len() {
            self.lanes = stripe(self.lanes, bytes, i);
            i += 32;
        }
        while i < bytes.len() {
            self.buf[self.buf_len] = bytes[i];
            self.buf_len += 1;
            i += 1;
        }
    }

    /// Feeds `v` as 8 little-endian bytes.
    #[inline]
    pub const fn update_u64(&mut self, v: u64) {
        self.update(&v.to_le_bytes());
    }

    /// Feeds `v` as 4 little-endian bytes.
    #[inline]
    pub const fn update_u32(&mut self, v: u32) {
        self.update(&v.to_le_bytes());
    }

    /// Feeds canonical `f32` bits: `±0.0` and every non-finite value hash as `0`, so structurally
    /// equal floats (style values, geometry) always produce equal digests.
    #[inline]
    pub const fn update_f32(&mut self, v: f32) {
        self.update_u32(if !v.is_finite() || v == 0.0 { 0 } else { v.to_bits() });
    }

    /// Digest of everything fed so far (the hasher stays usable).
    #[inline]
    pub const fn digest(&self) -> u64 {
        let h = if self.total_len >= 32 {
            converge(self.lanes)
        } else {
            self.seed.wrapping_add(PRIME5)
        };
        finalize(h.wrapping_add(self.total_len), &self.buf, 0, self.buf_len)
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
        self.digest()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.update(bytes);
    }
}

#[cfg(test)]
mod tests;
