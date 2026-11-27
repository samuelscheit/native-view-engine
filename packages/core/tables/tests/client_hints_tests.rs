//! Comprehensive unit & adversarial tests for MDN `Sec-CH-*` Client Hints parsing
//! (`nve_tables::client_hints` and `nve_tables::media` re-exports).

use nve_tables::client_hints::{
    ClientHintsSnapshot, SSR_ACCEPT_CH_HEADER, SSR_CRITICAL_CH_HEADER, SSR_VARY_HEADER,
};
use nve_tables::media::{
    ClientHintsSnapshot as MediaClientHintsSnapshot, MediaEnv,
    SSR_ACCEPT_CH_HEADER as MEDIA_ACCEPT_CH, SSR_CRITICAL_CH_HEADER as MEDIA_CRITICAL_CH,
    SSR_VARY_HEADER as MEDIA_VARY,
};

#[test]
fn header_constants_match_mdn_spec_and_reexport_from_media() {
    assert_eq!(SSR_ACCEPT_CH_HEADER, MEDIA_ACCEPT_CH);
    assert_eq!(SSR_CRITICAL_CH_HEADER, MEDIA_CRITICAL_CH);
    assert_eq!(SSR_VARY_HEADER, MEDIA_VARY);

    for token in [
        "Sec-CH-Prefers-Color-Scheme",
        "Sec-CH-Prefers-Reduced-Motion",
        "Sec-CH-Prefers-Reduced-Transparency",
        "Sec-CH-Viewport-Width",
        "Sec-CH-Viewport-Height",
        "Sec-CH-DPR",
        "Sec-CH-UA-Mobile",
        "Sec-CH-UA-Platform",
    ] {
        assert!(
            SSR_ACCEPT_CH_HEADER.contains(token),
            "Accept-CH missing {token}"
        );
    }
    assert_eq!(
        SSR_CRITICAL_CH_HEADER,
        "Sec-CH-Prefers-Color-Scheme, Sec-CH-Viewport-Width"
    );
    assert_eq!(
        SSR_VARY_HEADER,
        "Sec-CH-Prefers-Color-Scheme, Sec-CH-Prefers-Reduced-Motion, Sec-CH-Viewport-Width, Sec-CH-UA-Mobile"
    );
}

#[test]
fn parses_full_desktop_rfc8941_headers_case_insensitively() {
    let headers = [
        ("Sec-CH-Prefers-Color-Scheme", "\"dark\""),
        ("sec-ch-prefers-reduced-motion", "\"reduce\""),
        ("SEC-CH-PREFERS-REDUCED-TRANSPARENCY", "\"reduce\""),
        ("Sec-CH-Viewport-Width", "1440"),
        ("Sec-CH-Viewport-Height", "900"),
        ("Sec-CH-DPR", "2"),
        ("Sec-CH-UA-Mobile", "?0"),
        ("Sec-CH-UA-Platform", "\"macOS\""),
        ("Save-Data", "on"),
    ];

    let snap = ClientHintsSnapshot::from_headers(headers);
    let media_reexport = MediaClientHintsSnapshot::from_headers(headers);
    assert_eq!(snap, media_reexport);

    assert_eq!(snap.media.width_pt, 1440.0);
    assert_eq!(snap.media.height_pt, 900.0);
    assert!(snap.media.dark_mode);
    assert!(snap.media.reduced_motion);
    assert!(!snap.media.coarse_pointer);
    assert_eq!(snap.dpr, Some(2.0));
    assert_eq!(snap.reduced_transparency, Some(true));
    assert_eq!(snap.mobile, Some(false));
    assert_eq!(snap.platform.as_deref(), Some("macOS"));
    assert!(snap.save_data);

    let media_env = MediaEnv::from_client_hints(headers);
    assert_eq!(media_env, snap.media);
}

