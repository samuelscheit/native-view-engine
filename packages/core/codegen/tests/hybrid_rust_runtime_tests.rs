//! Real-`rustc` compile-and-run tests of generated Rust hybrid bindings over the static
//! `nve_abi::hybrid` runtime: vtable trampolines (success, error, invalid-argument, null-handle
//! paths), the `NveBinValue` codec of every type, direct props, and direct events.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_codegen::{HybridTarget, generate_hybrid_target, parse_hybrid_rust_spec};

const REALISTIC_HYBRID_SPEC: &str = include_str!("fixtures/realistic_hybrid.nve.rs");

/// Every codec shape (POD with a fixed-size array, nested `Option` / `Vec`, `Vec<Bytes>`, `Value`,
/// narrow integers, enums), plus parameter / field / event names that shadow the trampolines' and
/// emitters' own bindings (`handle`, `out_ok`, `out_err`, `instance`, `entries`, `props`,
/// `surface_id`, `sink`, `value`, `fields`, `m`).
const EDGE_HYBRID_SPEC: &str = r#"
use nve_abi::NveValue;

#[nve_hybrid_enum]
#[repr(u32)]
pub enum Tone {
    Low = 0,
    High = 9,
}

#[nve_hybrid_struct]
pub struct Rgba {
    pub channels: [f32; 4],
    pub tone: Tone,
}

#[nve_hybrid_struct]
pub struct Node {
    pub name: String,
    pub depth: u16,
    pub small: u8,
    pub size: usize,
    pub weight: f64,
    pub color: Rgba,
    pub accent: Option<Rgba>,
    pub children: Vec<Option<Vec<i32>>>,
    pub blobs: Vec<Vec<u8>>,
    pub extra: NveValue,
    pub m: Option<u64>,
    pub fields: Vec<Tone>,
}

#[nve_hybrid_module(name = "Shadow")]
pub trait ShadowSpec {
    fn shadow(&self, handle: i64, out_err: &str, out_ok: &[u8], instance: Node, entries: u32) -> Result<Node, String>;
    fn pick(&self, tone: Option<Tone>, color: Option<Rgba>) -> Option<Tone>;
    fn echo_value(&self, value: NveValue) -> Result<NveValue, String>;
    fn tones(&self, tones: Vec<Tone>) -> Vec<Tone>;
    fn strict(&self, tone: Tone) -> u32;
    fn fail_always(&self, nodes: Vec<Node>) -> Result<(), String>;
}

#[nve_hybrid_view(name = "Canvas")]
pub trait CanvasSpec {
    #[prop]
    fn node(&self) -> Node;
    #[prop(default = None)]
    fn label(&self) -> Option<String>;
    #[prop]
    fn layers(&self) -> Vec<Node>;
    #[prop]
    fn extra(&self) -> NveValue;
    #[event]
    fn on_change(&self, surface_id: u32, sink: Node, values: Vec<Option<String>>, extra: NveValue, maybe: Option<Rgba>, tone: Tone);
    #[event]
    fn on_ping(&self);
    #[method]
    fn snapshot(&self, props: u32) -> Node;
}

#[nve_hybrid_view(name = "Blank")]
pub trait BlankSpec {
    #[event]
    fn on_tap(&self, x: f32);
}
"#;

const EDGE_MAIN_RS: &str = r#"
use edge_hybrid::*;
use nve_abi::hybrid::{decode_bin_arg, encode_to_vec};
use nve_abi::{encode_bin_value_to_vec, NveBufferSlice, NveMapEntry, NveMapRef, NveStrSlice, NveValue, NveValueTag};

#[derive(Default)]
struct Shadow;

impl ShadowHybrid for Shadow {
    fn shadow(&mut self, handle: i64, out_err: &str, out_ok: &[u8], instance: Node, entries: u32) -> Result<Node, String> {
        if handle < 0 {
            return Err(format!("negative handle {handle} 😀"));
        }
        let mut node = instance;
        node.name = format!("{}|{out_err}", node.name);
        node.depth += entries as u16;
        node.blobs.push(out_ok.to_vec());
        node.children.push(Some(vec![handle as i32]));
        Ok(node)
    }
    fn pick(&mut self, tone: Option<Tone>, color: Option<Rgba>) -> Option<Tone> {
        color.map(|c| c.tone).or(tone)
    }
    fn echo_value(&mut self, value: NveHybridValue) -> Result<NveHybridValue, String> {
        Ok(value)
    }
    fn tones(&mut self, tones: Vec<Tone>) -> Vec<Tone> {
        tones.into_iter().rev().collect()
    }
    fn strict(&mut self, tone: Tone) -> u32 {
        tone.to_u32() + 1
    }
    fn fail_always(&mut self, nodes: Vec<Node>) -> Result<(), String> {
        Err(format!("{} nodes", nodes.len()))
    }
}

