//! Vulkan instance, physical/logical device, WSI surface, swapchain, sync primitive, and Skia
//! Ganesh `DirectContext` initialization helpers (`--features skia-vulkan`).

use std::ffi::CStr;
use std::ptr;

use ash::vk as ash_vk;
use ash::vk::Handle as AshHandle;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use skia_safe::{
    gpu::{direct_contexts, vk, DirectContext},
    ColorType,
};

use crate::gpu::{GpuSurfaceError, SwapchainExtent};

pub(super) struct FrameSyncObjects {
    pub(super) image_available: ash_vk::Semaphore,
    pub(super) render_finished: ash_vk::Semaphore,
    pub(super) in_flight: ash_vk::Fence,
    pub(super) in_flight_pending: bool,
}

pub(super) struct SkiaVulkanContextParams<'a> {
    pub(super) entry: &'a ash::Entry,
    pub(super) instance: &'a ash::Instance,
    pub(super) physical_device: ash_vk::PhysicalDevice,
    pub(super) device: &'a ash::Device,
    pub(super) queue: ash_vk::Queue,
    pub(super) queue_family_index: u32,
    pub(super) instance_exts: &'a [String],
    pub(super) device_exts: &'a [String],
}

pub(super) struct VkSwapchainParams<'a> {
    pub(super) surface_loader: &'a ash::khr::surface::Instance,
    pub(super) swapchain_loader: &'a ash::khr::swapchain::Device,
    pub(super) physical_device: ash_vk::PhysicalDevice,
    pub(super) surface: ash_vk::SurfaceKHR,
    pub(super) extent: SwapchainExtent,
    pub(super) vsync: bool,
    pub(super) requested_images: u32,
    pub(super) old_swapchain: ash_vk::SwapchainKHR,
}

pub(super) struct CreatedSwapchain {
    pub(super) swapchain: ash_vk::SwapchainKHR,
    pub(super) images: Vec<ash_vk::Image>,
    pub(super) extent: ash_vk::Extent2D,
    pub(super) format: ash_vk::Format,
    pub(super) color_type: ColorType,
}

pub(super) fn load_vulkan_entry() -> Result<ash::Entry, GpuSurfaceError> {
    // SAFETY: `ash::Entry::load` opens the platform Vulkan loader library via `dlopen`
    // (`libvulkan.so.1` / `libMoltenVK.dylib` / `vulkan-1.dll`) with standard OS semantics.
    unsafe { ash::Entry::load() }.map_err(|e| {
        GpuSurfaceError::DeviceUnavailable(format!("failed to load Vulkan loader: {e}"))
    })
}

pub(super) fn required_surface_extensions(
    display: RawDisplayHandle,
) -> Result<Vec<&'static CStr>, GpuSurfaceError> {
    let mut exts = vec![ash::khr::surface::NAME];
    match display {
        #[cfg(target_os = "macos")]
        RawDisplayHandle::AppKit(_) => exts.push(ash::ext::metal_surface::NAME),
        #[cfg(target_os = "ios")]
        RawDisplayHandle::UiKit(_) => exts.push(ash::ext::metal_surface::NAME),
        #[cfg(target_os = "windows")]
        RawDisplayHandle::Windows(_) => exts.push(ash::khr::win32_surface::NAME),
        #[cfg(target_os = "android")]
        RawDisplayHandle::Android(_) => exts.push(ash::khr::android_surface::NAME),
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        RawDisplayHandle::Xlib(_) => exts.push(ash::khr::xlib_surface::NAME),
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        RawDisplayHandle::Xcb(_) => exts.push(ash::khr::xcb_surface::NAME),
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        RawDisplayHandle::Wayland(_) => exts.push(ash::khr::wayland_surface::NAME),
        other => {
            return Err(GpuSurfaceError::WindowHandleError(format!(
                "unsupported RawDisplayHandle for Vulkan surface: {other:?}"
            )));
        }
    }
    Ok(exts)
}

