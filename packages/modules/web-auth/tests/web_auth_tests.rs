use std::sync::Arc;

use nve_abi::{NveHostEventBatch, NveHostStatus};
use nve_host_services::{
    FfiHostServices, HostCallbackBridge, HostServiceProvider, InvokeResult, MemoryHostServices,
    NativeHostServices, ServiceError,
};
use nve_web_auth::{
    decode_authenticate_request, decode_open_browser_request, decode_web_auth_result,
    encode_authenticate_request, encode_web_auth_result, BrowserPresentation, InAppBrowserOptions,
    MemoryWebAuth, NativeWebAuth, RecordedBrowserLaunch, WebAuthClient, WebAuthResult,
    METHOD_AUTHENTICATE, SERVICE_WEB_AUTH,
};

#[test]
fn web_auth_browser_oauth_async_nvh1_native_safety_and_fuzz() {
    let auth_mem = Arc::new(MemoryWebAuth::new());
    let mem = MemoryHostServices::new().with_handler_arc(auth_mem.clone());
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let client = WebAuthClient::new(&ffi);

    let browser_opts = InAppBrowserOptions {
        presentation: BrowserPresentation::FullScreen,
        toolbar_color_rgba: Some(0x1122_33FF),
        ephemeral_session: true,
        show_title: true,
    };
    client
        .open_in_app_browser("https://docs.nve.dev", &browser_opts)
        .unwrap();
    assert_eq!(
        auth_mem.opened_browsers(),
        vec![RecordedBrowserLaunch {
            url: "https://docs.nve.dev".into(),
            options: browser_opts,
        }]
    );
    assert!(matches!(
        client.open_in_app_browser("javascript:alert(1)", &InAppBrowserOptions::default()),
        Err(ServiceError::InvalidArg(_))
    ));
    assert!(matches!(
        client.authenticate("https://auth.nve.dev/login", "bad:scheme"),
        Err(ServiceError::InvalidArg(_))
    ));
    assert_eq!(
        client
            .authenticate("https://auth.nve.dev/authorize", "nveapp")
            .unwrap(),
        WebAuthResult::CallbackUri("nveapp://callback?code=mock_auth_code".into())
    );
    auth_mem.set_result(Some(WebAuthResult::Cancelled));
    assert_eq!(
        client
            .authenticate("https://auth.nve.dev/authorize", "nveapp")
            .unwrap(),
        WebAuthResult::Cancelled
    );

    // Async Pending + NVH1 resolution
    bridge
        .provider()
        .inject_async_pending(SERVICE_WEB_AUTH, METHOD_AUTHENTICATE);
    let req = encode_authenticate_request("https://auth.nve.dev/oauth", "nveapp").unwrap();
    let InvokeResult::Pending { request_id } = ffi
        .invoke_raw(SERVICE_WEB_AUTH, METHOD_AUTHENTICATE, &req)
        .unwrap()
    else {
        panic!("expected Pending");
    };
    let mut batch = NveHostEventBatch::new();
    batch.push_async_response(
        request_id,
        NveHostStatus::Ok,
        encode_web_auth_result(&WebAuthResult::CallbackUri(
            "nveapp://cb?code=async_42".into(),
        ))
        .unwrap(),
    );
    assert_eq!(ffi.dispatch_host_events(&batch.encode()).unwrap(), 1);
    let resolved = ffi.take_async_response(request_id).unwrap().unwrap();
    assert_eq!(
        decode_web_auth_result(&resolved).unwrap(),
        WebAuthResult::CallbackUri("nveapp://cb?code=async_42".into())
    );

    // Native safety
    let native = NativeHostServices::new().with_handler(NativeWebAuth::new());
    assert_eq!(
        WebAuthClient::new(&native).authenticate("https://example.com/auth", "myapp"),
        Err(ServiceError::Unsupported)
    );

    let mut seed: u64 = 0xAEB0_A074_5EED_0014;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_open_browser_request(&buf);
        let _ = decode_authenticate_request(&buf);
        let _ = decode_web_auth_result(&buf);
    }
}
