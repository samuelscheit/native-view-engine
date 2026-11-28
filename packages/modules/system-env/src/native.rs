//! Zero-dependency native OS system theme & locale backends (macOS AppKit/Foundation,
//! Windows Registry/Win32, Unix/GTK/POSIX, WASM fallback) + pure POSIX/BCP-47 parser.

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_SYSTEM_ENV;
use nve_host_services::{
    pack_typed, CompletionSink, HandlerReply, HostServiceHandler, ServiceError,
};

use crate::{dispatch_system_env_ffi, LocaleSnapshot, SystemEnvBackend, TextDirection, ThemeSnapshot};

/// Parses a POSIX (`en_US.UTF-8`, `ar_EG@euro`) or BCP-47 (`en-US`, `he-IL`) locale string
/// into a normalized [`LocaleSnapshot`].
pub fn parse_locale_tag(
    raw: &str,
    timezone: Option<String>,
    uses_24h_clock: bool,
) -> LocaleSnapshot {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("C")
        || trimmed.eq_ignore_ascii_case("POSIX")
    {
        return LocaleSnapshot {
            timezone: timezone.unwrap_or_else(|| "UTC".to_owned()),
            uses_24h_clock,
            ..LocaleSnapshot::default()
        };
    }

    let core = trimmed
        .split(['.', '@'])
        .next()
        .unwrap_or(trimmed)
        .replace('_', "-");
    let parts: Vec<&str> = core.split('-').filter(|p| !p.is_empty()).collect();
    #[expect(
        clippy::disallowed_methods,
        reason = "OS locale snapshot owns canonical lowercase language tag"
    )]
    let language = parts
        .first()
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "en".to_owned());

    let region = parts
        .iter()
        .skip(1)
        .find(|p| {
            (p.len() == 2 && p.bytes().all(|b| b.is_ascii_alphabetic()))
                || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_default();

    let tag = if region.is_empty() {
        language.clone()
    } else {
        format!("{language}-{region}")
    };

    let text_direction = if matches!(
        language.as_str(),
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "yi" | "dv"
    ) {
        TextDirection::Rtl
    } else {
        TextDirection::Ltr
    };

    LocaleSnapshot {
        tag,
        language,
        region,
        text_direction,
        uses_24h_clock,
        timezone: timezone.unwrap_or_else(|| "UTC".to_owned()),
    }
}

/// Native OS [`HostServiceHandler`] for [`SERVICE_SYSTEM_ENV`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeSystemEnv;

impl NativeSystemEnv {
    pub const fn new() -> Self {
        Self
    }
}

impl SystemEnvBackend for NativeSystemEnv {
    fn get_theme(&self) -> Result<ThemeSnapshot, ServiceError> {
        platform::get_theme()
    }

    fn get_locale(&self) -> Result<LocaleSnapshot, ServiceError> {
        platform::get_locale()
    }
}

impl HostServiceHandler for NativeSystemEnv {
    fn service_id(&self) -> u32 {
        SERVICE_SYSTEM_ENV
    }

    fn handle(
        &self,
        method_id: u32,
        _request_id: u64,
        request: &[u8],
        _done: &CompletionSink,
    ) -> Result<HandlerReply, ServiceError> {
        dispatch_system_env_ffi(self, method_id, request).map(HandlerReply::ready)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn SystemEnvBackend>(self))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use nve_host_services::apple_objc::{
        cls, msg_id0, msg_id1, nsstring_from_str, nsstring_to_string, objc_msgSend, sel,
        with_autorelease_pool, Id, Sel,
    };
    use nve_host_services::ServiceError;

    use crate::{ColorScheme, ContrastLevel, LocaleSnapshot, ThemeSnapshot};

    use super::parse_locale_tag;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSApp: Id;
    }

    pub fn get_theme() -> Result<ThemeSnapshot, ServiceError> {
        // SAFETY: Runs inside an active autorelease pool using static NUL-terminated AppKit/Foundation selectors.
        with_autorelease_pool(|| unsafe {
            let mut color_scheme = ColorScheme::Light;
            let app = NSApp;
            if !app.is_null() {
                let appearance = msg_id0(app, sel(b"effectiveAppearance\0"));
                if !appearance.is_null() {
                    let name_obj = msg_id0(appearance, sel(b"name\0"));
                    if let Some(name) = nsstring_to_string(name_obj) {
                        color_scheme = if name.contains("Dark") {
                            ColorScheme::Dark
                        } else {
                            ColorScheme::Light
                        };
                    }
                }
            } else {
                let ud_cls = cls(b"NSUserDefaults\0");
                if !ud_cls.is_null() {
                    let defaults = msg_id0(ud_cls, sel(b"standardUserDefaults\0"));
                    if !defaults.is_null() {
                        let key = nsstring_from_str("AppleInterfaceStyle");
                        let val = msg_id1(defaults, sel(b"stringForKey:\0"), key);
                        if let Some(style) = nsstring_to_string(val)
                            && style.eq_ignore_ascii_case("Dark")
                        {
                            color_scheme = ColorScheme::Dark;
                        }
                    }
                }
            }

            let mut contrast = ContrastLevel::Standard;
            let mut reduced_motion = false;
            let mut reduced_transparency = false;
            let ws_cls = cls(b"NSWorkspace\0");
            if !ws_cls.is_null() {
                let ws = msg_id0(ws_cls, sel(b"sharedWorkspace\0"));
                if !ws.is_null() {
                    let bool0: unsafe extern "C" fn(Id, Sel) -> i8 =
                        std::mem::transmute(objc_msgSend as *const ());
                    let resp_sel = sel(b"respondsToSelector:\0");
                    let resp_fn: unsafe extern "C" fn(Id, Sel, Sel) -> i8 =
                        std::mem::transmute(objc_msgSend as *const ());

                    let sel_contrast = sel(b"accessibilityDisplayShouldIncreaseContrast\0");
                    if resp_fn(ws, resp_sel, sel_contrast) != 0 && bool0(ws, sel_contrast) != 0 {
                        contrast = ContrastLevel::High;
                    }
                    let sel_motion = sel(b"accessibilityDisplayShouldReduceMotion\0");
                    if resp_fn(ws, resp_sel, sel_motion) != 0 && bool0(ws, sel_motion) != 0 {
                        reduced_motion = true;
                    }
                    let sel_trans = sel(b"accessibilityDisplayShouldReduceTransparency\0");
                    if resp_fn(ws, resp_sel, sel_trans) != 0 && bool0(ws, sel_trans) != 0 {
                        reduced_transparency = true;
                    }
                }
            }

            Ok(ThemeSnapshot {
                color_scheme: Some(color_scheme),
                contrast,
                reduced_motion,
                reduced_transparency,
                accent_rgba: 0x007A_FFFF,
                font_scale_x1000: 1000,
                scale_factor_x100: 200,
            })
        })
    }

    pub fn get_locale() -> Result<LocaleSnapshot, ServiceError> {
        // SAFETY: Runs inside an active autorelease pool using static NUL-terminated Foundation selectors.
        with_autorelease_pool(|| unsafe {
            let mut raw_tag = String::new();
            let locale_cls = cls(b"NSLocale\0");
            if !locale_cls.is_null() {
                let langs = msg_id0(locale_cls, sel(b"preferredLanguages\0"));
                if !langs.is_null() {
                    let first = msg_id0(langs, sel(b"firstObject\0"));
                    if let Some(tag) = nsstring_to_string(first) {
                        raw_tag = tag;
                    }
                }
            }
            if raw_tag.is_empty() {
                raw_tag = std::env::var("LC_ALL")
                    .or_else(|_| std::env::var("LANG"))
                    .unwrap_or_else(|_| "en-US".into());
            }

            let mut timezone = None;
            let tz_cls = cls(b"NSTimeZone\0");
            if !tz_cls.is_null() {
                let local_tz = msg_id0(tz_cls, sel(b"localTimeZone\0"));
                if !local_tz.is_null() {
                    let tz_name = msg_id0(local_tz, sel(b"name\0"));
                    timezone = nsstring_to_string(tz_name);
                }
            }

            Ok(parse_locale_tag(&raw_tag, timezone, false))
        })
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;

    use nve_host_services::win32_str::to_wide_nul;
    use nve_host_services::ServiceError;

    use crate::{ColorScheme, ContrastLevel, LocaleSnapshot, ThemeSnapshot};

    use super::parse_locale_tag;

    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const RRF_RT_REG_DWORD: u32 = 0x0000_0010;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            hkey: isize,
            sub_key: *const u16,
            value: *const u16,
            flags: u32,
            out_type: *mut u32,
            out_data: *mut c_void,
            out_cb: *mut u32,
        ) -> i32;
    }

    pub fn get_theme() -> Result<ThemeSnapshot, ServiceError> {
        let subkey = to_wide_nul("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let val_name = to_wide_nul("AppsUseLightTheme");
        let mut light_val: u32 = 1;
        let mut size: u32 = std::mem::size_of::<u32>() as u32;
        // SAFETY: `subkey` and `val_name` are NUL-terminated UTF-16 and `light_val`/`size` are valid out-pointers.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                val_name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut light_val as *mut u32).cast(),
                &mut size,
            )
        };
        let color_scheme = if status == 0 && light_val == 0 {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        Ok(ThemeSnapshot {
            color_scheme: Some(color_scheme),
            contrast: ContrastLevel::Standard,
            reduced_motion: false,
            reduced_transparency: false,
            accent_rgba: 0x0078_D4FF,
            font_scale_x1000: 1000,
            scale_factor_x100: 100,
        })
    }

    pub fn get_locale() -> Result<LocaleSnapshot, ServiceError> {
        let mut buf = [0u16; 85];
        // SAFETY: `buf` has capacity `buf.len()` UTF-16 code units (>= LOCALE_NAME_MAX_LENGTH).
        let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
        let raw = if len > 1 {
            String::from_utf16_lossy(&buf[..(len as usize - 1)])
        } else {
            "en-US".to_owned()
        };
        Ok(parse_locale_tag(&raw, None, false))
    }
}

