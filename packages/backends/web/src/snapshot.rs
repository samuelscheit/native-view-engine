//! Reusable 0ms Web Instant Snapshot & State Restore primitive (`WebSnapshotPayload`,
//! `WebSnapshotStore`, binary FFI length-prefixed `NVWS1` frame envelope, `NVB1` binary state
//! envelope, `< 900` byte synchronous `<script data-nve-snapshot-boot>`, and build-time cold SSR
//! shell HTML injection).
//!
//! Works identically in native unit tests ([`MemorySnapshotStorage`]) and browser `wasm32`
//! ([`BrowserLocalStorage`] backed by `window.localStorage`).

use core::fmt::{self, Write as _};
use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

use crate::css::NVE_BASE_RESET_CSS;
use crate::event_replay::pre_hydration_replay_script_tag;
use crate::ssr::SsrOutput;

/// ASCII header magic for serialized 0ms frame snapshots in `localStorage`.
pub const SNAPSHOT_ENVELOPE_MAGIC: &str = "NVWS1";

/// ASCII header magic for base64-framed binary app-state blobs.
pub const STATE_ENVELOPE_MAGIC: &str = "NVB1";

/// Storage key prefix for cached frame snapshots (`nve:snap:<slug>:v<ver>`).
pub const SNAPSHOT_KEY_PREFIX: &str = "nve:snap:";

/// Storage key prefix for cached binary app state (`nve:state:<slug>:v<ver>`).
pub const STATE_KEY_PREFIX: &str = "nve:state:";

/// Default size guard (512 KiB) preventing `QuotaExceededError` crashes on frame snapshots.
pub const DEFAULT_MAX_SNAPSHOT_BYTES: usize = 512 * 1024;

/// Default size guard (512 KiB) preventing `QuotaExceededError` crashes on binary state blobs.
pub const DEFAULT_MAX_STATE_BYTES: usize = 512 * 1024;

/// Hard upper bound (in bytes) on the inline `<script data-nve-snapshot-boot>` tag.
pub const SNAPSHOT_BOOTSTRAP_MAX_BYTES: usize = 900;

/// Minified, deterministic, `< 900` byte synchronous bootstrap `<script data-nve-snapshot-boot>`
/// for multi-app `hosts/web-shell` (`studio` v2 / `counter` v1 on `#app`).
///
/// Executed right after `<main id="app">...</main>` BEFORE `<script type="module">` fetches WASM:
/// 1. Handles `?reset=1` by clearing stored snapshot + state keys and skipping cached paint.
/// 2. Reads `nve:snap:<app>:v<ver>` synchronously from `localStorage` (only when matching
///    `nve:state:<app>:v<ver>` exists and header/UTF-16 slice lengths validate exactly).
/// 3. Injects `<style data-nve-base>` and `<style data-nve-ssr data-nve-hashes="...">`, sets
///    `#app.innerHTML`, marks `data-nve-restored="1"`, and restores `.nve-s` scroll offsets.
/// 4. Paired immediately with [`crate::PRE_HYDRATION_REPLAY_SCRIPT_TAG`] (`<script data-nve-replay>`) so
///    pre-hydration clicks, shortcuts, and form inputs are preserved and replayed on WASM boot.
pub const WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG: &str = concat!(
    "<script data-nve-snapshot-boot>",
    "(()=>{try{let L=localStorage,D=document,a=D.getElementById('app'),",
    "q=new URL(location).searchParams,g=x=>q.get(x),",
    "p=(g('app')||'studio').toLowerCase(),v=p=='kitchen-sink'?2:3,",
    "Z=':'+p+':v'+v,k='nve:snap'+Z,s='nve:state'+Z;",
    "if(g('reset')==1)return delete L[k],delete L[s];",
    "if(!a||g('pair')||g('ssr'))return;",
    "let r=L[k],i=r?.indexOf('\\n');",
    "if(!L[s]||i<6||!/^NVWS1\\|/.test(r))return;",
    "let[u,,z,...n]=r.slice(6,i).split('|'),o=i+1,[H,B,C,M]=n.map(x=>r.slice(o,o+=+x));",
    "if(+u!=v||n.length-4||!M||o!=r.length)return;",
    "let S=(t,c,h)=>{t='data-nve-'+t;let e=D.querySelector('['+t+']')||D.head.appendChild(D.createElement('style'));",
    "e.setAttribute(t,'');h&&(e.dataset.nveHashes=h);e.textContent=c};",
    "S('base',B);S('ssr',C,H);a.innerHTML=M;a.dataset.nveRestored=1;",
    "if(z){let V=a.querySelectorAll('.nve-s');z.replace(/(\\d+):(\\d+)/g,(_,j,y)=>V[j]&&(V[j].scrollTop=y))}",
    "}catch{}})();",
    "</script>"
);

