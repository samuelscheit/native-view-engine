//! Complete deterministic in-memory mock [`MemoryHostServices`] implementing
//! [`HostServiceProvider`] for sub-second unit, fault-injection, async `NVH1`, and
//! `NveChannelBus` retained-replay tests.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(feature = "clipboard")]
use nve_abi::SERVICE_CLIPBOARD;
#[cfg(feature = "filesystem")]
use nve_abi::SERVICE_FILESYSTEM;
#[cfg(feature = "haptics")]
use nve_abi::SERVICE_HAPTICS;
#[cfg(feature = "linking")]
use nve_abi::SERVICE_LINKING;
#[cfg(feature = "notifications")]
use nve_abi::SERVICE_NOTIFICATIONS;
#[cfg(feature = "secure-store")]
use nve_abi::SERVICE_SECURE_STORE;
#[cfg(feature = "system-env")]
use nve_abi::SERVICE_SYSTEM_ENV;
use nve_abi::{
    NveHostEventBatch, NveHostEventRecordRef, NveHostStatus, NVE_CHANNEL_FLAG_RETAIN,
};
use nve_interop::{ChannelPayload, NveChannelBus, PayloadEncoding};

#[cfg(feature = "clipboard")]
use crate::clipboard;
use crate::error::SdkError;
#[cfg(feature = "filesystem")]
use crate::filesystem::{self, StandardPathKind};
#[cfg(feature = "haptics")]
use crate::haptics::{self, ImpactStyle, NotificationHaptic};
#[cfg(feature = "linking")]
use crate::linking;
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
use crate::memory_tier2::MemoryTier2State;
#[cfg(feature = "notifications")]
use crate::notifications::{self, NotificationPermission, NotificationRequest};
use crate::provider::{HostChannelEvent, HostEvent, HostPushEvent, HostServiceProvider, InvokeResult};
#[cfg(feature = "secure-store")]
use crate::secure_store;
#[cfg(feature = "system-env")]
use crate::system_env::{self, LocaleSnapshot, ThemeSnapshot};

/// Recorded haptic trigger for test assertions.
#[cfg(feature = "haptics")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordedHaptic {
    Impact(ImpactStyle),
    Notification(NotificationHaptic),
    Selection,
}

#[derive(Debug, Clone)]
enum InjectedFault {
    Status(NveHostStatus),
    Pending,
}

pub(crate) struct MemoryState {
    #[cfg(feature = "clipboard")]
    clipboard_text: Option<String>,
    #[cfg(feature = "filesystem")]
    standard_paths: BTreeMap<u8, String>,
    #[cfg(feature = "filesystem")]
    pick_files_result: Result<Vec<String>, SdkError>,
    #[cfg(feature = "filesystem")]
    save_file_result: Result<Option<String>, SdkError>,
    #[cfg(feature = "secure-store")]
    secrets: BTreeMap<(String, String), Vec<u8>>,
    #[cfg(feature = "linking")]
    supported_url_schemes: Vec<String>,
    #[cfg(feature = "linking")]
    initial_url: Option<String>,
    #[cfg(feature = "linking")]
    opened_urls: Vec<String>,
    #[cfg(feature = "system-env")]
    theme: ThemeSnapshot,
    #[cfg(feature = "system-env")]
    locale: LocaleSnapshot,
    #[cfg(feature = "notifications")]
    notification_permission: NotificationPermission,
    #[cfg(feature = "notifications")]
    active_notifications: BTreeMap<String, NotificationRequest>,
    #[cfg(feature = "notifications")]
    cancelled_notifications: Vec<String>,
    #[cfg(feature = "haptics")]
    haptics_log: Vec<RecordedHaptic>,
    faults: BTreeMap<(u32, u32), VecDeque<InjectedFault>>,
    pending_async: BTreeMap<u64, Option<Result<Vec<u8>, SdkError>>>,
    events: VecDeque<HostEvent>,
    channel_bus: Arc<Mutex<NveChannelBus>>,
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
    pub(crate) tier2: MemoryTier2State,
}