#[derive(Default)]
struct Canvas {
    props: CanvasProps,
}

impl CanvasHybrid for Canvas {
    fn apply_props(&mut self, props: CanvasProps) {
        self.props = props;
    }
    fn snapshot(&mut self, props: u32) -> Node {
        let mut node = self.props.node.clone();
        node.size = props as usize;
        node
    }
}

#[derive(Default)]
struct Blank;
impl BlankHybrid for Blank {
    fn apply_props(&mut self, _props: BlankProps) {}
}

fn sample_extra() -> NveHybridValue {
    let blob = [7u8, 0, 255];
    let items = [NveValue::i64(-3), NveValue::from_bytes(&blob), NveValue::null()];
    let entries = [NveMapEntry::new("k", NveValue::array(&items)), NveMapEntry::new("s", NveValue::from_str("ü"))];
    let root = NveValue::map(&entries);
    NveHybridValue::from_ffi_ref(unsafe { root.as_ref() }).unwrap()
}

fn sample_node() -> Node {
    Node {
        name: "root".to_owned(),
        depth: u16::MAX - 10,
        small: u8::MAX,
        size: 1 << 40,
        weight: -1.25e300,
        color: Rgba { channels: [0.0, 0.5, 1.0, -0.0], tone: Tone::High },
        accent: Some(Rgba { channels: [1.0; 4], tone: Tone::Low }),
        children: vec![None, Some(vec![]), Some(vec![i32::MIN, i32::MAX])],
        blobs: vec![vec![], vec![1, 2, 3]],
        extra: sample_extra(),
        m: Some(u64::MAX),
        fields: vec![Tone::High, Tone::Low],
    }
}

unsafe fn take(bytes: NveHybridOwnedBytes) -> Vec<u8> {
    let out = unsafe { bytes.as_slice() }.to_vec();
    unsafe { nve_abi::hybrid::nve_hybrid_free_bytes(bytes) };
    out
}

unsafe fn take_str(bytes: NveHybridOwnedBytes) -> String {
    String::from_utf8(unsafe { take(bytes) }).unwrap()
}

