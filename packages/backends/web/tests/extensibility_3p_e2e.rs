//! Out-of-tree 3P extensibility E2E test (`MockMapTable` + `MockMapWebSystem`).
//!
//! Proves that an external crate can define a custom `#[derive(NveTable)]` capability with:
//! - typed word-table state patches
//! - typed imperative commands
//! - uniform pointer-free 64-byte `NveEvent` (`EventBatchBuf` + `NVEV` v2 wire codec) across in-process and FFI boundaries
//! - `NVE2` binary wire round-trip into a replica `NodeStore`
//! - zero-decode `NveTableView` publish inside `NveSceneView`
//! - uniform `WebTableSystem` live DOM + SSR plugin integration
//! - deferred DOM creation & `COMPOSITE`-only fast lane on `WebStoreRenderer`
//!
//! Zero edits to `nve-abi`, `nve-store`, `nve-tables`, `nve-engine`, or `nve-backend-web`.

use nve_abi::{EventBatchBuf, EventEnum, EventFlags, SurfaceId, TimestampUs, ValidatedEvents};
use nve_backend_web::{WebStoreRenderer, WebSystemCommitCtx, WebTableSystem};
use nve_tables::tables::{Layout, Transform, Visual};
use nve_engine::StoreBackend;
use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Str, TableSlot};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, EventEnum)]
enum MapEvent {
    RegionChanged = 0,
}

/// Third-party map widget table defined completely outside core crates.
#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(
    name = "acme.map_view",
    storage = "sparse",
    flags = "publish",
    events = MapEvent
)]
struct MockMapTable {
    #[effect(paint)]
    lat: f32,
    #[effect(paint)]
    lng: f32,
    #[effect(paint)]
    zoom: f32,
    #[effect(paint)]
    marker_title: Str,
}

const CMD_FLY_TO: u32 = 1;

fn encode_fly_to(lat: f32, lng: f32, zoom: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&lat.to_le_bytes());
    out.extend_from_slice(&lng.to_le_bytes());
    out.extend_from_slice(&zoom.to_le_bytes());
    out
}

#[derive(Debug, Clone, PartialEq)]
struct MapRegionChanged {
    lat: f32,
    lng: f32,
    zoom: f32,
}

impl MapRegionChanged {
    fn encode(&self) -> Vec<u8> {
        encode_fly_to(self.lat, self.lng, self.zoom)
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 12 {
            return None;
        }
        Some(Self {
            lat: f32::from_le_bytes(bytes[0..4].try_into().ok()?),
            lng: f32::from_le_bytes(bytes[4..8].try_into().ok()?),
            zoom: f32::from_le_bytes(bytes[8..12].try_into().ok()?),
        })
    }
}

struct MockMapWebSystem;

impl WebTableSystem for MockMapWebSystem {
    fn slot(&self) -> TableSlot {
        MockMapTable::slot()
    }

    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        Some("acme-map")
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        if let Some(row) = store.try_row::<MockMapTable>(node) {
            use std::fmt::Write;
            let _ = write!(
                out,
                " data-lat=\"{}\" data-lng=\"{}\" data-zoom=\"{}\"",
                row.lat(),
                row.lng(),
                row.zoom()
            );
            let title = row.marker_title();
            if !title.is_empty() {
                out.push_str(" data-marker=\"");
                out.push_str(title);
                out.push('"');
            }
        }
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        if let Some(row) = store.try_row::<MockMapTable>(node) {
            ctx.set_attr(node, "data-lat", &row.lat().to_string());
            ctx.set_attr(node, "data-lng", &row.lng().to_string());
            ctx.set_attr(node, "data-zoom", &row.zoom().to_string());
            let title = row.marker_title();
            if !title.is_empty() {
                ctx.set_attr(node, "data-marker", title);
            }
        }
    }

    fn handle_command(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        _store: &NodeStore,
        node: NodeId,
        op: u32,
        payload: &[u8],
    ) {
        if op == CMD_FLY_TO && let Some(ev) = MapRegionChanged::decode(payload) {
            ctx.set_attr(node, "data-lat", &ev.lat.to_string());
            ctx.set_attr(node, "data-lng", &ev.lng.to_string());
            ctx.set_attr(node, "data-zoom", &ev.zoom.to_string());
            ctx.events.push_event(
                MockMapTable::KEY,
                SurfaceId::ROOT,
                node,
                TimestampUs(1234),
                MapEvent::RegionChanged,
                EventFlags::EMPTY,
                [ev.lat.to_bits(), ev.lng.to_bits(), ev.zoom.to_bits(), 0, 0, 0],
                &ev.encode(),
            );
        }
    }
}