pub(super) fn create_instance(
    entry: &ash::Entry,
    extra_exts: &[&'static CStr],
) -> Result<(ash::Instance, Vec<String>), GpuSurfaceError> {
    let app_name = c"nve-backend-skia";
    let app_info = ash_vk::ApplicationInfo::default()
        .application_name(app_name)
        .engine_name(app_name)
        .api_version(ash_vk::API_VERSION_1_1);

    // SAFETY: `entry` is a loaded Vulkan loader entrypoint.
    let available =
        unsafe { entry.enumerate_instance_extension_properties(None) }.unwrap_or_default();
    let has_portability = available.iter().any(|p| {
        // SAFETY: Vulkan guarantees `extension_name` is a NUL-terminated C string.
        let ext_name = unsafe { CStr::from_ptr(p.extension_name.as_ptr()) };
        ext_name == ash::khr::portability_enumeration::NAME
    });

    let mut ext_ptrs: Vec<*const i8> = extra_exts.iter().map(|e| e.as_ptr()).collect();
    let mut ext_names: Vec<String> = extra_exts
        .iter()
        .map(|e| e.to_string_lossy().into_owned())
        .collect();
    let mut flags = ash_vk::InstanceCreateFlags::empty();
    if has_portability {
        ext_ptrs.push(ash::khr::portability_enumeration::NAME.as_ptr());
        ext_names.push(
            ash::khr::portability_enumeration::NAME
                .to_string_lossy()
                .into_owned(),
        );
        flags |= ash_vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
    }

    let create_info = ash_vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&ext_ptrs)
        .flags(flags);
    // SAFETY: `create_info` borrows live NUL-terminated extension names and `app_info` for this call.
    let instance = unsafe { entry.create_instance(&create_info, None) }.map_err(|e| {
        GpuSurfaceError::DeviceUnavailable(format!("vkCreateInstance failed: {e}"))
    })?;
    Ok((instance, ext_names))
}

pub(super) fn pick_physical_device(
    instance: &ash::Instance,
    surface: Option<(&ash::khr::surface::Instance, ash_vk::SurfaceKHR)>,
    required_device_exts: &[&'static CStr],
) -> Result<(ash_vk::PhysicalDevice, u32), GpuSurfaceError> {
    // SAFETY: `instance` is a valid live Vulkan instance.
    let devices = unsafe { instance.enumerate_physical_devices() }.map_err(|e| {
        GpuSurfaceError::DeviceUnavailable(format!("vkEnumeratePhysicalDevices failed: {e}"))
    })?;

    let mut best: Option<(u32, ash_vk::PhysicalDevice, u32)> = None;
    for pdev in devices {
        // SAFETY: `instance` and `pdev` are valid live Vulkan handles.
        let exts = unsafe { instance.enumerate_device_extension_properties(pdev) }.unwrap_or_default();
        let supports_all = required_device_exts.iter().all(|req| {
            exts.iter().any(|avail| {
                // SAFETY: Vulkan guarantees `extension_name` is a NUL-terminated C string.
                let ext_name = unsafe { CStr::from_ptr(avail.extension_name.as_ptr()) };
                ext_name == *req
            })
        });
        if !supports_all {
            continue;
        }
        // SAFETY: `instance` and `pdev` are valid live Vulkan handles.
        let queues = unsafe { instance.get_physical_device_queue_family_properties(pdev) };
        let found_queue = queues.iter().enumerate().find_map(|(idx, props)| {
            if !props.queue_flags.contains(ash_vk::QueueFlags::GRAPHICS) {
                return None;
            }
            if let Some((surf_loader, surf_khr)) = surface {
                // SAFETY: `surf_loader`, `pdev`, and `surf_khr` are live handles and `idx` is a valid queue family index.
                let ok = unsafe {
                    surf_loader.get_physical_device_surface_support(pdev, idx as u32, surf_khr)
                }
                .unwrap_or(false);
                if !ok {
                    return None;
                }
            }
            Some(idx as u32)
        });
        let Some(q_idx) = found_queue else {
            continue;
        };
        // SAFETY: `instance` and `pdev` are valid live Vulkan handles.
        let props = unsafe { instance.get_physical_device_properties(pdev) };
        let rank = match props.device_type {
            ash_vk::PhysicalDeviceType::DISCRETE_GPU => 0,
            ash_vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
            ash_vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
            ash_vk::PhysicalDeviceType::CPU => 3,
            _ => 4,
        };
        if best.is_none_or(|(best_rank, _, _)| rank < best_rank) {
            best = Some((rank, pdev, q_idx));
        }
    }

    best.map(|(_, pdev, q_idx)| (pdev, q_idx)).ok_or_else(|| {
        GpuSurfaceError::DeviceUnavailable("no suitable Vulkan physical device found".to_string())
    })
}

pub(super) fn create_device(
    instance: &ash::Instance,
    physical_device: ash_vk::PhysicalDevice,
    queue_family_index: u32,
    required_exts: &[&'static CStr],
) -> Result<(ash::Device, ash_vk::Queue, Vec<String>), GpuSurfaceError> {
    let priorities = [1.0f32];
    let queue_infos = [ash_vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family_index)
        .queue_priorities(&priorities)];
    let ext_ptrs: Vec<*const i8> = required_exts.iter().map(|e| e.as_ptr()).collect();
    let ext_names: Vec<String> = required_exts
        .iter()
        .map(|e| e.to_string_lossy().into_owned())
        .collect();
    let create_info = ash_vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_infos)
        .enabled_extension_names(&ext_ptrs);
    // SAFETY: `instance`, `physical_device`, and `create_info` slices are valid for the duration of this call.
    let device =
        unsafe { instance.create_device(physical_device, &create_info, None) }.map_err(|e| {
            GpuSurfaceError::DeviceUnavailable(format!("vkCreateDevice failed: {e}"))
        })?;
    // SAFETY: Queue index 0 of `queue_family_index` was requested in `queue_infos` above.
    let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
    Ok((device, queue, ext_names))
}