fn main() {
    let node = sample_node();
    let node_bin = encode_to_vec(&node).unwrap();
    assert_eq!(decode_bin_arg::<Node>(&node_bin), Ok(node.clone()));

    let vt = build_shadow_vtable::<Shadow>();
    assert_eq!(vt.abi_version, NVE_HYBRID_ABI_VERSION);
    unsafe {
        let h = (vt.create)();
        let mut out_ok = NveHybridOwnedBytes::EMPTY;
        let mut out_err = NveHybridOwnedBytes::EMPTY;
        let blob = [9u8, 8];

        // Ok path: every shadowing parameter reaches the implementation intact.
        let st = (vt.shadow)(h, 5, NveStrSlice::from_str("tail"), NveBufferSlice::from_slice(&blob), NveBufferSlice::from_slice(&node_bin), 3, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::Ok as i32);
        assert!(out_err.ptr.is_null());
        let back = decode_bin_arg::<Node>(&take(std::mem::replace(&mut out_ok, NveHybridOwnedBytes::EMPTY))).unwrap();
        let mut expected = node.clone();
        expected.name = "root|tail".to_owned();
        expected.depth += 3;
        expected.blobs.push(blob.to_vec());
        expected.children.push(Some(vec![5]));
        assert_eq!(back, expected);

        // Err path keeps UTF-8 messages.
        let st = (vt.shadow)(h, -1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::from_slice(&node_bin), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::Err as i32);
        assert_eq!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)), "negative handle -1 😀");

        // Malformed arguments never reach the implementation.
        let garbage = [0xFFu8, 1];
        let st = (vt.shadow)(h, 1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::from_slice(&garbage), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::InvalidArg as i32);
        assert!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)).starts_with("invalid hybrid argument 'instance': "));
        let bad_utf8 = [0xC3u8];
        let bad_str = NveStrSlice { ptr: bad_utf8.as_ptr(), len: 1 };
        let st = (vt.shadow)(h, 1, bad_str, NveBufferSlice::EMPTY, NveBufferSlice::from_slice(&node_bin), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::InvalidArg as i32);
        assert!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)).starts_with("invalid hybrid argument 'out_err': "));
        let wrong_shape = encode_to_vec(&vec![1i32]).unwrap();
        let st = (vt.shadow)(h, 1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::from_slice(&wrong_shape), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::InvalidArg as i32);
        let _ = take(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY));
        let st = (vt.shadow)(std::ptr::null_mut(), 1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::EMPTY, 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::NullHandle as i32);
        assert_eq!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)), "null hybrid handle");

        // POD options: an unknown discriminant is an invalid argument (zero value), not a cast.
        let none_rgba = NveHybridOptRgba::default();
        let r = (vt.pick)(h, NveHybridOptTone { is_some: true, value: 9 }, none_rgba);
        assert!(r.is_some && r.value == 9);
        let r = (vt.pick)(h, NveHybridOptTone { is_some: true, value: 5 }, none_rgba);
        assert!(!r.is_some);
        let some_rgba = NveHybridOptRgba { is_some: true, value: Rgba { channels: [0.0; 4], tone: Tone::Low } };
        let r = (vt.pick)(h, NveHybridOptTone::default(), some_rgba);
        assert!(r.is_some && r.value == 0);
        assert_eq!((vt.strict)(h, 9), 10);
        assert_eq!((vt.strict)(h, 3), 0);
        assert_eq!((vt.strict)(std::ptr::null_mut(), 9), 0);

        // `Value` parameters arrive as C trees and return as one `NveBinValue`, losslessly.
        let extra = sample_extra();
        let tree = extra.to_tree();
        let st = (vt.echo_value)(h, *tree.root(), &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::Ok as i32);
        let echoed = take(std::mem::replace(&mut out_ok, NveHybridOwnedBytes::EMPTY));
        assert_eq!(echoed, extra.as_bytes());
        assert_eq!(echoed, encode_bin_value_to_vec(tree.as_value_ref()).unwrap());

        let tones = encode_to_vec(&vec![Tone::Low, Tone::High, Tone::High]).unwrap();
        let reversed = take((vt.tones)(h, NveBufferSlice::from_slice(&tones)));
        assert_eq!(decode_bin_arg::<Vec<Tone>>(&reversed), Ok(vec![Tone::High, Tone::High, Tone::Low]));
        let bad_tones = encode_to_vec(&vec![4u32]).unwrap();
        assert!((vt.tones)(h, NveBufferSlice::from_slice(&bad_tones)).ptr.is_null());

        let nodes = encode_to_vec(&vec![node.clone(), Node::default()]).unwrap();
        let st = (vt.fail_always)(h, NveBufferSlice::from_slice(&nodes), &mut out_err);
        assert_eq!(st, NveHybridStatus::Err as i32);
        assert_eq!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)), "2 nodes");
        (vt.destroy)(h);
        (vt.destroy)(std::ptr::null_mut());
    }

    // Direct props: structs and `Value` as C trees, a `Vec` as `NveBinValue` bytes.
    let cvt = build_canvas_vtable::<Canvas>();
    unsafe {
        let h = (cvt.create)();
        let node_value = NveHybridValue::encode(&node).unwrap();
        let node_tree = node_value.to_tree();
        let layers_bin = encode_to_vec(&vec![Node::default(), node.clone()]).unwrap();
        let extra = sample_extra();
        let extra_tree = extra.to_tree();
        let entries = [
            NveMapEntry::new("node", *node_tree.root()),
            NveMapEntry::new("label", NveValue::from_str("hi")),
            NveMapEntry::new("layers", NveValue::from_bytes(&layers_bin)),
            NveMapEntry::new("extra", *extra_tree.root()),
            NveMapEntry::new("unknown", NveValue::bool(true)),
        ];
        assert_eq!((cvt.set_props_direct)(h, entries.as_ptr(), entries.len() as u32), NveHybridStatus::Ok as i32);
        let props = CanvasProps::decode_props(NveMapRef::from_slice(&entries)).unwrap();
        assert_eq!(props.node, node);
        assert_eq!(props.label.as_deref(), Some("hi"));
        assert_eq!(props.layers, vec![Node::default(), node.clone()]);
        assert_eq!(props.extra, extra);
        assert_eq!(props.extra.as_view().tag(), NveValueTag::Map);

        let snap = take((cvt.snapshot)(h, 77));
        let mut expected = node.clone();
        expected.size = 77;
        assert_eq!(decode_bin_arg::<Node>(&snap), Ok(expected));

        // Defaults: absent props keep them (a `Value` defaults to null, never to empty bytes).
        assert_eq!((cvt.set_props_direct)(h, std::ptr::null(), 0), NveHybridStatus::Ok as i32);
        let defaults = CanvasProps::default();
        assert_eq!(defaults.extra.as_bytes(), &[NveValueTag::Null as u8]);
        assert_eq!(defaults.label, None);

        let wrong = [NveMapEntry::new("node", NveValue::i64(1))];
        assert_eq!((cvt.set_props_direct)(h, wrong.as_ptr(), 1), NveHybridStatus::InvalidArg as i32);
        assert_eq!((cvt.set_props_direct)(h, std::ptr::null(), 3), NveHybridStatus::InvalidArg as i32);
        assert_eq!((cvt.set_props_direct)(std::ptr::null_mut(), entries.as_ptr(), 1), NveHybridStatus::NullHandle as i32);
        (cvt.destroy)(h);
    }

    // Direct events: composite parameters as C trees (never wrapped in `Bytes`).
    let mut seen = 0;
    let values = vec![Some("a".to_owned()), None];
    emit_canvas_on_change(1, 2, 3, 44, node.clone(), values.clone(), sample_extra(), None, Tone::High, |d| {
        seen += 1;
        assert_eq!((d.surface_id, d.node_id, d.timestamp_us), (1, 2, 3));
        assert_eq!(unsafe { d.name.as_str() }, Ok("on_change"));
        let payload = unsafe { d.value.as_ref() };
        assert_eq!(payload.get("surface_id").and_then(|v| v.as_i64()), Some(44));
        let sink = payload.get("sink").unwrap();
        assert!(sink.as_map().is_some());
        assert_eq!(<Node as nve_abi::hybrid::HybridCodec>::decode_value_ref(sink), Ok(node.clone()));
        let vals = payload.get("values").and_then(|v| v.as_array()).unwrap();
        assert_eq!(vals.get(0).and_then(|v| v.as_str()), Some("a"));
        assert!(vals.get(1).unwrap().is_null());
        let extra = payload.get("extra").unwrap();
        assert!(extra.as_map().is_some());
        assert_eq!(NveHybridValue::from_ffi_ref(extra), Ok(sample_extra()));
        assert!(payload.get("maybe").unwrap().is_null());
        assert_eq!(payload.get("tone").and_then(|v| v.as_i64()), Some(9));
    })
    .unwrap();
    CanvasOnPingEvent {}.emit_direct(0, 0, 0, |d| {
        seen += 1;
        assert_eq!(unsafe { d.value.as_ref() }.as_map().map(|m| m.len()), Some(0));
    })
    .unwrap();
    emit_blank_on_tap(0, 0, 0, 1.5, |d| {
        seen += 1;
        assert_eq!(unsafe { d.value.as_ref() }.get("x").and_then(|v| v.as_f64()), Some(1.5));
    })
    .unwrap();
    assert_eq!(seen, 3);
    let bvt = build_blank_vtable::<Blank>();
    unsafe {
        let h = (bvt.create)();
        assert_eq!((bvt.set_props_direct)(h, std::ptr::null(), 0), NveHybridStatus::Ok as i32);
        (bvt.destroy)(h);
    }
    println!("edge hybrid e2e ok");
}
"#;

