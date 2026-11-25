//! macOS native OS backend using direct Objective-C runtime, AppKit, Foundation,
//! and Security.framework FFI with zero third-party crates.

use std::ffi::{c_char, c_void, CStr};
use std::path::PathBuf;

use crate::error::SdkError;
use crate::filesystem::{FilePickerOptions, SaveFileOptions, StandardPathKind};
use crate::haptics::{ImpactStyle, NotificationHaptic};
use crate::notifications::{NotificationPermission, NotificationRequest};
use crate::system_env::{ColorScheme, ContrastLevel, LocaleSnapshot, ThemeSnapshot};

use super::parse_locale_tag;

#[path = "macos_keychain.rs"]
mod keychain;

type Id = *mut c_void;
type Sel = *mut c_void;

const NS_UTF8_STRING_ENCODING: usize = 4;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSApp: Id;
}

fn with_autorelease_pool<T>(f: impl FnOnce() -> T) -> T {
    let pool = unsafe { objc_autoreleasePoolPush() };
    let result = f();
    unsafe { objc_autoreleasePoolPop(pool) };
    result
}

unsafe fn cls(name: &[u8]) -> Id {
    debug_assert_eq!(name.last(), Some(&0));
    unsafe { objc_getClass(name.as_ptr().cast()) }
}

unsafe fn sel(name: &[u8]) -> Sel {
    debug_assert_eq!(name.last(), Some(&0));
    unsafe { sel_registerName(name.as_ptr().cast()) }
}

unsafe fn msg_id0(obj: Id, s: Sel) -> Id {
    let f: unsafe extern "C" fn(Id, Sel) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, s) }
}

unsafe fn msg_id1(obj: Id, s: Sel, a1: Id) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, Id) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, s, a1) }
}

unsafe fn msg_bool1(obj: Id, s: Sel, a1: Id) -> bool {
    let f: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, s, a1) != 0 }
}

unsafe fn msg_bool2(obj: Id, s: Sel, a1: Id, a2: Id) -> bool {
    let f: unsafe extern "C" fn(Id, Sel, Id, Id) -> i8 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, s, a1, a2) != 0 }
}

unsafe fn msg_isize0(obj: Id, s: Sel) -> isize {
    let f: unsafe extern "C" fn(Id, Sel) -> isize =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, s) }
}

unsafe fn msg_void_isize2(obj: Id, s: Sel, a1: isize, a2: usize) {
    let f: unsafe extern "C" fn(Id, Sel, isize, usize) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(obj, s, a1, a2) }
}

unsafe fn nsstring_from_str(s: &str) -> Id {
    let ns_string = unsafe { cls(b"NSString\0") };
    if ns_string.is_null() {
        return std::ptr::null_mut();
    }
    let alloc = unsafe { msg_id0(ns_string, sel(b"alloc\0")) };
    if alloc.is_null() {
        return std::ptr::null_mut();
    }
    let init_fn: unsafe extern "C" fn(Id, Sel, *const u8, usize, usize) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let obj = unsafe {
        init_fn(
            alloc,
            sel(b"initWithBytes:length:encoding:\0"),
            s.as_ptr(),
            s.len(),
            NS_UTF8_STRING_ENCODING,
        )
    };
    if obj.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { msg_id0(obj, sel(b"autorelease\0")) }
}

unsafe fn nsstring_to_string(obj: Id) -> Option<String> {
    if obj.is_null() {
        return None;
    }
    let utf8_fn: unsafe extern "C" fn(Id, Sel) -> *const c_char =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let ptr = unsafe { utf8_fn(obj, sel(b"UTF8String\0")) };
    if ptr.is_null() {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(ptr) };
    Some(cstr.to_string_lossy().into_owned())
}

// Clipboard (`NSPasteboard`)

pub fn clipboard_read_text() -> Result<Option<String>, SdkError> {
    with_autorelease_pool(|| unsafe {
        let pb_cls = cls(b"NSPasteboard\0");
        if pb_cls.is_null() {
            return Err(SdkError::Unsupported);
        }
        let pb = msg_id0(pb_cls, sel(b"generalPasteboard\0"));
        if pb.is_null() {
            return Err(SdkError::Unsupported);
        }
        let ptype = nsstring_from_str("public.utf8-plain-text");
        let str_obj = msg_id1(pb, sel(b"stringForType:\0"), ptype);
        Ok(nsstring_to_string(str_obj))
    })
}

