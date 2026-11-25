//! Cross-platform soft keyboard insets (`KeyboardInsets`), double-padding-free avoidance
//! geometry (`effective_bottom_inset`), and Web `window.visualViewport` occlusion calculation.

/// Minimum vertical visual-viewport occlusion (in logical CSS px) treated as a virtual keyboard
/// rather than browser chrome or subpixel rounding jitter.
pub const VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX: f32 = 80.0;

/// Soft/virtual keyboard occlusion geometry propagated from mobile and web hosts.
///
/// Compact 8-byte `#[repr(C)]` layout (zero padding, zero serialization) shared across
/// `nve-interop::traits::TraitEnvironment`, `nve-backend-web`, and `nve-ui`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct KeyboardInsets {
    /// Occluded vertical height at the bottom of the viewport in logical points/dp (`>= 0.0`).
    pub height: f32,
    /// Platform keyboard presentation/dismissal animation duration in milliseconds.
    pub animation_duration_ms: u16,
    /// Whether the on-screen keyboard is currently visible or interactively presenting.
    pub visible: bool,
    /// Whether the host supports interactive drag-to-dismiss tracking (e.g. iOS scroll dismiss).
    pub interactive_dismiss: bool,
}

const _: () = {
    assert!(core::mem::size_of::<KeyboardInsets>() == 8);
    assert!(core::mem::align_of::<KeyboardInsets>() == 4);
};

impl KeyboardInsets {
    /// Hidden / dismissed keyboard with zero occlusion.
    pub const HIDDEN: Self = Self {
        height: 0.0,
        animation_duration_ms: 0,
        visible: false,
        interactive_dismiss: false,
    };

    /// Constructs a [`KeyboardInsets`] value, clamping negative or non-finite heights to `0.0`.
    #[inline]
    pub const fn new(
        height: f32,
        visible: bool,
        animation_duration_ms: u16,
        interactive_dismiss: bool,
    ) -> Self {
        let sanitized = if height.is_finite() && height > 0.0 {
            height
        } else {
            0.0
        };
        Self {
            height: sanitized,
            animation_duration_ms,
            visible,
            interactive_dismiss,
        }
    }

    /// Computes the effective bottom inset combining `safe_area_bottom` and `self`; see
    /// [`effective_bottom_inset`].
    #[inline]
    pub fn effective_bottom_inset(self, safe_area_bottom: f32, os_adjusts_resize: bool) -> f32 {
        effective_bottom_inset(safe_area_bottom, self, os_adjusts_resize)
    }

    /// Encodes `self` into a fixed 8-byte little-endian binary payload (binary).
    #[inline]
    pub fn encode_bytes(self) -> [u8; 8] {
        let mut out = [0u8; 8];
        let h = if self.height.is_finite() && self.height > 0.0 {
            self.height
        } else {
            0.0
        };
        out[0..4].copy_from_slice(&h.to_le_bytes());
        out[4..6].copy_from_slice(&self.animation_duration_ms.to_le_bytes());
        out[6] = u8::from(self.visible);
        out[7] = u8::from(self.interactive_dismiss);
        out
    }

    /// Decodes a fixed 8-byte little-endian binary payload produced by [`Self::encode_bytes`].
    #[inline]
    pub fn decode_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 8 {
            return None;
        }
        let height = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let animation_duration_ms = u16::from_le_bytes([bytes[4], bytes[5]]);
        let visible = bytes[6] != 0;
        let interactive_dismiss = bytes[7] != 0;
        Some(Self::new(
            height,
            visible,
            animation_duration_ms,
            interactive_dismiss,
        ))
    }
}

/// Computes the effective bottom inset (in logical points/dp) for sticky bottom UI (such as chat
/// composers and bottom action bars), explicitly solving both classic mobile keyboard bugs:
///
/// 1. **Zero double-padding** when `os_adjusts_resize == true` (e.g. Android `adjustResize`):
///    when the keyboard is visible, the OS has already resized the window above the keyboard and
///    covered the bottom navigation bar, so the effective bottom inset is `0.0`; when hidden, the
///    normal `safe_area_bottom` applies.
/// 2. **No home-indicator + keyboard double sum** when `os_adjusts_resize == false` (iOS, Android
///    edge-to-edge `adjustNothing`, Web `visualViewport` overlay): uses
///    `safe_area_bottom.max(keyboard.height)` when the keyboard is visible so interactive drag
///    dismiss smoothly settles onto the home indicator without dipping underneath or double-adding.
#[inline]
pub fn effective_bottom_inset(
    safe_area_bottom: f32,
    keyboard: KeyboardInsets,
    os_adjusts_resize: bool,
) -> f32 {
    let safe = if safe_area_bottom.is_finite() {
        safe_area_bottom.max(0.0)
    } else {
        0.0
    };
    if !keyboard.visible {
        return safe;
    }
    if os_adjusts_resize {
        return 0.0;
    }
    let kb_height = if keyboard.height.is_finite() {
        keyboard.height.max(0.0)
    } else {
        0.0
    };
    safe.max(kb_height)
}