#[test]
fn mobile_hint_falls_back_to_390x844_and_coarse_pointer_when_viewport_absent() {
    let headers = [
        ("Sec-CH-UA-Mobile", "?1"),
        ("Sec-CH-UA-Platform", "\"iOS\""),
        ("Sec-CH-Prefers-Color-Scheme", "\"light\""),
        ("Sec-CH-Prefers-Reduced-Motion", "\"no-preference\""),
        ("Sec-CH-Prefers-Reduced-Transparency", "\"no-preference\""),
    ];

    let snap = ClientHintsSnapshot::from_headers(headers);
    assert_eq!(snap.mobile, Some(true));
    assert!(snap.media.coarse_pointer);
    assert_eq!(snap.media.width_pt, 390.0);
    assert_eq!(snap.media.height_pt, 844.0);
    assert!(!snap.media.dark_mode);
    assert!(!snap.media.reduced_motion);
    assert_eq!(snap.reduced_transparency, Some(false));
    assert_eq!(snap.platform.as_deref(), Some("iOS"));
    assert!(!snap.save_data);
}

#[test]
fn mobile_hint_respects_explicit_viewport_dimensions_when_present() {
    let snap = ClientHintsSnapshot::from_headers([
        ("Sec-CH-UA-Mobile", "?1"),
        ("Sec-CH-Viewport-Width", "412"),
        ("Sec-CH-Viewport-Height", "915"),
        ("Sec-CH-DPR", "2.625"),
        ("Sec-CH-UA-Platform", "\"Android\""),
    ]);

    assert_eq!(snap.mobile, Some(true));
    assert!(snap.media.coarse_pointer);
    assert_eq!(snap.media.width_pt, 412.0);
    assert_eq!(snap.media.height_pt, 915.0);
    assert_eq!(snap.dpr, Some(2.625));
    assert_eq!(snap.platform.as_deref(), Some("Android"));
}

#[test]
fn mobile_hint_keeps_explicit_height_when_only_width_missing() {
    let snap = ClientHintsSnapshot::from_headers([
        ("Sec-CH-UA-Mobile", "?1"),
        ("Sec-CH-Viewport-Height", "740"),
    ]);

    assert_eq!(snap.media.width_pt, 390.0);
    assert_eq!(snap.media.height_pt, 740.0);
    assert!(snap.media.coarse_pointer);
}

#[test]
fn accepts_unquoted_tokens_and_rfc8941_escapes() {
    let snap = ClientHintsSnapshot::from_headers([
        ("Sec-CH-Prefers-Color-Scheme", "dark"),
        ("Sec-CH-Prefers-Reduced-Motion", "reduce"),
        ("Sec-CH-UA-Platform", "\"Windows \\\"11\\\"\""),
        ("Sec-CH-Viewport-Width", "\"1024.5\""),
    ]);

    assert!(snap.media.dark_mode);
    assert!(snap.media.reduced_motion);
    assert_eq!(snap.platform.as_deref(), Some("Windows \"11\""));
    assert_eq!(snap.media.width_pt, 1024.5);
}

#[test]
fn malformed_and_hostile_headers_never_panic_and_preserve_safe_defaults() {
    let huge = "x".repeat(10_000);
    let headers: [(&str, &str); 18] = [
        ("Sec-CH-Prefers-Color-Scheme", "\""),
        ("Sec-CH-Prefers-Color-Scheme", "\"dark"),
        ("Sec-CH-Prefers-Color-Scheme", "\"\\x\""),
        ("Sec-CH-Prefers-Color-Scheme", "rainbow"),
        ("Sec-CH-Prefers-Reduced-Motion", "fast"),
        ("Sec-CH-Prefers-Reduced-Transparency", ""),
        ("Sec-CH-Viewport-Width", "0"),
        ("Sec-CH-Viewport-Width", "-390"),
        ("Sec-CH-Viewport-Width", "NaN"),
        ("Sec-CH-Viewport-Width", "Infinity"),
        ("Sec-CH-Viewport-Width", "99999999"),
        ("Sec-CH-Viewport-Height", "not-a-number"),
        ("Sec-CH-DPR", "-2.0"),
        ("Sec-CH-DPR", "1000"),
        ("Sec-CH-UA-Mobile", "true"),
        ("Sec-CH-UA-Platform", &huge),
        ("Sec-CH-UA-Platform", "\"bad\x00ctl\""),
        ("Save-Data", "maybe"),
    ];

    let snap = ClientHintsSnapshot::from_headers(headers);
    assert_eq!(snap, ClientHintsSnapshot::default());
}
