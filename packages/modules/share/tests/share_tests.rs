use std::sync::Arc;

use nve_host_services::{
    FfiHostServices, HostCallbackBridge, MemoryHostServices, NativeHostServices, ServiceError,
};
use nve_share::{
    decode_share_files_request, decode_share_text_request, decode_share_url_request, MemoryShare,
    NativeShare, RecordedShare, ShareClient, ShareResult,
};

#[test]
fn share_sheet_memory_c_abi_native_safety_and_fuzz() {
    let share_mem = Arc::new(MemoryShare::new());
    let mem = MemoryHostServices::new().with_handler_arc(share_mem.clone());
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let client = ShareClient::new(&ffi);

    assert_eq!(
        client
            .share_text("Check out NVE", Some("NVE Release"))
            .unwrap(),
        ShareResult::Completed
    );
    assert_eq!(
        client
            .share_url("https://nve.dev/docs", Some("Docs"))
            .unwrap(),
        ShareResult::Completed
    );
    share_mem.set_result(ShareResult::Dismissed);
    assert_eq!(
        client.share_files(&["/tmp/a.png", "/tmp/b.pdf"]).unwrap(),
        ShareResult::Dismissed
    );
    assert_eq!(
        share_mem.recorded_shares(),
        vec![
            RecordedShare::Text {
                text: "Check out NVE".into(),
                subject: Some("NVE Release".into()),
            },
            RecordedShare::Url {
                url: "https://nve.dev/docs".into(),
                title: Some("Docs".into()),
            },
            RecordedShare::Files(vec!["/tmp/a.png".into(), "/tmp/b.pdf".into()]),
        ]
    );
    assert!(matches!(
        client.share_files::<&str>(&[]),
        Err(ServiceError::InvalidArg(_))
    ));

    let native = NativeHostServices::new().with_handler(NativeShare::new());
    let native_client = ShareClient::new(&native);
    assert_eq!(
        native_client.share_text("hi", None),
        Err(ServiceError::Unsupported)
    );
    assert!(matches!(
        native_client.share_text("", None),
        Err(ServiceError::InvalidArg(_))
    ));

    let mut seed: u64 = 0x54A4_E000_5EED_0013;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_share_text_request(&buf);
        let _ = decode_share_url_request(&buf);
        let _ = decode_share_files_request(&buf);
    }
}
