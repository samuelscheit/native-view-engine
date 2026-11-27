//! Live Rust provider of `realistic_hybrid.nve.rs` and `swift_edge_hybrid.nve.rs` for the Swift <->
//! Rust end-to-end harness (`hybrid_swift_runtime_tests`). The view vtable wraps `set_props_direct`
//! to record the C `NveValue` tags each prop arrives with, so the client can prove they are not
//! double-wrapped; the edge vtable replaces entries with raw ones that send what no typed Rust
//! value can (unknown enum values, bare error statuses, malformed replies).

use std::ffi::c_void;
use crypto_hybrid::*;
use nve_abi::{NveAppHandle, NveMapEntry, NveValue, NveValueTag};

#[derive(Default)]
struct LiveCrypto;

impl CryptoHybrid for LiveCrypto {
    fn version(&mut self) -> String { "nve-hybrid-1.0".to_string() }
    fn add(&mut self, a: i64, b: i64) -> i64 { a + b }
    fn blake3(&mut self, data: &[u8]) -> Vec<u8> { data.iter().map(|b| b ^ 0x5A).collect() }
    fn hash_with(&mut self, algo: HashAlgorithm, data: &[u8]) -> DigestHeader {
        DigestHeader {
            algorithm: algo,
            byte_len: data.len() as u32,
            verified: true,
            anchor: FramePoint { x: 12.5, y: -4.25 },
        }
    }
    fn verify_payload(&mut self, payload: SignedPayload) -> Result<bool, String> {
        if payload.label.is_empty() {
            return Err("missing payload label".to_string());
        }
        Ok(payload.header.algorithm == HashAlgorithm::Sha256
            && payload.tags == vec!["alpha".to_string(), "beta".to_string()]
            && payload.nonce == Some(1337))
    }
    fn find_nonce(&mut self, label: &str, hint: Option<u64>) -> Option<u64> {
        hint.map(|h| h + label.len() as u64)
    }
    fn lookup_payload(&mut self, id: u32) -> Option<SignedPayload> {
        if id == 0 { return None; }
        Some(SignedPayload {
            header: DigestHeader {
                algorithm: HashAlgorithm::Blake3,
                byte_len: id * 4,
                verified: true,
                anchor: FramePoint { x: 7.0, y: 8.0 },
            },
            label: format!("rec-{id}"),
            digest: vec![0xDE, 0xAD, 0xBE, 0xEF],
            tags: vec!["fast".to_string(), "binary".to_string()],
            nonce: Some(4242),
        })
    }
    fn batch_sign(&mut self, inputs: Vec<String>, algo: HashAlgorithm) -> Result<Vec<SignedPayload>, String> {
        if inputs.is_empty() { return Err("empty batch".to_string()); }
        Ok(inputs.into_iter().map(|label| SignedPayload {
            header: DigestHeader {
                algorithm: algo,
                byte_len: label.len() as u32,
                verified: true,
                anchor: FramePoint { x: 1.0, y: 2.0 },
            },
            label,
            digest: vec![1, 2],
            tags: vec!["signed".to_string()],
            nonce: Some(1),
        }).collect())
    }
    fn inspect_dynamic(&mut self, meta: NveHybridValue) -> NveHybridValue { meta }
    fn transpose(&mut self, cursor: Cursor) -> Cursor {
        let cols = cursor.grid.first().map_or(0, Vec::len);
        let grid = (0..cols).map(|c| cursor.grid.iter().map(|row| row[c]).collect()).collect();
        Cursor {
            mark: cursor.mark + 1,
            w: cursor.w.iter().rev().copied().collect(),
            r: cursor.r.map(|s| s.to_uppercase()),
            count: cursor.count * 2,
            tag: cursor.tag.map(|v| -v),
            grid,
        }
    }
    fn reset_cache(&mut self) -> Result<(), String> { Ok(()) }
}

#[derive(Default)]
struct LiveVideo {
    props: VideoSurfaceProps,
    seek_pos: u64,
}

impl VideoSurfaceHybrid for LiveVideo {
    fn apply_props(&mut self, props: VideoSurfaceProps) { self.props = props; }
    fn seek(&mut self, position_ms: u64) { self.seek_pos = position_ms; }
    fn capture_frame(&mut self, at_point: FramePoint) -> Result<Vec<u8>, String> {
        if at_point.x < 0.0 {
            return Err("bad \u{1F600}".to_string());
        }
        if !self.props.muted || self.seek_pos != 9876 {
            return Err("unexpected view state".to_string());
        }
        let mut frame = vec![at_point.x as u8, at_point.y as u8, self.props.source_url.len() as u8];
        if let Some(focus) = self.props.focus_point {
            frame.push(focus.x as u8);
        }
        if let Some(poster) = &self.props.poster_url {
            frame.push(poster.len() as u8);
        }
        frame.extend_from_slice(&self.props.drm_key);
        for &ch in &self.props.chapters {
            frame.push(ch as u8);
        }
        Ok(frame)
    }
    fn current_metadata(&mut self) -> NveHybridValue { self.props.metadata.clone() }
}

static CRYPTO_VTABLE: NveHybridCryptoVTable = build_crypto_vtable::<LiveCrypto>();
static LIVE_VIDEO_VTABLE: NveHybridVideoSurfaceVTable = build_video_surface_vtable::<LiveVideo>();
static VIDEO_VTABLE: NveHybridVideoSurfaceVTable = NveHybridVideoSurfaceVTable {
    set_props_direct: probe_set_props,
    ..build_video_surface_vtable::<LiveVideo>()
};

