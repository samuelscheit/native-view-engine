use std::sync::Arc;

use nve_abi::NveHostEventBatch;
use nve_host_services::{
    FfiHostServices, HostCallbackBridge, HostEvent, HostServiceProvider, MemoryHostServices,
    NativeHostServices, ServiceError,
};
use nve_sensors::{
    decode_sensor_batch_event_ref, decode_sensor_subscribe_request, encode_sensor_batch_event,
    MemorySensors, NativeSensors, SensorClient, SensorKind, SensorSample, SensorSampleSlice,
    SensorSubscriptionId, EVENT_SENSOR_BATCH, SENSOR_SAMPLE_WIRE_LEN, SERVICE_SENSORS,
};

#[test]
fn sensors_subscriptions_zero_copy_sample_batch_native_and_fuzz() {
    let sensors_mem = Arc::new(MemorySensors::new());
    let mem = MemoryHostServices::new().with_handler_arc(sensors_mem.clone());
    let bridge = HostCallbackBridge::new(mem);
    let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
    let client = SensorClient::new(&ffi);

    for kind in [
        SensorKind::Accelerometer,
        SensorKind::Gyroscope,
        SensorKind::Magnetometer,
        SensorKind::Barometer,
        SensorKind::AmbientLight,
    ] {
        assert!(client.is_available(kind).unwrap());
    }
    sensors_mem.set_available(SensorKind::Barometer, false);
    assert!(!client.is_available(SensorKind::Barometer).unwrap());
    assert_eq!(
        client.subscribe(SensorKind::Barometer, 10),
        Err(ServiceError::Unsupported)
    );
    assert!(matches!(
        client.subscribe(SensorKind::Accelerometer, 0),
        Err(ServiceError::InvalidArg(_))
    ));
    let sub = client.subscribe(SensorKind::Accelerometer, 120).unwrap();
    assert_eq!(
        sensors_mem.active_subscriptions(),
        vec![(sub, SensorKind::Accelerometer, 120)]
    );
    client.unsubscribe(sub).unwrap();
    assert_eq!(client.unsubscribe(sub), Err(ServiceError::NotFound));

    // Zero-copy 20-byte SensorSample validation & NVH1 batch push.
    let samples = [
        SensorSample {
            x: 0.125,
            y: -9.81,
            z: 0.05,
            timestamp_ns: 1_000_000,
        },
        SensorSample {
            x: 0.25,
            y: -9.79,
            z: 0.02,
            timestamp_ns: 9_333_333,
        },
    ];
    let wire = samples[0].encode().unwrap();
    assert_eq!(wire.len(), SENSOR_SAMPLE_WIRE_LEN);
    assert_eq!(SensorSample::decode(&wire).unwrap(), samples[0]);
    for len in 0..SENSOR_SAMPLE_WIRE_LEN {
        assert!(SensorSample::decode(&wire[..len]).is_err());
    }
    let mut nan_wire = wire;
    nan_wire[0..4].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert!(SensorSample::decode(&nan_wire).is_err());
    assert!(SensorSampleSlice::from_bytes(&wire[..19]).is_err());

    let batch_wire =
        encode_sensor_batch_event(sub, SensorKind::Accelerometer, &samples).unwrap();
    for len in 0..batch_wire.len() {
        assert!(decode_sensor_batch_event_ref(&batch_wire[..len]).is_err());
    }
    let mut crafted = batch_wire.clone();
    crafted[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_sensor_batch_event_ref(&crafted).is_err());

    let mut nvh1 = NveHostEventBatch::new();
    nvh1.push_host_event(SERVICE_SENSORS, EVENT_SENSOR_BATCH, batch_wire);
    assert_eq!(ffi.dispatch_host_events(&nvh1.encode()).unwrap(), 1);
    let events = ffi.poll_host_events();
    let HostEvent::Push(p) = &events[0] else {
        panic!("expected push");
    };
    let decoded_ref = SensorClient::<FfiHostServices>::decode_push_event_ref(p)
        .unwrap()
        .unwrap();
    assert_eq!(decoded_ref.subscription_id, sub);
    assert_eq!(decoded_ref.samples.len(), 2);
    assert_eq!(decoded_ref.samples.get(0), Some(samples[0]));
    assert_eq!(decoded_ref.samples.get(1), Some(samples[1]));

    // Headless native safety
    let native = NativeHostServices::new().with_handler(NativeSensors::new());
    assert!(!SensorClient::new(&native)
        .is_available(SensorKind::Accelerometer)
        .unwrap());
    assert_eq!(
        SensorClient::new(&native).unsubscribe(SensorSubscriptionId(1)),
        Err(ServiceError::Unsupported)
    );

    let mut seed: u64 = 0x5E45_0450_5EED_0012;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_sensor_subscribe_request(&buf);
        let _ = decode_sensor_batch_event_ref(&buf);
        let _ = SensorSample::decode(&buf);
        let _ = SensorSampleSlice::from_bytes(&buf);
    }
}
