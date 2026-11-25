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

pub mod audio;
pub mod camera;
pub mod clipboard;
pub mod codec;
pub mod error;
pub mod ffi_client;
pub mod filesystem;
pub mod fonts;
pub mod haptics;
pub mod linking;
pub mod location;
pub mod memory;
mod memory_tier2;
pub mod native;
pub mod notifications;
pub mod provider;
pub mod secure_store;
pub mod sensors;
pub mod share_web;
pub mod storage;
pub mod system_env;

pub use audio::{
    AudioClient, AudioClipMeta, AudioEncoding, AudioEvent, AudioHandle, AudioLevelEvent,
    AudioPlaybackEvent, AudioPlaybackState, AudioRecordConfig, AudioSource, SERVICE_AUDIO,
};
pub use camera::{
    CameraCaptureOptions, CameraClient, CameraDevice, CameraFacing, CameraPreviewConfig,
    CameraSessionId, CapturedMedia, FlashMode, MediaFormat, SERVICE_CAMERA,
};
pub use clipboard::ClipboardClient;
pub use codec::{fnv1a64, BinReader, BinWriter, MAX_SEQ_ITEMS};
pub use error::SdkError;
pub use ffi_client::{FfiHostServices, RequestTable, FFI_MAX_RESPONSE_BYTES, FFI_STACK_BUF_SIZE};
pub use filesystem::{
    FileFilter, FilePickerOptions, FilesystemClient, SaveFileOptions, StandardPathKind,
};
pub use fonts::{FontClient, FontFormat, FontHandle, FontRegistry, RegisteredFont, SERVICE_FONTS};
pub use haptics::{HapticsClient, ImpactStyle, NotificationHaptic};
pub use linking::{LinkingClient, LinkingEvent};
pub use memory::{MemoryHostServices, RecordedHaptic};
pub use memory_tier2::{MockAudioTrack, RecordedBrowserLaunch, RecordedShare};
pub use native::storage::{KvStore, KvStoreManager, NVK1_COMMIT_MARKER, NVK1_MAGIC, NVK1_VERSION};
pub use native::{parse_locale_tag, HostCallbackBridge, NativeHostServices};
pub use notifications::{
    NotificationPermission, NotificationRequest, NotificationTapped, NotificationsClient,
};
pub use provider::{
    HostChannelEvent, HostEvent, HostPushEvent, HostServiceProvider, InvokeResult,
    PermissionStatus,
};
pub use secure_store::SecureStoreClient;
pub use sensors::{
    GeoFix, LocationAccuracy, LocationClient, LocationUpdateEvent, LocationWatchConfig,
    LocationWatchId, SensorBatchEvent, SensorBatchEventRef, SensorClient, SensorKind, SensorSample,
    SensorSampleSlice, SensorSubscriptionId, SENSOR_SAMPLE_WIRE_LEN, SERVICE_LOCATION,
    SERVICE_SENSORS,
};
pub use share_web::{
    BrowserPresentation, InAppBrowserOptions, ShareClient, ShareResult, WebAuthClient,
    WebAuthResult, SERVICE_SHARE, SERVICE_WEB_AUTH,
};
pub use storage::{
    CompactStats, StorageClient, StorageEntry, StorageOpenInfo, StorageWriteOp, SERVICE_STORAGE,
};
pub use system_env::{
    ColorScheme, ContrastLevel, LocaleSnapshot, SystemEnvBridge, SystemEnvClient, TextDirection,
    ThemeSnapshot, LOCALE_WIRE_MAGIC, LOCALE_WIRE_VERSION, THEME_WIRE_LEN, THEME_WIRE_MAGIC,
};

/// Ergonomic facade bundling all Tier-1/2/3 host capability clients over one [`HostServiceProvider`].
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

    pub const fn clipboard(&self) -> ClipboardClient<'a, P> {
        ClipboardClient::new(self.provider)
    }

    pub const fn filesystem(&self) -> FilesystemClient<'a, P> {
        FilesystemClient::new(self.provider)
    }

    pub const fn secure_store(&self) -> SecureStoreClient<'a, P> {
        SecureStoreClient::new(self.provider)
    }

    pub const fn linking(&self) -> LinkingClient<'a, P> {
        LinkingClient::new(self.provider)
    }

    pub const fn system_env(&self) -> SystemEnvClient<'a, P> {
        SystemEnvClient::new(self.provider)
    }

    pub const fn notifications(&self) -> NotificationsClient<'a, P> {
        NotificationsClient::new(self.provider)
    }

    pub const fn haptics(&self) -> HapticsClient<'a, P> {
        HapticsClient::new(self.provider)
    }

    pub const fn storage(&self) -> StorageClient<'a, P> {
        StorageClient::new(self.provider)
    }

    pub const fn fonts(&self) -> FontClient<'a, P> {
        FontClient::new(self.provider)
    }

    pub const fn audio(&self) -> AudioClient<'a, P> {
        AudioClient::new(self.provider)
    }

    pub const fn camera(&self) -> CameraClient<'a, P> {
        CameraClient::new(self.provider)
    }

    pub const fn location(&self) -> LocationClient<'a, P> {
        LocationClient::new(self.provider)
    }

    pub const fn sensors(&self) -> SensorClient<'a, P> {
        SensorClient::new(self.provider)
    }

    pub const fn share(&self) -> ShareClient<'a, P> {
        ShareClient::new(self.provider)
    }

    pub const fn web_auth(&self) -> WebAuthClient<'a, P> {
        WebAuthClient::new(self.provider)
    }
}
