//! Deterministic seeded property & fuzz-style tests for `nve-abi` binary codecs and FFI helpers.
//!
//! Uses a zero-dependency `XorShift64` PRNG so every failure is 100% reproducible.

use nve_abi::codec::{HEADER_SIZE, Opcode};
use nve_abi::{
    decode_events, encode_events, ColorRgba, Command, CommandBufferBuilder, CommandDecoder,
    ComponentKind, DecodeError, Dimension, EdgeInsets, EventPayload, FlexAlign, FlexDirection,
    FlexJustify, LayoutProps, NativeEvent, NativeEventKind, NveBufferSlice, NveGestureDisposition,
    NveSizeConstraints, RawPropValue, StyleProps, NVE_ABI_MAGIC, NVE_ABI_VERSION,
};

struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        assert_ne!(seed, 0);
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u8(&mut self) -> u8 {
        self.next_u64() as u8
    }

    fn next_usize(&mut self, bound: usize) -> usize {
        (self.next_u64() as usize) % bound
    }

    fn next_bool(&mut self) -> bool {
        (self.next_u64() & 1) != 0
    }

    fn next_f32(&mut self) -> f32 {
        let raw = (self.next_u32() % 200_000) as i32 - 100_000;
        raw as f32 / 100.0
    }

    fn next_f64(&mut self) -> f64 {
        let raw = (self.next_u64() % 2_000_000) as i64 - 1_000_000;
        raw as f64 / 1000.0
    }

    fn next_string(&mut self, max_len: usize) -> String {
        const ALPHABET: &[char] = &[
            'a', 'Z', '0', '9', '-', '_', ' ', '/', 'é', 'ü', '中', '日', '🚀', '✨',
        ];
        let len = if max_len == 0 {
            0
        } else {
            self.next_usize(max_len + 1)
        };
        (0..len)
            .map(|_| ALPHABET[self.next_usize(ALPHABET.len())])
            .collect()
    }

    fn next_bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = if max_len == 0 {
            0
        } else {
            self.next_usize(max_len + 1)
        };
        (0..len).map(|_| self.next_u8()).collect()
    }

    fn next_dim(&mut self) -> Dimension {
        match self.next_usize(3) {
            0 => Dimension::Auto,
            1 => Dimension::Points(self.next_f32()),
            _ => Dimension::Percent(self.next_f32()),
        }
    }

    fn next_insets(&mut self) -> EdgeInsets {
        EdgeInsets {
            top: self.next_f32(),
            right: self.next_f32(),
            bottom: self.next_f32(),
            left: self.next_f32(),
        }
    }

    fn next_color(&mut self) -> ColorRgba {
        ColorRgba::rgba(
            self.next_u8(),
            self.next_u8(),
            self.next_u8(),
            self.next_u8(),
        )
    }

    fn next_kind(&mut self) -> ComponentKind {
        match self.next_usize(9) {
            0 => ComponentKind::Root,
            1 => ComponentKind::View,
            2 => ComponentKind::Text,
            3 => ComponentKind::ScrollView,
            4 => ComponentKind::Image,
            5 => ComponentKind::TextInput,
            6 => ComponentKind::Pressable,
            7 => ComponentKind::SkiaCanvas,
            _ => ComponentKind::Custom,
        }
    }

    fn next_raw_prop(&mut self) -> RawPropValue {
        match self.next_usize(6) {
            0 => RawPropValue::Null,
            1 => RawPropValue::Bool(self.next_bool()),
            2 => RawPropValue::I64(self.next_u64() as i64),
            3 => RawPropValue::F64(self.next_f64()),
            4 => RawPropValue::String(self.next_string(32)),
            _ => RawPropValue::Bytes(self.next_bytes(32)),
        }
    }

    fn next_command(&mut self) -> Command {
        let node_id = (self.next_u32() % 5000) + 1;
        match self.next_usize(10) {
            0 => {
                let kind = self.next_kind();
                let custom_name = if self.next_bool() {
                    let s = self.next_string(16);
                    if s.is_empty() { None } else { Some(s) }
                } else {
                    None
                };
                Command::CreateNode {
                    node_id,
                    kind,
                    custom_name,
                }
            }
            1 => Command::RemoveNode { node_id },
            2 => {
                let count = self.next_usize(16);
                let children = (0..count).map(|_| (self.next_u32() % 5000) + 1).collect();
                Command::SetChildren {
                    parent_id: node_id,
                    children,
                }
            }
            3 => Command::SetText {
                node_id,
                text: self.next_string(48),
            },
            4 => Command::SetLayout {
                node_id,
                layout: LayoutProps {
                    width: self.next_dim(),
                    height: self.next_dim(),
                    min_width: self.next_dim(),
                    min_height: self.next_dim(),
                    max_width: self.next_dim(),
                    max_height: self.next_dim(),
                    flex_direction: match self.next_usize(4) {
                        0 => FlexDirection::Column,
                        1 => FlexDirection::Row,
                        2 => FlexDirection::ColumnReverse,
                        _ => FlexDirection::RowReverse,
                    },
                    justify_content: match self.next_usize(6) {
                        0 => FlexJustify::FlexStart,
                        1 => FlexJustify::Center,
                        2 => FlexJustify::FlexEnd,
                        3 => FlexJustify::SpaceBetween,
                        4 => FlexJustify::SpaceAround,
                        _ => FlexJustify::SpaceEvenly,
                    },
                    align_items: match self.next_usize(5) {
                        0 => FlexAlign::Stretch,
                        1 => FlexAlign::FlexStart,
                        2 => FlexAlign::Center,
                        3 => FlexAlign::FlexEnd,
                        _ => FlexAlign::Baseline,
                    },
                    flex_grow: self.next_f32(),
                    flex_shrink: self.next_f32(),
                    gap: self.next_f32(),
                    padding: self.next_insets(),
                    margin: self.next_insets(),
                },
            },
            5 => Command::SetStyle {
                node_id,
                style: StyleProps {
                    background: self.next_color(),
                    text_color: self.next_color(),
                    border_color: self.next_color(),
                    border_width: self.next_f32(),
                    border_radius: self.next_f32(),
                    opacity: self.next_f32(),
                    font_size: self.next_f32(),
                    z_index: self.next_u32() as i32,
                },
            },
            6 => Command::SetRawProp {
                node_id,
                key: self.next_string(20),
                value: self.next_raw_prop(),
            },
            7 => Command::SetEventMask {
                node_id,
                mask: self.next_u64(),
            },
            8 => Command::DispatchCommand {
                node_id,
                command_name: self.next_string(20),
                payload: self.next_bytes(32),
            },
            _ => Command::Commit {
                sync_mount: self.next_bool(),
            },
        }
    }

    fn next_event(&mut self) -> NativeEvent {
        let kind = match self.next_usize(8) {
            0 => NativeEventKind::Press,
            1 => NativeEventKind::LongPress,
            2 => NativeEventKind::Scroll,
            3 => NativeEventKind::TextChange,
            4 => NativeEventKind::Focus,
            5 => NativeEventKind::Blur,
            6 => NativeEventKind::Layout,
            _ => NativeEventKind::Custom,
        };
        let payload = match self.next_usize(6) {
            0 => EventPayload::Empty,
            1 => EventPayload::Point {
                x: self.next_f32(),
                y: self.next_f32(),
            },
            2 => EventPayload::Scroll {
                x: self.next_f32(),
                y: self.next_f32(),
                content_width: self.next_f32(),
                content_height: self.next_f32(),
            },
            3 => EventPayload::Text(self.next_string(48)),
            4 => EventPayload::Layout {
                x: self.next_f32(),
                y: self.next_f32(),
                width: self.next_f32(),
                height: self.next_f32(),
            },
            _ => EventPayload::Custom {
                name: self.next_string(24),
                data: self.next_bytes(48),
            },
        };
        NativeEvent {
            surface_id: (self.next_u32() % 16) + 1,
            node_id: (self.next_u32() % 5000) + 1,
            timestamp_us: self.next_u64(),
            kind,
            payload,
        }
    }
}

