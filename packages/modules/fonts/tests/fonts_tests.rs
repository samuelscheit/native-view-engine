use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nve_fonts::{
    decode_handle, decode_list_registered_response, decode_register_bytes_request,
    decode_register_file_request, validate_font_bytes, FontClient, FontFormat, MemoryFonts,
    NativeFonts,
};
use nve_host_services::{
    FfiHostServices, HostCallbackBridge, MemoryHostServices, NativeHostServices, ServiceError,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

fn unique_temp_dir(label: &str) -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nve_fonts_test_{label}_{}_{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn sample_ttf(extra: u8) -> Vec<u8> {
    let mut b = vec![0x00, 0x01, 0x00, 0x00, 0, 1, 0, 16, 0, 0, 0, 0];
    b.extend_from_slice(&[extra; 24]);
    b
}

fn sample_apple_ttf(extra: u8) -> Vec<u8> {
    let mut b = b"true\x00\x01\x00\x10\x00\x00\x00\x00".to_vec();
    b.extend_from_slice(&[extra; 16]);
    b
}

fn sample_otf(extra: u8) -> Vec<u8> {
    let mut b = b"OTTO\x00\x01\x00\x10\x00\x00\x00\x00".to_vec();
    b.extend_from_slice(&[extra; 20]);
    b
}

fn sample_woff(extra: u8) -> Vec<u8> {
    let mut b = b"wOFF".to_vec();
    b.extend_from_slice(&[extra; 44]);
    b
}

fn sample_woff2(extra: u8) -> Vec<u8> {
    let mut b = b"wOF2".to_vec();
    b.extend_from_slice(&[extra; 48]);
    b
}

fn sample_ttc(extra: u8) -> Vec<u8> {
    let mut b = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x01".to_vec();
    b.extend_from_slice(&[extra; 12]);
    b
}

#[test]
fn font_loader_magic_validation_dedup_file_registration_c_abi_and_fuzz() {
    let dir = unique_temp_dir("fonts");
    let native = NativeHostServices::new().with_handler(NativeFonts::new());
    let fc = FontClient::new(&native);

    let h_ttf = fc.register_bytes("Inter", 400, false, &sample_ttf(1)).unwrap();
    let h_apple = fc
        .register_bytes("SF Pro", 500, false, &sample_apple_ttf(2))
        .unwrap();
    let h_otf = fc.register_bytes("Inter", 700, true, &sample_otf(3)).unwrap();
    let h_woff = fc
        .register_bytes("JetBrains Mono", 400, false, &sample_woff(4))
        .unwrap();
    let h_woff2 = fc
        .register_bytes("JetBrains Mono", 700, false, &sample_woff2(5))
        .unwrap();
    let h_ttc = fc
        .register_bytes("Noto Sans", 400, false, &sample_ttc(6))
        .unwrap();

    assert_eq!(
        h_ttf,
        fc.register_bytes("Inter", 400, false, &sample_ttf(1)).unwrap()
    );

    let list = fc.list_registered().unwrap();
    assert_eq!(list.len(), 6);
    assert_eq!(
        list.iter().find(|f| f.handle == h_ttf).unwrap().format,
        FontFormat::TrueType
    );
    assert_eq!(
        list.iter().find(|f| f.handle == h_apple).unwrap().format,
        FontFormat::TrueType
    );
    assert_eq!(
        list.iter().find(|f| f.handle == h_otf).unwrap().format,
        FontFormat::OpenType
    );
    assert_eq!(
        list.iter().find(|f| f.handle == h_woff).unwrap().format,
        FontFormat::Woff
    );
    assert_eq!(
        list.iter().find(|f| f.handle == h_woff2).unwrap().format,
        FontFormat::Woff2
    );
    assert_eq!(
        list.iter().find(|f| f.handle == h_ttc).unwrap().format,
        FontFormat::TrueTypeCollection
    );

    assert!(matches!(
        fc.register_bytes("Bad", 400, false, &[]),
        Err(ServiceError::InvalidArg(_))
    ));
    assert!(matches!(
        fc.register_bytes("Bad", 400, false, b"NOT_A_FONT_HEADER_BYTES"),
        Err(ServiceError::InvalidArg(_))
    ));
    assert!(matches!(
        fc.register_bytes("BadWoff", 400, false, b"wOFF0123456789012345"),
        Err(ServiceError::InvalidArg(_))
    ));
    assert!(matches!(
        fc.register_bytes("Inter", 0, false, &sample_ttf(9)),
        Err(ServiceError::InvalidArg(_))
    ));

    let good_file = dir.join("custom.otf");
    let bad_file = dir.join("corrupt.otf");
    fs::write(&good_file, sample_otf(42)).unwrap();
    fs::write(&bad_file, b"corrupt_font_bytes_12345").unwrap();

    let h_file = fc
        .register_file("CustomFile", 600, false, good_file.to_str().unwrap())
        .unwrap();
    assert!(matches!(
        fc.register_file("CustomFile", 600, false, bad_file.to_str().unwrap()),
        Err(ServiceError::InvalidArg(_))
    ));
    assert_eq!(
        fc.register_file("Missing", 400, false, dir.join("nope.ttf").to_str().unwrap()),
        Err(ServiceError::NotFound)
    );
    assert!(fc.unregister(h_file).unwrap());
    assert!(!fc.unregister(h_file).unwrap());
    assert!(matches!(
        decode_handle(&[0u8; 8]),
        Err(ServiceError::InvalidArg(_))
    ));

    // MemoryFonts over C-ABI HostCallbackBridge
    let mem_fonts = Arc::new(MemoryFonts::new());
    mem_fonts.add_mock_font_file("/assets/ui.woff2", sample_woff2(77));
    let mem = MemoryHostServices::new().with_handler_arc(mem_fonts);
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let ffi_fonts = FontClient::new(&ffi);
    let fh = ffi_fonts
        .register_file("BrandUI", 500, false, "/assets/ui.woff2")
        .unwrap();
    assert_eq!(ffi_fonts.list_registered().unwrap()[0].handle, fh);

    let mut seed: u64 = 0xF047_5000_5EED_0009;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = validate_font_bytes(&buf);
        let _ = decode_register_bytes_request(&buf);
        let _ = decode_register_file_request(&buf);
        let _ = decode_list_registered_response(&buf);
    }

    let _ = fs::remove_dir_all(&dir);
}