const REALISTIC_MAIN_RS: &str = r#"
use crypto_hybrid::*;
use nve_abi::hybrid::{decode_bin_arg, encode_to_vec};
use nve_abi::{NveBufferSlice, NveMapEntry, NveMapRef, NveStrSlice, NveValue};

#[derive(Default)]
struct MyCrypto;

impl CryptoHybrid for MyCrypto {
    fn version(&mut self) -> String { "1.2.3".to_string() }
    fn add(&mut self, a: i64, b: i64) -> i64 { a + b }
    fn blake3(&mut self, data: &[u8]) -> Vec<u8> { data.iter().rev().copied().collect() }
    fn hash_with(&mut self, algo: HashAlgorithm, data: &[u8]) -> DigestHeader {
        DigestHeader { algorithm: algo, byte_len: data.len() as u32, verified: true, anchor: FramePoint { x: 3.5, y: -2.0 } }
    }
    fn verify_payload(&mut self, payload: SignedPayload) -> Result<bool, String> {
        if payload.label.is_empty() { return Err("empty label".to_string()); }
        Ok(payload.header.verified && payload.nonce == Some(77))
    }
    fn find_nonce(&mut self, _label: &str, hint: Option<u64>) -> Option<u64> { hint.map(|h| h + 1) }
    fn lookup_payload(&mut self, id: u32) -> Option<SignedPayload> {
        (id != 0).then(|| SignedPayload { label: format!("item-{id}"), nonce: Some(99), ..Default::default() })
    }
    fn batch_sign(&mut self, inputs: Vec<String>, algo: HashAlgorithm) -> Result<Vec<SignedPayload>, String> {
        if inputs.is_empty() { return Err("empty batch".to_string()); }
        Ok(inputs.into_iter().map(|label| SignedPayload {
            header: DigestHeader { algorithm: algo, ..Default::default() },
            label,
            ..Default::default()
        }).collect())
    }
    fn inspect_dynamic(&mut self, meta: NveHybridValue) -> NveHybridValue { meta }
    fn transpose(&mut self, cursor: Cursor) -> Cursor {
        let cols = cursor.grid.first().map_or(0, Vec::len);
        let grid = (0..cols).map(|c| cursor.grid.iter().map(|row| row[c]).collect()).collect();
        Cursor { mark: cursor.mark + 1, w: cursor.w.iter().rev().copied().collect(), r: cursor.r.map(|s| s.to_uppercase()), count: cursor.count * 2, tag: cursor.tag.map(|v| -v), grid }
    }
    fn reset_cache(&mut self) -> Result<(), String> { Ok(()) }
}

