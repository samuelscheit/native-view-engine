//! Zero-serialization in-process [`DrawList`], [`TypedNativeViewProps`], and
//! [`TypedNativeViewCommand`] synchronization for [`crate::HeadlessBackend`].

use nve_abi::hash::HashMap;
use nve_abi::NodeId;
use nve_components::canvas_ir::DrawList;
use nve_components::native_view::{NativeCommand, TypedNativeViewCommand, TypedNativeViewProps};
use nve_engine::LayoutEngine;

use crate::HeadlessBackend;

impl<L: LayoutEngine> HeadlessBackend<L> {
    /// Directly attaches or clears a zero-serialization [`DrawList`] (`Arc<[CanvasOp]>`) on `id`.
    /// Returns `true` if the node's draw list changed.
    pub fn set_node_draw_list(&mut self, id: NodeId, draw_list: Option<DrawList>) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        let unchanged = match (&node.draw_list, &draw_list) {
            (None, None) => true,
            (Some(a), Some(b)) => a.ptr_eq(b) || a == b,
            _ => false,
        };
        if unchanged {
            return false;
        }
        node.draw_list = draw_list;
        true
    }

    /// Bulk-synchronizes zero-serialization [`DrawList`]s from any producer
    /// (e.g. `DioxusSurface::iter_draw_lists`), clearing stale entries from nodes no longer present.
    /// Returns `true` if any node's draw list changed.
    pub fn sync_draw_lists_from<'a>(
        &mut self,
        entries: impl IntoIterator<Item = (NodeId, &'a DrawList)>,
    ) -> bool {
        let map: HashMap<NodeId, &'a DrawList> = entries.into_iter().collect();
        let mut changed = false;
        for node in self.nodes.values_mut() {
            match map.get(&node.id).copied() {
                Some(next) => {
                    if !node
                        .draw_list
                        .as_ref()
                        .is_some_and(|prev| prev.ptr_eq(next) || prev == next)
                    {
                        node.draw_list = Some(next.clone());
                        changed = true;
                    }
                }
                None => {
                    if node.draw_list.take().is_some() {
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// Directly attaches or clears zero-serialization [`TypedNativeViewProps`] on `id`.
    pub fn set_native_view_props(
        &mut self,
        id: NodeId,
        props: Option<TypedNativeViewProps>,
    ) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        if node.native_view_props == props {
            return false;
        }
        node.native_view_props = props;
        true
    }

    /// Bulk-synchronizes zero-serialization [`TypedNativeViewProps`] from any producer.
    pub fn sync_native_views_from<'a>(
        &mut self,
        entries: impl IntoIterator<Item = (NodeId, &'a TypedNativeViewProps)>,
    ) -> bool {
        let map: HashMap<NodeId, &'a TypedNativeViewProps> = entries.into_iter().collect();
        let mut changed = false;
        for node in self.nodes.values_mut() {
            match map.get(&node.id).copied() {
                Some(next) => {
                    if node.native_view_props.as_ref() != Some(next) {
                        node.native_view_props = Some(next.clone());
                        changed = true;
                    }
                }
                None => {
                    if node.native_view_props.take().is_some() {
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// Records a zero-serialization [`NativeCommand`] dispatched directly in-process.
    pub fn dispatch_native_command(&mut self, node_id: NodeId, cmd: NativeCommand) {
        self.typed_dispatched_commands
            .push((node_id, TypedNativeViewCommand::new(cmd)));
    }

    /// Records a zero-serialization [`TypedNativeViewCommand`] dispatched directly in-process.
    pub fn dispatch_typed_native_command(
        &mut self,
        node_id: NodeId,
        cmd: TypedNativeViewCommand,
    ) {
        self.typed_dispatched_commands.push((node_id, cmd));
    }

    /// Returns all zero-serialization typed commands dispatched in-process.
    #[inline]
    pub fn typed_dispatched_commands(&self) -> &[(NodeId, TypedNativeViewCommand)] {
        &self.typed_dispatched_commands
    }
}