/// Derives [`KeyboardInsets`] from browser `window.innerHeight` and `window.visualViewport`
/// (`height`, `offsetTop`, `scale`) geometry.
///
/// Ignores pinch-zoomed viewports (`visual_scale > 1.05`) and minor chrome/rounding deltas below
/// [`VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX`].
#[inline]
pub fn keyboard_insets_from_visual_viewport(
    layout_height: f32,
    visual_height: f32,
    visual_offset_top: f32,
    visual_scale: f32,
) -> KeyboardInsets {
    if !layout_height.is_finite()
        || !visual_height.is_finite()
        || layout_height <= 0.0
        || visual_height <= 0.0
    {
        return KeyboardInsets::HIDDEN;
    }
    if visual_scale.is_finite() && visual_scale > 1.05 {
        return KeyboardInsets::HIDDEN;
    }
    let offset = if visual_offset_top.is_finite() {
        visual_offset_top.max(0.0)
    } else {
        0.0
    };
    let occluded = (layout_height - (visual_height + offset)).max(0.0);
    if occluded >= VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX {
        KeyboardInsets::new(occluded, true, 0, false)
    } else {
        KeyboardInsets::HIDDEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_bottom_inset_prevents_double_padding_and_home_indicator_sum() {
        // Keyboard closed -> respects bottom safe area regardless of resize mode.
        assert_eq!(effective_bottom_inset(34.0, KeyboardInsets::HIDDEN, false), 34.0);
        assert_eq!(effective_bottom_inset(24.0, KeyboardInsets::HIDDEN, true), 24.0);

        // Stale non-zero height while visible == false is ignored.
        let stale_closed = KeyboardInsets::new(310.0, false, 250, true);
        assert_eq!(effective_bottom_inset(34.0, stale_closed, false), 34.0);
        assert_eq!(effective_bottom_inset(24.0, stale_closed, true), 24.0);

        // iOS / edge-to-edge overlay: max(safe_area_bottom, keyboard.height), NEVER sum!
        let ios_open = KeyboardInsets::new(336.0, true, 250, true);
        assert_eq!(effective_bottom_inset(34.0, ios_open, false), 336.0);

        // iOS interactive drag-dismiss below home-indicator height smoothly floors at safe area.
        let ios_dragging = KeyboardInsets::new(18.0, true, 0, true);
        assert_eq!(effective_bottom_inset(34.0, ios_dragging, false), 34.0);

        // Android adjustResize: zero double-padding when keyboard is open.
        let android_open = KeyboardInsets::new(300.0, true, 200, false);
        assert_eq!(effective_bottom_inset(24.0, android_open, true), 0.0);

        // Non-finite & negative inputs sanitize cleanly to 0.0.
        assert_eq!(
            effective_bottom_inset(f32::NAN, KeyboardInsets::new(-10.0, true, 0, false), false),
            0.0
        );
        assert_eq!(
            effective_bottom_inset(-5.0, KeyboardInsets::new(f32::INFINITY, true, 0, false), false),
            0.0
        );
    }

    #[test]
    fn visual_viewport_derivation_handles_keyboard_chrome_and_pinch_zoom() {
        // Mobile Safari keyboard opens: layout=844, visual=508 -> occluded=336px.
        let kb = keyboard_insets_from_visual_viewport(844.0, 508.0, 0.0, 1.0);
        assert!(kb.visible);
        assert_eq!(kb.height, 336.0);

        // Account for visualViewport.offsetTop when scrolled.
        let kb_scrolled = keyboard_insets_from_visual_viewport(844.0, 508.0, 20.0, 1.0);
        assert!(kb_scrolled.visible);
        assert_eq!(kb_scrolled.height, 316.0);

        // Small URL-bar delta (< 80px) does not falsely report a keyboard.
        assert_eq!(
            keyboard_insets_from_visual_viewport(844.0, 790.0, 0.0, 1.0),
            KeyboardInsets::HIDDEN
        );

        // Pinch-zoom shrinks visualViewport.height without opening a keyboard.
        assert_eq!(
            keyboard_insets_from_visual_viewport(844.0, 422.0, 0.0, 2.0),
            KeyboardInsets::HIDDEN
        );

        // Binary 8-byte round-trip.
        let sample = KeyboardInsets::new(294.5, true, 250, true);
        let raw = sample.encode_bytes();
        assert_eq!(KeyboardInsets::decode_bytes(&raw), Some(sample));
        assert_eq!(KeyboardInsets::decode_bytes(&[1, 2, 3]), None);
    }
}