pub(super) fn create_skia_vulkan_context(
    params: &SkiaVulkanContextParams<'_>,
) -> Result<DirectContext, GpuSurfaceError> {
    let get_proc = |gpo: vk::GetProcOf| {
        // SAFETY: Skia queries Vulkan entrypoints using valid handles and NUL-terminated names while `entry`/`instance` are live.
        unsafe {
            let f = match gpo {
                vk::GetProcOf::Instance(inst, name) => {
                    let vk_inst = ash_vk::Instance::from_raw(inst as _);
                    params.entry.get_instance_proc_addr(vk_inst, name)
                }
                vk::GetProcOf::Device(dev, name) => {
                    let vk_dev = ash_vk::Device::from_raw(dev as _);
                    (params.instance.fp_v1_0().get_device_proc_addr)(vk_dev, name)
                }
            };
            f.map(|fp| fp as *const std::ffi::c_void)
                .unwrap_or(ptr::null())
        }
    };

    let inst_ext_refs: Vec<&str> = params.instance_exts.iter().map(String::as_str).collect();
    let dev_ext_refs: Vec<&str> = params.device_exts.iter().map(String::as_str).collect();
    // SAFETY: All Vulkan handles are valid and kept alive in `VulkanNativeSwapchain` longer than the returned `DirectContext`.
    let backend_context = unsafe {
        vk::BackendContext::new_builder(
            params.instance.handle().as_raw() as _,
            params.physical_device.as_raw() as _,
            params.device.handle().as_raw() as _,
            (params.queue.as_raw() as _, params.queue_family_index as usize),
            &get_proc,
            Some((1, 1, 0).into()),
        )
        .with_extensions(&inst_ext_refs, &dev_ext_refs)
        .build()
    };

    direct_contexts::make_vulkan(&backend_context, None).ok_or_else(|| {
        GpuSurfaceError::SurfaceCreationFailed(
            "skia_safe::gpu::direct_contexts::make_vulkan returned None".to_string(),
        )
    })
}

