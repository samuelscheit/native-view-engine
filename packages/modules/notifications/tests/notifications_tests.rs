use std::sync::Arc;

use nve_host_services::{
    HostEvent, HostServiceProvider, MemoryHostServices, NativeHostServices, ServiceError,
};
use nve_notifications::{
    decode_cancel_request, decode_permission_response, decode_show_request, decode_tapped_event,
    encode_show_request, encode_tapped_event, MemoryNotifications, NativeNotifications,
    NotificationPermission, NotificationRequest, NotificationTapped, NotificationsClient,
    EVENT_NOTIFICATION_TAPPED, SERVICE_NOTIFICATIONS,
};

#[test]
fn notifications_memory_push_native_unbundled_safety_and_fuzz() {
    let notif_mem = Arc::new(MemoryNotifications::new());
    let host = MemoryHostServices::new().with_handler_arc(notif_mem.clone());
    let client = NotificationsClient::new(&host);

    assert_eq!(
        client.request_permission().unwrap(),
        NotificationPermission::Authorized
    );
    let mut notif = NotificationRequest::new("build-1", "Build Finished", "Release binary ready");
    notif.subtitle = Some("nve-notifications".into());
    notif.badge = Some(3);
    notif.action_url = Some("nve://builds/1".into());
    client.show(&notif).unwrap();
    assert_eq!(notif_mem.active_notifications(), vec![notif.clone()]);
    client.cancel("build-1").unwrap();
    assert!(notif_mem.active_notifications().is_empty());
    assert_eq!(
        notif_mem.cancelled_notifications(),
        vec!["build-1".to_string()]
    );

    notif_mem.set_permission(NotificationPermission::Denied);
    assert_eq!(
        client.show(&NotificationRequest::new("n2", "Blocked", "Denied")),
        Err(ServiceError::PermissionDenied)
    );

    let tapped = NotificationTapped {
        id: "build-1".into(),
        action_url: Some("nve://builds/1".into()),
    };
    host.push_host_event(
        SERVICE_NOTIFICATIONS,
        EVENT_NOTIFICATION_TAPPED,
        encode_tapped_event(&tapped).unwrap(),
    );
    let events = host.poll_host_events();
    let HostEvent::Push(ref push) = events[0] else {
        panic!("expected push");
    };
    assert_eq!(
        NotificationsClient::<MemoryHostServices>::decode_push_event(push).unwrap(),
        Some(tapped)
    );

    // Unbundled `cargo test` binary must NEVER crash via ObjC `UNUserNotificationCenter` exception!
    let native = NativeHostServices::new().with_handler(NativeNotifications::new());
    let native_client = NotificationsClient::new(&native);
    assert_eq!(
        native_client.request_permission(),
        Err(ServiceError::Unsupported)
    );
    assert_eq!(
        native_client.show(&NotificationRequest::new("t", "Title", "Body")),
        Err(ServiceError::Unsupported)
    );

    // Prefix truncation + PRNG fuzz.
    let valid_show = encode_show_request(&notif).unwrap();
    for len in 0..valid_show.len() {
        assert!(decode_show_request(&valid_show[..len]).is_err());
    }
    let mut seed: u64 = 0x4071_F100_5EED_0006;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..512 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_permission_response(&buf);
        let _ = decode_show_request(&buf);
        let _ = decode_cancel_request(&buf);
        let _ = decode_tapped_event(&buf);
    }
}