#[derive(Default)]
struct MyVideo {
    props: VideoSurfaceProps,
}

impl VideoSurfaceHybrid for MyVideo {
    fn apply_props(&mut self, props: VideoSurfaceProps) { self.props = props; }
    fn seek(&mut self, _position_ms: u64) {}
    fn capture_frame(&mut self, at_point: FramePoint) -> Result<Vec<u8>, String> {
        if at_point.x < 0.0 { return Err("negative x".to_string()); }
        Ok(vec![0xDE, 0xAD])
    }
    fn current_metadata(&mut self) -> NveHybridValue { self.props.metadata.clone() }
}

unsafe fn take(bytes: NveHybridOwnedBytes) -> Vec<u8> {
    let out = unsafe { bytes.as_slice() }.to_vec();
    unsafe { nve_abi::hybrid::nve_hybrid_free_bytes(bytes) };
    out
}

fn main() {
    let vt = build_crypto_vtable::<MyCrypto>();
    assert_eq!(vt.abi_version, NVE_HYBRID_ABI_VERSION);
    assert_eq!(vt.interface_id, NVE_HYBRID_CRYPTO_INTERFACE_ID);
    assert_eq!(vt.schema_hash, NVE_HYBRID_CRYPTO_SCHEMA_HASH);

    unsafe {
        let h = (vt.create)();
        assert_eq!((vt.add)(h, 19, 23), 42);
        assert_eq!(take((vt.version)(h)), b"1.2.3");
        assert_eq!(take((vt.blake3)(h, NveBufferSlice::from_slice(&[1, 2]))), vec![2, 1]);

        let hdr = (vt.hash_with)(h, HashAlgorithm::Sha512.to_u32(), NveBufferSlice::from_slice(&[1, 2, 3, 4]));
        assert_eq!((hdr.algorithm, hdr.byte_len, hdr.verified, hdr.anchor.x), (HashAlgorithm::Sha512, 4, true, 3.5));

        let some = (vt.find_nonce)(h, NveStrSlice::from_str("k"), NveHybridOptU64 { is_some: true, value: 41 });
        assert!(some.is_some && some.value == 42);
        assert!(!(vt.find_nonce)(h, NveStrSlice::from_str("k"), NveHybridOptU64::default()).is_some);

        let payload = SignedPayload { header: hdr, label: "hello".to_string(), digest: vec![9, 8], tags: vec!["a".to_string()], nonce: Some(77) };
        let payload_bin = encode_to_vec(&payload).unwrap();
        let mut ok_out = false;
        let mut err_out = NveHybridOwnedBytes::EMPTY;
        assert_eq!((vt.verify_payload)(h, NveBufferSlice::from_slice(&payload_bin), &mut ok_out, &mut err_out), NveHybridStatus::Ok as i32);
        assert!(ok_out);
        let bad_bin = encode_to_vec(&SignedPayload::default()).unwrap();
        assert_eq!((vt.verify_payload)(h, NveBufferSlice::from_slice(&bad_bin), &mut ok_out, &mut err_out), NveHybridStatus::Err as i32);
        assert_eq!(take(std::mem::replace(&mut err_out, NveHybridOwnedBytes::EMPTY)), b"empty label");

        let found = take((vt.lookup_payload)(h, 3));
        assert_eq!(decode_bin_arg::<Option<SignedPayload>>(&found).unwrap().map(|p| p.label), Some("item-3".to_string()));
        assert_eq!(decode_bin_arg::<Option<SignedPayload>>(&take((vt.lookup_payload)(h, 0))), Ok(None));

        let inputs = encode_to_vec(&vec!["x".to_string(), "yz".to_string()]).unwrap();
        let mut batch_out = NveHybridOwnedBytes::EMPTY;
        assert_eq!((vt.batch_sign)(h, NveBufferSlice::from_slice(&inputs), 1, &mut batch_out, &mut err_out), NveHybridStatus::Ok as i32);
        let batch = decode_bin_arg::<Vec<SignedPayload>>(&take(std::mem::replace(&mut batch_out, NveHybridOwnedBytes::EMPTY))).unwrap();
        assert_eq!(batch.iter().map(|p| (p.label.as_str(), p.header.algorithm)).collect::<Vec<_>>(), [("x", HashAlgorithm::Sha256), ("yz", HashAlgorithm::Sha256)]);
        assert_eq!((vt.batch_sign)(h, NveBufferSlice::from_slice(&inputs), 99, &mut batch_out, &mut err_out), NveHybridStatus::InvalidArg as i32);
        assert!(String::from_utf8(take(std::mem::replace(&mut err_out, NveHybridOwnedBytes::EMPTY))).unwrap().contains("'algo'"));

        let cursor = Cursor { mark: 1, w: vec![1, 2], r: Some("ab".into()), count: 3, tag: None, grid: vec![vec![1, 2], vec![3, 4]] };
        let flipped = take((vt.transpose)(h, NveBufferSlice::from_slice(&encode_to_vec(&cursor).unwrap())));
        assert_eq!(decode_bin_arg::<Cursor>(&flipped).unwrap().grid, vec![vec![1, 3], vec![2, 4]]);
        assert_eq!((vt.reset_cache)(h, &mut err_out), NveHybridStatus::Ok as i32);
        (vt.destroy)(h);

        // View props, including a `Value` prop that must not be double-wrapped into `Bytes`.
        let vvt = build_video_surface_vtable::<MyVideo>();
        let vh = (vvt.create)();
        let meta_items = [NveValue::i64(1), NveValue::from_str("two")];
        let chapters = encode_to_vec(&vec![10i32, 20]).unwrap();
        let entries = [
            NveMapEntry::new("source_url", NveValue::from_str("https://nve.dev/v.mp4")),
            NveMapEntry::new("muted", NveValue::bool(true)),
            NveMapEntry::new("quality", NveValue::i64(VideoQuality::Uhd4k.to_u32() as i64)),
            NveMapEntry::new("drm_key", NveValue::from_bytes(&[5, 6])),
            NveMapEntry::new("chapters", NveValue::from_bytes(&chapters)),
            NveMapEntry::new("metadata", NveValue::array(&meta_items)),
        ];
        assert_eq!((vvt.set_props_direct)(vh, entries.as_ptr(), entries.len() as u32), NveHybridStatus::Ok as i32);
        let decoded = VideoSurfaceProps::decode_props(NveMapRef::from_slice(&entries)).unwrap();
        assert_eq!(decoded.source_url, "https://nve.dev/v.mp4");
        assert!(decoded.muted);
        assert_eq!(decoded.playback_rate, 1.0);
        assert_eq!(decoded.quality, VideoQuality::Uhd4k);
        assert_eq!(decoded.poster_url, None);
        assert_eq!(decoded.drm_key, vec![5, 6]);
        assert_eq!(decoded.chapters, vec![10, 20]);
        let meta = take((vvt.current_metadata)(vh));
        assert_eq!(meta, nve_abi::encode_bin_value_to_vec(NveValue::array(&meta_items).as_ref()).unwrap());

        let mut frame = NveHybridOwnedBytes::EMPTY;
        let mut err = NveHybridOwnedBytes::EMPTY;
        assert_eq!((vvt.capture_frame)(vh, FramePoint { x: -1.0, y: 0.0 }, &mut frame, &mut err), NveHybridStatus::Err as i32);
        assert_eq!(take(err), b"negative x");
        (vvt.destroy)(vh);
    }

    let mut saw_direct = false;
    emit_video_surface_on_progress(1, 2, 333, 1200, u64::MAX, |d| {
        saw_direct = true;
        assert_eq!((d.surface_id, d.node_id, d.timestamp_us), (1, 2, 333));
        let payload = unsafe { d.value.as_ref() };
        assert_eq!(payload.get("position_ms").and_then(|v| v.as_i64()), Some(1200));
        assert_eq!(payload.get("duration_ms").and_then(|v| v.as_i64()), Some(-1));
    })
    .unwrap();
    assert!(saw_direct);
    println!("realistic hybrid e2e ok");
}
"#;

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nve_hybrid_rust_runtime_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

