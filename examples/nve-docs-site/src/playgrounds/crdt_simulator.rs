//! Interactive `"crdt-simulator"` demo exercising real `nve_interop::crdt` primitives
//! (`LwwRegister`, `PnCounter`, `OrSet`, `HlcClock`, `NveCrdtDelta` binary wire format).

use nve_dioxus::prelude::*;
use nve_interop::crdt::{
    NveCrdtDelta, CrdtRecord, CrdtValue, HlcClock, LwwRegister, OpId, OrSet, PeerId, PnCounter,
    VersionVector,
};
use nve_ui::{Badge, BadgeVariant, Button, ButtonSize, ButtonVariant, Switch, use_theme};

pub const CRDT_SIMULATOR_SOURCE: &str = r#"use nve_interop::crdt::{
    NveCrdtDelta, CrdtRecord, CrdtValue, HlcClock, LwwRegister, OrSet, PeerId, PnCounter,
};

let mut peer_a = PeerReplica::new(PeerId(1));
let mut peer_b = PeerReplica::new(PeerId(2));
peer_a.edit_title("Zero-Copy Engine");
peer_b.vote_delta(1);

// Encode binary NveCrdtDelta with XXH64 integrity checksum & merge deterministically:
let packet_bytes = peer_a.encode_delta_packet();
let decoded = NveCrdtDelta::decode(&packet_bytes)?;
peer_b.apply_packet(&decoded);"#;

const PEER_A: PeerId = PeerId(1);
const PEER_B: PeerId = PeerId(2);

#[derive(Clone)]
struct PeerReplica {
    peer_id: PeerId,
    clock: HlcClock,
    seq: u64,
    vv: VersionVector,
    title: LwwRegister<String>,
    votes: PnCounter,
    tags: OrSet<String>,
    pending_records: Vec<CrdtRecord>,
}

impl PeerReplica {
    fn new(peer_id: PeerId) -> Self {
        let mut replica = Self {
            peer_id,
            clock: HlcClock::new(peer_id),
            seq: 0,
            vv: VersionVector::new(),
            title: LwwRegister::default(),
            votes: PnCounter::new(),
            tags: OrSet::new(),
            pending_records: Vec::new(),
        };
        let init_ts = replica.clock.tick(1_700_000_000_000);
        replica.title.set("Native View Engine".to_string(), init_ts);
        replica
    }

    fn next_op(&mut self, wall_ms: u64) -> (OpId, nve_interop::crdt::HlcTimestamp) {
        self.seq += 1;
        let op = OpId::new(self.peer_id, self.seq);
        self.vv.set_max(self.peer_id, self.seq);
        let ts = self.clock.tick(wall_ms);
        (op, ts)
    }

    fn set_title(&mut self, next: String, wall_ms: u64) {
        let (op_id, timestamp) = self.next_op(wall_ms);
        self.title.set(next.clone(), timestamp);
        self.pending_records.push(CrdtRecord::LwwSet {
            namespace: 1,
            entity_id: 1,
            field_id: 1,
            op_id,
            timestamp,
            value: CrdtValue::Utf8(next),
        });
    }

    fn bump_votes(&mut self, delta: i64, wall_ms: u64) {
        let (op_id, _) = self.next_op(wall_ms);
        if delta >= 0 {
            self.votes.increment(self.peer_id, delta as u64);
        } else {
            self.votes.decrement(self.peer_id, delta.unsigned_abs());
        }
        let (pos, neg) = self.votes.peer_counts(self.peer_id);
        self.pending_records.push(CrdtRecord::PnCounterSet {
            namespace: 1,
            entity_id: 1,
            peer: self.peer_id,
            op_id,
            pos,
            neg,
        });
    }

    fn add_tag(&mut self, tag: &str, wall_ms: u64) {
        let (op_id, _) = self.next_op(wall_ms);
        self.tags.add(tag.to_string(), op_id);
        self.pending_records.push(CrdtRecord::OrSetAddString {
            namespace: 2,
            key: tag.to_string(),
            op_id,
        });
    }

    fn drain_crdt_packet(&mut self) -> Vec<u8> {
        let records = std::mem::take(&mut self.pending_records);
        let pkt = NveCrdtDelta::new(self.peer_id, self.vv.clone(), records);
        pkt.encode().unwrap_or_default()
    }

