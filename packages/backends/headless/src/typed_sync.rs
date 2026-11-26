//! Zero-serialization in-process [`DrawList`], [`TypedNativeViewProps`], [`BackgroundFill`],
//! [`SvgPicture`], and [`TypedNativeViewCommand`] setters for [`crate::HeadlessBackend`].

use nve_abi::NodeId;
use nve_components::canvas_ir::DrawList;
use nve_components::native_view::{NativeCommand, TypedNativeViewCommand, TypedNativeViewProps};
use nve_components::paint::BackgroundFill;
use nve_components::svg::SvgPicture;
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

    /// Directly attaches or clears a zero-serialization [`BackgroundFill`] on `id`.
    /// Returns `true` if the node's background fill changed.
    pub fn set_node_background_fill(&mut self, id: NodeId, fill: Option<BackgroundFill>) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        if node.background_fill == fill {
            return false;
        }
        node.background_fill = fill;
        true
    }

    /// Directly attaches or clears a zero-serialization [`SvgPicture`] on `id`.
    /// Returns `true` if the node's SVG picture changed.
    pub fn set_node_svg(&mut self, id: NodeId, svg: Option<SvgPicture>) -> bool {
        let Some(node) = self.nodes.get_mut(id) else {
            return false;
        };
        let unchanged = match (&node.svg, &svg) {
            (None, None) => true,
            (Some(a), Some(b)) => a.ptr_eq(b) || a == b,
            _ => false,
        };
        if unchanged {
            return false;
        }
        node.svg = svg;
        true
    }
}