pub(super) fn create_vk_surface(
    entry: &ash::Entry,
    instance: &ash::Instance,
    display: RawDisplayHandle,
    window: RawWindowHandle,
) -> Result<ash_vk::SurfaceKHR, GpuSurfaceError> {
    match (display, window) {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        (RawDisplayHandle::Xlib(d), RawWindowHandle::Xlib(w)) => {
            let dpy = d.display.ok_or_else(|| {
                GpuSurfaceError::WindowHandleError("Xlib display pointer is null".to_string())
            })?;
            let info = ash_vk::XlibSurfaceCreateInfoKHR::default()
                .dpy(dpy.as_ptr() as *mut _)
                .window(w.window);
            let loader = ash::khr::xlib_surface::Instance::new(entry, instance);
            // SAFETY: Caller supplies live Xlib display/window handles matching `instance`.
            unsafe { loader.create_xlib_surface(&info, None) }
                .map_err(|e| GpuSurfaceError::SurfaceCreationFailed(format!("xlib surface: {e}")))
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        (RawDisplayHandle::Xcb(d), RawWindowHandle::Xcb(w)) => {
            let conn = d.connection.ok_or_else(|| {
                GpuSurfaceError::WindowHandleError("Xcb connection pointer is null".to_string())
            })?;
            let info = ash_vk::XcbSurfaceCreateInfoKHR::default()
                .connection(conn.as_ptr())
                .window(w.window.get());
            let loader = ash::khr::xcb_surface::Instance::new(entry, instance);
            // SAFETY: Caller supplies live XCB connection/window handles matching `instance`.
            unsafe { loader.create_xcb_surface(&info, None) }
                .map_err(|e| GpuSurfaceError::SurfaceCreationFailed(format!("xcb surface: {e}")))
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) => {
            let info = ash_vk::WaylandSurfaceCreateInfoKHR::default()
                .display(d.display.as_ptr())
                .surface(w.surface.as_ptr());
            let loader = ash::khr::wayland_surface::Instance::new(entry, instance);
            // SAFETY: Caller supplies live Wayland display/surface pointers matching `instance`.
            unsafe { loader.create_wayland_surface(&info, None) }.map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!("wayland surface: {e}"))
            })
        }
        #[cfg(target_os = "windows")]
        (RawDisplayHandle::Windows(_), RawWindowHandle::Win32(w)) => {
            let hinstance = w.hinstance.map(|h| h.get() as isize).unwrap_or(0);
            let info = ash_vk::Win32SurfaceCreateInfoKHR::default()
                .hinstance(hinstance)
                .hwnd(w.hwnd.get());
            let loader = ash::khr::win32_surface::Instance::new(entry, instance);
            // SAFETY: Caller supplies live Win32 HINSTANCE/HWND handles matching `instance`.
            unsafe { loader.create_win32_surface(&info, None) }.map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!("win32 surface: {e}"))
            })
        }
        #[cfg(target_os = "android")]
        (RawDisplayHandle::Android(_), RawWindowHandle::AndroidNdk(w)) => {
            let info = ash_vk::AndroidSurfaceCreateInfoKHR::default()
                .window(w.a_native_window.as_ptr());
            let loader = ash::khr::android_surface::Instance::new(entry, instance);
            // SAFETY: Caller supplies a live Android `ANativeWindow` pointer matching `instance`.
            unsafe { loader.create_android_surface(&info, None) }.map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!("android surface: {e}"))
            })
        }
        #[cfg(target_os = "macos")]
        (RawDisplayHandle::AppKit(_), RawWindowHandle::AppKit(w)) => {
            let info = ash_vk::MetalSurfaceCreateInfoEXT::default()
                .layer(w.ns_view.as_ptr() as *const _);
            let loader = ash::ext::metal_surface::Instance::new(entry, instance);
            // SAFETY: Caller supplies a live AppKit `NSView` handle matching `instance`.
            unsafe { loader.create_metal_surface(&info, None) }.map_err(|e| {
                GpuSurfaceError::SurfaceCreationFailed(format!("metal vk surface: {e}"))
            })
        }
        (d, w) => {
            let _ = (entry, instance);
            Err(GpuSurfaceError::WindowHandleError(format!(
                "unsupported display/window handle pair for Vulkan: ({d:?}, {w:?})"
            )))
        }
    }
}

