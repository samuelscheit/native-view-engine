//! Scope-bound RAII drop guards that cleanly release partially initialized Vulkan handles on
//! early initialization errors (`--features skia-vulkan`).

use ash::vk as ash_vk;

use super::setup::{destroy_sync_objects, FrameSyncObjects};

pub(super) struct InstanceGuard(pub(super) Option<ash::Instance>);

impl InstanceGuard {
    pub(super) fn get(&self) -> &ash::Instance {
        self.0.as_ref().expect("InstanceGuard active")
    }

    pub(super) fn disarm(mut self) -> ash::Instance {
        self.0.take().expect("InstanceGuard active")
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Some(instance) = self.0.take() {
            // SAFETY: Partial initialization failed before ownership transfer; child handles dropped first.
            unsafe { instance.destroy_instance(None) };
        }
    }
}

pub(super) struct SurfaceGuard<'a>(
    pub(super) &'a ash::khr::surface::Instance,
    pub(super) Option<ash_vk::SurfaceKHR>,
);

impl SurfaceGuard<'_> {
    pub(super) fn get(&self) -> ash_vk::SurfaceKHR {
        self.1.expect("SurfaceGuard active")
    }

    pub(super) fn disarm(mut self) -> ash_vk::SurfaceKHR {
        self.1.take().expect("SurfaceGuard active")
    }
}

impl Drop for SurfaceGuard<'_> {
    fn drop(&mut self) {
        if let Some(surface) = self.1.take() {
            // SAFETY: Partial initialization failed; `surface` belongs to `self.0` and outlives swapchain.
            unsafe { self.0.destroy_surface(surface, None) };
        }
    }
}

pub(super) struct DeviceGuard(pub(super) Option<ash::Device>);

impl DeviceGuard {
    pub(super) fn get(&self) -> &ash::Device {
        self.0.as_ref().expect("DeviceGuard active")
    }

    pub(super) fn disarm(mut self) -> ash::Device {
        self.0.take().expect("DeviceGuard active")
    }
}

impl Drop for DeviceGuard {
    fn drop(&mut self) {
        if let Some(device) = self.0.take() {
            // SAFETY: Partial initialization failed; all child device objects dropped first.
            unsafe { device.destroy_device(None) };
        }
    }
}

pub(super) struct SwapchainGuard<'a>(
    pub(super) &'a ash::khr::swapchain::Device,
    pub(super) Option<ash_vk::SwapchainKHR>,
);

impl SwapchainGuard<'_> {
    pub(super) fn disarm(mut self) -> ash_vk::SwapchainKHR {
        self.1.take().expect("SwapchainGuard active")
    }
}

impl Drop for SwapchainGuard<'_> {
    fn drop(&mut self) {
        if let Some(sc) = self.1.take() {
            // SAFETY: Partial initialization failed; swapchain has no live images wrapped by Skia.
            unsafe { self.0.destroy_swapchain(sc, None) };
        }
    }
}

pub(super) struct SyncObjectsGuard<'a>(
    pub(super) &'a ash::Device,
    pub(super) Vec<FrameSyncObjects>,
);

impl SyncObjectsGuard<'_> {
    pub(super) fn disarm(mut self) -> Vec<FrameSyncObjects> {
        std::mem::take(&mut self.1)
    }
}

impl Drop for SyncObjectsGuard<'_> {
    fn drop(&mut self) {
        if !self.1.is_empty() {
            destroy_sync_objects(self.0, &self.1);
            self.1.clear();
        }
    }
}
