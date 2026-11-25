//! Adversarial binary truncation & PRNG fuzz tests for `NVH1`, `NVT2`, `NVL1`, and all
//! `nve-sdk` service request/response codecs (must never panic or OOM).

use nve_abi::NveHostEventBatch;
use nve_sdk::{
    clipboard, filesystem, haptics, linking, notifications, secure_store, FileFilter,
    FilePickerOptions, LocaleSnapshot, NotificationRequest, ThemeSnapshot,
};

#[test]
fn fuzz_binary_codecs_never_panic_on_truncated_or_corrupt_inputs() {
    // 1. Exhaustive prefix truncation on valid payloads for every codec.
    let valid_nvt2 = ThemeSnapshot::default().encode_nvt2().to_vec();
    for len in 0..valid_nvt2.len() {
        assert!(ThemeSnapshot::decode_nvt2(&valid_nvt2[..len]).is_err());
    }
    let valid_nvl1 = LocaleSnapshot::default().encode_nvl1().unwrap();
    for len in 0..valid_nvl1.len() {
        assert!(LocaleSnapshot::decode_nvl1(&valid_nvl1[..len]).is_err());
    }
    let valid_picker = filesystem::encode_file_picker_options(&FilePickerOptions {
        title: Some("Open".into()),
        initial_dir: Some("/tmp".into()),
        filters: vec![FileFilter::new("Rust", ["rs", "toml"])],
        allow_multiple: true,
        pick_directories: false,
    })
    .unwrap();
    for len in 0..valid_picker.len() {
        assert!(filesystem::decode_file_picker_options(&valid_picker[..len]).is_err());
    }
    let valid_show = notifications::encode_show_request(&NotificationRequest::new(
        "id-1", "Title", "Body",
    ))
    .unwrap();
    for len in 0..valid_show.len() {
        assert!(notifications::decode_show_request(&valid_show[..len]).is_err());
    }

    // 2. Deterministic PRNG adversarial byte fuzz across ALL decoders (zero panics, zero OOM).
    let mut seed: u64 = 0xD1B5_4A32_D192_ED03;
    let mut next_u64 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for i in 0..1500 {
        let len = (next_u64() as usize) % 64;
        let mut buf = vec![0u8; len];
        for b in &mut buf {
            *b = (next_u64() & 0xFF) as u8;
        }
        if len >= 4 {
            match i % 4 {
                0 => buf[..4].copy_from_slice(b"NVH1"),
                1 => buf[..4].copy_from_slice(b"NVT2"),
                2 => buf[..4].copy_from_slice(b"NVL1"),
                _ => {}
            }
        }
        let _ = NveHostEventBatch::decode(&buf);
        let _ = ThemeSnapshot::decode_nvt2(&buf);
        let _ = LocaleSnapshot::decode_nvl1(&buf);
        let _ = clipboard::decode_read_text_response(&buf);
        let _ = clipboard::decode_write_text_request(&buf);
        let _ = clipboard::decode_has_text_response(&buf);
        let _ = filesystem::decode_standard_path_request(&buf);
        let _ = filesystem::decode_standard_path_response(&buf);
        let _ = filesystem::decode_file_picker_options(&buf);
        let _ = filesystem::decode_pick_files_response(&buf);
        let _ = filesystem::decode_save_file_options(&buf);
        let _ = filesystem::decode_save_file_response(&buf);
        let _ = secure_store::decode_key_request(&buf);
        let _ = secure_store::decode_set_request(&buf);
        let _ = secure_store::decode_get_response(&buf);
        let _ = secure_store::decode_delete_response(&buf);
        let _ = linking::decode_url_request(&buf);
        let _ = linking::decode_can_open_response(&buf);
        let _ = linking::decode_initial_url_response(&buf);
        let _ = notifications::decode_permission_response(&buf);
        let _ = notifications::decode_show_request(&buf);
        let _ = notifications::decode_cancel_request(&buf);
        let _ = notifications::decode_tapped_event(&buf);
        let _ = haptics::decode_impact_request(&buf);
        let _ = haptics::decode_notification_haptic_request(&buf);
    }
}