/// Pre-order `NveValue` tags of every entry of the last `set_props_direct` call, one ASCII digit
/// per node (`"762403"` is a map holding an array of i64, str, null, f64).
static PROP_SHAPES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn shape(value: &NveValue, out: &mut String) {
    out.push(char::from(b'0' + value.tag as u8));
    // SAFETY: the union member read matches `tag`; the client keeps the tree alive for the call.
    unsafe {
        if value.tag == NveValueTag::Array as u32 && value.data.array_val.len > 0 {
            let a = value.data.array_val;
            for item in std::slice::from_raw_parts(a.ptr, a.len) {
                shape(item, out);
            }
        } else if value.tag == NveValueTag::Map as u32 && value.data.map_val.len > 0 {
            let m = value.data.map_val;
            for entry in std::slice::from_raw_parts(m.ptr, m.len) {
                shape(&entry.value, out);
            }
        }
    }
}

unsafe extern "C" fn probe_set_props(handle: NveAppHandle, entries: *const NveMapEntry, count: u32) -> i32 {
    let slice: &[NveMapEntry] = if count == 0 {
        &[]
    } else {
        // SAFETY: the client passes `count` live entries.
        unsafe { std::slice::from_raw_parts(entries, count as usize) }
    };
    let shapes = slice
        .iter()
        .map(|e| {
            let mut s = String::new();
            shape(&e.value, &mut s);
            s
        })
        .collect();
    *PROP_SHAPES.lock().expect("prop shapes lock") = shapes;
    // SAFETY: forwards the caller's arguments unchanged.
    unsafe { (LIVE_VIDEO_VTABLE.set_props_direct)(handle, entries, count) }
}

/// Writes the shape of entry `index` of the last `set_props` into `out` (at most `cap` bytes) and
/// returns its length, or `usize::MAX` when there is no such entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nve_test_prop_shape(index: u32, out: *mut u8, cap: usize) -> usize {
    let shapes = PROP_SHAPES.lock().expect("prop shapes lock");
    let Some(s) = shapes.get(index as usize) else { return usize::MAX };
    let n = s.len().min(cap);
    // SAFETY: the caller provides `cap` writable bytes.
    unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), out, n) };
    s.len()
}

mod edge_provider {
    use edge_hybrid::*;
    use nve_abi::NveAppHandle;

    #[derive(Default)]
    struct LiveEdge;

    impl EdgeHybrid for LiveEdge {
        fn mix(&mut self, a: &str, b: &[u8], meta: NveHybridValue, extra: NveHybridValue) -> String {
            format!("{a}|{b:?}|{:?}|{:?}", meta.bin_bytes, extra.bin_bytes)
        }
        fn pair(&mut self, xs: Vec<i32>, ys: Vec<String>) -> Bundle {
            Bundle {
                ints: xs,
                names: ys,
                blobs: vec![vec![], vec![1, 2]],
                not_a_number: f64::NAN,
                min: i64::MIN,
                level: Level::High,
            }
        }
        fn echo_bundle(&mut self, bundle: Bundle) -> Bundle { bundle }
        fn init(&mut self, k: Keywords) -> Keywords {
            Keywords { init: k.init + 1, deinit: -k.deinit, subscript: !k.subscript }
        }
        // Replaced by the raw entries below.
        fn level(&mut self, _raw: u32) -> Level { Level::Low }
        fn fail(&mut self, _code: i32) -> Result<(), String> { Ok(()) }
        fn junk(&mut self, _kind: u32) -> Bundle { Bundle::default() }
    }

    /// Echoes the wire value, so the client sees enum values no Rust `Level` can hold.
    unsafe extern "C" fn level_raw(_: NveAppHandle, raw: u32) -> u32 { raw }

    /// Fails with status `code` and no message.
    unsafe extern "C" fn fail_raw(_: NveAppHandle, code: i32, _: *mut NveHybridOwnedBytes) -> i32 { code }

    /// A malformed `Bundle` reply per `kind`.
    unsafe extern "C" fn junk_raw(_: NveAppHandle, kind: u32) -> NveHybridOwnedBytes {
        let mut bytes = Vec::new();
        match kind {
            0 => {
                Bundle::default().encode_bin(&mut bytes);
                bytes.push(0); // one trailing byte
            }
            1 => bytes.extend_from_slice(&[7, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]), // count past payload
            2 => {
                bytes.push(0);
                for _ in 0..65 {
                    // one level deeper than `NVE_BIN_VALUE_MAX_DEPTH` allows
                    let mut outer = vec![6];
                    outer.extend_from_slice(&(bytes.len() as u32 + 4).to_le_bytes());
                    outer.extend_from_slice(&1u32.to_le_bytes());
                    outer.extend_from_slice(&bytes);
                    bytes = outer;
                }
            }
            3 => bytes.push(0), // `null` where a map is expected
            _ => {}             // nothing at all
        }
        NveHybridOwnedBytes::from_vec(bytes)
    }

    pub static EDGE_VTABLE: NveHybridEdgeVTable = NveHybridEdgeVTable {
        level: level_raw,
        fail: fail_raw,
        junk: junk_raw,
        ..build_edge_vtable::<LiveEdge>()
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn nve_get_interface(interface_id: u32) -> *const c_void {
    match interface_id {
        NVE_HYBRID_CRYPTO_INTERFACE_ID => (&CRYPTO_VTABLE as *const NveHybridCryptoVTable).cast(),
        NVE_HYBRID_VIDEO_SURFACE_INTERFACE_ID => (&VIDEO_VTABLE as *const NveHybridVideoSurfaceVTable).cast(),
        edge_hybrid::NVE_HYBRID_EDGE_INTERFACE_ID => {
            (&edge_provider::EDGE_VTABLE as *const edge_hybrid::NveHybridEdgeVTable).cast()
        }
        _ => std::ptr::null(),
    }
}
