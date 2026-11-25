use std::sync::{Arc, Mutex};

use nve_host_services::{MemoryHostServices, NativeHostServices, ServiceError};
use nve_notifications::{
    emit_notification_event, MemoryNotifications, NativeNotifications, NotificationEvent,
    NotificationPermission, NotificationRequest, NotificationTapped, NotificationsClient,
};

#[test]
fn notifications_direct_typed_memory_events_validation_and_native_unbundled_safety() {
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

    // Validation rejects empty id/title cleanly.
    assert!(matches!(
        client.show(&NotificationRequest::new("", "Title", "Body")),
        Err(ServiceError::InvalidArg(_))
    ));
    assert!(matches!(
        client.show(&NotificationRequest::new("id", "", "Body")),
        Err(ServiceError::InvalidArg(_))
    ));
    assert!(matches!(client.cancel(""), Err(ServiceError::InvalidArg(_))));

    notif_mem.set_permission(NotificationPermission::Denied);
    assert_eq!(
        client.show(&NotificationRequest::new("n2", "Blocked", "Denied")),
        Err(ServiceError::PermissionDenied)
    );

    // Direct typed NotificationEvent delivery via TypedEventBus (zero serialization).
    let seen = Arc::new(Mutex::new(Vec::<NotificationEvent>::new()));
    let seen_clone = Arc::clone(&seen);
    let _sub = client
        .subscribe_events(move |ev| {
            seen_clone.lock().unwrap().push(ev.clone());
        })
        .unwrap();

    let tapped = NotificationTapped {
        id: "build-1".into(),
        action_url: Some("nve://builds/1".into()),
    };
    assert_eq!(
        notif_mem.emit_tapped(host.typed_events(), tapped.clone()).unwrap(),
        1
    );
    assert!(matches!(
        emit_notification_event(
            host.typed_events(),
            NotificationEvent::Tapped(NotificationTapped {
                id: String::new(),
                action_url: None,
            })
        ),
        Err(ServiceError::InvalidArg(_))
    ));

    let expected = NotificationEvent::Tapped(tapped);
    assert_eq!(*seen.lock().unwrap(), vec![expected.clone()]);
    assert_eq!(client.drain_events(), vec![Arc::new(expected)]);

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
}
