//! Zero-overhead, zero-JSON Tier-1/2/3 native platform capabilities (`nve-sdk`) for
//! `native-view-engine`, backed by the modular C ABI `NVE_IFACE_HOST = 5` and `NveChannelBus`.
//!
//! Provides ergonomic typed Rust clients for:
//! - **Tier 1**: Clipboard (`clipboard`), File Picker & Standard Paths (`filesystem`),
//!   Secure Credential Store (`secure_store`), Deep Linking (`linking`), Live System Theme
//!   (`NVT2`) & Locale (`NVL1`) (`system_env`), Local Notifications (`notifications`), Haptics (`haptics`)
//! - **Tier 2/3**: Crash-Safe `NVK1` Binary KV Store (`storage`), Custom Font Loader & Registry
//!   (`fonts`), Audio Playback & Recording (`audio`), Camera & Media Capture (`camera`),
//!   Geolocation & Hardware Sensors (`sensors`), Native Share Sheet & In-App WebAuth (`share_web`)

#[cfg(feature = "audio")]
pub mod audio;
#[cfg(feature = "camera")]
pub mod camera;
#[cfg(feature = "clipboard")]
pub mod clipboard;
pub mod codec;
pub mod error;
pub mod ffi_client;
#[cfg(feature = "filesystem")]
pub mod filesystem;
#[cfg(feature = "fonts")]
pub mod fonts;
#[cfg(feature = "haptics")]
pub mod haptics;
#[cfg(feature = "linking")]
pub mod linking;
#[cfg(feature = "location")]
pub mod location;
pub mod memory;
#[cfg(any(
    feature = "storage",
    feature = "fonts",
    feature = "audio",
    feature = "camera",
    feature = "location",
    feature = "sensors",
    feature = "share",
    feature = "web-auth",
))]
mod memory_tier2;
pub mod native;
#[cfg(feature = "notifications")]
pub mod notifications;
pub mod provider;
#[cfg(feature = "secure-store")]
pub mod secure_store;
#[cfg(feature = "sensors")]
pub mod sensors;
#[cfg(any(feature = "share", feature = "web-auth"))]
pub mod share_web;
#[cfg(feature = "storage")]
pub mod storage;
#[cfg(feature = "system-env")]
pub mod system_env;

#[cfg(feature = "audio")]
pub use audio::{
    AudioClient, AudioClipMeta, AudioEncoding, AudioEvent, AudioHandle, AudioLevelEvent,
    AudioPlaybackEvent, AudioPlaybackState, AudioRecordConfig, AudioSource, SERVICE_AUDIO,
};
#[cfg(feature = "camera")]
pub use camera::{
    CameraCaptureOptions, CameraClient, CameraDevice, CameraFacing, CameraPreviewConfig,
    CameraSessionId, CapturedMedia, FlashMode, MediaFormat, SERVICE_CAMERA,
};
#[cfg(feature = "clipboard")]
pub use clipboard::ClipboardClient;
pub use codec::{fnv1a64, BinReader, BinWriter, MAX_SEQ_ITEMS};
pub use error::SdkError;
pub use ffi_client::{FfiHostServices, RequestTable, FFI_MAX_RESPONSE_BYTES, FFI_STACK_BUF_SIZE};
#[cfg(feature = "filesystem")]
pub use filesystem::{
    FileFilter, FilePickerOptions, FilesystemClient, SaveFileOptions, StandardPathKind,
};
#[cfg(feature = "fonts")]
pub use fonts::{FontClient, FontFormat, FontHandle, FontRegistry, RegisteredFont, SERVICE_FONTS};
#[cfg(feature = "haptics")]
pub use haptics::{HapticsClient, ImpactStyle, NotificationHaptic};
#[cfg(feature = "linking")]
pub use linking::{LinkingClient, LinkingEvent};
#[cfg(feature = "location")]
pub use location::{
    GeoFix, LocationAccuracy, LocationClient, LocationUpdateEvent, LocationWatchConfig,
    LocationWatchId, SERVICE_LOCATION,
};
#[cfg(feature = "haptics")]
pub use memory::RecordedHaptic;
pub use memory::MemoryHostServices;
#[cfg(feature = "audio")]
pub use memory_tier2::MockAudioTrack;
#[cfg(feature = "web-auth")]
pub use memory_tier2::RecordedBrowserLaunch;
#[cfg(feature = "share")]
pub use memory_tier2::RecordedShare;
#[cfg(feature = "storage")]
pub use native::storage::{KvStore, KvStoreManager, NVK1_COMMIT_MARKER, NVK1_MAGIC, NVK1_VERSION};
#[cfg(feature = "system-env")]
pub use native::parse_locale_tag;
pub use native::{HostCallbackBridge, NativeHostServices};
#[cfg(feature = "notifications")]
pub use notifications::{
    NotificationPermission, NotificationRequest, NotificationTapped, NotificationsClient,
};
pub use provider::{
    HostChannelEvent, HostEvent, HostPushEvent, HostServiceProvider, InvokeResult,
    PermissionStatus,
};
#[cfg(feature = "secure-store")]
pub use secure_store::SecureStoreClient;
#[cfg(feature = "sensors")]
pub use sensors::{
    SensorBatchEvent, SensorBatchEventRef, SensorClient, SensorKind, SensorSample,
    SensorSampleSlice, SensorSubscriptionId, SENSOR_SAMPLE_WIRE_LEN, SERVICE_SENSORS,
};
#[cfg(feature = "share")]
pub use share_web::{ShareClient, ShareResult, SERVICE_SHARE};
#[cfg(feature = "web-auth")]
pub use share_web::{
    BrowserPresentation, InAppBrowserOptions, WebAuthClient, WebAuthResult, SERVICE_WEB_AUTH,
};
#[cfg(feature = "storage")]
pub use storage::{
    CompactStats, StorageClient, StorageEntry, StorageOpenInfo, StorageWriteOp, SERVICE_STORAGE,
};
#[cfg(feature = "system-env")]
pub use system_env::{
    ColorScheme, ContrastLevel, LocaleSnapshot, SystemEnvBridge, SystemEnvClient, TextDirection,
    ThemeSnapshot, LOCALE_WIRE_MAGIC, LOCALE_WIRE_VERSION, THEME_WIRE_LEN, THEME_WIRE_MAGIC,
};

