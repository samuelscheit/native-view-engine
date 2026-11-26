//! Out-of-tree 3P extensibility E2E test for `nve-backend-skia` (`SkiaTableSystem`).
//!
//! Proves that an external crate can define a custom `#[derive(NveTable)]` capability, paint onto
//! `PixelBuffer` via `SkiaTableRegistry`, handle imperative commands, emit zero-serialization
//! typed `TableEventBus` events, and round-trip over `NVE2` with zero modifications to `nve-backend-skia`.

use nve_abi::ColorRgba;
use nve_backend_skia::{PixelBuffer, SkiaSystemPaintCtx, SkiaTableRegistry, SkiaTableSystem};
use nve_engine::LayoutRect;
use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, Rgba, TableEventBus, TableSlot};

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "acme.skia_map", storage = "sparse", flags = "publish")]
struct MockSkiaMapTable {
    #[effect(paint)]
    pin_color: Rgba,
    #[effect(paint)]
    pin_x: f32,
    #[effect(paint)]
    pin_y: f32,
}

const EVENT_PIN_TAPPED: u32 = 7;

#[derive(Debug, Clone, PartialEq)]
struct PinTapped {
    pin_id: u32,
}

struct MockSkiaMapSystem;

impl SkiaTableSystem for MockSkiaMapSystem {
    fn slot(&self) -> TableSlot {
        MockSkiaMapTable::slot()
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
        let Some(row) = store.try_row::<MockSkiaMapTable>(node) else {
            return;
        };
        let px = (ctx.rect.x + row.pin_x()).round() as i32;
        let py = (ctx.rect.y + row.pin_y()).round() as i32;
        ctx.buffer
            .blend_pixel(px, py, ColorRgba::from_u32(row.pin_color().0), ctx.opacity);
    }

    fn handle_command(
        &self,
        _store: &NodeStore,
        node: NodeId,
        op: u32,
        payload: &[u8],
        events: &mut TableEventBus,
    ) {
        if op == 1
            && let Ok(bytes) = <[u8; 4]>::try_from(payload)
        {
            let pin_id = u32::from_le_bytes(bytes);
            events.emit::<MockSkiaMapTable, _>(1, node, 42, EVENT_PIN_TAPPED, PinTapped { pin_id });
        }
    }
}

#[test]
fn third_party_skia_table_system_paints_handles_commands_and_roundtrips_nve2() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node;
    {
        let mut tx = store.tx();
        node = tx.create();
        tx.set_children(NodeId::ROOT, &[node]);
        tx.edit::<MockSkiaMapTable>(node)
            .pin_color(Rgba::rgb(250, 100, 20))
            .pin_x(10.0)
            .pin_y(12.0);
        tx.command::<MockSkiaMapTable>(node, 1, &99u32.to_le_bytes());
        tx.commit_into(&mut changes);
    }

    // Round-trip over NVE2 into a replica store.
    let mut enc = WireEncoder::default();
    let mut bytes = Vec::new();
    enc.encode(&store, &changes, &mut bytes);
    let mut replica = NodeStore::replica();
    let mut replica_changes = ChangeSet::default();
    {
        let mut rtx = replica.tx();
        WireDecoder::default().apply(&mut rtx, &bytes).unwrap();
        rtx.commit_into(&mut replica_changes);
    }

    let mut registry = SkiaTableRegistry::new();
    registry.register(MockSkiaMapSystem);

    let mut buf = PixelBuffer::new(32, 32);
    buf.clear(ColorRgba::BLACK);
    let mut events = TableEventBus::new();
    {
        let mut ctx = SkiaSystemPaintCtx {
            buffer: &mut buf,
            rect: LayoutRect {
                x: 2.0,
                y: 3.0,
                width: 24.0,
                height: 24.0,
            },
            clip_rect: None,
            scale_factor: 1.0,
            opacity: 1.0,
            inherited_text_color: Rgba::WHITE,
            events: &mut events,
        };
        assert_eq!(registry.paint_node(&mut ctx, &replica, node), 1);
    }
    assert_eq!(buf.get_rgb(12, 15), Some((250, 100, 20)));
    assert_eq!(buf.get_rgb(0, 0), Some((0, 0, 0)));

    for cmd in &replica_changes.commands {
        assert!(registry.dispatch_command(
            &replica,
            cmd.node,
            cmd.table,
            cmd.op,
            replica_changes.command_payload(cmd),
            &mut events,
        ));
    }
    let drained: Vec<_> = events.drain().collect();
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].node, node);
    assert_eq!(drained[0].event_id, EVENT_PIN_TAPPED);
    assert_eq!(drained[0].payload::<PinTapped>(), Some(&PinTapped { pin_id: 99 }));
}
