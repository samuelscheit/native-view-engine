//! Per-frame `NveSceneTable` mirror of a `WebBackend` tree (feature `scene-view`), shared by
//! [`crate::WebAppSession`] and [`crate::PairedWebSession`].

use nve_abi::{
    component_kind_to_scene_u8, ComponentKind, Dimension, EventMask, FrameId, NodeId,
    NveRenderNode, NveSceneRef, NveSceneTable, NveSceneView, SceneNodeInput, NVE_NODE_CLIP_BOUNDS,
    NVE_NODE_INTERACTIVE, NVE_NODE_LEAF_TEXT,
};
use nve_backend_web::{MirrorElement, WebBackend};
use nve_engine::RenderBackend;

fn mirror_to_scene_input<'a>(
    backend: &WebBackend,
    node_id: NodeId,
    el: &'a MirrorElement,
) -> SceneNodeInput<'a> {
    let mut flags = 0u8;
    if el.kind == ComponentKind::Text && el.children.is_empty() {
        flags |= NVE_NODE_LEAF_TEXT;
    }
    if el.kind == ComponentKind::Pressable || el.event_mask != EventMask::NONE {
        flags |= NVE_NODE_INTERACTIVE;
    }
    if el.kind == ComponentKind::ScrollView {
        flags |= NVE_NODE_CLIP_BOUNDS;
    }

    let (x, y, width, height) = if let Some(rect) = backend.layout_of(node_id) {
        (rect.x, rect.y, rect.width, rect.height)
    } else {
        let w = match el.layout.width {
            Dimension::Points(v) => v,
            _ => 0.0,
        };
        let h = match el.layout.height {
            Dimension::Points(v) => v,
            _ => 0.0,
        };
        (0.0, 0.0, w, h)
    };

    SceneNodeInput {
        node_id,
        kind: component_kind_to_scene_u8(el.kind as u16),
        flags,
        opacity_u8: NveRenderNode::encode_opacity_u8(el.style.opacity),
        x,
        y,
        width,
        height,
        bg_rgba: el.style.background.to_u32(),
        text_rgba: el.style.text_color.to_u32(),
        border_rgba: el.style.border_color.to_u32(),
        border_radius: el.style.border_radius,
        border_width: el.style.border_width,
        font_size: el.style.font_size,
        z_index: el.style.z_index,
        text: el.text.as_deref().unwrap_or(""),
        children: &el.children,
    }
}


/// Per-frame `NveSceneTable` mirror of the `WebBackend` tree behind a stable `NveSceneView`
/// header in linear memory (`nve_web_scene_view_ptr`, zero-copy JS inspection).
pub struct SceneMirror {
    table: NveSceneTable,
    view: Box<NveSceneView>,
    sort_scratch: Vec<NodeId>,
}

impl SceneMirror {
    pub(crate) fn new() -> Self {
        let table = NveSceneTable::new();
        let view = Box::new(table.as_view());
        Self { table, view, sort_scratch: Vec::new() }
    }

    /// The incremental [`NveSceneTable`] mirroring the live `WebBackend` tree.
    pub fn table(&self) -> &NveSceneTable {
        &self.table
    }

    /// A zero-copy C ABI [`NveSceneView`] pointing directly into the scene slabs.
    pub fn view(&self) -> NveSceneView {
        *self.view
    }

    /// Stable pointer to the [`NveSceneView`] header in linear memory (suitable for
    /// `NveSceneViewReader.fromWasm32Pointer(memory.buffer, ptr)` on `wasm32`).
    pub fn view_ptr(&self) -> *const NveSceneView {
        &*self.view as *const NveSceneView
    }

    /// A safe zero-copy borrowed [`NveSceneRef`] over the current snapshot.
    pub fn as_ref(&self) -> NveSceneRef<'_> {
        self.table.as_ref()
    }

    pub(crate) fn sync(&mut self, backend: &WebBackend, frame_id: FrameId) {
        self.sort_scratch.clear();
        self.sort_scratch.extend(backend.iter_nodes().map(|(id, _)| id));
        self.sort_scratch.sort_unstable();
        let _ = self.table.sync_frame(
            frame_id,
            self.sort_scratch.iter().filter_map(|&node_id| {
                let el = backend.node(node_id)?;
                Some(mirror_to_scene_input(backend, node_id, el))
            }),
        );
        *self.view = self.table.as_view();
    }
}
