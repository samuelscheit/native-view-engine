use std::sync::{Arc, Mutex};

use nve_host_services::{MemoryHostServices, NativeHostServices, ServiceError};
use nve_sensors::{
    decode_sensor_subscribe_request, emit_sensor_batch, MemorySensors, NativeSensors,
    SensorBatchEvent, SensorClient, SensorKind, SensorSample, SensorSubscriptionId,
};

#[test]
fn sensors_subscriptions_direct_typed_sample_batches_and_native_safety() {
    let sensors_mem = Arc::new(MemorySensors::new());
    let mem = MemoryHostServices::new().with_handler_arc(sensors_mem.clone());
    let client = SensorClient::new(&mem);

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

    // Direct typed SensorSample & SensorBatchEvent delivery via TypedEventBus (zero serialization).
    let samples = vec![
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
    assert!(samples[0].validate().is_ok());
    assert!(SensorSample {
        x: f32::INFINITY,
        y: 0.0,
        z: 0.0,
        timestamp_ns: 1,
    }
    .validate()
    .is_err());
    assert!(SensorBatchEvent::new(SensorSubscriptionId(0), SensorKind::Accelerometer, samples.clone()).is_err());

    let seen = Arc::new(Mutex::new(Vec::<SensorBatchEvent>::new()));
    let seen_clone = Arc::clone(&seen);
    let _ev_sub = client
        .subscribe_events(move |ev| {
            seen_clone.lock().unwrap().push(ev.clone());
        })
        .unwrap();

    let batch = SensorBatchEvent::new(sub, SensorKind::Accelerometer, samples.clone()).unwrap();
    assert_eq!(emit_sensor_batch(mem.typed_events(), batch.clone()).unwrap(), 1);

    assert_eq!(*seen.lock().unwrap(), vec![batch.clone()]);
    let drained = client.drain_events();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].subscription_id, sub);
    assert_eq!(drained[0].kind, SensorKind::Accelerometer);
    assert_eq!(drained[0].samples, samples);

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
    }
}