fn push_command(builder: &mut CommandBufferBuilder, cmd: &Command) {
    match cmd {
        Command::CreateNode {
            node_id,
            kind,
            custom_name,
        } => builder.create_node(*node_id, *kind, custom_name.as_deref()),
        Command::RemoveNode { node_id } => builder.remove_node(*node_id),
        Command::SetChildren {
            parent_id,
            children,
        } => builder.set_children(*parent_id, children),
        Command::SetText { node_id, text } => builder.set_text(*node_id, text),
        Command::SetLayout { node_id, layout } => builder.set_layout(*node_id, layout),
        Command::SetStyle { node_id, style } => builder.set_style(*node_id, style),
        Command::SetRawProp {
            node_id,
            key,
            value,
        } => builder.set_raw_prop(*node_id, key, value),
        Command::SetEventMask { node_id, mask } => builder.set_event_mask(*node_id, *mask),
        Command::DispatchCommand {
            node_id,
            command_name,
            payload,
        } => builder.dispatch_command(*node_id, command_name, payload),
        Command::Commit { sync_mount } => builder.commit(*sync_mount),
    }
}

fn build_all_variants_command_buffer() -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(3, 99);
    b.create_node(1, ComponentKind::Root, None);
    b.create_node(2, ComponentKind::Custom, Some("RNSVGPath"));
    b.set_children(1, &[2, 3, 4]);
    b.set_text(2, "UTF-8 🚀 text");
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(120.5),
            height: Dimension::Percent(50.0),
            min_width: Dimension::Points(10.0),
            max_height: Dimension::Points(300.0),
            flex_direction: FlexDirection::RowReverse,
            justify_content: FlexJustify::SpaceEvenly,
            align_items: FlexAlign::Baseline,
            flex_grow: 1.5,
            flex_shrink: 0.25,
            gap: 8.0,
            padding: EdgeInsets::symmetric(4.0, 8.0),
            margin: EdgeInsets::all(2.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgba(10, 20, 30, 200),
            text_color: ColorRgba::WHITE,
            border_color: ColorRgba::rgb(255, 128, 0),
            border_width: 1.5,
            border_radius: 10.0,
            opacity: 0.75,
            font_size: 18.0,
            z_index: -3,
        },
    );
    b.set_raw_prop(2, "null", &RawPropValue::Null);
    b.set_raw_prop(2, "flag", &RawPropValue::Bool(true));
    b.set_raw_prop(2, "count", &RawPropValue::I64(-42));
    b.set_raw_prop(2, "ratio", &RawPropValue::F64(3.25));
    b.set_raw_prop(2, "label", &RawPropValue::String("hello".into()));
    b.set_raw_prop(2, "bytes", &RawPropValue::Bytes(vec![1, 2, 3, 4]));
    b.set_event_mask(2, 0xDEAD_BEEF_CAFE_BABE);
    b.dispatch_command(2, "scrollTo", &[9, 8, 7]);
    b.remove_node(2);
    b.commit(true);
    b.into_bytes()
}

