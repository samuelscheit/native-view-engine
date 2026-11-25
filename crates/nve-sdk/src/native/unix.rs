//! Linux & Unix fallback native OS backend using XDG Base Directory Specification,
//! POSIX locale env vars, and `xdg-open`, returning `SdkError::Unsupported` cleanly when
//! headless or desktop services are unavailable.

#[cfg(feature = "filesystem")]
use std::path::PathBuf;
#[cfg(feature = "linking")]
use std::process::Command;

use crate::error::SdkError;
#[cfg(feature = "filesystem")]
use crate::filesystem::{FilePickerOptions, SaveFileOptions, StandardPathKind};
#[cfg(feature = "haptics")]
use crate::haptics::{ImpactStyle, NotificationHaptic};
#[cfg(feature = "notifications")]
use crate::notifications::{NotificationPermission, NotificationRequest};
#[cfg(feature = "system-env")]
use crate::system_env::{ColorScheme, ContrastLevel, LocaleSnapshot, ThemeSnapshot};

#[cfg(feature = "system-env")]
use super::parse_locale_tag;

#[cfg(feature = "clipboard")]
pub fn clipboard_read_text() -> Result<Option<String>, SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "clipboard")]
pub fn clipboard_write_text(_text: &str) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "clipboard")]
pub fn clipboard_has_text() -> Result<bool, SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "clipboard")]
pub fn clipboard_clear() -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "filesystem")]
fn non_empty_env(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(feature = "filesystem")]
pub fn standard_path(kind: StandardPathKind) -> Result<String, SdkError> {
    if kind == StandardPathKind::Temp {
        return Ok(std::env::temp_dir().to_string_lossy().into_owned());
    }
    let home = non_empty_env("HOME")
        .ok_or_else(|| SdkError::IoError("HOME environment variable unset".into()))?;
    let path = match kind {
        StandardPathKind::Data => {
            non_empty_env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"))
        }
        StandardPathKind::Cache => {
            non_empty_env("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"))
        }
        StandardPathKind::Config => {
            non_empty_env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"))
        }
        StandardPathKind::Documents => {
            non_empty_env("XDG_DOCUMENTS_DIR").unwrap_or_else(|| home.join("Documents"))
        }
        StandardPathKind::Downloads => {
            non_empty_env("XDG_DOWNLOAD_DIR").unwrap_or_else(|| home.join("Downloads"))
        }
        StandardPathKind::Temp => std::env::temp_dir(),
    };
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(feature = "filesystem")]
pub fn pick_files(_opts: &FilePickerOptions) -> Result<Vec<String>, SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "filesystem")]
pub fn save_file(_opts: &SaveFileOptions) -> Result<Option<String>, SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "secure-store")]
pub fn secure_store_get(_service: &str, _account: &str) -> Result<Option<Vec<u8>>, SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "secure-store")]
pub fn secure_store_set(_service: &str, _account: &str, _secret: &[u8]) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "secure-store")]
pub fn secure_store_delete(_service: &str, _account: &str) -> Result<bool, SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "linking")]
fn has_xdg_open() -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join("xdg-open").is_file())
}

#[cfg(feature = "linking")]
pub fn linking_open_url(url: &str) -> Result<(), SdkError> {
    if !has_xdg_open() {
        return Err(SdkError::Unsupported);
    }
    let status = Command::new("xdg-open")
        .arg(url)
        .status()
        .map_err(|e| SdkError::IoError(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(SdkError::IoError(format!("xdg-open exited with {status}")))
    }
}

#[cfg(feature = "linking")]
pub fn linking_can_open_url(url: &str) -> Result<bool, SdkError> {
    Ok(has_xdg_open() && crate::linking::validate_url(url).is_ok())
}

#[cfg(feature = "system-env")]
pub fn system_env_get_theme() -> Result<ThemeSnapshot, SdkError> {
    let dark_hint = std::env::var("GTK_THEME")
        .ok()
        .is_some_and(|v| v.to_ascii_lowercase().contains("dark"));
    Ok(ThemeSnapshot {
        color_scheme: if dark_hint {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        },
        contrast: ContrastLevel::Standard,
        reduced_motion: false,
        reduced_transparency: false,
        accent_rgba: 0x3584_E4FF,
        font_scale_x1000: 1000,
        scale_factor_x100: 100,
    })
}

#[cfg(feature = "system-env")]
pub fn system_env_get_locale() -> Result<LocaleSnapshot, SdkError> {
    let raw = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "en_US.UTF-8".into());
    let tz = std::env::var("TZ").ok().filter(|s| !s.is_empty());
    Ok(parse_locale_tag(&raw, tz, true))
}

#[cfg(feature = "notifications")]
pub fn notifications_request_permission() -> Result<NotificationPermission, SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "notifications")]
pub fn notifications_show(_req: &NotificationRequest) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "notifications")]
pub fn notifications_cancel(_id: &str) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "haptics")]
pub fn haptics_impact(_style: ImpactStyle) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "haptics")]
pub fn haptics_notification(_kind: NotificationHaptic) -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}

#[cfg(feature = "haptics")]
pub fn haptics_selection() -> Result<(), SdkError> {
    Err(SdkError::Unsupported)
}