const _: () = {
    assert!(WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.len() < SNAPSHOT_BOOTSTRAP_MAX_BYTES);
};

/// Generates a deterministic, `< 900` byte synchronous `<script data-nve-snapshot-boot>` tag for
/// any single-app NVE web host mounting into `mount_id` with `(app_slug, schema_version)`.
pub fn snapshot_bootstrap_script_tag(
    mount_id: &str,
    app_slug: &str,
    schema_version: u32,
) -> String {
    let mount = sanitize_js_single_quoted(mount_id);
    let slug = sanitize_js_single_quoted(app_slug);
    format!(
        concat!(
            "<script data-nve-snapshot-boot>",
            "(()=>{{try{{let L=localStorage,D=document,a=D.getElementById('{mount}'),",
            "q=new URL(location).searchParams,g=x=>q.get(x),",
            "Z=':{slug}:v{ver}',k='nve:snap'+Z,s='nve:state'+Z;",
            "if(g('reset')==1)return delete L[k],delete L[s];",
            "if(!a||g('pair')||g('ssr'))return;",
            "let r=L[k],i=r?.indexOf('\\n');",
            "if(!L[s]||i<6||!/^NVWS1\\|/.test(r))return;",
            "let[u,,z,...n]=r.slice(6,i).split('|'),o=i+1,[H,B,C,M]=n.map(x=>r.slice(o,o+=+x));",
            "if(+u!={ver}||n.length-4||!M||o!=r.length)return;",
            "let S=(t,c,h)=>{{t='data-nve-'+t;let e=D.querySelector('['+t+']')||D.head.appendChild(D.createElement('style'));",
            "e.setAttribute(t,'');h&&(e.dataset.nveHashes=h);e.textContent=c}};",
            "S('base',B);S('ssr',C,H);a.innerHTML=M;a.dataset.nveRestored=1;",
            "if(z){{let V=a.querySelectorAll('.nve-s');z.replace(/(\\d+):(\\d+)/g,(_,j,y)=>V[j]&&(V[j].scrollTop=y))}}",
            "}}catch{{}}}})();",
            "</script>"
        ),
        mount = mount,
        slug = slug,
        ver = schema_version,
    )
}

/// Emits both `<script data-nve-snapshot-boot>` and `<script data-nve-replay>` bound to `mount_id`
/// so any NVE web app gets 0ms cached snapshot paint AND pre-hydration event capture.
pub fn snapshot_and_replay_script_tags(
    mount_id: &str,
    app_slug: &str,
    schema_version: u32,
) -> String {
    let boot = snapshot_bootstrap_script_tag(mount_id, app_slug, schema_version);
    let replay = pre_hydration_replay_script_tag(mount_id);
    format!("{boot}\n{replay}")
}