fn make_raw_header(surface_id: u32, frame_id: u64, op_count: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEADER_SIZE);
    buf.extend_from_slice(&NVE_ABI_MAGIC.to_le_bytes());
    buf.extend_from_slice(&NVE_ABI_VERSION.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&surface_id.to_le_bytes());
    buf.extend_from_slice(&frame_id.to_le_bytes());
    buf.extend_from_slice(&op_count.to_le_bytes());
    buf
}

#[test]
fn seeded_property_command_buffer_roundtrip_and_builder_reset() {
    let mut rng = XorShift64::new(0x5EED_C0DE_1234_5678);
    let mut builder = CommandBufferBuilder::new(1, 1);

    for iteration in 0..150u64 {
        let surface_id = (rng.next_u32() % 64) + 1;
        let frame_id = iteration + 1;
        builder.reset(surface_id, frame_id);
        assert!(builder.is_empty());
        assert_eq!(builder.op_count(), 0);

        let cmd_count = rng.next_usize(45);
        let mut expected = Vec::with_capacity(cmd_count);
        for _ in 0..cmd_count {
            let cmd = rng.next_command();
            push_command(&mut builder, &cmd);
            expected.push(cmd);
        }

        assert_eq!(builder.op_count() as usize, expected.len());
        assert_eq!(builder.is_empty(), expected.is_empty());

        let bytes = builder.finish();
        let decoder = CommandDecoder::new(bytes).expect("valid encoded header");
        let hdr = decoder.header();
        assert_eq!(hdr.magic, NVE_ABI_MAGIC);
        assert_eq!(hdr.version, NVE_ABI_VERSION);
        assert_eq!(hdr.surface_id, surface_id);
        assert_eq!(hdr.frame_id, frame_id);
        assert_eq!(hdr.op_count as usize, expected.len());

        let (decoded_hdr, decoded_cmds) = decoder.decode_all().expect("valid command stream");
        assert_eq!(decoded_hdr, hdr);
        assert_eq!(decoded_cmds, expected, "mismatch on iteration {iteration}");
    }
}

#[test]
fn seeded_property_event_ring_roundtrip() {
    let mut rng = XorShift64::new(0xE7E4_7A11_CAFE_BEEF);

    for iteration in 0..150usize {
        let count = rng.next_usize(35);
        let events: Vec<NativeEvent> = (0..count).map(|_| rng.next_event()).collect();
        let encoded = encode_events(&events);
        let decoded = decode_events(&encoded)
            .unwrap_or_else(|err| panic!("iteration {iteration} decode failed: {err:?}"));
        assert_eq!(decoded, events, "mismatch on iteration {iteration}");
    }
}

#[test]
fn all_truncation_prefixes_return_truncated_without_panicking() {
    let cmd_bytes = build_all_variants_command_buffer();
    for len in 0..cmd_bytes.len() {
        let res = CommandDecoder::new(&cmd_bytes[..len]).and_then(CommandDecoder::decode_all);
        assert_eq!(
            res,
            Err(DecodeError::Truncated),
            "expected Truncated for command prefix len={len}/{}",
            cmd_bytes.len()
        );
    }

    let events = vec![
        NativeEvent {
            surface_id: 1,
            node_id: 2,
            timestamp_us: 10,
            kind: NativeEventKind::Focus,
            payload: EventPayload::Empty,
        },
        NativeEvent {
            surface_id: 1,
            node_id: 3,
            timestamp_us: 20,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 1.5, y: 2.5 },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 4,
            timestamp_us: 30,
            kind: NativeEventKind::Scroll,
            payload: EventPayload::Scroll {
                x: 1.0,
                y: 2.0,
                content_width: 300.0,
                content_height: 900.0,
            },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 5,
            timestamp_us: 40,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text("hello 🚀".into()),
        },
        NativeEvent {
            surface_id: 1,
            node_id: 6,
            timestamp_us: 50,
            kind: NativeEventKind::Layout,
            payload: EventPayload::Layout {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
            },
        },
        NativeEvent {
            surface_id: 1,
            node_id: 7,
            timestamp_us: 60,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "evt".into(),
                data: vec![0xAA, 0xBB, 0xCC],
            },
        },
    ];
    let ev_bytes = encode_events(&events);
    for len in 0..ev_bytes.len() {
        let res = decode_events(&ev_bytes[..len]);
        assert_eq!(
            res,
            Err(DecodeError::Truncated),
            "expected Truncated for event prefix len={len}/{}",
            ev_bytes.len()
        );
    }
}

