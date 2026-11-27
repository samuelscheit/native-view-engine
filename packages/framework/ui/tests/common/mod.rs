//! Shared headless integration test harness driving real `DioxusSurface` + `HeadlessBackend`
//! directly over `NodeStore` word tables.

use std::collections::HashMap;

use nve_abi::{
    pack_key_modifiers, ColorRgba, EdgeInsets, EventPayload, FrameId, NativeEvent, NativeEventKind,
    NodeId, SurfaceId,
};
use nve_backend_headless::HeadlessBackend;
use nve_components::tables::{
    Accessibility, AriaAutoComplete, AriaLive, AriaOrientation, AriaRelations, AriaTriState,
    BoxShadows, DebugId, Interaction, Layout, TextContent, TextInput, Visual,
};
use nve_dioxus::elements::backend::{StoreBackend, Viewport};
use nve_dioxus::prelude::Element;
use nve_dioxus::DioxusSurface;
use nve_web_html::{DataAttrList, HtmlDataAttrs, HtmlSemantics};

pub struct UiHarness {
    pub surface: DioxusSurface,
    pub backend: HeadlessBackend,
    num_attrs: HashMap<(NodeId, &'static str), String>,
    frame_id: FrameId,
    timestamp_us: u64,
}

#[allow(dead_code)]
impl UiHarness {
    pub fn new(root: fn() -> Element) -> Self {
        let mut backend = HeadlessBackend::new();
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 800.0,
                height: 600.0,
                scale_factor: 1.0,
            },
        );
        let mut h = Self {
            surface: DioxusSurface::new(SurfaceId::ROOT, root),
            backend,
            num_attrs: HashMap::new(),
            frame_id: FrameId::ZERO,
            timestamp_us: 0,
        };
        h.render();
        h
    }

    pub fn render(&mut self) {
        self.frame_id = self.frame_id.next();
        self.surface
            .commit_to(self.frame_id, &mut self.backend)
            .expect("headless commit_to succeeded");
        self.refresh_num_attrs();
    }

    fn refresh_num_attrs(&mut self) {
        self.num_attrs.clear();
        let store = self.surface.store();
        let mut stack = vec![NodeId::ROOT];
        while let Some(id) = stack.pop() {
            if id != NodeId::ROOT {
                if let Some(a) = store.try_row::<Accessibility>(id) {
                    if let Some(v) = a.value_now() {
                        self.num_attrs.insert((id, "ariaValueNow"), format_num(v));
                    }
                    if let Some(v) = a.value_min() {
                        self.num_attrs.insert((id, "ariaValueMin"), format_num(v));
                    }
                    if let Some(v) = a.value_max() {
                        self.num_attrs.insert((id, "ariaValueMax"), format_num(v));
                    }
                }
                if let Some(i) = store.try_row::<Interaction>(id)
                    && let Some(tab) = i.tab_index()
                {
                    self.num_attrs.insert((id, "tabIndex"), tab.to_string());
                }
            }
            stack.extend_from_slice(store.children(id));
        }
    }

    pub fn find(&self, tid: &str) -> NodeId {
        let found = self.maybe_find(tid);
        assert!(found.is_some(), "expected node with testID='{tid}'");
        found.expect("checked above")
    }

    pub fn maybe_find(&self, tid: &str) -> Option<NodeId> {
        let store = self.surface.store();
        let mut stack = vec![NodeId::ROOT];
        while let Some(id) = stack.pop() {
            if id != NodeId::ROOT
                && let Some(dbg) = store.try_row::<DebugId>(id)
                && dbg.test_id() == tid
            {
                return Some(id);
            }
            stack.extend_from_slice(store.children(id));
        }
        None
    }

    pub fn live_node_count(&self) -> usize {
        self.surface.store().stats().live_nodes
    }

    pub fn children_of(&self, id: NodeId) -> Vec<NodeId> {
        self.surface.store().children(id).to_vec()
    }

    pub fn bg(&self, id: NodeId) -> ColorRgba {
        self.surface
            .store()
            .try_row::<Visual>(id)
            .map_or(ColorRgba::TRANSPARENT, |v| ColorRgba::from_u32(v.background().0))
    }

    pub fn border_width(&self, id: NodeId) -> f32 {
        self.surface
            .store()
            .try_row::<Visual>(id)
            .map_or(0.0, |v| v.border_width())
    }

    pub fn border_color(&self, id: NodeId) -> ColorRgba {
        self.surface
            .store()
            .try_row::<Visual>(id)
            .map_or(ColorRgba::TRANSPARENT, |v| {
                ColorRgba::from_u32(v.border_color().0)
            })
    }

    pub fn border_radius(&self, id: NodeId) -> f32 {
        self.surface
            .store()
            .try_row::<Visual>(id)
            .map_or(0.0, |v| v.border_radius())
    }

    pub fn opacity(&self, id: NodeId) -> f32 {
        self.surface
            .store()
            .try_row::<Visual>(id)
            .map_or(1.0, |v| v.opacity())
    }

    pub fn has_box_shadow(&self, id: NodeId) -> bool {
        let store = self.surface.store();
        store
            .try_row::<Visual>(id)
            .and_then(|v| {
                let r = v.box_shadows();
                (!r.is_none())
                    .then(|| store.resource::<BoxShadows>(r).map(|b| !b.0.is_empty()))
                    .flatten()
            })
            .unwrap_or(false)
    }

    pub fn padding(&self, id: NodeId) -> EdgeInsets {
        self.surface
            .store()
            .try_row::<Layout>(id)
            .map_or(EdgeInsets::default(), |l| EdgeInsets {
                top: l.padding_top().value(),
                right: l.padding_right().value(),
                bottom: l.padding_bottom().value(),
                left: l.padding_left().value(),
            })
    }

    /// Reads typed semantic/ARIA/data attributes directly from `NodeStore` word tables.
    pub fn raw_str(&self, id: NodeId, key: &str) -> Option<&str> {
        let store = self.surface.store();
        let nid = id;
        if let Some(data_key) = key.strip_prefix("data-") {
            let res = store.try_row::<HtmlDataAttrs>(nid)?.entries();
            if res.is_none() {
                return None;
            }
            let list = store.resource::<DataAttrList>(res)?;
            return list
                .0
                .iter()
                .find(|(k, _)| k == data_key || k == key)
                .map(|(_, v)| v.as_str());
        }
        match key {
            "testID" => store
                .try_row::<DebugId>(nid)
                .map(|d| d.test_id())
                .filter(|s| !s.is_empty()),
            "id" => store
                .try_row::<DebugId>(nid)
                .map(|d| d.id())
                .filter(|s| !s.is_empty()),
            "accessibilityRole" => store
                .try_row::<Accessibility>(nid)?
                .role()
                .map(|r| r.as_aria_str()),
            "accessibilityLabel" => store
                .try_row::<Accessibility>(nid)
                .map(|a| a.label())
                .filter(|s| !s.is_empty()),
            "ariaChecked" => tri_str(store.try_row::<Accessibility>(nid)?.checked()),
            "ariaSelected" => tri_str(store.try_row::<Accessibility>(nid)?.selected()),
            "ariaExpanded" => tri_str(store.try_row::<Accessibility>(nid)?.expanded()),
            "ariaModal" => {
                let a = store.try_row::<Accessibility>(nid)?;
                a.modal().then_some("true")
            }
            "ariaInvalid" => {
                let a = store.try_row::<Accessibility>(nid)?;
                Some(if a.invalid() { "true" } else { "false" })
            }
            "ariaMultiSelectable" => {
                let a = store.try_row::<Accessibility>(nid)?;
                a.multiselectable().then_some("true")
            }
            "ariaDisabled" => {
                let i = store.try_row::<Interaction>(nid)?;
                Some(if i.disabled() { "true" } else { "false" })
            }
            "ariaLive" => match store.try_row::<Accessibility>(nid)?.live() {
                Some(AriaLive::Polite) => Some("polite"),
                Some(AriaLive::Assertive) => Some("assertive"),
                Some(AriaLive::Off) | None => None,
            },
            "ariaOrientation" => match store.try_row::<Accessibility>(nid)?.orientation() {
                Some(AriaOrientation::Horizontal) => Some("horizontal"),
                Some(AriaOrientation::Vertical) => Some("vertical"),
                None => None,
            },
            "ariaAutoComplete" => match store.try_row::<Accessibility>(nid)?.autocomplete() {
                Some(AriaAutoComplete::Inline) => Some("inline"),
                Some(AriaAutoComplete::List) => Some("list"),
                Some(AriaAutoComplete::None) => Some("none"),
                None => None,
            },
            "ariaControls" | "ariaLabelledBy" | "ariaDescribedBy" | "ariaActiveDescendant" => {
                let res = store.try_row::<Accessibility>(nid)?.relations();
                if res.is_none() {
                    return None;
                }
                let rel = store.resource::<AriaRelations>(res)?;
                let val = match key {
                    "ariaControls" => rel.controls.as_str(),
                    "ariaLabelledBy" => rel.labelled_by.as_str(),
                    "ariaDescribedBy" => rel.described_by.as_str(),
                    _ => rel.active_descendant.as_str(),
                };
                (!val.is_empty()).then_some(val)
            }
            "ariaValueNow" => self.num_attrs.get(&(id, "ariaValueNow")).map(String::as_str),
            "ariaValueMin" => self.num_attrs.get(&(id, "ariaValueMin")).map(String::as_str),
            "ariaValueMax" => self.num_attrs.get(&(id, "ariaValueMax")).map(String::as_str),
            "tabIndex" => self.num_attrs.get(&(id, "tabIndex")).map(String::as_str),
            "box-shadow" => {
                if self.has_box_shadow(id) {
                    Some("shadow")
                } else {
                    Some("none")
                }
            }
            "value" => store
                .try_row::<TextInput>(nid)
                .map(|ti| ti.value()),
            "placeholder" => store
                .try_row::<TextInput>(nid)
                .map(|ti| ti.placeholder()),
            _ => None,
        }
    }

    pub fn raw_bool(&self, id: NodeId, key: &str) -> Option<bool> {
        let store = self.surface.store();
        let nid = id;
        match key {
            "disabled" => store.try_row::<Interaction>(nid).map(|i| i.disabled()),
            "secureTextEntry" => store.try_row::<TextInput>(nid).map(|ti| ti.secure()),
            "readOnly" => store.try_row::<TextInput>(nid).map(|ti| ti.read_only()),
            "multiline" => store.try_row::<TextInput>(nid).map(|ti| ti.multiline()),
            "required" => store.try_row::<TextInput>(nid).map(|ti| ti.required()),
            "open" => store.try_row::<HtmlSemantics>(nid).map(|s| s.open()),
            _ => None,
        }
    }

    pub fn text_of(&self, tid: &str) -> String {
        let id = self.find(tid);
        self.collect_text(id)
    }

    fn collect_text(&self, id: NodeId) -> String {
        let store = self.surface.store();
        let mut out = String::new();
        if let Some(tc) = store.try_row::<TextContent>(id) {
            out.push_str(tc.text());
        }
        for &cid in store.children(id) {
            out.push_str(&self.collect_text(cid));
        }
        out
    }

    fn next_ts(&mut self) -> u64 {
        self.timestamp_us += 16_000;
        self.timestamp_us
    }

    pub fn press(&mut self, tid: &str) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 4.0, y: 4.0 },
        }]));
        self.render();
    }

    pub fn keydown(&mut self, tid: &str, key: &str, shift: bool) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::KeyDown,
            payload: EventPayload::Key {
                key: key.to_owned(),
                code: String::new(),
                key_code: 0,
                modifiers: pack_key_modifiers(false, false, shift, false, false),
                is_composing: false,
            },
        }]));
        self.render();
    }

    pub fn focus(&mut self, tid: &str) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Focus,
            payload: EventPayload::Empty,
        }]));
        self.render();
    }

    pub fn blur(&mut self, tid: &str) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Blur,
            payload: EventPayload::Empty,
        }]));
        self.render();
    }

    pub fn text_change(&mut self, tid: &str, value: &str) {
        self.change_text(tid, value);
    }

    pub fn change_text(&mut self, tid: &str, value: &str) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(value.to_owned()),
        }]));
        self.render();
    }

    pub fn submit(&mut self, tid: &str, value: &str) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Submit,
            payload: EventPayload::Text(value.to_owned()),
        }]));
        self.render();
    }

    pub fn scroll(&mut self, tid: &str, y: f32) {
        let id = self.find(tid);
        let ts = self.next_ts();
        assert!(self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: id,
            timestamp_us: ts,
            kind: NativeEventKind::Scroll,
            payload: EventPayload::Scroll {
                x: 0.0,
                y,
                content_width: 800.0,
                content_height: y + 600.0,
            },
        }]));
        self.render();
    }
}

fn tri_str(v: Option<AriaTriState>) -> Option<&'static str> {
    match v {
        Some(AriaTriState::True) => Some("true"),
        Some(AriaTriState::False) => Some("false"),
        Some(AriaTriState::Mixed) => Some("mixed"),
        None => None,
    }
}

fn format_num(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}