pub fn clipboard_write_text(text: &str) -> Result<(), SdkError> {
    with_autorelease_pool(|| unsafe {
        let pb_cls = cls(b"NSPasteboard\0");
        if pb_cls.is_null() {
            return Err(SdkError::Unsupported);
        }
        let pb = msg_id0(pb_cls, sel(b"generalPasteboard\0"));
        if pb.is_null() {
            return Err(SdkError::Unsupported);
        }
        let _ = msg_isize0(pb, sel(b"clearContents\0"));
        let ptype = nsstring_from_str("public.utf8-plain-text");
        let val = nsstring_from_str(text);
        if msg_bool2(pb, sel(b"setString:forType:\0"), val, ptype) {
            Ok(())
        } else {
            Err(SdkError::IoError("NSPasteboard setString:forType: failed".into()))
        }
    })
}

pub fn clipboard_has_text() -> Result<bool, SdkError> {
    Ok(clipboard_read_text()?.is_some_and(|s| !s.is_empty()))
}

pub fn clipboard_clear() -> Result<(), SdkError> {
    with_autorelease_pool(|| unsafe {
        let pb_cls = cls(b"NSPasteboard\0");
        if pb_cls.is_null() {
            return Err(SdkError::Unsupported);
        }
        let pb = msg_id0(pb_cls, sel(b"generalPasteboard\0"));
        if pb.is_null() {
            return Err(SdkError::Unsupported);
        }
        let _ = msg_isize0(pb, sel(b"clearContents\0"));
        Ok(())
    })
}

// Standard Paths & File Picker

pub fn standard_path(kind: StandardPathKind) -> Result<String, SdkError> {
    if kind == StandardPathKind::Temp {
        return Ok(std::env::temp_dir().to_string_lossy().into_owned());
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| SdkError::IoError("HOME environment variable unset".into()))?;
    let path = match kind {
        StandardPathKind::Data => home.join("Library").join("Application Support"),
        StandardPathKind::Cache => home.join("Library").join("Caches"),
        StandardPathKind::Config => home.join("Library").join("Preferences"),
        StandardPathKind::Documents => home.join("Documents"),
        StandardPathKind::Downloads => home.join("Downloads"),
        StandardPathKind::Temp => std::env::temp_dir(),
    };
    Ok(path.to_string_lossy().into_owned())
}

pub fn pick_files(_opts: &FilePickerOptions) -> Result<Vec<String>, SdkError> {
    Err(SdkError::Unsupported)
}

pub fn save_file(_opts: &SaveFileOptions) -> Result<Option<String>, SdkError> {
    Err(SdkError::Unsupported)
}

// Secure Store (`Security.framework` Keychain)

pub fn secure_store_get(service: &str, account: &str) -> Result<Option<Vec<u8>>, SdkError> {
    keychain::get(service, account)
}

pub fn secure_store_set(service: &str, account: &str, secret: &[u8]) -> Result<(), SdkError> {
    keychain::set(service, account, secret)
}

pub fn secure_store_delete(service: &str, account: &str) -> Result<bool, SdkError> {
    keychain::delete(service, account)
}

// Linking (`NSWorkspace`)

pub fn linking_open_url(url: &str) -> Result<(), SdkError> {
    with_autorelease_pool(|| unsafe {
        let ns_url_cls = cls(b"NSURL\0");
        let ws_cls = cls(b"NSWorkspace\0");
        if ns_url_cls.is_null() || ws_cls.is_null() {
            return Err(SdkError::Unsupported);
        }
        let url_str = nsstring_from_str(url);
        let ns_url = msg_id1(ns_url_cls, sel(b"URLWithString:\0"), url_str);
        if ns_url.is_null() {
            return Err(SdkError::InvalidArg("NSURL failed to parse URL".into()));
        }
        let ws = msg_id0(ws_cls, sel(b"sharedWorkspace\0"));
        if ws.is_null() {
            return Err(SdkError::Unsupported);
        }
        if msg_bool1(ws, sel(b"openURL:\0"), ns_url) {
            Ok(())
        } else {
            Err(SdkError::IoError("NSWorkspace openURL: returned false".into()))
        }
    })
}

