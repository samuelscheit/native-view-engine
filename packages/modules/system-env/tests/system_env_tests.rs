use std::sync::{Arc, Mutex};

use nve_abi::{NveHostEventBatch, NVE_CHANNEL_FLAG_RETAIN, SERVICE_SYSTEM_ENV};
use nve_host_services::{HostServiceProvider, MemoryHostServices, NativeHostServices};
use nve_interop::{NveChannelBus, CHANNEL_ID_LOCALE, CHANNEL_ID_THEME};
use nve_system_env::{
    parse_locale_tag, ColorScheme, ContrastLevel, LocaleSnapshot, MemorySystemEnv, NativeSystemEnv,
    SystemEnvBridge, SystemEnvClient, TextDirection, ThemeSnapshot, EVENT_THEME_CHANGED,
    THEME_WIRE_LEN,
};

#[test]
fn system_env_memory_bridge_retained_replay_native_and_fuzz() {
    let sys_mem = Arc::new(MemorySystemEnv::new());
    let mem = MemoryHostServices::new().with_handler_arc(sys_mem.clone());

    let initial_theme = ThemeSnapshot {
        color_scheme: ColorScheme::Dark,
        contrast: ContrastLevel::High,
        reduced_motion: true,
        reduced_transparency: true,
        accent_rgba: 0x10B9_81FF,
        font_scale_x1000: 1100,
        scale_factor_x100: 200,
    };
    let initial_locale = LocaleSnapshot {
        tag: "de-DE".into(),
        language: "de".into(),
        region: "DE".into(),
        text_direction: TextDirection::Ltr,
        uses_24h_clock: true,
        timezone: "Europe/Berlin".into(),
    };
    sys_mem.set_theme(initial_theme);
    sys_mem.set_locale(initial_locale.clone());

    let mut bus = NveChannelBus::new();
    let bridge = SystemEnvBridge::new();
    let (t, l) = bridge.sync_from_provider(&mem, &mut bus).unwrap();
    assert_eq!(t, initial_theme);
    assert_eq!(l, initial_locale);

    let seen_themes = Arc::new(Mutex::new(Vec::new()));
    let seen_locales = Arc::new(Mutex::new(Vec::new()));
    let st_clone = seen_themes.clone();
    let sl_clone = seen_locales.clone();

    SystemEnvBridge::subscribe_theme(&mut bus, Some(1), true, move |snap| {
        st_clone.lock().unwrap().push(snap);
    })
    .unwrap();
    SystemEnvBridge::subscribe_locale(&mut bus, Some(1), true, move |snap| {
        sl_clone.lock().unwrap().push(snap);
    })
    .unwrap();

    assert_eq!(*seen_themes.lock().unwrap(), vec![initial_theme]);
    assert_eq!(*seen_locales.lock().unwrap(), vec![initial_locale]);

    let updated_theme = ThemeSnapshot {
        color_scheme: ColorScheme::Light,
        ..initial_theme
    };
    let mut nvh1 = NveHostEventBatch::new();
    nvh1.push_host_event(
        SERVICE_SYSTEM_ENV,
        EVENT_THEME_CHANGED,
        updated_theme.encode_nvt2().to_vec(),
    );
    nvh1.push_channel_message(
        CHANNEL_ID_LOCALE,
        NVE_CHANNEL_FLAG_RETAIN,
        LocaleSnapshot {
            tag: "ja-JP".into(),
            language: "ja".into(),
            region: "JP".into(),
            text_direction: TextDirection::Ltr,
            uses_24h_clock: true,
            timezone: "Asia/Tokyo".into(),
        }
        .encode_nvl1()
        .unwrap(),
    );
    assert_eq!(mem.dispatch_host_events(&nvh1.encode()).unwrap(), 2);

    for ev in mem.poll_host_events() {
        assert!(SystemEnvBridge::handle_host_event(&mut bus, &ev).unwrap());
    }
    assert_eq!(
        *seen_themes.lock().unwrap(),
        vec![initial_theme, updated_theme]
    );
    assert_eq!(seen_locales.lock().unwrap().len(), 2);
    assert_eq!(seen_locales.lock().unwrap()[1].tag, "ja-JP");

    let retained_theme = bus.retained("theme").expect("retained theme message");
    assert_eq!(retained_theme.channel_id, CHANNEL_ID_THEME);
    assert_eq!(retained_theme.as_bytes().len(), THEME_WIRE_LEN);
    assert_eq!(&retained_theme.as_bytes()[..4], b"NVT2");

    // Locale tag parser + safe native OS query.
    let ar = parse_locale_tag("ar_EG.UTF-8", None, true);
    assert_eq!(ar.tag, "ar-EG");
    assert_eq!(ar.text_direction, TextDirection::Rtl);
    assert_eq!(parse_locale_tag("POSIX", None, false).tag, "en-US");

    let native = NativeHostServices::new().with_handler(NativeSystemEnv::new());
    let native_client = SystemEnvClient::new(&native);
    let nt = native_client.get_theme().unwrap();
    assert_eq!(ThemeSnapshot::decode_nvt2(&nt.encode_nvt2()).unwrap(), nt);
    let nl = native_client.get_locale().unwrap();
    assert!(!nl.tag.is_empty());

    // Prefix truncation + PRNG fuzz.
    let valid_nvt2 = ThemeSnapshot::default().encode_nvt2().to_vec();
    for len in 0..valid_nvt2.len() {
        assert!(ThemeSnapshot::decode_nvt2(&valid_nvt2[..len]).is_err());
    }
    let valid_nvl1 = LocaleSnapshot::default().encode_nvl1().unwrap();
    for len in 0..valid_nvl1.len() {
        assert!(LocaleSnapshot::decode_nvl1(&valid_nvl1[..len]).is_err());
    }
    let mut seed: u64 = 0x5757_E401_5EED_0005;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..512 {
        let len = i % 64;
        let mut buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        if len >= 4 {
            match i % 3 {
                0 => buf[..4].copy_from_slice(b"NVT2"),
                1 => buf[..4].copy_from_slice(b"NVL1"),
                _ => {}
            }
        }
        let _ = ThemeSnapshot::decode_nvt2(&buf);
        let _ = LocaleSnapshot::decode_nvl1(&buf);
    }
}
