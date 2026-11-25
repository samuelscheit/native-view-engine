//! Multi-Viewport Meta Workbench state, preset modes, and connected host surface catalog
//! for `ActiveView::Workbench`.

use super::WorkspaceState;

/// Synchronized live preview viewport preset for the Meta Workbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkbenchPreset {
    #[default]
    All,
    Desktop,
    Tablet,
    Mobile,
}

impl WorkbenchPreset {
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All (3)",
            Self::Desktop => "Desktop",
            Self::Tablet => "Tablet",
            Self::Mobile => "Mobile",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Desktop => "desktop",
            Self::Tablet => "tablet",
            Self::Mobile => "mobile",
        }
    }

    pub const fn includes_desktop(self) -> bool {
        matches!(self, Self::All | Self::Desktop)
    }

    pub const fn includes_tablet(self) -> bool {
        matches!(self, Self::All | Self::Tablet)
    }

    pub const fn includes_mobile(self) -> bool {
        matches!(self, Self::All | Self::Mobile)
    }
}

/// Persistent Meta Workbench configuration serialized in `NWS1` v2 snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchState {
    pub preset: WorkbenchPreset,
    pub show_device_chrome: bool,
    pub selected_host_slug: String,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            preset: WorkbenchPreset::All,
            show_device_chrome: true,
            selected_host_slug: "macos-appkit".into(),
        }
    }
}

/// Static descriptor for one connected NVE host shell surface rendered in the Meta Workbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSurfaceSpec {
    pub slug: &'static str,
    pub name: &'static str,
    pub pipeline: &'static str,
    pub badge_bg: &'static str,
    pub badge_fg: &'static str,
    pub rtt_offset_tenths: u32,
}

/// All 9 native & web host surfaces driven by the single `nve-abi` CommandBuffer.
pub const HOST_SURFACES: &[HostSurfaceSpec] = &[
    HostSurfaceSpec {
        slug: "macos-appkit",
        name: "macOS AppKit",
        pipeline: "C ABI | NSView",
        badge_bg: "#0c4a6e",
        badge_fg: "#7dd3fc",
        rtt_offset_tenths: 0,
    },
    HostSurfaceSpec {
        slug: "skia-metal",
        name: "Skia Metal",
        pipeline: "GPU Metal | winit",
        badge_bg: "#2e1065",
        badge_fg: "#c4b5fd",
        rtt_offset_tenths: 1,
    },
    HostSurfaceSpec {
        slug: "web-wasm",
        name: "Web WASM",
        pipeline: "DOM | Canvas 2D",
        badge_bg: "#052e16",
        badge_fg: "#86efac",
        rtt_offset_tenths: 2,
    },
    HostSurfaceSpec {
        slug: "ios-uikit",
        name: "iOS UIKit",
        pipeline: "Fabric | UIView",
        badge_bg: "#172554",
        badge_fg: "#93c5fd",
        rtt_offset_tenths: 3,
    },
    HostSurfaceSpec {
        slug: "android-jni",
        name: "Android JNI",
        pipeline: "JNI | Compose",
        badge_bg: "#064e3b",
        badge_fg: "#6ee7b7",
        rtt_offset_tenths: 4,
    },
    HostSurfaceSpec {
        slug: "qt6",
        name: "Qt 6",
        pipeline: "Qt Quick | RHI",
        badge_bg: "#1e3a8a",
        badge_fg: "#bfdbfe",
        rtt_offset_tenths: 2,
    },
    HostSurfaceSpec {
        slug: "gtk4",
        name: "GTK4",
        pipeline: "GTK4 | GSK",
        badge_bg: "#3b0764",
        badge_fg: "#e9d5ff",
        rtt_offset_tenths: 2,
    },
    HostSurfaceSpec {
        slug: "flutter",
        name: "Flutter",
        pipeline: "dart:ffi | Skia",
        badge_bg: "#0c4a6e",
        badge_fg: "#bae6fd",
        rtt_offset_tenths: 3,
    },
    HostSurfaceSpec {
        slug: "swiftui",
        name: "SwiftUI",
        pipeline: "Swift ABI | Canvas",
        badge_bg: "#451a03",
        badge_fg: "#fcd34d",
        rtt_offset_tenths: 1,
    },
];

impl WorkspaceState {
    pub fn set_workbench_preset(&mut self, preset: WorkbenchPreset) {
        if self.current.workbench.preset == preset {
            return;
        }
        let label = format!("Workbench -> {}", preset.label());
        self.commit_action(label, |s| s.workbench.preset = preset);
    }

    pub fn toggle_workbench_chrome(&mut self) {
        let next = !self.current.workbench.show_device_chrome;
        let label = if next {
            "Workbench Chrome -> ON"
        } else {
            "Workbench Chrome -> OFF"
        };
        self.commit_action(label, |s| s.workbench.show_device_chrome = next);
    }

    pub fn select_workbench_host(&mut self, slug: impl Into<String>) {
        let slug_owned = slug.into();
        let Some(spec) = HOST_SURFACES.iter().find(|h| h.slug == slug_owned) else {
            return;
        };
        if self.current.workbench.selected_host_slug == slug_owned {
            return;
        }
        let label = format!("Host -> {}", spec.name);
        self.commit_action(label, |s| s.workbench.selected_host_slug = slug_owned);
    }
}
