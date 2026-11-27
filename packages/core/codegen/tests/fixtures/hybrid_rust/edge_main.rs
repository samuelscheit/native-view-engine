// Driver of the edge spec's generated bindings, compiled by real `rustc -D warnings` and run.
use edge_hybrid::*;
use nve_abi::hybrid::{decode_bin_arg, encode_to_vec};
use nve_abi::{
    NveBufferSlice, NveMapEntry, NveMapRef, NveStrSlice, NveValue, NveValueTag,
    encode_bin_value_to_vec,
};

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
    fn default(&mut self) -> u32 {
        7
    }
    fn check(&mut self, status: Pulse, maybe: Option<Pulse>) -> Result<Option<Pulse>, String> {
        Ok(maybe.map(|m| Pulse { level: status.level + m.level, ..m }))
    }
    fn advance(&mut self, frame: Frame) -> Frame {
        let mut next = frame;
        next.history = [frame.status.phase, Phase::Paused];
        next.flags = frame.flags.map(|f| !f);
        next.pad[39] = frame.pad[0];
        next.status.live = !frame.status.live;
        next.grid[1][32] = !frame.grid[0][0];
        next
    }
    fn toggle(&mut self, on: bool) -> Result<bool, String> {
        Ok(!on)
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
    let mut digest = [0u8; 64];
    digest[63] = 0xAB;
    Node {
        name: "root".to_owned(),
        depth: u16::MAX - 10,
        small: u8::MAX,
        // Past `u32::MAX` where `usize` is 64-bit; still exact on 32-bit targets.
        size: usize::MAX >> 1,
        weight: -1.25e300,
        color: Rgba { channels: [0.0, 0.5, 1.0, -0.0], tone: Tone::High },
        accent: Some(Rgba { channels: [1.0; 4], tone: Tone::Low }),
        children: vec![None, Some(vec![]), Some(vec![i32::MIN, i32::MAX])],
        blobs: vec![vec![], vec![1, 2, 3]],
        extra: sample_extra(),
        m: Some(u64::MAX),
        fields: vec![Tone::High, Tone::Low],
        pair: ["left".to_owned(), "ríght".to_owned()],
        digest,
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

fn shadow_module(node: &Node, node_bin: &[u8]) {
    let vt = build_shadow_vtable::<Shadow>();
    assert_eq!(vt.abi_version, NVE_HYBRID_ABI_VERSION);
    unsafe {
        let h = (vt.create)();
        let mut out_ok = NveHybridOwnedBytes::EMPTY;
        let mut out_err = NveHybridOwnedBytes::EMPTY;
        let blob = [9u8, 8];

        // Ok path: every shadowing parameter reaches the implementation intact.
        let st = (vt.shadow)(h, 5, NveStrSlice::from_str("tail"), NveBufferSlice::from_slice(&blob), NveBufferSlice::from_slice(node_bin), 3, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::Ok as i32);
        assert!(out_err.ptr.is_null());
        let back = decode_bin_arg::<Node>(&take(std::mem::replace(&mut out_ok, NveHybridOwnedBytes::EMPTY))).unwrap();
        let mut expected = node.clone();
        expected.name = "root|tail".to_owned();
        expected.depth += 3;
        expected.blobs.push(blob.to_vec());
        expected.children.push(Some(vec![5]));
        assert_eq!(back, expected);
        // A null `out_ok` releases the result instead of leaking or writing through it.
        let st = (vt.shadow)(h, 5, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::from_slice(node_bin), 0, std::ptr::null_mut(), &mut out_err);
        assert_eq!(st, NveHybridStatus::Ok as i32);

        // Err path keeps UTF-8 messages.
        let st = (vt.shadow)(h, -1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::from_slice(node_bin), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::Err as i32);
        assert_eq!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)), "negative handle -1 😀");

        // Malformed arguments never reach the implementation.
        let garbage = [0xFFu8, 1];
        let st = (vt.shadow)(h, 1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::from_slice(&garbage), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::InvalidArg as i32);
        assert!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)).starts_with("invalid hybrid argument 'instance': "));
        let bad_utf8 = [0xC3u8];
        let bad_str = NveStrSlice { ptr: bad_utf8.as_ptr(), len: 1 };
        let st = (vt.shadow)(h, 1, bad_str, NveBufferSlice::EMPTY, NveBufferSlice::from_slice(node_bin), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::InvalidArg as i32);
        assert!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)).starts_with("invalid hybrid argument 'out_err': "));
        // `(null, len > 0)` is rejected, not read as empty or dereferenced.
        let dangling = NveBufferSlice { ptr: std::ptr::null(), len: 100 };
        let st = (vt.shadow)(h, 1, NveStrSlice::from_str(""), dangling, NveBufferSlice::from_slice(node_bin), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::InvalidArg as i32);
        assert!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)).starts_with("invalid hybrid argument 'out_ok': "));
        let st = (vt.shadow)(h, 1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, dangling, 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::InvalidArg as i32);
        let _ = take(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY));
        let wrong_shape = encode_to_vec(&vec![1i32]).unwrap();
        let st = (vt.shadow)(h, 1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::from_slice(&wrong_shape), 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::InvalidArg as i32);
        let _ = take(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY));
        // A null `out_err` drops the message (no leak, no write).
        let st = (vt.shadow)(h, -1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::from_slice(node_bin), 0, &mut out_ok, std::ptr::null_mut());
        assert_eq!(st, NveHybridStatus::Err as i32);
        let st = (vt.shadow)(std::ptr::null_mut(), 1, NveStrSlice::from_str(""), NveBufferSlice::EMPTY, NveBufferSlice::EMPTY, 0, &mut out_ok, &mut out_err);
        assert_eq!(st, NveHybridStatus::NullHandle as i32);
        assert_eq!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)), "null hybrid handle");

        // POD options: an unknown discriminant is an invalid argument (zero value), not a cast.
        let none_rgba = NveHybridOptRgba::default();
        let r = (vt.pick)(h, NveHybridOptTone { is_some: 1, value: 9 }, none_rgba);
        assert!(r.is_some == 1 && r.value == 9);
        let r = (vt.pick)(h, NveHybridOptTone { is_some: 1, value: 5 }, none_rgba);
        assert_eq!(r.is_some, 0);
        let r = (vt.pick)(h, NveHybridOptTone { is_some: 2, value: 9 }, none_rgba);
        assert_eq!(r.is_some, 0, "`is_some` must be 0 or 1");
        let some_rgba = NveHybridOptRgba { is_some: 1, value: NveHybridRawRgba { channels: [0.0; 4], tone: 0 } };
        let r = (vt.pick)(h, NveHybridOptTone::default(), some_rgba);
        assert!(r.is_some == 1 && r.value == 0);
        assert_eq!((vt.strict)(h, 9), 10);
        assert_eq!((vt.strict)(h, 3), 0);
        assert_eq!((vt.strict)(std::ptr::null_mut(), 9), 0);
        // A spec method named `default` coexists with `Default` on the implementation.
        assert_eq!((vt.default)(h), 7);

        raw_carriers(&vt, h);

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
}

