//! Hot-swappable `cdylib` app slot ([`HotAppSlot`]) preserving state snapshots and host
//! bindings across Tier-2 reloads.

use std::path::Path;
use std::sync::Arc;

use nve_abi::{
    encode_single_push_into, FrameId, SurfaceId, ValidatedEvents, SERVICE_SYSTEM_ENV,
};
use nve_engine::loader::HotAppInstance;
use nve_system_env::{ThemeSnapshot, EVENT_THEME_CHANGED};

use super::{DesktopApp, DesktopHostError};
use crate::host_services::{SkiaHostEvent, SkiaHostServices, SkiaSlotHostBinding};

const THEME_PUSH_BATCH_LEN: usize = 56;

#[inline]
fn encode_theme_push_batch(theme: ThemeSnapshot, buf: &mut [u8; THEME_PUSH_BATCH_LEN]) -> &[u8] {
    let raw = theme.to_repr_c_bytes();
    encode_single_push_into(buf, SERVICE_SYSTEM_ENV, EVENT_THEME_CHANGED, &raw).unwrap_or(&[])
}

/// Owns a live [`HotAppInstance`] and preserves the last state snapshot and bound
/// [`SkiaHostServices`] across Tier-2 reloads (including recovery if an intermediate reload fails).
pub struct HotAppSlot {
    // Field order is load-bearing: `instance` MUST drop before `slot_binding` and `host_services`
    // so the loaded cdylib never observes a dangling `host_ctx` pointer during `destroy`.
    instance: Option<HotAppInstance>,
    generation: u64,
    last_saved_state: Vec<u8>,
    slot_binding: Option<Arc<SkiaSlotHostBinding>>,
    host_services: Option<Arc<SkiaHostServices>>,
}

impl HotAppSlot {
    pub fn load(dylib_path: &Path, surface_id: SurfaceId) -> Result<Self, DesktopHostError> {
        Self::load_with_state(dylib_path, surface_id, None)
    }