impl MemoryState {
    fn new() -> Self {
        #[cfg(feature = "filesystem")]
        let standard_paths = {
            let mut m = BTreeMap::new();
            m.insert(StandardPathKind::Data as u8, "/mem/data".into());
            m.insert(StandardPathKind::Cache as u8, "/mem/cache".into());
            m.insert(StandardPathKind::Config as u8, "/mem/config".into());
            m.insert(StandardPathKind::Temp as u8, "/mem/tmp".into());
            m.insert(StandardPathKind::Documents as u8, "/mem/documents".into());
            m.insert(StandardPathKind::Downloads as u8, "/mem/downloads".into());
            m
        };
        Self {
            #[cfg(feature = "clipboard")]
            clipboard_text: None,
            #[cfg(feature = "filesystem")]
            standard_paths,
            #[cfg(feature = "filesystem")]
            pick_files_result: Ok(vec!["/mem/documents/sample.txt".into()]),
            #[cfg(feature = "filesystem")]
            save_file_result: Ok(Some("/mem/documents/output.bin".into())),
            #[cfg(feature = "secure-store")]
            secrets: BTreeMap::new(),
            #[cfg(feature = "linking")]
            supported_url_schemes: vec!["https".into(), "http".into(), "mailto".into(), "nve".into()],
            #[cfg(feature = "linking")]
            initial_url: None,
            #[cfg(feature = "linking")]
            opened_urls: Vec::new(),
            #[cfg(feature = "system-env")]
            theme: ThemeSnapshot::default(),
            #[cfg(feature = "system-env")]
            locale: LocaleSnapshot::default(),
            #[cfg(feature = "notifications")]
            notification_permission: NotificationPermission::Authorized,
            #[cfg(feature = "notifications")]
            active_notifications: BTreeMap::new(),
            #[cfg(feature = "notifications")]
            cancelled_notifications: Vec::new(),
            #[cfg(feature = "haptics")]
            haptics_log: Vec::new(),
            faults: BTreeMap::new(),
            pending_async: BTreeMap::new(),
            events: VecDeque::new(),
            channel_bus: Arc::new(Mutex::new(NveChannelBus::new())),
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
            tier2: MemoryTier2State::new(),
        }
    }
}

/// Deterministic in-memory mock implementing enabled Tier-1/2/3 SDK capabilities + fault injection.
pub struct MemoryHostServices {
    next_request_id: AtomicU64,
    pub(crate) state: Mutex<MemoryState>,
}

impl Default for MemoryHostServices {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryHostServices {
    pub fn new() -> Self {
        Self {
            next_request_id: AtomicU64::new(1),
            state: Mutex::new(MemoryState::new()),
        }
    }

    pub fn channel_bus(&self) -> Arc<Mutex<NveChannelBus>> {
        self.state.lock().unwrap().channel_bus.clone()
    }

    #[cfg(feature = "filesystem")]
    pub fn set_standard_path(&self, kind: StandardPathKind, path: impl Into<String>) {
        self.state
            .lock()
            .unwrap()
            .standard_paths
            .insert(kind as u8, path.into());
    }

    #[cfg(feature = "filesystem")]
    pub fn set_pick_files_result(&self, res: Result<Vec<String>, SdkError>) {
        self.state.lock().unwrap().pick_files_result = res;
    }

    #[cfg(feature = "filesystem")]
    pub fn set_save_file_result(&self, res: Result<Option<String>, SdkError>) {
        self.state.lock().unwrap().save_file_result = res;
    }

    #[cfg(feature = "linking")]
    pub fn set_initial_url(&self, url: Option<String>) {
        self.state.lock().unwrap().initial_url = url;
    }

    #[cfg(feature = "linking")]
    pub fn set_supported_url_schemes(&self, schemes: Vec<String>) {
        self.state.lock().unwrap().supported_url_schemes = schemes;
    }

    #[cfg(feature = "linking")]
    pub fn opened_urls(&self) -> Vec<String> {
        self.state.lock().unwrap().opened_urls.clone()
    }

    #[cfg(feature = "system-env")]
    pub fn set_theme(&self, theme: ThemeSnapshot) {
        self.state.lock().unwrap().theme = theme;
    }

    #[cfg(feature = "system-env")]
    pub fn set_locale(&self, locale: LocaleSnapshot) {
        self.state.lock().unwrap().locale = locale;
    }

