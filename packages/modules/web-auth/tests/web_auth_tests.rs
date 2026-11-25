use std::sync::{Arc, Mutex};

use nve_host_services::{MemoryHostServices, NativeHostServices, ServiceError};
use nve_web_auth::{
    decode_authenticate_request, decode_open_browser_request, decode_web_auth_result,
    emit_web_auth_completed, BrowserPresentation, InAppBrowserOptions, MemoryWebAuth,
    NativeWebAuth, RecordedBrowserLaunch, WebAuthClient, WebAuthCompleted, WebAuthResult,
};

#[test]
fn web_auth_browser_oauth_typed_async_events_and_native_safety() {
    let auth_mem = Arc::new(MemoryWebAuth::new());
    let mem = MemoryHostServices::new().with_handler_arc(auth_mem.clone());
    let client = WebAuthClient::new(&mem);

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

    // Direct typed async completion via TypedAsyncTable + TypedEventBus (zero serialization).
    let req_id = mem.register_typed_async::<WebAuthResult, fn(_)>(None);
    assert!(mem.take_typed_async::<WebAuthResult>(req_id).is_none());
    let async_res = WebAuthResult::CallbackUri("nveapp://cb?code=async_42".into());
    mem.complete_typed_async(req_id, Ok(async_res.clone()));
    assert_eq!(
        mem.take_typed_async::<WebAuthResult>(req_id)
            .unwrap()
            .unwrap()
            .as_ref(),
        &async_res
    );

    let seen = Arc::new(Mutex::new(Vec::<WebAuthCompleted>::new()));
    let seen_clone = Arc::clone(&seen);
    let _sub = client
        .subscribe_completions(move |ev| {
            seen_clone.lock().unwrap().push(ev.clone());
        })
        .unwrap();
    let completed = WebAuthCompleted {
        request_id: req_id,
        result: Ok(async_res),
    };
    assert_eq!(
        emit_web_auth_completed(mem.typed_events(), completed.clone()),
        1
    );
    assert_eq!(*seen.lock().unwrap(), vec![completed.clone()]);
    assert_eq!(client.drain_completions(), vec![Arc::new(completed)]);

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