/// By-value structs holding enums / `bool` cross as `NveHybridRaw*` carriers: any bits a C caller
/// passes are defined on entry, and invalid ones are rejected as `InvalidArg`.
unsafe fn raw_carriers(vt: &NveHybridShadowVTable, h: nve_abi::NveAppHandle) {
    unsafe {
        let status = NveHybridRawPulse { phase: 1, live: 1, level: 3 };
        let zeroed = NveHybridRawPulse { phase: 0, live: 0, level: 0 };
        let mut out = NveHybridOptPulse::default();
        let mut out_err = NveHybridOwnedBytes::EMPTY;
        // `std::nullopt` from C++ is a zeroed carrier: `phase: 0` is not a `Phase`, yet it is fine.
        let none = NveHybridOptPulse { is_some: 0, value: zeroed };
        assert_eq!((vt.check)(h, status, none, &mut out, &mut out_err), NveHybridStatus::Ok as i32);
        assert_eq!(out.is_some, 0);
        let some = NveHybridOptPulse { is_some: 1, value: NveHybridRawPulse { phase: 2, live: 0, level: 4 } };
        assert_eq!((vt.check)(h, status, some, &mut out, &mut out_err), NveHybridStatus::Ok as i32);
        assert_eq!((out.is_some, out.value.phase, out.value.live, out.value.level), (1, 2, 0, 7));
        assert_eq!(out.value.decode(), Ok(Pulse { phase: Phase::Paused, live: false, level: 7 }));

        for (bad, field) in [(zeroed, "status"), (NveHybridRawPulse { phase: 99, ..status }, "status"), (NveHybridRawPulse { live: 2, ..status }, "status")] {
            assert_eq!((vt.check)(h, bad, none, &mut out, &mut out_err), NveHybridStatus::InvalidArg as i32);
            assert!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)).starts_with(&format!("invalid hybrid argument '{field}': ")));
        }
        let bad_some = NveHybridOptPulse { is_some: 1, value: zeroed };
        assert_eq!((vt.check)(h, status, bad_some, &mut out, &mut out_err), NveHybridStatus::InvalidArg as i32);
        assert!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)).contains("Phase"));

        // Nested carriers and fixed-size arrays of enums / bools (one longer than 32).
        let mut pad = [0u8; 40];
        pad[0] = 5;
        let frame = NveHybridRawFrame { status, history: [1, 1], flags: [1, 0, 1], pad, grid: [[1; 33]; 2] };
        let next = (vt.advance)(h, frame).decode().unwrap();
        assert_eq!(next.history, [Phase::Active, Phase::Paused]);
        assert_eq!(next.flags, [false, true, false]);
        assert_eq!((next.pad[39], next.status.live), (5, false));
        assert_eq!((next.grid[0][0], next.grid[1][32]), (true, false));
        let zero = (vt.advance)(h, NveHybridRawFrame { history: [1, 3], ..frame });
        assert_eq!(zero.status.phase, 0, "an invalid nested element returns the zero carrier");
        let mut bad_grid = frame;
        bad_grid.grid[1][20] = 2;
        assert_eq!((vt.advance)(h, bad_grid).status.phase, 0, "a non-0/1 `bool` deep in a nested array is rejected");
        let defaults = NveHybridRawFrame::from(Frame::default());
        assert_eq!((defaults.pad, defaults.grid), ([0; 40], [[0; 33]; 2]));

        let mut on = 9u8;
        assert_eq!((vt.toggle)(h, 0, &mut on, &mut out_err), NveHybridStatus::Ok as i32);
        assert_eq!(on, 1);
        assert_eq!((vt.toggle)(h, 1, &mut on, &mut out_err), NveHybridStatus::Ok as i32);
        assert_eq!(on, 0);
        // A byte other than 0 / 1 is not a `bool`: rejected, never read as `true` and negated.
        on = 9;
        assert_eq!((vt.toggle)(h, 2, &mut on, &mut out_err), NveHybridStatus::InvalidArg as i32);
        assert_eq!(on, 9, "the out value is untouched");
        assert!(take_str(std::mem::replace(&mut out_err, NveHybridOwnedBytes::EMPTY)).starts_with("invalid hybrid argument 'on': "));
    }
}