pub fn linking_can_open_url(url: &str) -> Result<bool, SdkError> {
    with_autorelease_pool(|| unsafe {
        let ns_url_cls = cls(b"NSURL\0");
        let ws_cls = cls(b"NSWorkspace\0");
        if ns_url_cls.is_null() || ws_cls.is_null() {
            return Ok(false);
        }
        let url_str = nsstring_from_str(url);
        let ns_url = msg_id1(ns_url_cls, sel(b"URLWithString:\0"), url_str);
        if ns_url.is_null() {
            return Ok(false);
        }
        let ws = msg_id0(ws_cls, sel(b"sharedWorkspace\0"));
        if ws.is_null() {
            return Ok(false);
        }
        let app_url = msg_id1(ws, sel(b"URLForApplicationToOpenURL:\0"), ns_url);
        Ok(!app_url.is_null())
    })
}

// System Env (`NSApp.effectiveAppearance` / `NSUserDefaults` / `NSLocale`)

pub fn system_env_get_theme() -> Result<ThemeSnapshot, SdkError> {
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
            color_scheme,
            contrast,
            reduced_motion,
            reduced_transparency,
            accent_rgba: 0x007A_FFFF,
            font_scale_x1000: 1000,
            scale_factor_x100: 200,
        })
    })
}

pub fn system_env_get_locale() -> Result<LocaleSnapshot, SdkError> {
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

// Notifications (Safe Bundle-ID Guard for `UNUserNotificationCenter`)

fn has_valid_app_bundle_identifier() -> bool {
    with_autorelease_pool(|| unsafe {
        let bundle_cls = cls(b"NSBundle\0");
        if bundle_cls.is_null() {
            return false;
        }
        let main_bundle = msg_id0(bundle_cls, sel(b"mainBundle\0"));
        if main_bundle.is_null() {
            return false;
        }
        let bundle_id = msg_id0(main_bundle, sel(b"bundleIdentifier\0"));
        nsstring_to_string(bundle_id).is_some_and(|id| !id.trim().is_empty())
    })
}

pub fn notifications_request_permission() -> Result<NotificationPermission, SdkError> {
    // Guard BEFORE touching `UNUserNotificationCenter.currentNotificationCenter`, which throws
    // an uncaught `NSInternalInconsistencyException` in unbundled CLI/test processes.
    if !has_valid_app_bundle_identifier() {
        return Err(SdkError::Unsupported);
    }
    Ok(NotificationPermission::NotDetermined)
}

pub fn notifications_show(_req: &NotificationRequest) -> Result<(), SdkError> {
    if !has_valid_app_bundle_identifier() {
        return Err(SdkError::Unsupported);
    }
    Err(SdkError::Unsupported)
}

pub fn notifications_cancel(_id: &str) -> Result<(), SdkError> {
    if !has_valid_app_bundle_identifier() {
        return Err(SdkError::Unsupported);
    }
    Ok(())
}

// Haptics (`NSHapticFeedbackManager`)

fn perform_ns_haptic(pattern: isize) -> Result<(), SdkError> {
    with_autorelease_pool(|| unsafe {
        let mgr_cls = cls(b"NSHapticFeedbackManager\0");
        if mgr_cls.is_null() {
            return Err(SdkError::Unsupported);
        }
        let performer = msg_id0(mgr_cls, sel(b"defaultPerformer\0"));
        if performer.is_null() {
            return Err(SdkError::Unsupported);
        }
        msg_void_isize2(
            performer,
            sel(b"performFeedbackPattern:performanceTime:\0"),
            pattern,
            0,
        );
        Ok(())
    })
}

pub fn haptics_impact(style: ImpactStyle) -> Result<(), SdkError> {
    let pattern = match style {
        ImpactStyle::Light | ImpactStyle::Soft => 1,
        ImpactStyle::Medium | ImpactStyle::Heavy | ImpactStyle::Rigid => 2,
    };
    perform_ns_haptic(pattern)
}

pub fn haptics_notification(_kind: NotificationHaptic) -> Result<(), SdkError> {
    perform_ns_haptic(0)
}

pub fn haptics_selection() -> Result<(), SdkError> {
    perform_ns_haptic(1)
}