fn sanitize_js_single_quoted(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

/// Returns the versioned `localStorage` key for an app's frame snapshot.
pub fn snapshot_storage_key(app_slug: &str, schema_version: u32) -> String {
    format!("{SNAPSHOT_KEY_PREFIX}{app_slug}:v{schema_version}")
}

/// Returns the versioned `localStorage` key for an app's binary state blob.
pub fn state_storage_key(app_slug: &str, schema_version: u32) -> String {
    format!("{STATE_KEY_PREFIX}{app_slug}:v{schema_version}")
}

/// Returns `true` if a URL query string (with or without leading `?`) contains `reset=1`.
pub fn query_has_reset_param(search: &str) -> bool {
    let trimmed = search.trim_start_matches('?');
    trimmed.split('&').any(|pair| {
        pair.split_once('=')
            .is_some_and(|(k, v)| k == "reset" && v == "1")
    })
}

/// Errors returned by [`WebSnapshotStore`] write operations (never panics on quota exhaustion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    SizeLimitExceeded { bytes: usize, limit: usize },
    StorageQuotaExceeded,
    StorageUnavailable,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeLimitExceeded { bytes, limit } => {
                write!(f, "snapshot size {bytes} exceeds limit {limit}")
            }
            Self::StorageQuotaExceeded => write!(f, "browser storage quota exceeded"),
            Self::StorageUnavailable => write!(f, "browser storage unavailable"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Cached Frame-0 SSR HTML/CSS + optional `.nve-s` scroll offsets persisted for 0ms pre-WASM paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSnapshotPayload {
    pub schema_version: u32,
    pub root_hash: u64,
    pub base_css: String,
    pub style_css: String,
    pub rule_hashes: Vec<String>,
    pub html: String,
    /// Optional `(scroll_view_index, scroll_top_px)` pairs for `.nve-s` containers in DOM order.
    pub scroll_offsets: Vec<(u32, u32)>,
}

impl WebSnapshotPayload {
    /// Builds a persistable snapshot payload from an [`SsrOutput`].
    pub fn from_ssr(
        ssr: &SsrOutput,
        schema_version: u32,
        scroll_offsets: Vec<(u32, u32)>,
    ) -> Self {
        Self {
            schema_version,
            root_hash: ssr.root_hash,
            base_css: ssr.base_css.to_owned(),
            style_css: ssr.style_css.clone(),
            rule_hashes: ssr.rule_hashes.clone(),
            html: ssr.html.clone(),
            scroll_offsets,
        }
    }

    /// Converts this snapshot back into an [`SsrOutput`] for headless native hydration tests.
    pub fn to_ssr_output(&self) -> SsrOutput {
        SsrOutput {
            base_css: NVE_BASE_RESET_CSS,
            style_css: self.style_css.clone(),
            rule_hashes: self.rule_hashes.clone(),
            html: self.html.clone(),
            root_hash: self.root_hash,
        }
    }

    /// Encodes the payload into a compact, binary FFI UTF-16 length-prefixed `NVWS1` envelope:
    /// `NVWS1|<ver>|<hash16>|<scrolls>|<hashes_u16>|<base_u16>|<style_u16>|<html_u16>\n<payload>`.
    pub fn encode_envelope(&self) -> String {
        let hashes_csv = self.rule_hashes.join(",");
        let mut scrolls = String::new();
        for &(sv_idx, top) in &self.scroll_offsets {
            if top == 0 {
                continue;
            }
            if !scrolls.is_empty() {
                scrolls.push(';');
            }
            let _ = write!(&mut scrolls, "{sv_idx}:{top}");
        }
        let hl = utf16_len(&hashes_csv);
        let bl = utf16_len(&self.base_css);
        let sl = utf16_len(&self.style_css);
        let ml = utf16_len(&self.html);

        let mut out = String::with_capacity(
            64 + hashes_csv.len() + self.base_css.len() + self.style_css.len() + self.html.len(),
        );
        let _ = writeln!(
            &mut out,
            "{SNAPSHOT_ENVELOPE_MAGIC}|{}|{:016x}|{scrolls}|{hl}|{bl}|{sl}|{ml}",
            self.schema_version, self.root_hash
        );
        out.push_str(&hashes_csv);
        out.push_str(&self.base_css);
        out.push_str(&self.style_css);
        out.push_str(&self.html);
        out
    }

    /// Strictly decodes an `NVWS1` envelope, rejecting schema mismatches, corruption, or truncation.
    pub fn decode_envelope(raw: &str, expected_schema_version: u32) -> Option<Self> {
        let rest = raw.strip_prefix("NVWS1|")?;
        let nl = rest.find('\n')?;
        let header = &rest[..nl];
        let body = &rest[nl + 1..];

        let mut parts = header.split('|');
        let ver: u32 = parts.next()?.parse().ok()?;
        if ver != expected_schema_version {
            return None;
        }
        let root_hash = u64::from_str_radix(parts.next()?, 16).ok()?;
        let scrolls_raw = parts.next()?;
        let hl: usize = parts.next()?.parse().ok()?;
        let bl: usize = parts.next()?.parse().ok()?;
        let sl: usize = parts.next()?.parse().ok()?;
        let ml: usize = parts.next()?.parse().ok()?;
        if parts.next().is_some() || ml == 0 {
            return None;
        }

        let (hashes_csv, rem) = split_utf16_prefix(body, hl)?;
        let (base_css, rem) = split_utf16_prefix(rem, bl)?;
        let (style_css, rem) = split_utf16_prefix(rem, sl)?;
        let (html, trailing) = split_utf16_prefix(rem, ml)?;
        if !trailing.is_empty() || html.is_empty() {
            return None;
        }

        let mut scroll_offsets = Vec::new();
        if !scrolls_raw.is_empty() {
            for item in scrolls_raw.split(';') {
                let (idx_s, top_s) = item.split_once(':')?;
                let idx: u32 = idx_s.parse().ok()?;
                let top: u32 = top_s.parse().ok()?;
                scroll_offsets.push((idx, top));
            }
        }

        let rule_hashes = if hashes_csv.is_empty() {
            Vec::new()
        } else {
            hashes_csv
                .split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        };

        Some(Self {
            schema_version: ver,
            root_hash,
            base_css: base_css.to_owned(),
            style_css: style_css.to_owned(),
            rule_hashes,
            html: html.to_owned(),
            scroll_offsets,
        })
    }
}

#[inline]
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn split_utf16_prefix(s: &str, count: usize) -> Option<(&str, &str)> {
    if count == 0 {
        return Some(("", s));
    }
    let mut seen = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        seen += ch.len_utf16();
        if seen == count {
            let split = byte_idx + ch.len_utf8();
            return Some((&s[..split], &s[split..]));
        }
        if seen > count {
            return None;
        }
    }
    None
}

/// Encodes binary application state bytes into a versioned `NVB1`
/// base64 envelope suitable for UTF-16 `localStorage`.
pub fn encode_state_envelope(schema_version: u32, bytes: &[u8]) -> String {
    let mut out = String::with_capacity(16 + (bytes.len().div_ceil(3) * 4));
    let _ = write!(&mut out, "{STATE_ENVELOPE_MAGIC}|{schema_version}|");
    BASE64_STANDARD.encode_string(bytes, &mut out);
    out
}

/// Decodes and validates a versioned `NVB1` binary state envelope.
pub fn decode_state_envelope(raw: &str, expected_schema_version: u32) -> Option<Vec<u8>> {
    let rest = raw.strip_prefix("NVB1|")?;
    let (ver_s, b64) = rest.split_once('|')?;
    let ver: u32 = ver_s.parse().ok()?;
    if ver != expected_schema_version {
        return None;
    }
    BASE64_STANDARD.decode(b64).ok()
}

/// Abstract key-value backing store for [`WebSnapshotStore`] (`localStorage` on `wasm32`,
/// [`MemorySnapshotStorage`] in native tests).
pub trait SnapshotStorage {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&mut self, key: &str, value: &str) -> Result<(), SnapshotError>;
    fn remove(&mut self, key: &str);
    fn remove_prefix(&mut self, prefix: &str);
}