#[test]
fn seeded_fuzz_mutated_and_random_streams_never_panic() {
    let mut rng = XorShift64::new(0xF022_B175_9988_7766);
    let base_cmd = build_all_variants_command_buffer();
    let base_ev = encode_events(&[
        NativeEvent {
            surface_id: 1,
            node_id: 10,
            timestamp_us: 55,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text("fuzz target".into()),
        },
        NativeEvent {
            surface_id: 2,
            node_id: 11,
            timestamp_us: 99,
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "custom".into(),
                data: vec![1, 2, 3, 4, 5],
            },
        },
    ]);

    // 400 mutated command buffers
    for _ in 0..400 {
        let mut mutated = base_cmd.clone();
        let flips = rng.next_usize(6) + 1;
        for _ in 0..flips {
            let idx = rng.next_usize(mutated.len());
            mutated[idx] ^= 1u8 << rng.next_usize(8);
        }
        if rng.next_bool() {
            let new_len = rng.next_usize(mutated.len() + 1);
            mutated.truncate(new_len);
        } else if rng.next_bool() {
            let extra = rng.next_bytes(16);
            mutated.extend_from_slice(&extra);
        }
        let _ = CommandDecoder::new(&mutated).and_then(CommandDecoder::decode_all);
    }

    // 400 mutated event rings
    for _ in 0..400 {
        let mut mutated = base_ev.clone();
        let flips = rng.next_usize(5) + 1;
        for _ in 0..flips {
            let idx = rng.next_usize(mutated.len());
            mutated[idx] ^= 1u8 << rng.next_usize(8);
        }
        if rng.next_bool() {
            let new_len = rng.next_usize(mutated.len() + 1);
            mutated.truncate(new_len);
        } else if rng.next_bool() {
            let extra = rng.next_bytes(16);
            mutated.extend_from_slice(&extra);
        }
        let _ = decode_events(&mutated);
    }

    // 300 pure random byte streams against both decoders
    for _ in 0..300 {
        let noise = rng.next_bytes(256);
        let _ = CommandDecoder::new(&noise).and_then(CommandDecoder::decode_all);
        let _ = decode_events(&noise);
    }
}

