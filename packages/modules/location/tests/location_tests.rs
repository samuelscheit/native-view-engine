use std::sync::Arc;

use nve_abi::NveHostEventBatch;
use nve_host_services::{
    FfiHostServices, HostCallbackBridge, HostEvent, HostServiceProvider, MemoryHostServices,
    NativeHostServices, PermissionStatus, ServiceError,
};
use nve_location::{
    decode_geo_fix, decode_location_update_event, decode_watch_config, encode_geo_fix,
    encode_location_update_event, GeoFix, LocationAccuracy, LocationClient, LocationUpdateEvent,
    LocationWatchConfig, MemoryLocation, NativeLocation, EVENT_LOCATION_UPDATE, SERVICE_LOCATION,
};

#[test]
fn location_memory_watches_nvh1_push_native_safety_and_fuzz() {
    let loc_mem = Arc::new(MemoryLocation::new());
    let mem = MemoryHostServices::new().with_handler_arc(loc_mem.clone());
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let client = LocationClient::new(&ffi);

    assert_eq!(
        client.request_permission().unwrap(),
        PermissionStatus::Authorized
    );
    let custom_fix = GeoFix {
        latitude: 48.1351,
        longitude: 11.5820,
        altitude_m: Some(519.0),
        accuracy_m: 3.5,
        speed_mps: Some(1.2),
        heading_deg: Some(180.0),
        timestamp_ms: 1_750_000_000_000,
    };
    loc_mem.set_current_fix(custom_fix);
    assert_eq!(
        client
            .current_position(LocationAccuracy::BestForNavigation)
            .unwrap(),
        custom_fix
    );

    let watch = client
        .watch_position(&LocationWatchConfig {
            accuracy: LocationAccuracy::High,
            min_interval_ms: 500,
            min_distance_m: 2.5,
        })
        .unwrap();
    assert_eq!(loc_mem.active_watches(), vec![watch]);
    client.stop_watch(watch).unwrap();
    assert_eq!(client.stop_watch(watch), Err(ServiceError::NotFound));

    loc_mem.set_permission(PermissionStatus::Restricted);
    assert_eq!(
        client.current_position(LocationAccuracy::Balanced),
        Err(ServiceError::PermissionDenied)
    );

    assert!(encode_geo_fix(&GeoFix {
        latitude: 95.0,
        longitude: 0.0,
        altitude_m: None,
        accuracy_m: 1.0,
        speed_mps: None,
        heading_deg: None,
        timestamp_ms: 1,
    })
    .is_err());

    let loc_ev = LocationUpdateEvent {
        watch_id: watch,
        fix: custom_fix,
    };
    let mut batch = NveHostEventBatch::new();
    batch.push_host_event(
        SERVICE_LOCATION,
        EVENT_LOCATION_UPDATE,
        encode_location_update_event(&loc_ev).unwrap(),
    );
    assert_eq!(ffi.dispatch_host_events(&batch.encode()).unwrap(), 1);
    let events = ffi.poll_host_events();
    let HostEvent::Push(p) = &events[0] else {
        panic!("expected push");
    };
    assert_eq!(
        LocationClient::<FfiHostServices>::decode_push_event(p).unwrap(),
        Some(loc_ev)
    );

    let native = NativeHostServices::new().with_handler(NativeLocation::new());
    assert_eq!(
        LocationClient::new(&native).request_permission(),
        Err(ServiceError::Unsupported)
    );

    let mut seed: u64 = 0x10CA_7104_5EED_0011;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_geo_fix(&buf);
        let _ = decode_watch_config(&buf);
        let _ = decode_location_update_event(&buf);
    }
}