    pub fn load_with_state(
        dylib_path: &Path,
        surface_id: SurfaceId,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, DesktopHostError> {
        let instance = HotAppInstance::load(dylib_path, surface_id, 1, prior_state)?;
        let generation = instance.generation();
        Ok(Self {
            instance: Some(instance),
            generation,
            last_saved_state: prior_state.map(|s| s.to_vec()).unwrap_or_default(),
            slot_binding: None,
            host_services: None,
        })
    }
}

impl Drop for HotAppSlot {
    fn drop(&mut self) {
        if let Some(inst) = self.instance.as_mut() {
            let _ = inst.bind_host(None);
        }
        if let (Some(svc), Some(binding)) =
            (self.host_services.as_ref(), self.slot_binding.as_ref())
        {
            let _ = svc.discard_pending_for_owner(binding.owner_tag());
        }
        self.instance.take();
        self.slot_binding.take();
        self.host_services.take();
    }
}

impl DesktopApp for HotAppSlot {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        match self.instance.as_mut() {
            Some(inst) => inst.render(frame_id),
            None => &[],
        }
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        match self.instance.as_mut() {
            Some(inst) => inst.dispatch_events(events),
            None => false,
        }
    }

    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        match self.instance.as_mut() {
            Some(inst) => inst.apply_hot_patch(patch_nvp1),
            None => false,
        }
    }

    fn reload(
        &mut self,
        new_dylib_path: &Path,
        surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        if let Some(inst) = self.instance.as_mut() {
            let state = inst.snapshot_state();
            if !state.is_empty() {
                self.last_saved_state.clear();
                self.last_saved_state.extend_from_slice(state);
            }
            let next_binding = self
                .host_services
                .as_ref()
                .map(SkiaHostServices::new_slot_binding);
            let next_cbs = next_binding.as_ref().map(SkiaSlotHostBinding::callbacks);
            let outcome = match inst.reload_with_host(new_dylib_path, next_cbs) {
                Ok(outcome) => outcome,
                Err(err) => {
                    if let (Some(svc), Some(nb)) =
                        (self.host_services.as_ref(), next_binding.as_ref())
                    {
                        let _ = svc.discard_pending_for_owner(nb.owner_tag());
                    }
                    return Err(err.into());
                }
            };
            self.generation = outcome.generation;
            let prev_binding = std::mem::replace(&mut self.slot_binding, next_binding);
            if let Some(svc) = self.host_services.as_ref() {
                if let Some(old) = prev_binding.as_ref() {
                    let _ = svc.discard_pending_for_owner(old.owner_tag());
                }
                if inst.has_host_interface() {
                    let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                    let _ = inst.dispatch_host_events(encode_theme_push_batch(
                        svc.current_theme(),
                        &mut batch,
                    ));
                }
            }
            Ok(self.generation)
        } else {
            let next_gen = self.generation + 1;
            let prior = (!self.last_saved_state.is_empty())
                .then_some(self.last_saved_state.as_slice());
            let mut loaded = HotAppInstance::load(new_dylib_path, surface_id, next_gen, prior)?;
            let next_binding = self
                .host_services
                .as_ref()
                .map(SkiaHostServices::new_slot_binding);
            if let (Some(svc), Some(nb)) = (self.host_services.as_ref(), next_binding.as_ref())
                && loaded.bind_host(Some(nb.callbacks()))
            {
                let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                let _ = loaded.dispatch_host_events(encode_theme_push_batch(
                    svc.current_theme(),
                    &mut batch,
                ));
            }
            let old_binding = std::mem::replace(&mut self.slot_binding, next_binding);
            if let (Some(svc), Some(old)) = (self.host_services.as_ref(), old_binding.as_ref()) {
                let _ = svc.discard_pending_for_owner(old.owner_tag());
            }
            self.generation = loaded.generation();
            self.instance = Some(loaded);
            Ok(self.generation)
        }
    }

    #[cfg(test)]
    fn has_host_interface(&self) -> bool {
        self.instance
            .as_ref()
            .is_some_and(HotAppInstance::has_host_interface)
    }

    fn bind_host_services(&mut self, services: Option<Arc<SkiaHostServices>>) -> bool {
        match services {
            Some(svc) => {
                let next_binding = svc.new_slot_binding();
                let cb = next_binding.callbacks();
                let bound = self
                    .instance
                    .as_mut()
                    .is_some_and(|inst| inst.bind_host(Some(cb)));
                let prev_binding = self.slot_binding.replace(next_binding);
                let prev_svc = self.host_services.replace(svc);
                if let (Some(old_svc), Some(old_binding)) =
                    (prev_svc.as_ref(), prev_binding.as_ref())
                {
                    let _ = old_svc.discard_pending_for_owner(old_binding.owner_tag());
                }
                if bound && let Some(svc_ref) = self.host_services.as_ref() {
                    let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                    let encoded = encode_theme_push_batch(svc_ref.current_theme(), &mut batch);
                    if let Some(inst) = self.instance.as_mut() {
                        let _ = inst.dispatch_host_events(encoded);
                    }
                }
                bound
            }
            None => {
                let had = self
                    .instance
                    .as_mut()
                    .is_some_and(|inst| inst.bind_host(None));
                let prev_binding = self.slot_binding.take();
                let prev_svc = self.host_services.take();
                if let (Some(old_svc), Some(old_binding)) =
                    (prev_svc.as_ref(), prev_binding.as_ref())
                {
                    let _ = old_svc.discard_pending_for_owner(old_binding.owner_tag());
                }
                had
            }
        }
    }

    fn dispatch_host_batch(&mut self, batch: &[u8]) -> bool {
        self.instance
            .as_mut()
            .is_some_and(|inst| inst.dispatch_host_events(batch))
    }

    fn host_owner_tag(&self) -> u64 {
        self.slot_binding
            .as_ref()
            .map_or(0, |binding| binding.owner_tag())
    }

    fn dispatch_host_event(&mut self, event: &SkiaHostEvent) -> bool {
        let Some(inst) = self.instance.as_mut() else {
            return false;
        };
        match event {
            SkiaHostEvent::ThemeChanged(theme) => {
                let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                inst.dispatch_host_events(encode_theme_push_batch(*theme, &mut batch))
            }
            #[cfg(test)]
            SkiaHostEvent::SystemEnv { theme, .. } => {
                let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                inst.dispatch_host_events(encode_theme_push_batch(*theme, &mut batch))
            }
            #[cfg(test)]
            SkiaHostEvent::LocaleChanged(_) => false,
            SkiaHostEvent::WindowLifecycle(_) => false,
        }
    }
}