#[test]
fn third_party_mock_map_e2e_nve2_scene_view_events_and_web_fast_lane() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();

    // Frame 1: create node with Layout + Visual + MockMapTable (deferred realization).
    let map_node;
    {
        let mut tx = store.tx();
        map_node = tx.create();
        tx.set_children(NodeId::ROOT, &[map_node]);
        tx.edit::<Layout>(map_node)
            .width(Length::points(400.0))
            .height(Length::points(300.0));
        tx.edit::<Visual>(map_node).opacity(1.0);
        tx.edit::<MockMapTable>(map_node)
            .lat(37.7749)
            .lng(-122.4194)
            .zoom(12.0)
            .marker_title("SF");
        tx.commit_into(&mut changes);
    }

    // 1. NveTableView zero-decode publish exposes MockMapTable words directly.
    let mut views = Vec::new();
    let mut dirty_scratch = Vec::new();
    store.publish_table_views(&changes, &mut views, &mut dirty_scratch);
    let map_view = views
        .iter()
        .find(|v| ((v.key_lo as u64) | ((v.key_hi as u64) << 32)) == MockMapTable::DESC.key.0)
        .expect("MockMapTable has publish flag");
    assert_eq!(map_view.row_count, 1);
    assert_eq!(map_view.words, MockMapTable::DESC.words() as u32);
    // SAFETY: `map_view.rows` points to 1 valid `MockMapTable` row inside the live scene table.
    unsafe {
        let words = std::slice::from_raw_parts(map_view.rows, MockMapTable::DESC.words());
        // Word 0 is the table's `events_word` (`EventSet<MapEvent>`); user fields start at word 1.
        assert!((f32::from_bits(words[1]) - 37.7749).abs() < 1e-4);
        assert!((f32::from_bits(words[2]) - (-122.4194)).abs() < 1e-4);
        assert!((f32::from_bits(words[3]) - 12.0).abs() < 1e-4);
    }

    // 2. WebStoreRenderer realizes `<acme-map>` on first commit (zero create-then-replace churn).
    let mut renderer = WebStoreRenderer::new().with_table_system(MockMapWebSystem);
    renderer.commit(&store, &changes).unwrap();
    let realized = renderer.realized(map_node).expect("realized on commit");
    assert_eq!(realized.tag, "acme-map");
    assert_eq!(realized.full_style_flushes, 1);
    assert_eq!(realized.composite_fast_flushes, 0);

    let ssr = renderer.render_ssr(&store, NodeId::ROOT);
    assert!(ssr.contains("<acme-map "), "SSR: {ssr}");
    assert!(ssr.contains("data-lat=\"37.7749\""));
    assert!(ssr.contains("data-marker=\"SF\""));

    // 3. NVE2 wire encode -> decode into a replica NodeStore preserves MockMapTable + commands.
    let mut wire_enc = WireEncoder::default();
    let mut frame1_bytes = Vec::new();
    wire_enc.encode(&store, &changes, &mut frame1_bytes);

    // Frame 2: COMPOSITE-only mutation (opacity + translate_x) + FlyTo typed command.
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(map_node).opacity(0.65);
        tx.edit::<Transform>(map_node).translate_x(18.0);
        tx.command::<MockMapTable>(map_node, CMD_FLY_TO, &encode_fly_to(48.8566, 2.3522, 14.0));
        tx.commit_into(&mut changes);
    }
    let mut frame2_bytes = Vec::new();
    wire_enc.encode(&store, &changes, &mut frame2_bytes);

    let prev_full = renderer.full_style_rebuilds;
    renderer.commit(&store, &changes).unwrap();
    // COMPOSITE-only mutation hits the fast lane with ZERO full style rebuilds!
    assert_eq!(renderer.full_style_rebuilds, prev_full);
    assert_eq!(renderer.composite_fast_lane_hits, 1);
    assert_eq!(renderer.realized(map_node).unwrap().composite_fast_flushes, 1);
    assert!((renderer.realized(map_node).unwrap().opacity - 0.65).abs() < 1e-6);

    // 4. Uniform 64 B NveEvent in-process batch + cross-ABI NveEventBatch + NVEV v2 wire roundtrip.
    let expected = MapRegionChanged {
        lat: 48.8566,
        lng: 2.3522,
        zoom: 14.0,
    };
    let validated = renderer.events.as_validated();
    assert_eq!(validated.len(), 1);
    let ev = validated.get(0).unwrap();
    assert_eq!(ev.node(), map_node);
    assert_eq!(ev.table(), MockMapTable::KEY);
    assert_eq!(ev.as_event::<MapEvent>(MockMapTable::KEY), Some(MapEvent::RegionChanged));
    assert_eq!(MapRegionChanged::decode(ev.bytes()), Some(expected.clone()));

    let encoded_wire = renderer.events.to_wire_bytes();
    let decoded_wire = EventBatchBuf::decode_wire(&encoded_wire).unwrap();
    assert_eq!(decoded_wire.len(), 1);
    let dec_ev = decoded_wire.as_validated().get(0).unwrap();
    assert_eq!(dec_ev.as_event::<MapEvent>(MockMapTable::KEY), Some(MapEvent::RegionChanged));
    assert_eq!(MapRegionChanged::decode(dec_ev.bytes()), Some(expected.clone()));

    // Zero-copy FFI NveEventBatch ingress validation round-trip.
    // SAFETY: `renderer.events` outlives `ffi_view`.
    let ffi_view = unsafe { ValidatedEvents::try_from_raw(renderer.events.as_raw_batch()) }.unwrap();
    assert_eq!(ffi_view.len(), 1);
    assert_eq!(ffi_view.get(0).unwrap().timestamp(), TimestampUs(1234));
    renderer.events.clear();

    // 5. Verify NVE2 replica store receives both frames identically (including MockMapTable + command).
    let mut replica = NodeStore::replica();
    let mut replica_changes = ChangeSet::default();
    let mut dec = WireDecoder::default();
    {
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &frame1_bytes).unwrap();
        rtx.commit_into(&mut replica_changes);
    }
    let r_row = replica.try_row::<MockMapTable>(map_node).unwrap();
    assert!((r_row.lat() - 37.7749).abs() < 1e-4);
    assert_eq!(r_row.marker_title(), "SF");

    {
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &frame2_bytes).unwrap();
        rtx.commit_into(&mut replica_changes);
    }
    assert_eq!(replica_changes.commands.len(), 1);
    assert_eq!(replica_changes.commands[0].table, MockMapTable::slot());
    assert_eq!(replica_changes.commands[0].op, CMD_FLY_TO);

    let mut replica_web = WebStoreRenderer::new().with_table_system(MockMapWebSystem);
    replica_web.commit(&replica, &replica_changes).unwrap();
    let r_val = replica_web.events.as_validated();
    assert_eq!(r_val.len(), 1);
    let rev = r_val.get(0).unwrap();
    assert_eq!(rev.as_event::<MapEvent>(MockMapTable::KEY), Some(MapEvent::RegionChanged));
    assert_eq!(MapRegionChanged::decode(rev.bytes()), Some(expected));
}