#[test]
fn rejects_adversarial_headers_oversized_counts_enums_trailing_and_invalid_utf8() {
    // 1. Unsupported version
    let mut bad_ver = make_raw_header(1, 1, 0);
    bad_ver[4..6].copy_from_slice(&99u16.to_le_bytes());
    assert_eq!(
        CommandDecoder::new(&bad_ver).err(),
        Some(DecodeError::UnsupportedVersion(99))
    );

    // 2. Trailing bytes in CommandBuffer & EventRing
    let mut cmd_trailing = make_raw_header(1, 1, 0);
    cmd_trailing.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
    assert_eq!(
        CommandDecoder::new(&cmd_trailing)
            .unwrap()
            .decode_all()
            .err(),
        Some(DecodeError::TrailingBytes(3))
    );

    let mut ev_trailing = encode_events(&[]);
    ev_trailing.extend_from_slice(&[1, 2]);
    assert_eq!(decode_events(&ev_trailing), Err(DecodeError::TrailingBytes(2)));

    // 3. Oversized op_count (u32::MAX) must NOT OOM/panic; returns Truncated
    let huge_ops = make_raw_header(1, 1, u32::MAX);
    assert_eq!(
        CommandDecoder::new(&huge_ops).unwrap().decode_all(),
        Err(DecodeError::Truncated)
    );

    // 4. Oversized SetChildren count (u32::MAX) must NOT OOM/panic
    let mut huge_children = make_raw_header(1, 1, 1);
    huge_children.push(Opcode::SetChildren as u8);
    huge_children.extend_from_slice(&1u32.to_le_bytes());
    huge_children.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        CommandDecoder::new(&huge_children).unwrap().decode_all(),
        Err(DecodeError::Truncated)
    );

    // 5. Oversized SetText length (u32::MAX)
    let mut huge_text = make_raw_header(1, 1, 1);
    huge_text.push(Opcode::SetText as u8);
    huge_text.extend_from_slice(&2u32.to_le_bytes());
    huge_text.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        CommandDecoder::new(&huge_text).unwrap().decode_all(),
        Err(DecodeError::Truncated)
    );

    // 6. Oversized EventRing count (u32::MAX) must NOT OOM/panic
    assert_eq!(
        decode_events(&u32::MAX.to_le_bytes()),
        Err(DecodeError::Truncated)
    );

    // 7. Unknown opcode
    for bad_op in [0u8, 11, 255] {
        let mut buf = make_raw_header(1, 1, 1);
        buf.push(bad_op);
        assert_eq!(
            CommandDecoder::new(&buf).unwrap().decode_all(),
            Err(DecodeError::UnknownOpcode(bad_op))
        );
    }

    // 8. Invalid ComponentKind
    let mut bad_kind = make_raw_header(1, 1, 1);
    bad_kind.push(Opcode::CreateNode as u8);
    bad_kind.extend_from_slice(&2u32.to_le_bytes());
    bad_kind.extend_from_slice(&0x1234u16.to_le_bytes());
    bad_kind.extend_from_slice(&0u16.to_le_bytes());
    assert_eq!(
        CommandDecoder::new(&bad_kind).unwrap().decode_all(),
        Err(DecodeError::InvalidEnum("ComponentKind", 0x1234))
    );

    // 9. Invalid Dimension tag in SetLayout
    let mut b_layout = CommandBufferBuilder::new(1, 1);
    b_layout.set_layout(2, &LayoutProps::default());
    let mut bad_dim = b_layout.into_bytes();
    bad_dim[HEADER_SIZE + 5] = 9; // first Dimension tag
    assert_eq!(
        CommandDecoder::new(&bad_dim).unwrap().decode_all(),
        Err(DecodeError::InvalidEnum("Dimension", 9))
    );

    // 10. Invalid RawPropValue tag
    let mut bad_raw = make_raw_header(1, 1, 1);
    bad_raw.push(Opcode::SetRawProp as u8);
    bad_raw.extend_from_slice(&2u32.to_le_bytes());
    bad_raw.extend_from_slice(&1u16.to_le_bytes());
    bad_raw.push(b'k');
    bad_raw.push(99);
    assert_eq!(
        CommandDecoder::new(&bad_raw).unwrap().decode_all(),
        Err(DecodeError::InvalidEnum("RawPropValue", 99))
    );

    // 11. Invalid NativeEventKind & EventPayload tags
    let mut bad_ev_kind = vec![];
    bad_ev_kind.extend_from_slice(&1u32.to_le_bytes());
    bad_ev_kind.extend_from_slice(&1u32.to_le_bytes());
    bad_ev_kind.extend_from_slice(&2u32.to_le_bytes());
    bad_ev_kind.extend_from_slice(&0u64.to_le_bytes());
    bad_ev_kind.push(99); // bad kind
    bad_ev_kind.push(0);
    assert_eq!(
        decode_events(&bad_ev_kind),
        Err(DecodeError::InvalidEnum("NativeEventKind", 99))
    );

    let mut bad_ev_payload = bad_ev_kind.clone();
    bad_ev_payload[20] = NativeEventKind::Press as u8;
    bad_ev_payload[21] = 42; // bad payload tag
    assert_eq!(
        decode_events(&bad_ev_payload),
        Err(DecodeError::InvalidEnum("EventPayload", 42))
    );

    // 12. Invalid UTF-8 in every string-bearing command & event field
    let bad_utf8: &[u8] = &[0xFF, 0xFE, 0x80];

    // CreateNode custom_name
    let mut bad_utf8_create = make_raw_header(1, 1, 1);
    bad_utf8_create.push(Opcode::CreateNode as u8);
    bad_utf8_create.extend_from_slice(&2u32.to_le_bytes());
    bad_utf8_create.extend_from_slice(&(ComponentKind::Custom as u16).to_le_bytes());
    bad_utf8_create.extend_from_slice(&(bad_utf8.len() as u16).to_le_bytes());
    bad_utf8_create.extend_from_slice(bad_utf8);
    assert_eq!(
        CommandDecoder::new(&bad_utf8_create).unwrap().decode_all(),
        Err(DecodeError::InvalidUtf8)
    );

    // SetText
    let mut bad_utf8_text = make_raw_header(1, 1, 1);
    bad_utf8_text.push(Opcode::SetText as u8);
    bad_utf8_text.extend_from_slice(&2u32.to_le_bytes());
    bad_utf8_text.extend_from_slice(&(bad_utf8.len() as u32).to_le_bytes());
    bad_utf8_text.extend_from_slice(bad_utf8);
    assert_eq!(
        CommandDecoder::new(&bad_utf8_text).unwrap().decode_all(),
        Err(DecodeError::InvalidUtf8)
    );

    // SetRawProp key & string value
    let mut bad_utf8_prop_key = make_raw_header(1, 1, 1);
    bad_utf8_prop_key.push(Opcode::SetRawProp as u8);
    bad_utf8_prop_key.extend_from_slice(&2u32.to_le_bytes());
    bad_utf8_prop_key.extend_from_slice(&(bad_utf8.len() as u16).to_le_bytes());
    bad_utf8_prop_key.extend_from_slice(bad_utf8);
    bad_utf8_prop_key.push(0);
    assert_eq!(
        CommandDecoder::new(&bad_utf8_prop_key)
            .unwrap()
            .decode_all(),
        Err(DecodeError::InvalidUtf8)
    );

    let mut bad_utf8_prop_val = make_raw_header(1, 1, 1);
    bad_utf8_prop_val.push(Opcode::SetRawProp as u8);
    bad_utf8_prop_val.extend_from_slice(&2u32.to_le_bytes());
    bad_utf8_prop_val.extend_from_slice(&1u16.to_le_bytes());
    bad_utf8_prop_val.push(b's');
    bad_utf8_prop_val.push(4); // String tag
    bad_utf8_prop_val.extend_from_slice(&(bad_utf8.len() as u32).to_le_bytes());
    bad_utf8_prop_val.extend_from_slice(bad_utf8);
    assert_eq!(
        CommandDecoder::new(&bad_utf8_prop_val)
            .unwrap()
            .decode_all(),
        Err(DecodeError::InvalidUtf8)
    );

    // DispatchCommand name
    let mut bad_utf8_dispatch = make_raw_header(1, 1, 1);
    bad_utf8_dispatch.push(Opcode::DispatchCommand as u8);
    bad_utf8_dispatch.extend_from_slice(&2u32.to_le_bytes());
    bad_utf8_dispatch.extend_from_slice(&(bad_utf8.len() as u16).to_le_bytes());
    bad_utf8_dispatch.extend_from_slice(bad_utf8);
    bad_utf8_dispatch.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        CommandDecoder::new(&bad_utf8_dispatch)
            .unwrap()
            .decode_all(),
        Err(DecodeError::InvalidUtf8)
    );

    // EventPayload::Text invalid UTF-8
    let mut bad_ev_text = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: 2,
        timestamp_us: 1,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("abc".into()),
    }]);
    let n = bad_ev_text.len();
    bad_ev_text[n - 3..].copy_from_slice(bad_utf8);
    assert_eq!(decode_events(&bad_ev_text), Err(DecodeError::InvalidUtf8));

    // EventPayload::Custom name invalid UTF-8
    let mut bad_ev_custom = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: 2,
        timestamp_us: 1,
        kind: NativeEventKind::Custom,
        payload: EventPayload::Custom {
            name: "abc".into(),
            data: vec![1, 2],
        },
    }]);
    // 4 (count) + 4 (surface) + 4 (node) + 8 (ts) + 1 (kind) + 1 (ptag) + 2 (nlen) = 24
    bad_ev_custom[24..27].copy_from_slice(bad_utf8);
    assert_eq!(decode_events(&bad_ev_custom), Err(DecodeError::InvalidUtf8));
}

