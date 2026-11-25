//! Windows native OS backend using direct Win32 FFI (`user32`, `kernel32`, `advapi32`,
//! `shell32`, `ole32`) with zero third-party crates.

use std::ffi::c_void;
use std::path::PathBuf;

use crate::error::SdkError;
use crate::filesystem::{FilePickerOptions, SaveFileOptions, StandardPathKind};
use crate::haptics::{ImpactStyle, NotificationHaptic};
use crate::notifications::{NotificationPermission, NotificationRequest};
use crate::system_env::{ColorScheme, ContrastLevel, LocaleSnapshot, ThemeSnapshot};

use super::parse_locale_tag;

const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;
const CRED_TYPE_GENERIC: u32 = 1;
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
const ERROR_NOT_FOUND: u32 = 1168;
const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
const SW_SHOWNORMAL: i32 = 1;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
struct CredentialW {
    flags: u32,
    cred_type: u32,
    target_name: *mut u16,
    comment: *mut u16,
    last_written_low: u32,
    last_written_high: u32,
    credential_blob_size: u32,
    credential_blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut c_void,
    target_alias: *mut u16,
    user_name: *mut u16,
}

const FOLDERID_ROAMING_APP_DATA: Guid = Guid {
    data1: 0x3EB685DB,
    data2: 0x65F9,
    data3: 0x4CF6,
    data4: [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D],
};
const FOLDERID_LOCAL_APP_DATA: Guid = Guid {
    data1: 0xF1B32785,
    data2: 0x6FBA,
    data3: 0x4FCF,
    data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
};
const FOLDERID_DOCUMENTS: Guid = Guid {
    data1: 0xFDD39AD0,
    data2: 0x238F,
    data3: 0x46AF,
    data4: [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7],
};
const FOLDERID_DOWNLOADS: Guid = Guid {
    data1: 0x374DE290,
    data2: 0x123F,
    data3: 0x4565,
    data4: [0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B],
};

#[link(name = "user32")]
unsafe extern "system" {
    fn OpenClipboard(hwnd: *mut c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
    fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalLock(mem: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(mem: *mut c_void) -> i32;
    fn GlobalFree(mem: *mut c_void) -> *mut c_void;
    fn GetLastError() -> u32;
    fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn CredWriteW(cred: *const CredentialW, flags: u32) -> i32;
    fn CredReadW(
        target_name: *const u16,
        cred_type: u32,
        flags: u32,
        out_cred: *mut *mut CredentialW,
    ) -> i32;
    fn CredDeleteW(target_name: *const u16, cred_type: u32, flags: u32) -> i32;
    fn CredFree(buffer: *mut c_void);
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

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(
        rfid: *const Guid,
        flags: u32,
        token: *mut c_void,
        out_path: *mut *mut u16,
    ) -> i32;
    fn ShellExecuteW(
        hwnd: *mut c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show_cmd: i32,
    ) -> isize;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(pv: *mut c_void);
}

fn to_wide_nul(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

unsafe fn wide_ptr_to_string(ptr: *const u16) -> String {
    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf16_lossy(slice)
}

pub fn clipboard_read_text() -> Result<Option<String>, SdkError> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
            return Ok(None);
        }
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(SdkError::IoError("OpenClipboard failed".into()));
        }
        let handle = GetClipboardData(CF_UNICODETEXT);
        let result = if handle.is_null() {
            Ok(None)
        } else {
            let ptr = GlobalLock(handle) as *const u16;
            if ptr.is_null() {
                Err(SdkError::IoError("GlobalLock failed".into()))
            } else {
                let s = wide_ptr_to_string(ptr);
                GlobalUnlock(handle);
                Ok(Some(s))
            }
        };
        CloseClipboard();
        result
    }
}

