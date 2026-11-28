//! Scope-bound RAII drop guards that cleanly release partially initialized Vulkan handles on
//! early initialization errors (`--features skia-vulkan`).

use std::mem::ManuallyDrop;

use ash::vk as ash_vk;
use skia_safe::ColorType;

use super::setup::{destroy_sync_objects, FrameSyncObjects};

/// Owns `VkSurfaceKHR`, `VkSwapchainKHR`, and per-frame sync primitives.
///
/// Placed AFTER `surface` and `direct_context` and BEFORE `VulkanDeviceOwner` so Rust drops
/// Skia's `Surface` and `DirectContext` strictly BEFORE destroying swapchain/surface handles,
/// and destroys those handles strictly BEFORE `vkDestroyDevice`/`vkDestroyInstance`.
pub(super) struct VulkanWindowState {
    pub(super) device: ash::Device,
    pub(super) surface_loader: ash::khr::surface::Instance,
    pub(super) swapchain_loader: ash::khr::swapchain::Device,
    pub(super) surface_khr: ash_vk::SurfaceKHR,
    pub(super) swapchain_khr: Option<ash_vk::SwapchainKHR>,
    pub(super) images: Vec<ash_vk::Image>,
    pub(super) vk_extent: ash_vk::Extent2D,
    pub(super) format: ash_vk::Format,
    pub(super) color_type: ColorType,
    pub(super) sync_frames: Vec<FrameSyncObjects>,
    pub(super) current_sync: usize,
    pub(super) active_image_index: Option<u32>,
    pub(super) swapchain_dirty: bool,
    pub(super) vsync: bool,
    pub(super) min_image_count: u32,
}

impl Drop for VulkanWindowState {
    fn drop(&mut self) {
        // SAFETY: Earlier fields of `VulkanNativeSwapchain` already flushed, waited idle,
        // dropped every Skia `Surface` and `DirectContext`, while `VulkanDeviceOwner` (last
        // field) still holds `device` and `instance` alive.
        unsafe {
            let _ = self.device.device_wait_idle();
            destroy_sync_objects(&self.device, &self.sync_frames);
            self.sync_frames.clear();
            if let Some(sc) = self.swapchain_khr.take() {
                self.swapchain_loader.destroy_swapchain(sc, None);
            }
            self.surface_loader.destroy_surface(self.surface_khr, None);
        }
    }
}

/// Owns `VkDevice`, `VkInstance`, and `ash::Entry` as the **last** field of
/// [`super::VulkanNativeSwapchain`] so `vkDestroyDevice` / `vkDestroyInstance` ALWAYS run
/// strictly AFTER every Skia `Surface`, `DirectContext`, swapchain, and `VkSurfaceKHR` drop.
///
/// If an external Skia handle (such as a GPU-backed `Image`) still holds a reference to
/// `GrDirectContext` at swapchain drop, `skip_destroy` is set so the device, instance, and
/// `libvulkan` loader library are intentionally leaked instead of triggering a use-after-free.
pub(super) struct VulkanDeviceOwner {
    pub(super) device: ash::Device,
    pub(super) instance: ash::Instance,
    pub(super) entry: ManuallyDrop<ash::Entry>,
    pub(super) skip_destroy: bool,
    #[cfg(test)]
    pub(super) leaked_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for VulkanDeviceOwner {
    fn drop(&mut self) {
        if self.skip_destroy {
            #[cfg(test)]
            self.leaked_flag
                .store(true, std::sync::atomic::Ordering::Release);
            eprintln!(
                "[nve-backend-skia] Vulkan GrDirectContext still has external references at \
                 swapchain drop; leaking VkDevice/VkInstance/libvulkan to prevent use-after-free"
            );
            return;
        }
        // SAFETY: All Skia surfaces, `DirectContext` (verified unique), and child Vulkan objects
        // live in earlier fields of `VulkanNativeSwapchain` and have already been dropped.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
            ManuallyDrop::drop(&mut self.entry);
        }
    }
}

pub(super) struct InstanceGuard {
    instance: ManuallyDrop<ash::Instance>,
    active: bool,
}

impl InstanceGuard {
    pub(super) fn new(instance: ash::Instance) -> Self {
        Self {
            instance: ManuallyDrop::new(instance),
            active: true,
        }
    }

    pub(super) fn get(&self) -> &ash::Instance {
        &self.instance
    }

    pub(super) fn disarm(mut self) -> ash::Instance {
        self.active = false;
        // SAFETY: `self.active` is now `false`, so `Drop` will never touch `self.instance` again,
        // and `disarm` moves out by value.
        unsafe { ManuallyDrop::take(&mut self.instance) }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: Partial initialization failed before ownership transfer; child handles
            // dropped first and `self.instance` is destroyed and dropped exactly once here.
            unsafe {
                self.instance.destroy_instance(None);
                ManuallyDrop::drop(&mut self.instance);
            }
        }
    }
}

pub(super) struct SurfaceGuard<'a> {
    loader: &'a ash::khr::surface::Instance,
    surface: ash_vk::SurfaceKHR,
    active: bool,
}

impl<'a> SurfaceGuard<'a> {
    pub(super) fn new(loader: &'a ash::khr::surface::Instance, surface: ash_vk::SurfaceKHR) -> Self {
        Self {
            loader,
            surface,
            active: true,
        }
    }

    pub(super) fn get(&self) -> ash_vk::SurfaceKHR {
        self.surface
    }

    pub(super) fn disarm(mut self) -> ash_vk::SurfaceKHR {
        self.active = false;
        self.surface
    }
}

impl Drop for SurfaceGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: Partial initialization failed; `self.surface` belongs to `self.loader` and outlives swapchain.
            unsafe { self.loader.destroy_surface(self.surface, None) };
        }
    }
}

pub(super) struct DeviceGuard {
    device: ManuallyDrop<ash::Device>,
    active: bool,
}

impl DeviceGuard {
    pub(super) fn new(device: ash::Device) -> Self {
        Self {
            device: ManuallyDrop::new(device),
            active: true,
        }
    }

    pub(super) fn get(&self) -> &ash::Device {
        &self.device
    }

    pub(super) fn disarm(mut self) -> ash::Device {
        self.active = false;
        // SAFETY: `self.active` is now `false`, so `Drop` will never touch `self.device` again,
        // and `disarm` moves out by value.
        unsafe { ManuallyDrop::take(&mut self.device) }
    }
}

impl Drop for DeviceGuard {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: Partial initialization failed; wait for device idle before destroying
            // `self.device` and dropping the wrapper exactly once here.
            unsafe {
                let _ = self.device.device_wait_idle();
                self.device.destroy_device(None);
                ManuallyDrop::drop(&mut self.device);
            }
        }
    }
}

pub(super) struct SwapchainGuard<'a> {
    loader: &'a ash::khr::swapchain::Device,
    swapchain: ash_vk::SwapchainKHR,
    active: bool,
}

impl<'a> SwapchainGuard<'a> {
    pub(super) fn new(
        loader: &'a ash::khr::swapchain::Device,
        swapchain: ash_vk::SwapchainKHR,
    ) -> Self {
        Self {
            loader,
            swapchain,
            active: true,
        }
    }

    pub(super) fn disarm(mut self) -> ash_vk::SwapchainKHR {
        self.active = false;
        self.swapchain
    }
}

impl Drop for SwapchainGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: Partial initialization failed; swapchain has no live images wrapped by Skia.
            unsafe { self.loader.destroy_swapchain(self.swapchain, None) };
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
