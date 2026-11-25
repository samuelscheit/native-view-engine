use std::sync::{Arc, Mutex};

use nve_host_services::{MemoryHostServices, NativeHostServices, TypedEventBus};
use nve_system_env::{
    parse_locale_tag, ColorScheme, ContrastLevel, LocaleSnapshot, MemorySystemEnv, NativeSystemEnv,
    SystemEnvBridge, SystemEnvClient, SystemEnvEvent, TextDirection, ThemeSnapshot,
    THEME_SNAPSHOT_REPR_C_LEN,
};

#[test]
fn system_env_direct_typed_bridge_retained_replay_listeners_and_native() {
    let sys_mem = Arc::new(MemorySystemEnv::new());
    let mem = MemoryHostServices::new().with_handler_arc(sys_mem.clone());

    let direct_events = Arc::new(Mutex::new(Vec::<SystemEnvEvent>::new()));
    let de_clone = Arc::clone(&direct_events);
    sys_mem.add_listener(move |ev| {
        de_clone.lock().unwrap().push(ev.clone());
    });

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
    assert_eq!(
        *direct_events.lock().unwrap(),
        vec![
            SystemEnvEvent::ThemeChanged(initial_theme),
            SystemEnvEvent::LocaleChanged(initial_locale.clone()),
        ]
    );

    let bus = TypedEventBus::new();
    let bridge = SystemEnvBridge::new();
    let (t, l) = bridge.sync_from_provider(&mem, &bus).unwrap();
    assert_eq!(t, initial_theme);
    assert_eq!(l, initial_locale);

    // Late subscribers with replay_retained=true receive typed snapshots directly (zero bytes).
    let seen_themes = Arc::new(Mutex::new(Vec::<ThemeSnapshot>::new()));
    let seen_locales = Arc::new(Mutex::new(Vec::<LocaleSnapshot>::new()));
    let st_clone = Arc::clone(&seen_themes);
    let sl_clone = Arc::clone(&seen_locales);

    let _sub_t = SystemEnvBridge::subscribe_theme(&bus, true, move |snap| {
        st_clone.lock().unwrap().push(*snap);
    });
    let _sub_l = SystemEnvBridge::subscribe_locale(&bus, true, move |snap| {
        sl_clone.lock().unwrap().push(snap.clone());
    });
    assert_eq!(*seen_themes.lock().unwrap(), vec![initial_theme]);
    assert_eq!(*seen_locales.lock().unwrap(), vec![initial_locale]);

    // Live updates flow directly as typed Rust structs without byte codecs.
    let updated_theme = ThemeSnapshot {
        color_scheme: ColorScheme::Light,
        ..initial_theme
    };
    let ja_locale = LocaleSnapshot {
        tag: "ja-JP".into(),
        language: "ja".into(),
        region: "JP".into(),
        text_direction: TextDirection::Ltr,
        uses_24h_clock: true,
        timezone: "Asia/Tokyo".into(),
    };
    sys_mem.set_theme_and_publish(&bus, updated_theme);
    sys_mem.set_locale_and_publish(&bus, ja_locale.clone());

    assert_eq!(
        *seen_themes.lock().unwrap(),
        vec![initial_theme, updated_theme]
    );
    assert_eq!(seen_locales.lock().unwrap().len(), 2);
    assert_eq!(seen_locales.lock().unwrap()[1], ja_locale);

    let retained_theme = bus
        .retained::<ThemeSnapshot>()
        .expect("retained ThemeSnapshot");
    assert_eq!(*retained_theme, updated_theme);
    let retained_locale = bus
        .retained::<LocaleSnapshot>()
        .expect("retained LocaleSnapshot");
    assert_eq!(*retained_locale, ja_locale);

    // Client-level typed subscriptions and queue draining over MemoryHostServices's TypedEventBus.
    let client = SystemEnvClient::new(&mem);
    let client_themes = Arc::new(Mutex::new(Vec::new()));
    let ct_clone = Arc::clone(&client_themes);
    let _c_sub = client
        .subscribe_theme(false, move |snap| {
            ct_clone.lock().unwrap().push(*snap);
        })
        .unwrap();
    SystemEnvBridge::publish_theme(mem.typed_events(), updated_theme);
    assert_eq!(*client_themes.lock().unwrap(), vec![updated_theme]);
    assert_eq!(
        client.drain_events().as_slice(),
        &[Arc::new(SystemEnvEvent::ThemeChanged(updated_theme))]
    );

    // Locale tag parser + safe native OS query.
    let ar = parse_locale_tag("ar_EG.UTF-8", None, true);
    assert_eq!(ar.tag, "ar-EG");
    assert_eq!(ar.text_direction, TextDirection::Rtl);
    assert_eq!(parse_locale_tag("POSIX", None, false).tag, "en-US");

    let native = NativeHostServices::new().with_handler(NativeSystemEnv::new());
    let native_client = SystemEnvClient::new(&native);
    let nt = native_client.get_theme().unwrap();
    let repr_c = nt.to_repr_c_bytes();
    assert_eq!(repr_c.len(), THEME_SNAPSHOT_REPR_C_LEN);
    assert_eq!(ThemeSnapshot::from_repr_c_bytes(&repr_c).unwrap(), nt);
    let nl = native_client.get_locale().unwrap();
    assert!(!nl.tag.is_empty());

    // Invalid C-ABI POD lengths & enum discriminants rejected cleanly.
    for len in 0..THEME_SNAPSHOT_REPR_C_LEN {
        assert!(ThemeSnapshot::from_repr_c_bytes(&repr_c[..len]).is_err());
    }
    let mut bad = repr_c;
    bad[0] = 99;
    assert!(ThemeSnapshot::from_repr_c_bytes(&bad).is_err());
}