    #[cfg(feature = "notifications")]
    pub fn set_notification_permission(&self, perm: NotificationPermission) {
        self.state.lock().unwrap().notification_permission = perm;
    }

    #[cfg(feature = "notifications")]
    pub fn active_notifications(&self) -> Vec<NotificationRequest> {
        self.state
            .lock()
            .unwrap()
            .active_notifications
            .values()
            .cloned()
            .collect()
    }

    #[cfg(feature = "notifications")]
    pub fn cancelled_notifications(&self) -> Vec<String> {
        self.state.lock().unwrap().cancelled_notifications.clone()
    }

    #[cfg(feature = "haptics")]
    pub fn haptics_log(&self) -> Vec<RecordedHaptic> {
        self.state.lock().unwrap().haptics_log.clone()
    }

    /// Injects a one-shot [`NveHostStatus`] failure (e.g., `PermissionDenied`, `Cancelled`,
    /// `BufferTooSmall`, `Unsupported`) for the next invocation of `(service_id, method_id)`.
    pub fn inject_status(&self, service_id: u32, method_id: u32, status: NveHostStatus) {
        self.state
            .lock()
            .unwrap()
            .faults
            .entry((service_id, method_id))
            .or_default()
            .push_back(InjectedFault::Status(status));
    }

    /// Injects a one-shot async `Pending` response for the next invocation of `(service_id, method_id)`.
    pub fn inject_async_pending(&self, service_id: u32, method_id: u32) {
        self.state
            .lock()
            .unwrap()
            .faults
            .entry((service_id, method_id))
            .or_default()
            .push_back(InjectedFault::Pending);
    }

    /// Enqueues an unsolicited host push event (e.g. `UrlReceived`, `NotificationTapped`).
    pub fn push_host_event(&self, service_id: u32, event_id: u32, payload: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .events
            .push_back(HostEvent::Push(HostPushEvent {
                service_id,
                event_id,
                payload,
            }));
    }

    /// Completes an injected async pending request directly.
    pub fn complete_async(&self, request_id: u64, status: NveHostStatus, payload: Vec<u8>) {
        let outcome = if status.is_ok() {
            Ok(payload)
        } else {
            Err(SdkError::from_status(status))
        };
        self.state
            .lock()
            .unwrap()
            .pending_async
            .insert(request_id, Some(outcome));
    }

    /// Decodes an `NVH1` batch into this mock's async request table, event queue, and `NveChannelBus`.
    pub fn dispatch_host_events(&self, batch_bytes: &[u8]) -> Result<usize, SdkError> {
        let records = NveHostEventBatch::decode_refs(batch_bytes)?;
        let count = records.len();
        let mut bus_messages = Vec::new();
        let bus_arc;
        {
            let mut st = self.state.lock().unwrap();
            bus_arc = st.channel_bus.clone();
            for rec in records {
                match rec {
                    NveHostEventRecordRef::AsyncResponse {
                        request_id,
                        status,
                        payload,
                    } => {
                        let outcome = if status.is_ok() {
                            Ok(payload.to_vec())
                        } else {
                            Err(SdkError::from_status(status))
                        };
                        st.pending_async.insert(request_id, Some(outcome));
                    }
                    NveHostEventRecordRef::HostPushEvent {
                        service_id,
                        event_id,
                        payload,
                    } => {
                        st.events.push_back(HostEvent::Push(HostPushEvent {
                            service_id,
                            event_id,
                            payload: payload.to_vec(),
                        }));
                    }
                    NveHostEventRecordRef::ChannelMessage {
                        channel_id,
                        flags,
                        payload,
                    } => {
                        let retain = (flags & NVE_CHANNEL_FLAG_RETAIN) != 0;
                        let owned = payload.to_vec();
                        bus_messages.push((channel_id, retain, owned.clone()));
                        st.events.push_back(HostEvent::Channel(HostChannelEvent {
                            channel_id,
                            retain,
                            payload: owned,
                        }));
                    }
                }
            }
        }
        if !bus_messages.is_empty() {
            let mut bus = bus_arc.lock().unwrap();
            for (channel_id, retain, payload) in bus_messages {
                let _ = bus.publish_by_id(
                    channel_id,
                    None,
                    PayloadEncoding::Binary,
                    ChannelPayload::from_vec(payload),
                    retain,
                );
            }
        }
        Ok(count)
    }
}

impl HostServiceProvider for MemoryHostServices {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError> {
        let _ = request;
        let mut st = self.state.lock().unwrap();