pub fn clipboard_write_text(text: &str) -> Result<(), SdkError> {
    let wide = to_wide_nul(text);
    let byte_len = wide.len() * 2;
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(SdkError::IoError("OpenClipboard failed".into()));
        }
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, byte_len);
        if mem.is_null() {
            CloseClipboard();
            return Err(SdkError::IoError("GlobalAlloc failed".into()));
        }
        let dst = GlobalLock(mem) as *mut u16;
        if dst.is_null() {
            GlobalFree(mem);
            CloseClipboard();
            return Err(SdkError::IoError("GlobalLock failed".into()));
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        GlobalUnlock(mem);
        if SetClipboardData(CF_UNICODETEXT, mem).is_null() {
            GlobalFree(mem);
            CloseClipboard();
            return Err(SdkError::IoError("SetClipboardData failed".into()));
        }
        CloseClipboard();
        Ok(())
    }
}

pub fn clipboard_has_text() -> Result<bool, SdkError> {
    Ok(unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 })
}

pub fn clipboard_clear() -> Result<(), SdkError> {
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(SdkError::IoError("OpenClipboard failed".into()));
        }
        EmptyClipboard();
        CloseClipboard();
        Ok(())
    }
}

fn known_folder(guid: &Guid) -> Option<String> {
    let mut ptr: *mut u16 = std::ptr::null_mut();
    let hr = unsafe { SHGetKnownFolderPath(guid, 0, std::ptr::null_mut(), &mut ptr) };
    if hr == 0 && !ptr.is_null() {
        let s = unsafe { wide_ptr_to_string(ptr) };
        unsafe { CoTaskMemFree(ptr.cast()) };
        Some(s)
    } else {
        None
    }
}

pub fn standard_path(kind: StandardPathKind) -> Result<String, SdkError> {
    match kind {
        StandardPathKind::Temp => Ok(std::env::temp_dir().to_string_lossy().into_owned()),
        StandardPathKind::Data => known_folder(&FOLDERID_ROAMING_APP_DATA)
            .or_else(|| std::env::var("APPDATA").ok())
            .ok_or_else(|| SdkError::IoError("APPDATA unavailable".into())),
        StandardPathKind::Cache => known_folder(&FOLDERID_LOCAL_APP_DATA)
            .or_else(|| std::env::var("LOCALAPPDATA").ok())
            .ok_or_else(|| SdkError::IoError("LOCALAPPDATA unavailable".into())),
        StandardPathKind::Config => known_folder(&FOLDERID_ROAMING_APP_DATA)
            .or_else(|| std::env::var("APPDATA").ok())
            .ok_or_else(|| SdkError::IoError("APPDATA unavailable".into())),
        StandardPathKind::Documents => known_folder(&FOLDERID_DOCUMENTS)
            .or_else(|| std::env::var("USERPROFILE").ok().map(|p| PathBuf::from(p).join("Documents").to_string_lossy().into_owned()))
            .ok_or_else(|| SdkError::IoError("Documents path unavailable".into())),
        StandardPathKind::Downloads => known_folder(&FOLDERID_DOWNLOADS)
            .or_else(|| std::env::var("USERPROFILE").ok().map(|p| PathBuf::from(p).join("Downloads").to_string_lossy().into_owned()))
            .ok_or_else(|| SdkError::IoError("Downloads path unavailable".into())),
    }
}

pub fn pick_files(_opts: &FilePickerOptions) -> Result<Vec<String>, SdkError> {
    Err(SdkError::Unsupported)
}

pub fn save_file(_opts: &SaveFileOptions) -> Result<Option<String>, SdkError> {
    Err(SdkError::Unsupported)
}

fn cred_target(service: &str, account: &str) -> Vec<u16> {
    to_wide_nul(&format!("nve:{service}/{account}"))
}

