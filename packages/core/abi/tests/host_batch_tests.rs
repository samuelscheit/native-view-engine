//! Regression tests for `nve-abi` host event batch sizing and const enum decoding (P2#1).

use nve_abi::{
    host_batch_padded_record_len, NveHostEventKind, NVE_HOST_RECORD_HEADER_LEN,
};

#[test]
fn host_batch_padded_record_len_is_checked_against_overflow() {
    assert_eq!(host_batch_padded_record_len(0), Some(NVE_HOST_RECORD_HEADER_LEN));
    assert_eq!(host_batch_padded_record_len(1), Some(NVE_HOST_RECORD_HEADER_LEN + 8));
    assert_eq!(host_batch_padded_record_len(8), Some(NVE_HOST_RECORD_HEADER_LEN + 8));
    assert_eq!(host_batch_padded_record_len(9), Some(NVE_HOST_RECORD_HEADER_LEN + 16));
    assert_eq!(host_batch_padded_record_len(usize::MAX), None);
    assert_eq!(host_batch_padded_record_len(usize::MAX - 7), None);
    assert_eq!(host_batch_padded_record_len(usize::MAX - NVE_HOST_RECORD_HEADER_LEN), None);
}

#[test]
fn host_event_kind_try_from_u8_is_const_and_matches_from_repr() {
    const K0: Option<NveHostEventKind> = NveHostEventKind::try_from_u8(0);
    const K1: Option<NveHostEventKind> = NveHostEventKind::try_from_u8(1);
    const K2: Option<NveHostEventKind> = NveHostEventKind::try_from_u8(2);
    const K3: Option<NveHostEventKind> = NveHostEventKind::try_from_u8(3);
    const K4: Option<NveHostEventKind> = NveHostEventKind::try_from_u8(4);
    assert_eq!(K0, None);
    assert_eq!(K1, Some(NveHostEventKind::Response));
    assert_eq!(K2, Some(NveHostEventKind::Push));
    assert_eq!(K3, Some(NveHostEventKind::Channel));
    assert_eq!(K4, None);
    for raw in 0..=u8::MAX {
        assert_eq!(NveHostEventKind::try_from_u8(raw), NveHostEventKind::from_repr(raw));
    }
}
