//! Standalone hardware-accelerated video player capability (`packages/modules/video`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use nve_store::{NodeId, NveTable, Str, Transaction};

/// Native video player table (`nve.video_player`).
#[derive(NveTable)]
#[nve_table(name = "nve.video_player", storage = "sparse", flags = "publish")]
pub struct VideoPlayer {
    #[effect(realize, paint)]
    pub source_uri: Str,
    #[effect(paint)]
    #[bits(1)]
    pub autoplay: bool,
    #[effect(paint)]
    #[bits(1)]
    pub muted: bool,
    #[effect(paint)]
    #[bits(1)]
    pub looping: bool,
    #[effect(paint)]
    #[bits(1)]
    #[default(true)]
    pub controls: bool,
}

/// Typed imperative commands dispatched on [`VideoPlayer`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoPlayerCommand {
    Play,
    Pause,
    SeekMs(u64),
    SetVolume(f32),
}

impl VideoPlayerCommand {
    pub const OP_PLAY: u32 = 1;
    pub const OP_PAUSE: u32 = 2;
    pub const OP_SEEK_MS: u32 = 3;
    pub const OP_SET_VOLUME: u32 = 4;

    pub fn dispatch(self, tx: &mut Transaction<'_>, node: NodeId) {
        match self {
            Self::Play => tx.command::<VideoPlayer>(node, Self::OP_PLAY, &[]),
            Self::Pause => tx.command::<VideoPlayer>(node, Self::OP_PAUSE, &[]),
            Self::SeekMs(ms) => {
                tx.command::<VideoPlayer>(node, Self::OP_SEEK_MS, &ms.to_le_bytes())
            }
            Self::SetVolume(vol) => {
                tx.command::<VideoPlayer>(node, Self::OP_SET_VOLUME, &vol.to_le_bytes())
            }
        }
    }

    pub fn decode(op: u32, payload: &[u8]) -> Option<Self> {
        match op {
            Self::OP_PLAY if payload.is_empty() => Some(Self::Play),
            Self::OP_PAUSE if payload.is_empty() => Some(Self::Pause),
            Self::OP_SEEK_MS if payload.len() == 8 => {
                Some(Self::SeekMs(u64::from_le_bytes(payload.try_into().ok()?)))
            }
            Self::OP_SET_VOLUME if payload.len() == 4 => {
                Some(Self::SetVolume(f32::from_le_bytes(payload.try_into().ok()?)))
            }
            _ => None,
        }
    }
}

/// Typed `TableEvent` IDs emitted back from video player hosts.
pub mod events {
    pub const READY: u32 = 1;
    pub const PROGRESS: u32 = 2;
    pub const ENDED: u32 = 3;
    pub const ERROR: u32 = 4;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, NodeStore};

    #[test]
    fn video_table_and_commands_roundtrip() {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<VideoPlayer>(n)
            .source_uri("https://cdn.nve.dev/demo.mp4")
            .autoplay(true)
            .muted(true);
        VideoPlayerCommand::SeekMs(12_500).dispatch(&mut tx, n);
        VideoPlayerCommand::SetVolume(0.5).dispatch(&mut tx, n);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let row = store.try_row::<VideoPlayer>(n).unwrap();
        assert_eq!(row.source_uri(), "https://cdn.nve.dev/demo.mp4");
        assert!(row.controls());
        assert!(row.autoplay());
        assert_eq!(
            VideoPlayerCommand::decode(
                changes.commands[0].op,
                changes.command_payload(&changes.commands[0])
            ),
            Some(VideoPlayerCommand::SeekMs(12_500))
        );
        assert_eq!(
            VideoPlayerCommand::decode(
                changes.commands[1].op,
                changes.command_payload(&changes.commands[1])
            ),
            Some(VideoPlayerCommand::SetVolume(0.5))
        );
    }
}