    fn apply_crdt_bytes(&mut self, bytes: &[u8]) -> usize {
        let Ok(pkt) = NveCrdtDelta::decode(bytes) else {
            return 0;
        };
        let applied = pkt.records.len();
        self.vv.merge(&pkt.version_vector);
        for rec in pkt.records {
            match rec {
                CrdtRecord::LwwSet {
                    timestamp,
                    value: CrdtValue::Utf8(s),
                    ..
                } => {
                    let _ = self.clock.receive(timestamp.physical_ms, timestamp);
                    self.title.set(s, timestamp);
                }
                CrdtRecord::PnCounterSet {
                    peer, pos, neg, ..
                } => {
                    let mut remote = PnCounter::new();
                    remote.increment(peer, pos);
                    remote.decrement(peer, neg);
                    self.votes.merge(&remote);
                }
                CrdtRecord::OrSetAddString { key, op_id, .. } => {
                    self.tags.add(key, op_id);
                }
                CrdtRecord::OrSetRemove {
                    removed_tag,
                    remove_op,
                    ..
                } => {
                    self.tags.remove_tag(removed_tag, remove_op);
                }
                _ => {}
            }
        }
        applied
    }

    fn tags_csv(&self) -> String {
        let items: Vec<&str> = self.tags.iter().map(String::as_str).collect();
        if items.is_empty() {
            "none".to_string()
        } else {
            items.join(", ")
        }
    }
}

#[derive(Clone)]
struct CrdtSimState {
    peer_a: PeerReplica,
    peer_b: PeerReplica,
    partitioned: bool,
    sim_clock_ms: u64,
    last_sync_bytes: usize,
    sync_rounds: u32,
    edit_step: usize,
}

impl CrdtSimState {
    fn new() -> Self {
        Self {
            peer_a: PeerReplica::new(PEER_A),
            peer_b: PeerReplica::new(PEER_B),
            partitioned: true,
            sim_clock_ms: 1_700_000_001_000,
            last_sync_bytes: 0,
            sync_rounds: 0,
            edit_step: 0,
        }
    }

    fn is_converged(&self) -> bool {
        self.peer_a.title.value() == self.peer_b.title.value()
            && self.peer_a.votes.value() == self.peer_b.votes.value()
            && self.peer_a.tags == self.peer_b.tags
    }

    fn next_ms(&mut self) -> u64 {
        self.sim_clock_ms += 250;
        self.sim_clock_ms
    }

    fn sync_now(&mut self) {
        let a_to_b = self.peer_a.drain_crdt_packet();
        let b_to_a = self.peer_b.drain_crdt_packet();
        let total = a_to_b.len() + b_to_a.len();
        self.peer_b.apply_crdt_bytes(&a_to_b);
        self.peer_a.apply_crdt_bytes(&b_to_a);
        self.last_sync_bytes = total;
        self.sync_rounds += 1;
    }

    fn maybe_auto_sync(&mut self) {
        if !self.partitioned {
            self.sync_now();
        }
    }
}