fn canvas_view(node: &Node) {
    let cvt = build_canvas_vtable::<Canvas>();
    unsafe {
        let h = (cvt.create)();
        let node_value = NveHybridValue::encode(node).unwrap();
        let node_tree = node_value.to_tree();
        let layers_bin = encode_to_vec(&vec![Node::default(), node.clone()]).unwrap();
        let extra = sample_extra();
        let extra_tree = extra.to_tree();
        let seed_items = [NveValue::i64(3); 48];
        let entries = [
            NveMapEntry::new("node", *node_tree.root()),
            NveMapEntry::new("label", NveValue::from_str("hi")),
            NveMapEntry::new("layers", NveValue::from_bytes(&layers_bin)),
            NveMapEntry::new("extra", *extra_tree.root()),
            NveMapEntry::new("seed", NveValue::array(&seed_items)),
            NveMapEntry::new("unknown", NveValue::bool(true)),
        ];
        assert_eq!((cvt.set_props_direct)(h, entries.as_ptr(), entries.len() as u32), NveHybridStatus::Ok as i32);
        let props = CanvasProps::decode_props(NveMapRef::from_slice(&entries)).unwrap();
        assert_eq!(&props.node, node);
        assert_eq!(props.label.as_deref(), Some("hi"));
        assert_eq!(props.layers, vec![Node::default(), node.clone()]);
        assert_eq!(props.extra, extra);
        assert_eq!(props.extra.as_view().tag(), NveValueTag::Map);
        assert_eq!(props.seed, [3; 48]);

        let snap = take((cvt.snapshot)(h, 77));
        let mut expected = node.clone();
        expected.size = 77;
        assert_eq!(decode_bin_arg::<Node>(&snap), Ok(expected));

        // Defaults: absent props keep them (a `Value` defaults to null, never to empty bytes).
        assert_eq!((cvt.set_props_direct)(h, std::ptr::null(), 0), NveHybridStatus::Ok as i32);
        let defaults = CanvasProps::default();
        assert_eq!(defaults.extra.as_bytes(), &[NveValueTag::Null as u8]);
        assert_eq!((defaults.label, defaults.seed), (None, [0; 48]));

        let wrong = [NveMapEntry::new("node", NveValue::i64(1))];
        assert_eq!((cvt.set_props_direct)(h, wrong.as_ptr(), 1), NveHybridStatus::InvalidArg as i32);
        // A corrupt key is an error, not a silently skipped prop.
        let bad_key = [0xFFu8];
        let corrupt = [NveMapEntry { key: NveStrSlice { ptr: bad_key.as_ptr(), len: 1 }, value: NveValue::null() }];
        assert_eq!((cvt.set_props_direct)(h, corrupt.as_ptr(), 1), NveHybridStatus::InvalidArg as i32);
        assert_eq!((cvt.set_props_direct)(h, std::ptr::null(), 3), NveHybridStatus::InvalidArg as i32);
        assert_eq!((cvt.set_props_direct)(std::ptr::null_mut(), entries.as_ptr(), 1), NveHybridStatus::NullHandle as i32);
        (cvt.destroy)(h);
    }
}

