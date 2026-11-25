//! Zero-dependency native OS secure credential backends (macOS `Security.framework` Keychain,
//! Windows Credential Manager `advapi32`, Unix/WASM fallback).

use std::any::{Any, TypeId};
use std::sync::Arc;

use nve_abi::SERVICE_SECURE_STORE;
use nve_host_services::{pack_typed, HostServiceHandler, ServiceError};

use crate::{dispatch_secure_store_wire, SecureStoreBackend};

#[cfg(target_os = "macos")]
#[path = "macos_keychain.rs"]
mod platform;

/// Native OS [`HostServiceHandler`] for [`SERVICE_SECURE_STORE`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeSecureStore;

impl NativeSecureStore {
    pub const fn new() -> Self {
        Self
    }
}

impl SecureStoreBackend for NativeSecureStore {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, ServiceError> {
        platform::get(service, account)
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), ServiceError> {
        platform::set(service, account, secret)
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool, ServiceError> {
        platform::delete(service, account)
    }
}

impl HostServiceHandler for NativeSecureStore {
    fn service_id(&self) -> u32 {
        SERVICE_SECURE_STORE
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        dispatch_secure_store_wire(self, method_id, request)
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn SecureStoreBackend>(self))
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::ffi::c_void;

    use nve_host_services::win32_str::to_wide_nul;
    use nve_host_services::ServiceError;

    const CRED_TYPE_GENERIC: u32 = 1;
    const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
    const ERROR_NOT_FOUND: u32 = 1168;

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

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLastError() -> u32;
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
    }

    fn cred_target(service: &str, account: &str) -> Vec<u16> {
        to_wide_nul(&format!("nve:{service}/{account}"))
    }

    pub fn get(service: &str, account: &str) -> Result<Option<Vec<u8>>, ServiceError> {
        let target = cred_target(service, account);
        let mut out: *mut CredentialW = std::ptr::null_mut();
        let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut out) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            return if err == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(ServiceError::IoError(format!("CredReadW error {err}")))
            };
        }
        let bytes = unsafe {
            let c = &*out;
            if c.credential_blob.is_null() || c.credential_blob_size == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(c.credential_blob, c.credential_blob_size as usize)
                    .to_vec()
            }
        };
        unsafe { CredFree(out.cast()) };
        Ok(Some(bytes))
    }

    pub fn set(service: &str, account: &str, secret: &[u8]) -> Result<(), ServiceError> {
        let mut target = cred_target(service, account);
        let mut user = to_wide_nul(account);
        let mut blob = secret.to_vec();
        let blob_len = u32::try_from(blob.len())
            .map_err(|_| ServiceError::InvalidArg("secret exceeds u32::MAX".into()))?;
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
            Err(ServiceError::IoError(format!("CredWriteW error {err}")))
        }
    }

    pub fn delete(service: &str, account: &str) -> Result<bool, ServiceError> {
        let target = cred_target(service, account);
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            Ok(true)
        } else {
            let err = unsafe { GetLastError() };
            if err == ERROR_NOT_FOUND {
                Ok(false)
            } else {
                Err(ServiceError::IoError(format!("CredDeleteW error {err}")))
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use nve_host_services::ServiceError;

    pub fn get(_service: &str, _account: &str) -> Result<Option<Vec<u8>>, ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn set(_service: &str, _account: &str, _secret: &[u8]) -> Result<(), ServiceError> {
        Err(ServiceError::Unsupported)
    }

    pub fn delete(_service: &str, _account: &str) -> Result<bool, ServiceError> {
        Err(ServiceError::Unsupported)
    }
}
