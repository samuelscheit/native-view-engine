extern crate std;

use std::vec::Vec;

use super::*;

const XXH64_PRIME1: u64 = PRIME1;

// Compile-time evaluation is part of the contract: these must stay `const`.
const EMPTY: u64 = xxh64(b"", 0);
const KEY: Key64 = Key64::of("nve.visual");
const STREAMED: u64 = {
    let mut h = Xxh64Hasher::new(7);
    h.update(b"0123456789abcdef0123456789");
    h.update(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    h.update_u32(0x0403_0201);
    h.update_u64(0x0c0b_0a09_0807_0605);
    h.digest()
};
const CANONICAL_F32: [u64; 4] = {
    let mut out = [0; 4];
    let vals = [0.0f32, -0.0, f32::NAN, f32::INFINITY];
    let mut i = 0;
    while i < vals.len() {
        let mut h = Xxh64Hasher::new(0);
        h.update_f32(vals[i]);
        out[i] = h.digest();
        i += 1;
    }
    out
};

#[test]
fn update_f32_collapses_zero_signs_and_non_finite_values() {
    let zero = xxh64(&[0; 4], 0);
    assert_eq!(CANONICAL_F32, [zero; 4]);
    let mut h = Xxh64Hasher::new(0);
    h.update_f32(1.5);
    assert_eq!(h.digest(), xxh64(&1.5f32.to_bits().to_le_bytes(), 0));
}

#[test]
fn const_evaluation_matches_runtime() {
    assert_eq!(EMPTY, 0xEF46_DB37_51D8_E999);
    assert_eq!(KEY.0, xxh64(std::hint::black_box(b"nve.visual"), 0));
    assert_eq!(Key64::from_halves(KEY.lo(), KEY.hi()), KEY);
    assert_eq!(const_id("abc"), 0x44BC_2CF5_AD77_0999);
    assert_eq!(
        STREAMED,
        xxh64(
            b"0123456789abcdef0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c",
            7
        )
    );
}
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
                    hasher.digest(),
                    expected,
                    "mismatch for len={len}, step={step}, seed={seed:#x}"
                );
            }

            // Specifically test NVERELOC header split (first 24 bytes, then tail).
            if len >= 32 {
                let mut hasher = Xxh64Hasher::new(seed);
                hasher.update(&slice[..24]);
                hasher.update(&slice[24..]);
                assert_eq!(hasher.digest(), expected);
            }
        }
    }
}
