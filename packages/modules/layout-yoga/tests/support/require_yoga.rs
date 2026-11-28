//! Skips real-Yoga tests on machines built without `RN_REACT_COMMON_DIR` (fails instead when
//! `NVE_REQUIRE_YOGA` is set).

use nve_layout_yoga::YogaLayoutEngine;

pub fn require_yoga_or_skip() -> bool {
    if YogaLayoutEngine::is_available() {
        return true;
    }
    assert!(
        std::env::var_os("NVE_REQUIRE_YOGA").is_none(),
        "NVE_REQUIRE_YOGA is set but real React Native Yoga was not compiled in (set RN_REACT_COMMON_DIR)"
    );
    false
}
