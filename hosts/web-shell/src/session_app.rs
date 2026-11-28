//! Hosted application variant enum (`SessionApp`) for [`crate::session::WebAppSession`].

use nve_abi::{FrameId, NveHostCallbacks, ValidatedEvents};
use nve_backend_web::WebBackend;
#[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
use nve_backend_web::WebTableRegistry;
#[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
use nve_dioxus::DioxusSurface;
#[cfg(any(feature = "app-studio", feature = "app-kitchen-sink"))]
use nve_engine::NveApp;
use nve_engine::{BackendApplyStats, BackendError, StoreBackend};
#[cfg(feature = "app-kitchen-sink")]
use nve_example_kitchen_sink::{KitchenSinkModule, KITCHEN_SINK_STATE_VERSION};
use nve_store::NodeStore;
#[cfg(feature = "app-studio")]
use nve_studio_workspace::{StudioWorkspaceModule, STUDIO_SNAPSHOT_SCHEMA_VERSION};

pub(crate) enum SessionApp {
    /// A Dioxus surface plus its app family's web plugin registration.
    #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
    Dioxus(Box<DioxusSurface>, fn(&mut WebTableRegistry)),
    #[cfg(feature = "app-studio")]
    Studio(Box<StudioWorkspaceModule>),
    #[cfg(feature = "app-kitchen-sink")]
    KitchenSink(Box<KitchenSinkModule>),
}

impl SessionApp {
    /// Registers the hosted app family's shared web plugin set (the same function its SSR server
    /// or prerenderer calls), so client and server resolve identical tags.
    pub(crate) fn register_web_systems(&self, backend: &mut WebBackend) {
        let registry = backend.table_registry_mut();
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(_, register) => register(registry),
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => nve_studio_workspace::register_web_systems(registry),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(_) => nve_example_kitchen_sink::register_web_systems(registry),
        }
    }

    pub(crate) fn app_slug(&self) -> Option<&'static str> {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => None,
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => Some("studio"),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(_) => Some("kitchen-sink"),
        }
    }

    pub(crate) fn schema_version(&self) -> u32 {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => 1,
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => STUDIO_SNAPSHOT_SCHEMA_VERSION,
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(_) => KITCHEN_SINK_STATE_VERSION,
        }
    }

    pub(crate) fn commit_to_timed<B: StoreBackend>(
        &mut self,
        frame_id: FrameId,
        backend: &mut B,
    ) -> Result<(u64, BackendApplyStats), BackendError> {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.commit_to_timed(frame_id, backend),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.commit_to_timed(frame_id, backend),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.commit_to_timed(frame_id, backend),
        }
    }

    pub(crate) fn store(&self) -> &NodeStore {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.store(),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.surface().store(),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.surface().store(),
        }
    }

    #[cfg(feature = "dev")]
    pub(crate) fn apply_hot_patch(&mut self, patch: &nve_engine::HotPatchMsg) -> bool {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.apply_hot_patch(patch),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.apply_hot_patch(patch),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.apply_hot_patch(patch),
        }
    }

    #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
    pub(crate) fn poll_work(&mut self, waker: &core::task::Waker) -> bool {
        match self {
            Self::Dioxus(surface, _) => surface.poll_work(waker),
            #[cfg(feature = "app-studio")]
            Self::Studio(_) => false,
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(_) => false,
        }
    }

    pub(crate) fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(surface, _) => surface.dispatch_events(events),
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.dispatch_events(events),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.dispatch_events(events),
        }
    }

    /// Binds or unbinds `NVE_IFACE_HOST = 5` callbacks onto the hosted app.
    ///
    /// Returns `true` only when syncing the newly bound host mutated app state.
    #[cfg_attr(
        not(any(feature = "app-studio", feature = "app-kitchen-sink")),
        allow(unused_variables)
    )]
    pub(crate) fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) -> bool {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => false,
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.bind_host_sync(callbacks),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => {
                module.bind_host(callbacks);
                false
            }
        }
    }

    /// Forwards a canonical `NveHostEventBatch` (`NVHB` v1) byte buffer to the hosted app.
    #[cfg_attr(
        not(any(feature = "app-studio", feature = "app-kitchen-sink")),
        allow(unused_variables)
    )]
    pub(crate) fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => false,
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.dispatch_host_events(batch),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.dispatch_host_events(batch),
        }
    }

    pub(crate) fn snapshot_state(&mut self) -> Option<Vec<u8>> {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => None,
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => Some(module.snapshot_state().to_vec()),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => Some(module.snapshot_state().to_vec()),
        }
    }

    #[cfg_attr(
        not(any(feature = "app-studio", feature = "app-kitchen-sink")),
        allow(unused_variables)
    )]
    pub(crate) fn try_restore_state(&mut self, bytes: &[u8]) -> bool {
        match self {
            #[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
            Self::Dioxus(..) => false,
            #[cfg(feature = "app-studio")]
            Self::Studio(module) => module.try_restore_state(bytes),
            #[cfg(feature = "app-kitchen-sink")]
            Self::KitchenSink(module) => module.try_restore_state(bytes),
        }
    }
}