/// Ergonomic facade bundling enabled Tier-1/2/3 host capability clients over one [`HostServiceProvider`].
#[derive(Debug, Clone, Copy)]
pub struct SdkClient<'a, P: HostServiceProvider + ?Sized> {
    provider: &'a P,
}

impl<'a, P: HostServiceProvider + ?Sized> SdkClient<'a, P> {
    pub const fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub const fn provider(&self) -> &'a P {
        self.provider
    }

    #[cfg(feature = "clipboard")]
    pub const fn clipboard(&self) -> ClipboardClient<'a, P> {
        ClipboardClient::new(self.provider)
    }

    #[cfg(feature = "filesystem")]
    pub const fn filesystem(&self) -> FilesystemClient<'a, P> {
        FilesystemClient::new(self.provider)
    }

    #[cfg(feature = "secure-store")]
    pub const fn secure_store(&self) -> SecureStoreClient<'a, P> {
        SecureStoreClient::new(self.provider)
    }

    #[cfg(feature = "linking")]
    pub const fn linking(&self) -> LinkingClient<'a, P> {
        LinkingClient::new(self.provider)
    }

    #[cfg(feature = "system-env")]
    pub const fn system_env(&self) -> SystemEnvClient<'a, P> {
        SystemEnvClient::new(self.provider)
    }

    #[cfg(feature = "notifications")]
    pub const fn notifications(&self) -> NotificationsClient<'a, P> {
        NotificationsClient::new(self.provider)
    }

    #[cfg(feature = "haptics")]
    pub const fn haptics(&self) -> HapticsClient<'a, P> {
        HapticsClient::new(self.provider)
    }

    #[cfg(feature = "storage")]
    pub const fn storage(&self) -> StorageClient<'a, P> {
        StorageClient::new(self.provider)
    }

    #[cfg(feature = "fonts")]
    pub const fn fonts(&self) -> FontClient<'a, P> {
        FontClient::new(self.provider)
    }

    #[cfg(feature = "audio")]
    pub const fn audio(&self) -> AudioClient<'a, P> {
        AudioClient::new(self.provider)
    }

    #[cfg(feature = "camera")]
    pub const fn camera(&self) -> CameraClient<'a, P> {
        CameraClient::new(self.provider)
    }

    #[cfg(feature = "location")]
    pub const fn location(&self) -> LocationClient<'a, P> {
        LocationClient::new(self.provider)
    }

    #[cfg(feature = "sensors")]
    pub const fn sensors(&self) -> SensorClient<'a, P> {
        SensorClient::new(self.provider)
    }

    #[cfg(feature = "share")]
    pub const fn share(&self) -> ShareClient<'a, P> {
        ShareClient::new(self.provider)
    }

    #[cfg(feature = "web-auth")]
    pub const fn web_auth(&self) -> WebAuthClient<'a, P> {
        WebAuthClient::new(self.provider)
    }
}
