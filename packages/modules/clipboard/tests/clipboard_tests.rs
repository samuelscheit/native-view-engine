use nve_abi::NveHostStatus;
use nve_clipboard::{
    decode_has_text_response, decode_read_text_response, decode_write_text_request,
    ClipboardClient, MemoryClipboard, SERVICE_CLIPBOARD, METHOD_READ_TEXT,
};
use nve_host_services::{
    FfiHostServices, HostCallbackBridge, MemoryHostServices, ServiceError, FFI_STACK_BUF_SIZE,
};

#[test]
fn clipboard_memory_roundtrip_c_abi_large_retry_and_fault_injection() {
    let mem = MemoryHostServices::new().with_handler(MemoryClipboard::new());
    let clip = ClipboardClient::new(&mem);

    assert!(!clip.has_text().unwrap());
    assert_eq!(clip.read_text().unwrap(), None);
    clip.write_text("Hello NVE 🚀").unwrap();
    assert!(clip.has_text().unwrap());
    assert_eq!(clip.read_text().unwrap().as_deref(), Some("Hello NVE 🚀"));
    clip.clear().unwrap();
    assert!(!clip.has_text().unwrap());
    assert_eq!(clip.read_text().unwrap(), None);

    mem.inject_status(
        SERVICE_CLIPBOARD,
        METHOD_READ_TEXT,
        NveHostStatus::PermissionDenied,
    );
    assert_eq!(clip.read_text(), Err(ServiceError::PermissionDenied));

    // >4 KiB payload across C-ABI HostCallbackBridge triggers automatic BufferTooSmall retry.
    let big = "NVE-CLIPBOARD-".repeat(FFI_STACK_BUF_SIZE / 8);
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let ffi_clip = ClipboardClient::new(&ffi);
    ffi_clip.write_text(&big).unwrap();
    assert_eq!(ffi_clip.read_text().unwrap().as_deref(), Some(big.as_str()));
}

#[test]
fn clipboard_codec_fuzz_never_panics() {
    let mut seed: u64 = 0xC11B_0A4D_5EED_0001;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..512 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_read_text_response(&buf);
        let _ = decode_write_text_request(&buf);
        let _ = decode_has_text_response(&buf);
    }
}
