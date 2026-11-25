use std::sync::Arc;

use nve_haptics::{
    decode_impact_request, decode_notification_haptic_request, HapticsClient, ImpactStyle,
    MemoryHaptics, NotificationHaptic, RecordedHaptic,
};
use nve_host_services::MemoryHostServices;

#[test]
fn haptics_memory_recording_and_codec_fuzz() {
    let haptics_mem = Arc::new(MemoryHaptics::new());
    let host = MemoryHostServices::new().with_handler_arc(haptics_mem.clone());
    let client = HapticsClient::new(&host);

    client.impact(ImpactStyle::Heavy).unwrap();
    client.notification(NotificationHaptic::Success).unwrap();
    client.selection().unwrap();
    assert_eq!(
        haptics_mem.log(),
        vec![
            RecordedHaptic::Impact(ImpactStyle::Heavy),
            RecordedHaptic::Notification(NotificationHaptic::Success),
            RecordedHaptic::Selection,
        ]
    );

    let mut seed: u64 = 0x4A97_1C50_5EED_0007;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..256 {
        let len = i % 16;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_impact_request(&buf);
        let _ = decode_notification_haptic_request(&buf);
    }
}