pub(super) fn create_vk_swapchain(
    params: &VkSwapchainParams<'_>,
) -> Result<CreatedSwapchain, GpuSurfaceError> {
    // SAFETY: `surface_loader`, `physical_device`, and `surface` are valid live Vulkan handles.
    let caps = unsafe {
        params
            .surface_loader
            .get_physical_device_surface_capabilities(params.physical_device, params.surface)
    }
    .map_err(|e| GpuSurfaceError::SurfaceCreationFailed(format!("surface caps: {e}")))?;
    // SAFETY: `surface_loader`, `physical_device`, and `surface` are valid live Vulkan handles.
    let formats = unsafe {
        params
            .surface_loader
            .get_physical_device_surface_formats(params.physical_device, params.surface)
    }
    .map_err(|e| GpuSurfaceError::SurfaceCreationFailed(format!("surface formats: {e}")))?;

    let surface_fmt = formats
        .iter()
        .copied()
        .find(|f| {
            f.format == ash_vk::Format::B8G8R8A8_UNORM || f.format == ash_vk::Format::R8G8B8A8_UNORM
        })
        .or_else(|| formats.first().copied())
        .ok_or_else(|| {
            GpuSurfaceError::SurfaceCreationFailed(
                "Vulkan surface reported zero formats".to_string(),
            )
        })?;

    let color_type = match surface_fmt.format {
        ash_vk::Format::R8G8B8A8_UNORM | ash_vk::Format::R8G8B8A8_SRGB => ColorType::RGBA8888,
        _ => ColorType::BGRA8888,
    };

    let mut image_count = params.requested_images.max(caps.min_image_count).max(2);
    if caps.max_image_count > 0 && image_count > caps.max_image_count {
        image_count = caps.max_image_count;
    }

    let present_mode = if params.vsync {
        ash_vk::PresentModeKHR::FIFO
    } else {
        // SAFETY: `surface_loader`, `physical_device`, and `surface` are valid live Vulkan handles.
        let modes = unsafe {
            params
                .surface_loader
                .get_physical_device_surface_present_modes(params.physical_device, params.surface)
        }
        .unwrap_or_default();
        if modes.contains(&ash_vk::PresentModeKHR::MAILBOX) {
            ash_vk::PresentModeKHR::MAILBOX
        } else if modes.contains(&ash_vk::PresentModeKHR::IMMEDIATE) {
            ash_vk::PresentModeKHR::IMMEDIATE
        } else {
            ash_vk::PresentModeKHR::FIFO
        }
    };

    let vk_extent = if caps.current_extent.width != u32::MAX {
        caps.current_extent
    } else {
        ash_vk::Extent2D {
            width: params
                .extent
                .width
                .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
            height: params
                .extent
                .height
                .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        }
    };

    let info = ash_vk::SwapchainCreateInfoKHR::default()
        .surface(params.surface)
        .min_image_count(image_count)
        .image_format(surface_fmt.format)
        .image_color_space(surface_fmt.color_space)
        .image_extent(vk_extent)
        .image_array_layers(1)
        .image_usage(ash_vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(ash_vk::SharingMode::EXCLUSIVE)
        .pre_transform(caps.current_transform)
        .composite_alpha(ash_vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(present_mode)
        .clipped(true)
        .old_swapchain(params.old_swapchain);

    // SAFETY: `swapchain_loader` and `info` reference valid live surface/swapchain handles.
    let swapchain = unsafe { params.swapchain_loader.create_swapchain(&info, None) }.map_err(
        |e| GpuSurfaceError::SurfaceCreationFailed(format!("vkCreateSwapchainKHR failed: {e}")),
    )?;
    // SAFETY: `swapchain` was just created on `swapchain_loader`.
    let images = match unsafe { params.swapchain_loader.get_swapchain_images(swapchain) } {
        Ok(imgs) => imgs,
        Err(e) => {
            // SAFETY: `swapchain` was created above and has no active references on error.
            unsafe {
                params.swapchain_loader.destroy_swapchain(swapchain, None);
            }
            return Err(GpuSurfaceError::SurfaceCreationFailed(format!(
                "vkGetSwapchainImagesKHR failed: {e}"
            )));
        }
    };
    Ok(CreatedSwapchain {
        swapchain,
        images,
        extent: vk_extent,
        format: surface_fmt.format,
        color_type,
    })
}

pub(super) fn destroy_sync_objects(device: &ash::Device, sync_frames: &[FrameSyncObjects]) {
    // SAFETY: All semaphores and fences in `sync_frames` belong to `device` and are no longer in use.
    unsafe {
        for sync in sync_frames {
            device.destroy_semaphore(sync.image_available, None);
            device.destroy_semaphore(sync.render_finished, None);
            device.destroy_fence(sync.in_flight, None);
        }
    }
}

pub(super) fn create_sync_objects(
    device: &ash::Device,
    count: usize,
) -> Result<Vec<FrameSyncObjects>, GpuSurfaceError> {
    let mut out = Vec::with_capacity(count);
    let sem_info = ash_vk::SemaphoreCreateInfo::default();
    let fence_info = ash_vk::FenceCreateInfo::default().flags(ash_vk::FenceCreateFlags::SIGNALED);
    for _ in 0..count {
        // SAFETY: `device` is a valid live Vulkan logical device and `sem_info` is initialized.
        let image_available = match unsafe { device.create_semaphore(&sem_info, None) } {
            Ok(s) => s,
            Err(e) => {
                destroy_sync_objects(device, &out);
                return Err(GpuSurfaceError::SurfaceCreationFailed(format!(
                    "create_semaphore: {e}"
                )));
            }
        };
        // SAFETY: `device` is a valid live Vulkan logical device and `sem_info` is initialized.
        let render_finished = match unsafe { device.create_semaphore(&sem_info, None) } {
            Ok(s) => s,
            Err(e) => {
                // SAFETY: `image_available` was just created on `device`.
                unsafe {
                    device.destroy_semaphore(image_available, None);
                }
                destroy_sync_objects(device, &out);
                return Err(GpuSurfaceError::SurfaceCreationFailed(format!(
                    "create_semaphore: {e}"
                )));
            }
        };
        // SAFETY: `device` is a valid live Vulkan logical device and `fence_info` is initialized.
        let in_flight = match unsafe { device.create_fence(&fence_info, None) } {
            Ok(f) => f,
            Err(e) => {
                // SAFETY: Both semaphores were just created on `device`.
                unsafe {
                    device.destroy_semaphore(render_finished, None);
                    device.destroy_semaphore(image_available, None);
                }
                destroy_sync_objects(device, &out);
                return Err(GpuSurfaceError::SurfaceCreationFailed(format!(
                    "create_fence: {e}"
                )));
            }
        };
        out.push(FrameSyncObjects {
            image_available,
            render_finished,
            in_flight,
            in_flight_pending: false,
        });
    }
    Ok(out)
}

pub(super) fn map_ash_to_skia_format(fmt: ash_vk::Format) -> vk::Format {
    match fmt {
        ash_vk::Format::R8G8B8A8_UNORM => vk::Format::R8G8B8A8_UNORM,
        ash_vk::Format::R8G8B8A8_SRGB => vk::Format::R8G8B8A8_SRGB,
        ash_vk::Format::B8G8R8A8_SRGB => vk::Format::B8G8R8A8_SRGB,
        _ => vk::Format::B8G8R8A8_UNORM,
    }
}
