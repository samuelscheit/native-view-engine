use nve_abi::NveHostStatus;
use nve_host_services::{
    FfiHostServices, HostCallbackBridge, MemoryHostServices, ServiceError,
};
use nve_secure_store::{
    decode_delete_response, decode_get_response, decode_key_request, decode_set_request,
    MemorySecureStore, SecureStoreClient, METHOD_GET, SERVICE_SECURE_STORE,
};

#[test]
fn secure_store_memory_c_abi_validation_and_fuzz() {
    let mem = MemoryHostServices::new().with_handler(MemorySecureStore::new());
    let client = SecureStoreClient::new(&mem);

    assert_eq!(client.get("app.nve", "token").unwrap(), None);
    client.set("app.nve", "raw", &[0x00, 0xFF, 0x10, 0x20]).unwrap();
    assert_eq!(
        client.get("app.nve", "raw").unwrap(),
        Some(vec![0x00, 0xFF, 0x10, 0x20])
    );
    client.set_string("app.nve", "token", "s3cr3t-jwt").unwrap();
    assert_eq!(
        client.get_string("app.nve", "token").unwrap().as_deref(),
        Some("s3cr3t-jwt")
    );
    assert!(client.delete("app.nve", "token").unwrap());
    assert!(!client.delete("app.nve", "token").unwrap());
    assert!(matches!(
        client.get("", "token"),
        Err(ServiceError::InvalidArg(_))
    ));

    mem.inject_status(
        SERVICE_SECURE_STORE,
        METHOD_GET,
        NveHostStatus::PermissionDenied,
    );
    assert_eq!(
        client.get("app.nve", "raw"),
        Err(ServiceError::PermissionDenied)
    );

    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let ffi_client = SecureStoreClient::new(&ffi);
    ffi_client.set_string("svc.cabi", "user", "tok-123").unwrap();
    assert_eq!(
        ffi_client.get_string("svc.cabi", "user").unwrap().as_deref(),
        Some("tok-123")
    );

    let mut seed: u64 = 0x5EC0_4E57_5EED_0003;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..512 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_key_request(&buf);
        let _ = decode_set_request(&buf);
        let _ = decode_get_response(&buf);
        let _ = decode_delete_response(&buf);
    }
}
