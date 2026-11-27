//! Web DOM & SSR adapter for `nve-video` (`WebTableSystem`).

use nve_backend_web::html::write_attr;
use nve_backend_web::{is_safe_web_url, WebSystemCommitCtx, WebTableSystem};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};
use nve_video::{VideoPlayer, VideoPlayerCommand};

/// `WebTableSystem` plugin for [`VideoPlayer`] (`nve.video_player`).
#[derive(Debug, Default, Clone, Copy)]
pub struct VideoWebSystem;

impl WebTableSystem for VideoWebSystem {
    fn slot(&self) -> TableSlot {
        VideoPlayer::slot()
    }

    fn priority(&self) -> u8 {
        110
    }

    fn tag_name<'a>(&self, _store: &'a NodeStore, _node: NodeId) -> Option<&'a str> {
        Some("video")
    }

    fn write_ssr_attrs(&self, store: &NodeStore, node: NodeId, out: &mut String) {
        let Some(row) = store.try_row::<VideoPlayer>(node) else {
            return;
        };
        let uri = row.source_uri();
        if is_safe_web_url(uri) {
            write_attr("src", uri, out);
        }
        if row.autoplay() {
            out.push_str(" autoplay");
        }
        if row.muted() {
            out.push_str(" muted");
        }
        if row.looping() {
            out.push_str(" loop");
        }
        if row.controls() {
            out.push_str(" controls");
        }
        out.push_str(" playsinline");
    }

    fn commit_node(
        &self,
        ctx: &mut WebSystemCommitCtx<'_>,
        store: &NodeStore,
        node: NodeId,
        _dirty_words: u32,
    ) {
        let Some(row) = store.try_row::<VideoPlayer>(node) else {
            return;
        };
        let uri = row.source_uri();
        if is_safe_web_url(uri) {
            ctx.set_attr(node, "src", uri);
        }
        ctx.set_attr(node, "playsinline", "");
        if row.controls() {
            ctx.set_attr(node, "controls", "");
        } else {
            ctx.remove_attr(node, "controls");
        }
        if row.muted() {
            ctx.set_attr(node, "muted", "");
        } else {
            ctx.remove_attr(node, "muted");
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
        if let Some(cmd) = VideoPlayerCommand::decode(op, payload) {
            match cmd {
                VideoPlayerCommand::Play => ctx.set_attr(node, "data-nve-playing", "1"),
                VideoPlayerCommand::Pause => ctx.set_attr(node, "data-nve-playing", "0"),
                VideoPlayerCommand::SeekMs(ms) => {
                    ctx.set_attr(node, "data-nve-seek-ms", &ms.to_string())
                }
                VideoPlayerCommand::SetVolume(vol) => {
                    ctx.set_attr(node, "data-nve-volume", &format!("{:.2}", vol.clamp(0.0, 1.0)))
                }
            }
        }
    }
}