#[test]
fn ffi_slice_and_value_helpers_handle_edge_cases() {
    // Empty and null slices must yield empty slice safely.
    assert_eq!(unsafe { NveBufferSlice::EMPTY.as_slice() }, &[]);
    let null_with_len = NveBufferSlice {
        ptr: std::ptr::null(),
        len: 42,
    };
    assert_eq!(unsafe { null_with_len.as_slice() }, &[]);

    let data = [10u8, 20, 30];
    let slice = NveBufferSlice::from_slice(&data);
    assert_eq!(unsafe { slice.as_slice() }, &data);

    assert_eq!(
        NveGestureDisposition::from_u8(1),
        NveGestureDisposition::Claim
    );
    assert_eq!(
        NveGestureDisposition::from_u8(2),
        NveGestureDisposition::PassThrough
    );
    assert_eq!(
        NveGestureDisposition::from_u8(3),
        NveGestureDisposition::DeferToParent
    );
    assert_eq!(
        NveGestureDisposition::from_u8(0),
        NveGestureDisposition::PassThrough
    );
    assert_eq!(
        NveGestureDisposition::from_u8(255),
        NveGestureDisposition::PassThrough
    );

    let loose = NveSizeConstraints::loose(393.0, 852.0);
    assert_eq!(loose.min_w, 0.0);
    assert_eq!(loose.min_h, 0.0);
    assert_eq!(loose.max_w, 393.0);
    assert_eq!(loose.max_h, 852.0);
}

