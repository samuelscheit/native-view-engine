// Driver of the realistic spec's generated bindings, compiled by real `rustc -D warnings` and run.
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

        // `DigestHeader` holds an enum and a `bool`, so it returns as its raw carrier.
        let raw_hdr = (vt.hash_with)(h, HashAlgorithm::Sha512.to_u32(), NveBufferSlice::from_slice(&[1, 2, 3, 4]));
        assert_eq!((raw_hdr.algorithm, raw_hdr.verified), (2, 1));
        let hdr = raw_hdr.decode().unwrap();
        assert_eq!((hdr.algorithm, hdr.byte_len, hdr.verified, hdr.anchor.x), (HashAlgorithm::Sha512, 4, true, 3.5));
        // An undeclared discriminant never reaches the implementation: the zero carrier returns.
        let rejected = (vt.hash_with)(h, 7, NveBufferSlice::from_slice(&[1]));
        assert_eq!((rejected.algorithm, rejected.byte_len, rejected.verified), (0, 0, 0));

        let some = (vt.find_nonce)(h, NveStrSlice::from_str("k"), NveHybridOptU64 { is_some: 1, value: 41 });
        assert!(some.is_some == 1 && some.value == 42);
        assert_eq!((vt.find_nonce)(h, NveStrSlice::from_str("k"), NveHybridOptU64::default()).is_some, 0);

        let payload = SignedPayload { header: hdr, label: "hello".to_string(), digest: vec![9, 8], tags: vec!["a".to_string()], nonce: Some(77) };
        let payload_bin = encode_to_vec(&payload).unwrap();
        let mut ok_out = 0u8;
        let mut err_out = NveHybridOwnedBytes::EMPTY;
        assert_eq!((vt.verify_payload)(h, NveBufferSlice::from_slice(&payload_bin), &mut ok_out, &mut err_out), NveHybridStatus::Ok as i32);
        assert_eq!(ok_out, 1);
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

        // A `Value` parameter crosses as a C tree and returns as one `NveBinValue`.
        let fields = [NveMapEntry::new("fps", NveValue::f64(59.94)), NveMapEntry::new("tags", NveValue::from_bytes(&[1, 2]))];
        let meta = NveValue::map(&fields);
        let inspected = take((vt.inspect_dynamic)(h, meta));
        assert_eq!(inspected, nve_abi::encode_bin_value_to_vec(meta.as_ref()).unwrap());
        let bad_meta = NveValue { tag: 99, data: nve_abi::value::NveValueData { i64_val: 0 } };
        assert!((vt.inspect_dynamic)(h, bad_meta).ptr.is_null());
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
