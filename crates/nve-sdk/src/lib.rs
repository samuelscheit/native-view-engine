//! Zero-overhead, zero-JSON Tier-1 native platform capabilities (`nve-sdk`) for
//! `native-view-engine`, backed by the modular C ABI `NVE_IFACE_HOST = 5` and `NveChannelBus`.
//!
//! Provides ergonomic typed Rust clients for:
//! - Clipboard (`clipboard`)
//! - File Picker & Standard Paths (`filesystem`)
//! - Secure Credential Store (`secure_store`)
//! - Deep Linking & External URL Launch (`linking`)
//! - Live System Theme (`NVT2` 16-byte) & Locale (`NVL1`) wired onto `NveChannelBus` (`system_env`)
//! - Local User Notifications (`notifications`)
//! - Haptic Feedback (`haptics`)

pub mod clipboard;
pub mod codec;
pub mod error;
pub mod ffi_client;
pub mod filesystem;
pub mod haptics;
pub mod linking;
pub mod memory;
pub mod native;
pub mod notifications;
pub mod provider;
pub mod secure_store;
pub mod system_env;

pub use clipboard::ClipboardClient;
pub use codec::{BinReader, BinWriter, MAX_SEQ_ITEMS};
pub use error::SdkError;
pub use ffi_client::{FfiHostServices, RequestTable, FFI_MAX_RESPONSE_BYTES, FFI_STACK_BUF_SIZE};
pub use filesystem::{
    FileFilter, FilePickerOptions, FilesystemClient, SaveFileOptions, StandardPathKind,
};
pub use haptics::{HapticsClient, ImpactStyle, NotificationHaptic};
pub use linking::{LinkingClient, LinkingEvent};
pub use memory::{MemoryHostServices, RecordedHaptic};
pub use native::{parse_locale_tag, HostCallbackBridge, NativeHostServices};
pub use notifications::{
    NotificationPermission, NotificationRequest, NotificationTapped, NotificationsClient,
};
pub use provider::{HostChannelEvent, HostEvent, HostPushEvent, HostServiceProvider, InvokeResult};
pub use secure_store::SecureStoreClient;
pub use system_env::{
    ColorScheme, ContrastLevel, LocaleSnapshot, SystemEnvBridge, SystemEnvClient, TextDirection,
    ThemeSnapshot, LOCALE_WIRE_MAGIC, LOCALE_WIRE_VERSION, THEME_WIRE_LEN, THEME_WIRE_MAGIC,
};

/// Ergonomic facade bundling all 7 Tier-1 host capability clients over one [`HostServiceProvider`].
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
}