#[cfg(all(
    not(any(target_os = "macos", target_os = "windows")),
    not(target_arch = "wasm32")
))]
mod platform {
    use nve_host_services::ServiceError;

    use crate::{ColorScheme, ContrastLevel, LocaleSnapshot, ThemeSnapshot};

    use super::parse_locale_tag;

    pub fn get_theme() -> Result<ThemeSnapshot, ServiceError> {
        let dark_hint = std::env::var("GTK_THEME").ok().is_some_and(|v| {
            v.as_bytes()
                .windows(4)
                .any(|w| w.eq_ignore_ascii_case(b"dark"))
        });
        Ok(ThemeSnapshot {
            color_scheme: Some(if dark_hint {
                ColorScheme::Dark
            } else {
                ColorScheme::Light
            }),
            contrast: ContrastLevel::Standard,
            reduced_motion: false,
            reduced_transparency: false,
            accent_rgba: 0x3584_E4FF,
            font_scale_x1000: 1000,
            scale_factor_x100: 100,
        })
    }

    pub fn get_locale() -> Result<LocaleSnapshot, ServiceError> {
        let raw = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LC_MESSAGES"))
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_else(|_| "en_US.UTF-8".into());
        let tz = std::env::var("TZ").ok().filter(|s| !s.is_empty());
        Ok(parse_locale_tag(&raw, tz, true))
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    use nve_host_services::ServiceError;

    use crate::{LocaleSnapshot, ThemeSnapshot};

    pub fn get_theme() -> Result<ThemeSnapshot, ServiceError> {
        Ok(ThemeSnapshot::default())
    }

    pub fn get_locale() -> Result<LocaleSnapshot, ServiceError> {
        Ok(LocaleSnapshot::default())
    }
}
