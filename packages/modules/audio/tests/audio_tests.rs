use std::sync::{Arc, Mutex};

use nve_audio::{
    decode_clip_meta, decode_load_source_request, decode_record_config, emit_audio_event,
    AudioClient, AudioClipMeta, AudioEncoding, AudioEvent, AudioLevelEvent, AudioPlaybackEvent,
    AudioPlaybackState, AudioRecordConfig, AudioSource, MemoryAudio, NativeAudio,
};
use nve_host_services::{MemoryHostServices, NativeHostServices, ServiceError};

#[test]
fn audio_playback_recording_typed_events_and_native_safety() {
    let audio_mem = Arc::new(MemoryAudio::new());
    let mem = MemoryHostServices::new().with_handler_arc(audio_mem.clone());
    let client = AudioClient::new(&mem);

    let h_uri = client
        .load_source(&AudioSource::Uri("https://cdn.nve.dev/track.opus".into()))
        .unwrap();
    let h_bytes = client
        .load_source(&AudioSource::Bytes(vec![0x52, 0x49, 0x46, 0x46]))
        .unwrap();
    client.play(h_uri).unwrap();
    client.seek_ms(h_uri, 12_345).unwrap();
    client.set_volume(h_uri, 0.75).unwrap();
    client.set_looping(h_uri, true).unwrap();
    client.pause(h_uri).unwrap();

    let track = audio_mem.track(h_uri).unwrap();
    assert_eq!(track.state, AudioPlaybackState::Paused);
    assert_eq!(track.position_ms, 12_345);
    assert_eq!(track.volume, 0.75);
    assert!(track.looping);

    assert!(matches!(
        client.set_volume(h_uri, 1.5),
        Err(ServiceError::InvalidArg(_))
    ));
    assert!(matches!(
        client.set_volume(h_uri, f32::NAN),
        Err(ServiceError::InvalidArg(_))
    ));
    client.stop(h_uri).unwrap();
    client.unload(h_bytes).unwrap();
    assert_eq!(client.play(h_bytes), Err(ServiceError::NotFound));

    let rec_cfg = AudioRecordConfig {
        sample_rate_hz: 44_100,
        channels: 2,
        encoding: AudioEncoding::Opus,
        bit_rate_bps: Some(128_000),
        output_path: Some("/mem/tmp/note.opus".into()),
        max_duration_ms: Some(30_000),
    };
    client.start_recording(&rec_cfg).unwrap();
    assert!(matches!(
        client.start_recording(&rec_cfg),
        Err(ServiceError::InvalidArg(_))
    ));
    let clip: AudioClipMeta = client.stop_recording().unwrap();
    assert_eq!(clip.encoding, AudioEncoding::Opus);
    assert_eq!(clip.sample_rate_hz, 44_100);
    assert_eq!(clip.channels, 2);
    assert_eq!(clip.path.as_deref(), Some("/mem/tmp/note.opus"));
    assert_eq!(client.stop_recording(), Err(ServiceError::NotFound));

    // Direct typed AudioEvent delivery via TypedEventBus (zero serialization).
    let seen = Arc::new(Mutex::new(Vec::<AudioEvent>::new()));
    let seen_clone = Arc::clone(&seen);
    let _sub = client
        .subscribe_events(move |ev| {
            seen_clone.lock().unwrap().push(*ev);
        })
        .unwrap();

    let playback_ev = AudioPlaybackEvent {
        handle: h_uri,
        state: AudioPlaybackState::Playing,
        position_ms: 4200,
        duration_ms: 180_000,
    };
    let level_ev = AudioLevelEvent {
        handle: Some(h_uri),
        rms_db: -18.5,
        peak_db: -6.2,
        timestamp_ms: 99_001,
    };
    assert!(emit_audio_event(
        mem.typed_events(),
        AudioEvent::Level(AudioLevelEvent {
            handle: None,
            rms_db: f32::NEG_INFINITY,
            peak_db: -1.0,
            timestamp_ms: 1,
        }),
    )
    .is_err());

    assert_eq!(
        emit_audio_event(mem.typed_events(), AudioEvent::Playback(playback_ev)).unwrap(),
        1
    );
    assert_eq!(
        emit_audio_event(mem.typed_events(), AudioEvent::Level(level_ev)).unwrap(),
        1
    );

    assert_eq!(
        *seen.lock().unwrap(),
        vec![AudioEvent::Playback(playback_ev), AudioEvent::Level(level_ev)]
    );
    let drained = client.drain_events();
    assert_eq!(drained.len(), 2);
    assert_eq!(*drained[0], AudioEvent::Playback(playback_ev));
    assert_eq!(*drained[1], AudioEvent::Level(level_ev));

    // Native headless safety
    let native = NativeHostServices::new().with_handler(NativeAudio::new());
    assert_eq!(
        AudioClient::new(&native).stop_recording(),
        Err(ServiceError::Unsupported)
    );

    let mut seed: u64 = 0xA0D1_0000_5EED_0010;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_load_source_request(&buf);
        let _ = decode_record_config(&buf);
        let _ = decode_clip_meta(&buf);
    }
}