#[test]
fn direct_ffi_value_and_event_zero_copy_pointer_identity() {
    use nve_abi::{
        NveDirectEvent, NveEventBatch, NveEventBatchRef, NveMapEntry, NvePropsRef, NveValue,
        NveValueTag,
    };

    let name_src = "onGestureUpdate";
    let str_src = "pan-active";
    let bytes_src: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
    let vel_elems = [NveValue::f64(12.5), NveValue::f64(-4.25)];
    let map_entries = [
        NveMapEntry::new("state", NveValue::from_str(str_src)),
        NveMapEntry::new("count", NveValue::i64(-9_007_199_254_740_991)),
        NveMapEntry::new("x", NveValue::f64(42.75)),
        NveMapEntry::new("active", NveValue::bool(true)),
        NveMapEntry::new("nothing", NveValue::null()),
        NveMapEntry::new("raw", NveValue::from_bytes(&bytes_src)),
        NveMapEntry::new("velocity", NveValue::array(&vel_elems)),
    ];
    let root_val = NveValue::map(&map_entries);
    let events = [
        NveDirectEvent::press(1, 10, 1000, 12.5, 34.5),
        NveDirectEvent::text_change(1, 11, 2000, str_src),
        NveDirectEvent::custom(1, 12, 3000, name_src, root_val),
    ];

    let batch = NveEventBatch::from_slice(&events);
    let batch_ref = unsafe { NveEventBatchRef::from_raw(batch) };
    assert_eq!(batch_ref.len(), 3);

    // Event 0: Press
    let e0 = batch_ref.get(0).unwrap();
    assert_eq!(e0.kind(), Some(NativeEventKind::Press));
    assert_eq!(e0.x(), 12.5);
    assert_eq!(e0.y(), 34.5);

    // Event 1: TextChange borrows directly without copying
    let e1 = batch_ref.get(1).unwrap();
    assert_eq!(e1.kind(), Some(NativeEventKind::TextChange));
    assert_eq!(e1.text(), str_src);
    assert_eq!(e1.text().as_ptr(), str_src.as_ptr());

    // Event 2: Custom structured value borrows directly with zero copy
    let e2 = batch_ref.get(2).unwrap();
    assert_eq!(e2.kind(), Some(NativeEventKind::Custom));
    assert_eq!(e2.name(), name_src);
    assert_eq!(e2.name().as_ptr(), name_src.as_ptr());

    let val = e2.value();
    assert_eq!(val.tag(), Some(NveValueTag::Map));
    assert_eq!(val.get("state").unwrap().as_str(), Some(str_src));
    assert_eq!(
        val.get("state").unwrap().as_str().unwrap().as_ptr(),
        str_src.as_ptr()
    );
    assert_eq!(
        val.get("count").unwrap().as_i64(),
        Some(-9_007_199_254_740_991)
    );
    assert_eq!(val.get("x").unwrap().as_f64(), Some(42.75));
    assert_eq!(val.get("active").unwrap().as_bool(), Some(true));
    assert!(val.get("nothing").unwrap().is_null());
    let raw = val.get("raw").unwrap().as_bytes().unwrap();
    assert_eq!(raw, &bytes_src);
    assert_eq!(raw.as_ptr(), bytes_src.as_ptr());
    let vel = val.get("velocity").unwrap().as_array().unwrap();
    assert_eq!(vel.len(), 2);
    assert_eq!(vel.get(0).unwrap().as_f64(), Some(12.5));
    assert_eq!(vel.get(1).unwrap().as_f64(), Some(-4.25));
    assert!(val.get("missing").is_none());

    // Direct props slice view
    let props = unsafe { NvePropsRef::from_raw_parts(map_entries.as_ptr(), map_entries.len() as u32) };
    assert_eq!(props.len(), map_entries.len());
    assert_eq!(props.get("state").unwrap().as_str().unwrap().as_ptr(), str_src.as_ptr());
    assert_eq!(props.iter().count(), map_entries.len());
}

#[test]
fn bin_value_roundtrip_depth_guard_and_zero_copy_view() {
    use nve_abi::{
        encode_bin_value_to_vec, BinValueView, NveMapEntry, NveValue, NveValueTag,
        NVE_BIN_VALUE_MAX_DEPTH,
    };

    let nested_arr = [
        NveValue::null(),
        NveValue::bool(false),
        NveValue::i64(i64::MIN),
        NveValue::i64(i64::MAX),
        NveValue::f64(std::f64::consts::PI),
        NveValue::from_str("hello 🚀"),
        NveValue::from_bytes(&[1, 2, 3, 4, 5]),
    ];
    let inner_entries = [
        NveMapEntry::new("nested", NveValue::array(&nested_arr)),
        NveMapEntry::new("empty_arr", NveValue::array(&[])),
        NveMapEntry::new("empty_map", NveValue::map(&[])),
    ];
    let root_entries = [
        NveMapEntry::new("event", NveValue::from_str("onGestureUpdate")),
        NveMapEntry::new("inner", NveValue::map(&inner_entries)),
        NveMapEntry::new("after_nested", NveValue::i64(777)),
    ];
    let root = NveValue::map(&root_entries);
    let encoded = encode_bin_value_to_vec(unsafe { root.as_ref() }).unwrap();

    let view = BinValueView::parse(&encoded).unwrap();
    assert_eq!(view.tag(), NveValueTag::Map);
    assert_eq!(view.map_len(), Some(3));

    // Zero-copy pointer identity: borrowed str slices point directly inside `encoded`.
    let ev_str = view.get("event").unwrap().as_str().unwrap();
    assert_eq!(ev_str, "onGestureUpdate");
    let buf_start = encoded.as_ptr() as usize;
    let buf_end = buf_start + encoded.len();
    assert!(ev_str.as_ptr() as usize >= buf_start && (ev_str.as_ptr() as usize) < buf_end);

    // O(1) skip over nested map lands cleanly on `after_nested`.
    assert_eq!(view.get("after_nested").unwrap().as_i64(), Some(777));

    let inner = view.get("inner").unwrap();
    let arr = inner.get("nested").unwrap();
    assert_eq!(arr.array_len(), Some(7));
    assert!(arr.array_at(0).unwrap().is_null());
    assert_eq!(arr.array_at(1).unwrap().as_bool(), Some(false));
    assert_eq!(arr.array_at(2).unwrap().as_i64(), Some(i64::MIN));
    assert_eq!(arr.array_at(3).unwrap().as_i64(), Some(i64::MAX));
    assert_eq!(arr.array_at(4).unwrap().as_f64(), Some(std::f64::consts::PI));
    assert_eq!(arr.array_at(5).unwrap().as_str(), Some("hello 🚀"));
    assert_eq!(arr.array_at(6).unwrap().as_bytes(), Some(&[1, 2, 3, 4, 5][..]));
    assert!(arr.array_at(7).is_none());
    assert_eq!(inner.get("empty_arr").unwrap().array_len(), Some(0));
    assert_eq!(inner.get("empty_map").unwrap().map_len(), Some(0));

    // Exact depth boundary: depth 64 succeeds, depth 65 fails with MaxDepthExceeded.
    let build_nested_arrays = |depth: usize| -> Vec<u8> {
        let mut cur = vec![NveValueTag::Null as u8];
        for _ in 0..depth {
            let mut next = Vec::with_capacity(cur.len() + 9);
            next.push(NveValueTag::Array as u8);
            next.extend_from_slice(&((4 + cur.len()) as u32).to_le_bytes());
            next.extend_from_slice(&1u32.to_le_bytes());
            next.extend_from_slice(&cur);
            cur = next;
        }
        cur
    };

    let depth_ok = build_nested_arrays(NVE_BIN_VALUE_MAX_DEPTH);
    assert!(BinValueView::parse(&depth_ok).is_ok());

    let depth_too_deep = build_nested_arrays(NVE_BIN_VALUE_MAX_DEPTH + 1);
    assert_eq!(
        BinValueView::parse(&depth_too_deep),
        Err(DecodeError::MaxDepthExceeded)
    );
}