fn direct_events(node: &Node) {
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
        assert_eq!(<Node as nve_abi::hybrid::HybridCodec>::decode_value_ref(sink).as_ref(), Ok(node));
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
    emit_canvas_on_blob(0, 0, 0, [1; 33], 12, |d| {
        seen += 1;
        let payload = unsafe { d.value.as_ref() };
        assert_eq!(payload.get("digest").and_then(|v| v.as_array()).map(|a| a.len()), Some(33));
        assert_eq!(payload.get("size").and_then(|v| v.as_i64()), Some(12));
    })
    .unwrap();
    assert_eq!(CanvasOnBlobEvent::default().digest, [0; 33]);
    if let Ok(too_big) = usize::try_from(u64::MAX) {
        let result = emit_canvas_on_blob(0, 0, 0, [0; 33], too_big, |_| seen += 100);
        assert_eq!(result, Err(nve_abi::DecodeError::OutOfRange("usize")));
    }
    emit_blank_on_tap(0, 0, 0, 1.5, |d| {
        seen += 1;
        assert_eq!(unsafe { d.value.as_ref() }.get("x").and_then(|v| v.as_f64()), Some(1.5));
    })
    .unwrap();
    assert_eq!(seen, 4);
    let bvt = build_blank_vtable::<Blank>();
    unsafe {
        let h = (bvt.create)();
        assert_eq!((bvt.set_props_direct)(h, std::ptr::null(), 0), NveHybridStatus::Ok as i32);
        (bvt.destroy)(h);
    }
}

fn main() {
    let node = sample_node();
    let node_bin = encode_to_vec(&node).unwrap();
    assert_eq!(decode_bin_arg::<Node>(&node_bin), Ok(node.clone()));
    shadow_module(&node, &node_bin);
    canvas_view(&node);
    direct_events(&node);
    println!("edge hybrid e2e ok");
}