/// Generates the Rust bindings of `spec_src`, builds them as an rlib, then builds and runs
/// `main_src` against them; returns the program's stdout.
fn compile_and_run(tag: &str, spec_src: &str, crate_name: &str, main_src: &str) -> String {
    let spec = parse_hybrid_rust_spec(spec_src).expect("spec parses");
    let out = generate_hybrid_target(&spec, HybridTarget::Rust).expect("Rust bindings generate");
    let dir = temp_test_dir(tag);
    out.write_to_dir(&dir).expect("write bindings");
    assert_eq!(out.files.len(), 1, "the Rust target emits one module");

    let rustc = common::rustc::Rustc::locate();
    let generated = dir.join(&out.files[0].relative_path);
    let rlib = rustc.build_lib(&dir, crate_name, &generated);
    let main_rs = dir.join("main.rs");
    std::fs::write(&main_rs, main_src).expect("write main.rs");
    let bin = dir.join(format!("e2e{}", std::env::consts::EXE_SUFFIX));
    rustc.build(&dir, &[], &[(crate_name, &rlib)], &main_rs, &bin);

    let run = Command::new(&bin).output().expect("run e2e binary");
    assert!(
        run.status.success(),
        "e2e binary failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn generated_rust(spec_src: &str) -> String {
    let spec = parse_hybrid_rust_spec(spec_src).expect("spec parses");
    let out = generate_hybrid_target(&spec, HybridTarget::Rust).expect("Rust bindings generate");
    out.files.into_iter().map(|f| f.content).collect()
}

#[test]
fn realistic_spec_round_trips_through_rustc_compiled_vtables() {
    let stdout = compile_and_run(
        "realistic",
        REALISTIC_HYBRID_SPEC,
        "crypto_hybrid",
        REALISTIC_MAIN_RS,
    );
    assert!(stdout.contains("realistic hybrid e2e ok"));
}

#[test]
fn edge_spec_round_trips_every_codec_shape_and_error_path() {
    let stdout = compile_and_run("edge", EDGE_HYBRID_SPEC, "edge_hybrid", EDGE_MAIN_RS);
    assert!(stdout.contains("edge hybrid e2e ok"));
}

#[test]
fn generated_rust_reuses_the_static_runtime_instead_of_embedding_it() {
    for spec in [REALISTIC_HYBRID_SPEC, EDGE_HYBRID_SPEC] {
        let src = generated_rust(spec);
        for embedded in [
            "pub mod nve_bin_codec",
            "pub struct NveHybridOwnedBytes",
            "pub enum NveHybridStatus",
            "pub struct NveHybridValue",
            "fn nve_hybrid_alloc_bytes",
            "Vec::from_raw_parts",
            "to_le_bytes",
        ] {
            assert!(
                !src.contains(embedded),
                "generated Rust embeds runtime code `{embedded}`"
            );
        }
        assert!(src.contains("pub use nve_abi::hybrid::{"));
        assert!(src.contains("nve_abi::hybrid_struct_codec!("));
        assert!(src.contains("nve_abi::hybrid_enum_codec!("));
        // `Value` props decode via the codec, which keeps the C tree's structure.
        assert!(
            !src.contains("NveValue::from_bytes(_"),
            "composite event params must not be wrapped in bytes"
        );
        assert!(!src.contains("unwrap()") && !src.contains(".expect(") && !src.contains("panic!"));
    }
}

#[test]
fn rust_target_rejects_members_that_cannot_cross_a_vtable() {
    let spec_src = r#"
#[nve_hybrid_struct]
pub struct Packet {
    pub data: *const u8,
    pub len: usize,
}

#[nve_hybrid_module(name = "Net")]
pub trait NetSpec {
    fn send(&self, packets: Vec<Packet>);
}
"#;
    let spec = parse_hybrid_rust_spec(spec_src).expect("spec parses");
    let err =
        generate_hybrid_target(&spec, HybridTarget::Rust).expect_err("pointer payloads are C-only");
    let message = err.to_string();
    assert!(
        message.contains("module 'Net' method 'send' param 'packets'"),
        "{message}"
    );
    assert!(message.contains("raw pointer"), "{message}");
}

/// Minimal specs whose bindings need few `nve_abi` symbols: each must still compile under
/// `-D warnings` (no unused import such as `NveValue` without events or `Value` members).
#[test]
fn minimal_specs_import_only_what_they_use() {
    let specs = [
        (
            "enum_only",
            "#[nve_hybrid_enum]\n#[repr(u32)]\npub enum Mode { A = 0, B = 1 }\n",
        ),
        (
            "pod_struct_only",
            "#[nve_hybrid_struct]\npub struct P { pub x: f32 }\n",
        ),
        (
            "scalar_module",
            "#[nve_hybrid_module(name = \"Calc\")]\npub trait CalcSpec { fn add(&self, a: i64, b: i64) -> i64; fn ping(&self); }\n",
        ),
        (
            "string_module",
            "#[nve_hybrid_module(name = \"Echo\")]\npub trait EchoSpec { fn echo(&self, s: &str) -> Result<String, String>; }\n",
        ),
        (
            "eventless_view",
            "#[nve_hybrid_view(name = \"Label\")]\npub trait LabelSpec {\n    #[prop]\n    fn text(&self) -> String;\n    #[prop(default = 1.0)]\n    fn scale(&self) -> f32;\n}\n",
        ),
        (
            "empty_view",
            "#[nve_hybrid_view(name = \"Spacer\")]\npub trait SpacerSpec {\n    #[method]\n    fn reset(&mut self);\n}\n",
        ),
    ];
    let rustc = common::rustc::Rustc::locate();
    for (tag, spec_src) in specs {
        let spec = parse_hybrid_rust_spec(spec_src).unwrap_or_else(|e| panic!("{tag}: {e}"));
        let out =
            generate_hybrid_target(&spec, HybridTarget::Rust).expect("Rust bindings generate");
        let dir = temp_test_dir(tag);
        out.write_to_dir(&dir).expect("write bindings");
        rustc.build_lib(&dir, tag, &dir.join(&out.files[0].relative_path));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn shared_rustc_helper_finds_nve_abi() {
    let rustc = common::rustc::Rustc::locate();
    let dir = temp_test_dir("probe");
    let src = dir.join("probe.rs");
    std::fs::write(
        &src,
        "pub fn probe() -> u32 { nve_abi::hybrid::NVE_HYBRID_ABI_VERSION }\n",
    )
    .expect("write probe");
    let rlib = rustc.build_lib(&dir, "probe", &src);
    assert!(Path::new(&rlib).exists());
    let _ = std::fs::remove_dir_all(&dir);
}
