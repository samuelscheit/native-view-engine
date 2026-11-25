//! macOS `Security.framework` Keychain FFI helpers (`SecItemAdd`/`SecItemCopyMatching`/
//! `SecItemUpdate`/`SecItemDelete`) with RAII `CFRelease` guards and zero third-party crates.

use std::ffi::c_void;

use crate::error::SdkError;

type CfTypeRef = *const c_void;
type CfAllocatorRef = *const c_void;
type CfMutableDictionaryRef = *mut c_void;
type CfStringRef = *const c_void;
type CfDataRef = *const c_void;
type OsStatus = i32;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const ERR_SEC_SUCCESS: OsStatus = 0;
const ERR_SEC_USER_CANCELED: OsStatus = -128;
const ERR_SEC_AUTH_FAILED: OsStatus = -25293;
const ERR_SEC_DUPLICATE_ITEM: OsStatus = -25299;
const ERR_SEC_ITEM_NOT_FOUND: OsStatus = -25300;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFAllocatorDefault: CfAllocatorRef;
    static kCFBooleanTrue: CfTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;

    fn CFDictionaryCreateMutable(
        allocator: CfAllocatorRef,
        capacity: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CfMutableDictionaryRef;
    fn CFDictionarySetValue(dict: CfMutableDictionaryRef, key: CfTypeRef, value: CfTypeRef);
    fn CFStringCreateWithBytes(
        alloc: CfAllocatorRef,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external_representation: u8,
    ) -> CfStringRef;
    fn CFDataCreate(allocator: CfAllocatorRef, bytes: *const u8, length: isize) -> CfDataRef;
    fn CFDataGetLength(data: CfDataRef) -> isize;
    fn CFDataGetBytePtr(data: CfDataRef) -> *const u8;
    fn CFRelease(cf: CfTypeRef);
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: CfTypeRef;
    static kSecClassGenericPassword: CfTypeRef;
    static kSecAttrService: CfTypeRef;
    static kSecAttrAccount: CfTypeRef;
    static kSecValueData: CfTypeRef;
    static kSecReturnData: CfTypeRef;
    static kSecMatchLimit: CfTypeRef;
    static kSecMatchLimitOne: CfTypeRef;

    fn SecItemAdd(attributes: CfTypeRef, result: *mut CfTypeRef) -> OsStatus;
    fn SecItemCopyMatching(query: CfTypeRef, result: *mut CfTypeRef) -> OsStatus;
    fn SecItemUpdate(query: CfTypeRef, attributes_to_update: CfTypeRef) -> OsStatus;
    fn SecItemDelete(query: CfTypeRef) -> OsStatus;
}

struct CfGuard(CfTypeRef);

impl CfGuard {
    fn new(ptr: CfTypeRef) -> Option<Self> {
        (!ptr.is_null()).then_some(Self(ptr))
    }

    fn as_ptr(&self) -> CfTypeRef {
        self.0
    }
}

impl Drop for CfGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cf_string(s: &str) -> Result<CfGuard, SdkError> {
    let ptr = unsafe {
        CFStringCreateWithBytes(
            kCFAllocatorDefault,
            s.as_ptr(),
            s.len() as isize,
            CF_STRING_ENCODING_UTF8,
            0,
        )
    };
    CfGuard::new(ptr).ok_or_else(|| SdkError::InternalError("CFStringCreateWithBytes failed".into()))
}

fn cf_data(bytes: &[u8]) -> Result<CfGuard, SdkError> {
    let ptr = unsafe { CFDataCreate(kCFAllocatorDefault, bytes.as_ptr(), bytes.len() as isize) };
    CfGuard::new(ptr).ok_or_else(|| SdkError::InternalError("CFDataCreate failed".into()))
}

fn new_cf_dict(cap: isize) -> Result<CfGuard, SdkError> {
    let dict = unsafe {
        CFDictionaryCreateMutable(
            kCFAllocatorDefault,
            cap,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        )
    };
    CfGuard::new(dict.cast())
        .ok_or_else(|| SdkError::InternalError("CFDictionaryCreateMutable failed".into()))
}

fn map_os_status(status: OsStatus) -> SdkError {
    match status {
        ERR_SEC_ITEM_NOT_FOUND => SdkError::NotFound,
        ERR_SEC_AUTH_FAILED => SdkError::PermissionDenied,
        ERR_SEC_USER_CANCELED => SdkError::Cancelled,
        other => SdkError::IoError(format!("Security.framework OSStatus {other}")),
    }
}

pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>, SdkError> {
    let svc_cf = cf_string(service)?;
    let acc_cf = cf_string(account)?;
    let query = new_cf_dict(5)?;
    let dict = query.as_ptr() as CfMutableDictionaryRef;
    unsafe {
        CFDictionarySetValue(dict, kSecClass, kSecClassGenericPassword);
        CFDictionarySetValue(dict, kSecAttrService, svc_cf.as_ptr());
        CFDictionarySetValue(dict, kSecAttrAccount, acc_cf.as_ptr());
        CFDictionarySetValue(dict, kSecReturnData, kCFBooleanTrue);
        CFDictionarySetValue(dict, kSecMatchLimit, kSecMatchLimitOne);
    }

    let mut out: CfTypeRef = std::ptr::null();
    let status = unsafe { SecItemCopyMatching(query.as_ptr(), &mut out) };
    if status == ERR_SEC_ITEM_NOT_FOUND {
        return Ok(None);
    }
    if status != ERR_SEC_SUCCESS {
        return Err(map_os_status(status));
    }
    let Some(data_guard) = CfGuard::new(out) else {
        return Ok(None);
    };
    let len = unsafe { CFDataGetLength(data_guard.as_ptr()) };
    if len <= 0 {
        return Ok(Some(Vec::new()));
    }
    let ptr = unsafe { CFDataGetBytePtr(data_guard.as_ptr()) };
    if ptr.is_null() {
        return Ok(Some(Vec::new()));
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec();
    Ok(Some(bytes))
}

pub fn set(service: &str, account: &str, secret: &[u8]) -> Result<(), SdkError> {
    let svc_cf = cf_string(service)?;
    let acc_cf = cf_string(account)?;
    let data_cf = cf_data(secret)?;

    let add_dict = new_cf_dict(4)?;
    let raw_add = add_dict.as_ptr() as CfMutableDictionaryRef;
    unsafe {
        CFDictionarySetValue(raw_add, kSecClass, kSecClassGenericPassword);
        CFDictionarySetValue(raw_add, kSecAttrService, svc_cf.as_ptr());
        CFDictionarySetValue(raw_add, kSecAttrAccount, acc_cf.as_ptr());
        CFDictionarySetValue(raw_add, kSecValueData, data_cf.as_ptr());
    }

    let status = unsafe { SecItemAdd(add_dict.as_ptr(), std::ptr::null_mut()) };
    if status == ERR_SEC_SUCCESS {
        return Ok(());
    }
    if status == ERR_SEC_DUPLICATE_ITEM {
        let query = new_cf_dict(3)?;
        let raw_q = query.as_ptr() as CfMutableDictionaryRef;
        let update = new_cf_dict(1)?;
        let raw_u = update.as_ptr() as CfMutableDictionaryRef;
        unsafe {
            CFDictionarySetValue(raw_q, kSecClass, kSecClassGenericPassword);
            CFDictionarySetValue(raw_q, kSecAttrService, svc_cf.as_ptr());
            CFDictionarySetValue(raw_q, kSecAttrAccount, acc_cf.as_ptr());
            CFDictionarySetValue(raw_u, kSecValueData, data_cf.as_ptr());
        }
        let upd_status = unsafe { SecItemUpdate(query.as_ptr(), update.as_ptr()) };
        return if upd_status == ERR_SEC_SUCCESS {
            Ok(())
        } else {
            Err(map_os_status(upd_status))
        };
    }
    Err(map_os_status(status))
}

pub fn delete(service: &str, account: &str) -> Result<bool, SdkError> {
    let svc_cf = cf_string(service)?;
    let acc_cf = cf_string(account)?;
    let query = new_cf_dict(3)?;
    let raw_q = query.as_ptr() as CfMutableDictionaryRef;
    unsafe {
        CFDictionarySetValue(raw_q, kSecClass, kSecClassGenericPassword);
        CFDictionarySetValue(raw_q, kSecAttrService, svc_cf.as_ptr());
        CFDictionarySetValue(raw_q, kSecAttrAccount, acc_cf.as_ptr());
    }
    let status = unsafe { SecItemDelete(query.as_ptr()) };
    match status {
        ERR_SEC_SUCCESS => Ok(true),
        ERR_SEC_ITEM_NOT_FOUND => Ok(false),
        other => Err(map_os_status(other)),
    }
}