#[test]
fn bin_value_seeded_fuzz_corpus_rejects_corrupt_buffers_without_panics() {
    use nve_abi::{encode_bin_value_to_vec, BinValueView, NveMapEntry, NveValue};

    let arr = [NveValue::i64(1), NveValue::from_str("abc"), NveValue::bool(true)];
    let entries = [
        NveMapEntry::new("k1", NveValue::array(&arr)),
        NveMapEntry::new("k2", NveValue::f64(99.5)),
        NveMapEntry::new("k3", NveValue::from_bytes(&[7, 8, 9])),
    ];
    let root_map = NveValue::map(&entries);
    let baseline = encode_bin_value_to_vec(unsafe { root_map.as_ref() }).unwrap();

    let mut cases = 0usize;

    // 1. Every prefix truncation must fail cleanly.
    for cut in 0..baseline.len() {
        assert!(BinValueView::parse(&baseline[..cut]).is_err());
        cases += 1;
    }

    // 2. Trailing byte must fail with TrailingBytes.
    let mut trailing = baseline.clone();
    trailing.push(0x00);
    assert_eq!(BinValueView::parse(&trailing), Err(DecodeError::TrailingBytes(1)));
    cases += 1;

    // 3. Invalid bool byte, invalid tag, invalid UTF-8 string, oversized array/map counts.
    assert_eq!(BinValueView::parse(&[1, 2]), Err(DecodeError::InvalidEnum("NveBinBool", 2)));
    assert_eq!(BinValueView::parse(&[99]), Err(DecodeError::InvalidEnum("NveValueTag", 99)));
    assert_eq!(
        BinValueView::parse(&[4, 3, 0, 0, 0, 0xFF, 0xFE, 0x80]),
        Err(DecodeError::InvalidUtf8)
    );
    assert_eq!(
        BinValueView::parse(&[6, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]),
        Err(DecodeError::Truncated)
    );
    assert_eq!(
        BinValueView::parse(&[7, 4, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]),
        Err(DecodeError::Truncated)
    );
    cases += 5;

    // 4. 550 seeded random & mutated buffers must never panic or OOM.
    let mut rng = XorShift64::new(0xB1A7_C0DE_5EED_CAFE);
    for _ in 0..300 {
        let len = rng.next_usize(513);
        let buf: Vec<u8> = (0..len).map(|_| rng.next_u8()).collect();
        let _ = BinValueView::parse(&buf);
        cases += 1;
    }
    for _ in 0..250 {
        let mut mut_buf = baseline.clone();
        let edits = 1 + rng.next_usize(6);
        for _ in 0..edits {
            match rng.next_usize(4) {
                0 if !mut_buf.is_empty() => {
                    let idx = rng.next_usize(mut_buf.len());
                    mut_buf[idx] ^= 1u8 << rng.next_usize(8);
                }
                1 if !mut_buf.is_empty() => {
                    let idx = rng.next_usize(mut_buf.len());
                    mut_buf[idx] = rng.next_u8();
                }
                2 if mut_buf.len() > 1 => {
                    let cut = 1 + rng.next_usize(mut_buf.len() - 1);
                    mut_buf.truncate(cut);
                }
                _ => {
                    mut_buf.push(rng.next_u8());
                }
            }
        }
        if let Ok(view) = BinValueView::parse(&mut_buf) {
            // Exercise accessors on any accidentally valid mutation.
            let _ = view.as_str();
            let _ = view.as_bytes();
            let _ = view.get("k1");
            let _ = view.array_at(0);
        }
        cases += 1;
    }
    assert!(cases >= 600);
}