pub fn secure_store_get(service: &str, account: &str) -> Result<Option<Vec<u8>>, SdkError> {
    let target = cred_target(service, account);
    let mut out: *mut CredentialW = std::ptr::null_mut();
    let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut out) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        return if err == ERROR_NOT_FOUND {
            Ok(None)
        } else {
            Err(SdkError::IoError(format!("CredReadW error {err}")))
        };
    }
    let bytes = unsafe {
        let c = &*out;
        if c.credential_blob.is_null() || c.credential_blob_size == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.credential_blob, c.credential_blob_size as usize).to_vec()
        }
    };
    unsafe { CredFree(out.cast()) };
    Ok(Some(bytes))
}

pub fn secure_store_set(service: &str, account: &str, secret: &[u8]) -> Result<(), SdkError> {
    let mut target = cred_target(service, account);
    let mut user = to_wide_nul(account);
    let mut blob = secret.to_vec();
    let blob_len = u32::try_from(blob.len())
        .map_err(|_| SdkError::InvalidArg("secret exceeds u32::MAX".into()))?;
    let cred = CredentialW {
        flags: 0,
        cred_type: CRED_TYPE_GENERIC,
        target_name: target.as_mut_ptr(),
        comment: std::ptr::null_mut(),
        last_written_low: 0,
        last_written_high: 0,
        credential_blob_size: blob_len,
        credential_blob: if blob.is_empty() {
            std::ptr::null_mut()
        } else {
            blob.as_mut_ptr()
        },
        persist: CRED_PERSIST_LOCAL_MACHINE,
        attribute_count: 0,
        attributes: std::ptr::null_mut(),
        target_alias: std::ptr::null_mut(),
        user_name: user.as_mut_ptr(),
    };
    if unsafe { CredWriteW(&cred, 0) } != 0 {
        Ok(())
    } else {
        let err = unsafe { GetLastError() };
        Err(SdkError::IoError(format!("CredWriteW error {err}")))
    }
}

pub fn secure_store_delete(service: &str, account: &str) -> Result<bool, SdkError> {
    let target = cred_target(service, account);
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
        Ok(true)
    } else {
        let err = unsafe { GetLastError() };
        if err == ERROR_NOT_FOUND {
            Ok(false)
        } else {
            Err(SdkError::IoError(format!("CredDeleteW error {err}")))
        }
    }
}

pub fn linking_open_url(url: &str) -> Result<(), SdkError> {
    let op = to_wide_nul("open");
    let target = to_wide_nul(url);
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if rc > 32 {
        Ok(())
    } else {
        Err(SdkError::IoError(format!("ShellExecuteW failed ({rc})")))
    }
}

pub fn linking_can_open_url(url: &str) -> Result<bool, SdkError> {
    Ok(crate::linking::validate_url(url).is_ok())
}

pub fn system_env_get_theme() -> Result<ThemeSnapshot, SdkError> {
    let subkey = to_wide_nul("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let val_name = to_wide_nul("AppsUseLightTheme");
    let mut light_val: u32 = 1;
    let mut size: u32 = std::mem::size_of::<u32>() as u32;
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
        color_scheme,
        contrast: ContrastLevel::Standard,
        reduced_motion: false,
        reduced_transparency: false,
        accent_rgba: 0x0078_D4FF,
        font_scale_x1000: 1000,
        scale_factor_x100: 100,
    })
}

pub fn system_env_get_locale() -> Result<LocaleSnapshot, SdkError> {
    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    let raw = if len > 1 {
        String::from_utf16_lossy(&buf[..(len as usize - 1)])
    } else {
        "en-US".to_owned()
    };
    Ok(parse_locale_tag(&raw, None, false))
}

pub fn notifications_request_permission() -> Result<NotificationPermission, SdkError> {
    Err(SdkError::Unsupported)
}

pub fn notifications_show(_req: &NotificationRequest) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

pub fn notifications_cancel(_id: &str) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

pub fn haptics_impact(_style: ImpactStyle) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

pub fn haptics_notification(_kind: NotificationHaptic) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

pub fn haptics_selection() -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}