        if let Some(queue) = st.faults.get_mut(&(service_id, method_id))
            && let Some(fault) = queue.pop_front()
        {
            match fault {
                InjectedFault::Status(status) => {
                    return if status.is_ok() {
                        Ok(InvokeResult::Ready(Vec::new()))
                    } else {
                        Err(SdkError::from_status(status))
                    };
                }
                InjectedFault::Pending => {
                    let req_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
                    st.pending_async.insert(req_id, None);
                    return Ok(InvokeResult::Pending { request_id: req_id });
                }
            }
        }

        let payload: Result<Vec<u8>, SdkError> = match (service_id, method_id) {
                // Clipboard
                #[cfg(feature = "clipboard")]
                (SERVICE_CLIPBOARD, clipboard::METHOD_READ_TEXT) => {
                    clipboard::encode_read_text_response(st.clipboard_text.as_deref())
                }
                #[cfg(feature = "clipboard")]
                (SERVICE_CLIPBOARD, clipboard::METHOD_WRITE_TEXT) => {
                    let text = clipboard::decode_write_text_request(request)?;
                    st.clipboard_text = Some(text);
                    Ok(Vec::new())
                }
                #[cfg(feature = "clipboard")]
                (SERVICE_CLIPBOARD, clipboard::METHOD_HAS_TEXT) => Ok(
                    clipboard::encode_has_text_response(
                        st.clipboard_text.as_ref().is_some_and(|s| !s.is_empty()),
                    ),
                ),
                #[cfg(feature = "clipboard")]
                (SERVICE_CLIPBOARD, clipboard::METHOD_CLEAR) => {
                    st.clipboard_text = None;
                    Ok(Vec::new())
                }

                // Filesystem
                #[cfg(feature = "filesystem")]
                (SERVICE_FILESYSTEM, filesystem::METHOD_STANDARD_PATH) => {
                    let kind = filesystem::decode_standard_path_request(request)?;
                    let path = st
                        .standard_paths
                        .get(&(kind as u8))
                        .ok_or(SdkError::NotFound)?;
                    filesystem::encode_standard_path_response(path)
                }
                #[cfg(feature = "filesystem")]
                (SERVICE_FILESYSTEM, filesystem::METHOD_PICK_FILES) => {
                    let _opts = filesystem::decode_file_picker_options(request)?;
                    let paths = st.pick_files_result.clone()?;
                    filesystem::encode_pick_files_response(&paths)
                }
                #[cfg(feature = "filesystem")]
                (SERVICE_FILESYSTEM, filesystem::METHOD_SAVE_FILE) => {
                    let _opts = filesystem::decode_save_file_options(request)?;
                    let saved = st.save_file_result.clone()?;
                    filesystem::encode_save_file_response(saved.as_deref())
                }

                // Secure Store
                #[cfg(feature = "secure-store")]
                (SERVICE_SECURE_STORE, secure_store::METHOD_GET) => {
                    let (svc, acc) = secure_store::decode_key_request(request)?;
                    let found = st.secrets.get(&(svc, acc)).map(Vec::as_slice);
                    secure_store::encode_get_response(found)
                }
                #[cfg(feature = "secure-store")]
                (SERVICE_SECURE_STORE, secure_store::METHOD_SET) => {
                    let (svc, acc, secret) = secure_store::decode_set_request(request)?;
                    st.secrets.insert((svc, acc), secret);
                    Ok(Vec::new())
                }
                #[cfg(feature = "secure-store")]
                (SERVICE_SECURE_STORE, secure_store::METHOD_DELETE) => {
                    let (svc, acc) = secure_store::decode_key_request(request)?;
                    let removed = st.secrets.remove(&(svc, acc)).is_some();
                    Ok(secure_store::encode_delete_response(removed))
                }

                // Linking
                #[cfg(feature = "linking")]
                (SERVICE_LINKING, linking::METHOD_OPEN_URL) => {
                    let url = linking::decode_url_request(request)?;
                    let scheme = url.split(':').next().unwrap_or_default().to_ascii_lowercase();
                    if !st.supported_url_schemes.iter().any(|s| s == &scheme) {
                        return Err(SdkError::Unsupported);
                    }
                    st.opened_urls.push(url);
                    Ok(Vec::new())
                }
                #[cfg(feature = "linking")]
                (SERVICE_LINKING, linking::METHOD_CAN_OPEN_URL) => {
                    let url = linking::decode_url_request(request)?;
                    let scheme = url.split(':').next().unwrap_or_default().to_ascii_lowercase();
                    let ok = st.supported_url_schemes.iter().any(|s| s == &scheme);
                    Ok(linking::encode_can_open_response(ok))
                }
                #[cfg(feature = "linking")]
                (SERVICE_LINKING, linking::METHOD_INITIAL_URL) => {
                    linking::encode_initial_url_response(st.initial_url.as_deref())
                }

                // System Env
                #[cfg(feature = "system-env")]
                (SERVICE_SYSTEM_ENV, system_env::METHOD_GET_THEME) => {
                    Ok(st.theme.encode_nvt2().to_vec())
                }
                #[cfg(feature = "system-env")]
                (SERVICE_SYSTEM_ENV, system_env::METHOD_GET_LOCALE) => st.locale.encode_nvl1(),

                // Notifications
                #[cfg(feature = "notifications")]
                (SERVICE_NOTIFICATIONS, notifications::METHOD_REQUEST_PERMISSION) => Ok(
                    notifications::encode_permission_response(st.notification_permission),
                ),
                #[cfg(feature = "notifications")]
                (SERVICE_NOTIFICATIONS, notifications::METHOD_SHOW) => {
                    if st.notification_permission == NotificationPermission::Denied {
                        return Err(SdkError::PermissionDenied);
                    }
                    let req = notifications::decode_show_request(request)?;
                    st.active_notifications.insert(req.id.clone(), req);
                    Ok(Vec::new())
                }
                #[cfg(feature = "notifications")]
                (SERVICE_NOTIFICATIONS, notifications::METHOD_CANCEL) => {
                    let id = notifications::decode_cancel_request(request)?;
                    st.active_notifications.remove(&id);
                    st.cancelled_notifications.push(id);
                    Ok(Vec::new())
                }

                // Haptics
                #[cfg(feature = "haptics")]
                (SERVICE_HAPTICS, haptics::METHOD_IMPACT) => {
                    let style = haptics::decode_impact_request(request)?;
                    st.haptics_log.push(RecordedHaptic::Impact(style));
                    Ok(Vec::new())
                }
                #[cfg(feature = "haptics")]
                (SERVICE_HAPTICS, haptics::METHOD_NOTIFICATION) => {
                    let kind = haptics::decode_notification_haptic_request(request)?;
                    st.haptics_log.push(RecordedHaptic::Notification(kind));
                    Ok(Vec::new())
                }
                #[cfg(feature = "haptics")]
                (SERVICE_HAPTICS, haptics::METHOD_SELECTION) => {
                    st.haptics_log.push(RecordedHaptic::Selection);
                    Ok(Vec::new())
                }

                // Tier-2/3 capabilities
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
                (svc, method) if MemoryTier2State::handles_service(svc) => {
                    st.tier2.dispatch(svc, method, request)
                }

                _ => Err(SdkError::Unsupported),
            };

        payload.map(InvokeResult::Ready)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), SdkError> {
        let bus_arc = self.state.lock().unwrap().channel_bus.clone();
        let mut bus = bus_arc.lock().unwrap();
        bus.publish_by_id(
            channel_id,
            None,
            PayloadEncoding::Binary,
            ChannelPayload::from_slice(payload),
            retain,
        )
        .map(|_| ())
        .map_err(|e| SdkError::InternalError(e.to_string()))
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.state.lock().unwrap().events.drain(..).collect()
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, SdkError>> {
        let mut st = self.state.lock().unwrap();
        let ready = matches!(st.pending_async.get(&request_id), Some(Some(_)));
        if ready {
            st.pending_async.remove(&request_id).flatten()
        } else {
            None
        }
    }
}