/// Deterministic in-memory [`SnapshotStorage`] with optional total byte quota for unit tests.
#[derive(Debug, Clone, Default)]
pub struct MemorySnapshotStorage {
    entries: BTreeMap<String, String>,
    quota_bytes: Option<usize>,
}

impl MemorySnapshotStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_quota_bytes(quota_bytes: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            quota_bytes: Some(quota_bytes),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn raw_entries(&self) -> &BTreeMap<String, String> {
        &self.entries
    }
}

impl SnapshotStorage for MemorySnapshotStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.entries.get(key).cloned()
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), SnapshotError> {
        if let Some(quota) = self.quota_bytes {
            let current: usize = self
                .entries
                .iter()
                .filter(|(k, _)| k.as_str() != key)
                .map(|(k, v)| k.len() + v.len())
                .sum();
            if current + key.len() + value.len() > quota {
                return Err(SnapshotError::StorageQuotaExceeded);
            }
        }
        self.entries.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }

    fn remove_prefix(&mut self, prefix: &str) {
        self.entries.retain(|k, _| !k.starts_with(prefix));
    }
}

/// Browser `window.localStorage` adapter for `wasm32`, catching `QuotaExceededError` cleanly.
#[cfg(target_arch = "wasm32")]
pub struct BrowserLocalStorage {
    storage: web_sys::Storage,
}

#[cfg(target_arch = "wasm32")]
impl BrowserLocalStorage {
    pub fn open() -> Option<Self> {
        let storage = web_sys::window()?.local_storage().ok()??;
        Some(Self { storage })
    }
}

#[cfg(target_arch = "wasm32")]
impl SnapshotStorage for BrowserLocalStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.storage.get_item(key).ok().flatten()
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), SnapshotError> {
        self.storage
            .set_item(key, value)
            .map_err(|_| SnapshotError::StorageQuotaExceeded)
    }

    fn remove(&mut self, key: &str) {
        let _ = self.storage.remove_item(key);
    }

    fn remove_prefix(&mut self, prefix: &str) {
        let len = self.storage.length().unwrap_or(0);
        let mut to_remove = Vec::new();
        for i in 0..len {
            if let Ok(Some(k)) = self.storage.key(i)
                && k.starts_with(prefix)
            {
                to_remove.push(k);
            }
        }
        for k in to_remove {
            let _ = self.storage.remove_item(&k);
        }
    }
}

/// Versioned, quota-safe 0ms snapshot + binary state store keyed by `(app_slug, schema_version)`.
pub struct WebSnapshotStore<S: SnapshotStorage> {
    app_slug: String,
    schema_version: u32,
    max_snapshot_bytes: usize,
    max_state_bytes: usize,
    storage: S,
}