#[component]
pub fn CrdtSimulatorDemo() -> Element {
    let theme = use_theme();
    let mut sim = use_signal(CrdtSimState::new);

    let snap = sim.read();
    let converged = snap.is_converged();
    let partitioned = snap.partitioned;
    let last_bytes = snap.last_sync_bytes;
    let rounds = snap.sync_rounds;
    let a_title = snap.peer_a.title.value().clone();
    let a_votes = snap.peer_a.votes.value();
    let a_tags = snap.peer_a.tags_csv();
    let a_pending = snap.peer_a.pending_records.len();
    let b_title = snap.peer_b.title.value().clone();
    let b_votes = snap.peer_b.votes.value();
    let b_tags = snap.peer_b.tags_csv();
    let b_pending = snap.peer_b.pending_records.len();
    drop(snap);

    rsx! {
        view {
            test_id: "pg-crdt-root",
            nve_style: TypedStyle::col().width_pct(100.0).gap(12.0),

            // Convergence & NveCrdtDelta binary wire telemetry bar
            view {
                nve_style: TypedStyle::row()
                    .width_pct(100.0)
                    .justify(FlexJustify::SpaceBetween)
                    .align(FlexAlign::Center)
                    .flex_wrap(FlexWrap::Wrap)
                    .gap(8.0),
                view {
                    nve_style: TypedStyle::row()
                        .flex_wrap(FlexWrap::Wrap)
                        .gap(6.0)
                        .align(FlexAlign::Center),
                    Badge {
                        test_id: Some("pg-crdt-convergence".to_string()),
                        label: if converged { "Converged ✓".to_string() } else { "Diverged (Offline)".to_string() },
                        variant: if converged { BadgeVariant::Success } else { BadgeVariant::Warning },
                    }
                    Badge {
                        test_id: Some("pg-crdt-bytes".to_string()),
                        label: format!("Last CRDT Sync: {last_bytes} B ({rounds}x)"),
                        variant: BadgeVariant::Accent,
                    }
                }
                Switch {
                    test_id: Some("pg-crdt-partition".to_string()),
                    checked: partitioned,
                    label: Some("Network Partition".to_string()),
                    on_change: move |p| {
                        let mut s = sim.write();
                        s.partitioned = p;
                        if !p {
                            s.sync_now();
                        }
                    },
                }
            }

            // Dual Peer A / Peer B cards
            view {
                nve_style: TypedStyle::row()
                    .width_pct(100.0)
                    .flex_wrap(FlexWrap::Wrap)
                    .gap(10.0),

                // Peer A
                view {
                    test_id: "pg-crdt-peer-a",
                    nve_style: TypedStyle::col()
                        .flex_grow(1.0)
                        .min_width_px(220.0)
                        .gap(8.0)
                        .padding(12.0)
                        .radius(8.0)
                        .bg(theme.surface.raised)
                        .border(1.0, theme.border.default),
                    view {
                        nve_style: TypedStyle::row()
                            .justify(FlexJustify::SpaceBetween)
                            .align(FlexAlign::Center),
                        text {
                            nve_style: TypedStyle::new()
                                .font_size(13.0)
                                .font_weight(700)
                                .color(theme.status.accent.text),
                            "Peer A (iOS · id=1)"
                        }
                        Badge {
                            label: format!("Queue: {a_pending}"),
                            variant: BadgeVariant::Neutral,
                        }
                    }
                    text {
                        test_id: "pg-crdt-a-title",
                        nve_style: TypedStyle::new().font_size(12.0).color(theme.text.primary),
                        "LWW Title: {a_title}"
                    }
                    text {
                        test_id: "pg-crdt-a-votes",
                        nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                        "PN-Counter: {a_votes} · OR-Set: [{a_tags}]"
                    }
                    view {
                        nve_style: TypedStyle::row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                        Button {
                            test_id: Some("pg-crdt-a-edit".to_string()),
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                let mut s = sim.write();
                                s.edit_step += 1;
                                let step = s.edit_step;
                                let ms = s.next_ms();
                                s.peer_a.set_title(format!("Peer A Draft v{step}"), ms);
                                s.peer_a.bump_votes(1, ms + 1);
                                s.peer_a.add_tag("rust", ms + 2);
                                s.maybe_auto_sync();
                            },
                            "A: Edit +1 +rust"
                        }
                    }
                }

                // Peer B
                view {
                    test_id: "pg-crdt-peer-b",
                    nve_style: TypedStyle::col()
                        .flex_grow(1.0)
                        .min_width_px(220.0)
                        .gap(8.0)
                        .padding(12.0)
                        .radius(8.0)
                        .bg(theme.surface.raised)
                        .border(1.0, theme.border.default),
                    view {
                        nve_style: TypedStyle::row()
                            .justify(FlexJustify::SpaceBetween)
                            .align(FlexAlign::Center),
                        text {
                            nve_style: TypedStyle::new()
                                .font_size(13.0)
                                .font_weight(700)
                                .color(theme.status.success.text),
                            "Peer B (Web · id=2)"
                        }
                        Badge {
                            label: format!("Queue: {b_pending}"),
                            variant: BadgeVariant::Neutral,
                        }
                    }
                    text {
                        test_id: "pg-crdt-b-title",
                        nve_style: TypedStyle::new().font_size(12.0).color(theme.text.primary),
                        "LWW Title: {b_title}"
                    }
                    text {
                        test_id: "pg-crdt-b-votes",
                        nve_style: TypedStyle::new().font_size(12.0).color(theme.text.secondary),
                        "PN-Counter: {b_votes} · OR-Set: [{b_tags}]"
                    }
                    view {
                        nve_style: TypedStyle::row().flex_wrap(FlexWrap::Wrap).gap(6.0),
                        Button {
                            test_id: Some("pg-crdt-b-edit".to_string()),
                            variant: ButtonVariant::Secondary,
                            size: ButtonSize::Sm,
                            on_press: move |_| {
                                let mut s = sim.write();
                                s.edit_step += 1;
                                let step = s.edit_step;
                                let ms = s.next_ms();
                                s.peer_b.set_title(format!("Peer B Release v{step}"), ms);
                                s.peer_b.bump_votes(2, ms + 1);
                                s.peer_b.add_tag("wasm", ms + 2);
                                s.maybe_auto_sync();
                            },
                            "B: Edit +2 +wasm"
                        }
                    }
                }
            }

            // Binary NveCrdtDelta Sync / Heal action bar
            view {
                nve_style: TypedStyle::row()
                    .width_pct(100.0)
                    .gap(8.0)
                    .align(FlexAlign::Center),
                Button {
                    test_id: Some("pg-crdt-sync".to_string()),
                    variant: ButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        sim.write().sync_now();
                    },
                    "Sync Binary CRDT Delta / Heal"
                }
                Button {
                    test_id: Some("pg-crdt-reset".to_string()),
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    on_press: move |_| {
                        sim.set(CrdtSimState::new());
                    },
                    "Reset Peers"
                }
            }
        }
    }
}
