use std::sync::Arc;

use nve_host_services::{HostEvent, HostServiceProvider, MemoryHostServices, ServiceError};
use nve_linking::{
    decode_can_open_response, decode_initial_url_response, decode_url_request,
    encode_url_received_event, LinkingClient, LinkingEvent, MemoryLinking, EVENT_URL_RECEIVED,
    SERVICE_LINKING,
};

#[test]
fn linking_roundtrip_push_events_and_codec_fuzz() {
    let link_mem = Arc::new(MemoryLinking::new());
    let host = MemoryHostServices::new().with_handler_arc(link_mem.clone());
    let client = LinkingClient::new(&host);

    link_mem.set_initial_url(Some("nve://workspace/42".into()));
    assert_eq!(
        client.initial_url().unwrap().as_deref(),
        Some("nve://workspace/42")
    );
    assert!(client.can_open_url("https://nve.dev").unwrap());
    assert!(!client.can_open_url("customscheme://x").unwrap());
    client.open_url("https://nve.dev/docs").unwrap();
    assert_eq!(
        link_mem.opened_urls(),
        vec!["https://nve.dev/docs".to_string()]
    );
    assert!(matches!(
        client.open_url("not a valid url"),
        Err(ServiceError::InvalidArg(_))
    ));

    let push_bytes = encode_url_received_event("nve://doc/99?tab=inspect").unwrap();
    host.push_host_event(SERVICE_LINKING, EVENT_URL_RECEIVED, push_bytes);
    let polled = host.poll_host_events();
    assert_eq!(polled.len(), 1);
    let HostEvent::Push(ref push) = polled[0] else {
        panic!("expected push event");
    };
    assert_eq!(
        LinkingClient::<MemoryHostServices>::decode_push_event(push).unwrap(),
        Some(LinkingEvent::UrlReceived {
            url: "nve://doc/99?tab=inspect".into(),
        })
    );

    let mut seed: u64 = 0x114B_1460_5EED_0004;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..512 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_url_request(&buf);
        let _ = decode_can_open_response(&buf);
        let _ = decode_initial_url_response(&buf);
    }
}
