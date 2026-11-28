//! Multi-Viewport Meta Workbench state, preset modes, and connected host surface catalog
//! for `ActiveView::Workbench`.

use super::WorkspaceState;

/// Synchronized live preview viewport preset for the Meta Workbench.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, nve_abi::ReprEnum)]
pub enum WorkbenchPreset {
    #[default]
    All = 0,
    Desktop = 1,
    Tablet = 2,
    Mobile = 3,
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
    pub rtt_offset_tenths: u32,
}

/// All 9 native & web host surfaces driven by the single `nve-abi` CommandBuffer.
pub const HOST_SURFACES: &[HostSurfaceSpec] = &[
    HostSurfaceSpec {
        slug: "macos-appkit",
        name: "macOS AppKit",
        pipeline: "C ABI | NSView",
        rtt_offset_tenths: 0,
    },
    HostSurfaceSpec {
        slug: "skia-metal",
        name: "Skia Metal",
        pipeline: "GPU Metal | winit",
        rtt_offset_tenths: 1,
    },
    HostSurfaceSpec {
        slug: "web-wasm",
        name: "Web WASM",
        pipeline: "DOM | Canvas 2D",
        rtt_offset_tenths: 2,
    },
    HostSurfaceSpec {
        slug: "ios-uikit",
        name: "iOS UIKit",
        pipeline: "Fabric | UIView",
        rtt_offset_tenths: 3,
    },
    HostSurfaceSpec {
        slug: "android-jni",
        name: "Android JNI",
        pipeline: "JNI | Compose",
        rtt_offset_tenths: 4,
    },
    HostSurfaceSpec {
        slug: "qt6",
        name: "Qt 6",
        pipeline: "Qt Quick | RHI",
        rtt_offset_tenths: 2,
    },
    HostSurfaceSpec {
        slug: "gtk4",
        name: "GTK4",
        pipeline: "GTK4 | GSK",
        rtt_offset_tenths: 2,
    },
    HostSurfaceSpec {
        slug: "flutter",
        name: "Flutter",
        pipeline: "dart:ffi | Skia",
        rtt_offset_tenths: 3,
    },
    HostSurfaceSpec {
        slug: "swiftui",
        name: "SwiftUI",
        pipeline: "Swift ABI | Canvas",
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