impl<S: SnapshotStorage> WebSnapshotStore<S> {
    pub fn new(storage: S, app_slug: impl Into<String>, schema_version: u32) -> Self {
        Self {
            app_slug: app_slug.into(),
            schema_version,
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
            max_state_bytes: DEFAULT_MAX_STATE_BYTES,
            storage,
        }
    }

    pub fn with_limits(mut self, max_snapshot_bytes: usize, max_state_bytes: usize) -> Self {
        self.max_snapshot_bytes = max_snapshot_bytes;
        self.max_state_bytes = max_state_bytes;
        self
    }

    #[inline]
    pub fn app_slug(&self) -> &str {
        &self.app_slug
    }

    #[inline]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[inline]
    pub fn storage(&self) -> &S {
        &self.storage
    }

    #[inline]
    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }

    pub fn frame_key(&self) -> String {
        snapshot_storage_key(&self.app_slug, self.schema_version)
    }

    pub fn state_key(&self) -> String {
        state_storage_key(&self.app_slug, self.schema_version)
    }

    /// Persists a Frame-0 [`WebSnapshotPayload`] with size and quota guards.
    pub fn save_frame(&mut self, payload: &WebSnapshotPayload) -> Result<usize, SnapshotError> {
        let encoded = payload.encode_envelope();
        if encoded.len() > self.max_snapshot_bytes {
            return Err(SnapshotError::SizeLimitExceeded {
                bytes: encoded.len(),
                limit: self.max_snapshot_bytes,
            });
        }
        let key = self.frame_key();
        self.storage.set(&key, &encoded)?;
        Ok(encoded.len())
    }

    /// Loads and validates the stored Frame-0 [`WebSnapshotPayload`]; returns `None` on missing,
    /// corrupt, truncated, or schema-mismatched data.
    pub fn load_frame(&self) -> Option<WebSnapshotPayload> {
        let raw = self.storage.get(&self.frame_key())?;
        WebSnapshotPayload::decode_envelope(&raw, self.schema_version)
    }

    /// Persists binary application state bytes wrapped in a versioned `NVB1` base64 envelope.
    pub fn save_state(&mut self, state_bytes: &[u8]) -> Result<usize, SnapshotError> {
        let encoded = encode_state_envelope(self.schema_version, state_bytes);
        if encoded.len() > self.max_state_bytes {
            return Err(SnapshotError::SizeLimitExceeded {
                bytes: encoded.len(),
                limit: self.max_state_bytes,
            });
        }
        let key = self.state_key();
        self.storage.set(&key, &encoded)?;
        Ok(encoded.len())
    }

    /// Loads and validates persisted binary state bytes for `(app_slug, schema_version)`.
    pub fn load_state(&self) -> Option<Vec<u8>> {
        let raw = self.storage.get(&self.state_key())?;
        decode_state_envelope(&raw, self.schema_version)
    }

    /// Atomically persists both binary state and Frame-0 HTML/CSS snapshot.
    ///
    /// Writes state first and removes any stale frame snapshot if either write fails, guaranteeing
    /// Layer 1 never paints an HTML frame whose binary state failed to persist.
    pub fn save_all(
        &mut self,
        payload: &WebSnapshotPayload,
        state_bytes: &[u8],
    ) -> Result<(usize, usize), SnapshotError> {
        let state_len = match self.save_state(state_bytes) {
            Ok(len) => len,
            Err(err) => {
                self.storage.remove(&self.frame_key());
                return Err(err);
            }
        };
        match self.save_frame(payload) {
            Ok(frame_len) => Ok((frame_len, state_len)),
            Err(err) => {
                self.storage.remove(&self.frame_key());
                Err(err)
            }
        }
    }

    /// Clears every snapshot and state entry for `app_slug` across all schema versions.
    pub fn clear(&mut self) {
        let snap_prefix = format!("{SNAPSHOT_KEY_PREFIX}{}:", self.app_slug);
        let state_prefix = format!("{STATE_KEY_PREFIX}{}:", self.app_slug);
        self.storage.remove_prefix(&snap_prefix);
        self.storage.remove_prefix(&state_prefix);
    }

    /// If `search` contains `?reset=1`, clears stored snapshot + state entries and returns `true`.
    pub fn handle_url_query(&mut self, search: &str) -> bool {
        if query_has_reset_param(search) {
            self.clear();
            true
        } else {
            false
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl WebSnapshotStore<BrowserLocalStorage> {
    /// Opens a browser `localStorage`-backed store for `(app_slug, schema_version)`.
    pub fn browser(app_slug: impl Into<String>, schema_version: u32) -> Option<Self> {
        let storage = BrowserLocalStorage::open()?;
        Some(Self::new(storage, app_slug, schema_version))
    }
}
