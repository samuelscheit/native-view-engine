#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nve_abi::{ColorRgba, EventPayload, FrameId, NativeEvent, NativeEventKind, NodeId, SurfaceId};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::{PixelBuffer, SkiaBackend};
use nve_backend_web::WebBackend;
use nve_canvas_skia::CanvasSkiaSystem;
use nve_canvas_web::CanvasWebSystem;
use nve_components::tables::{DebugId, TextContent};
use nve_dioxus::DioxusSurface;
use nve_engine::{StoreBackend, Viewport};
use nve_store::{NodeStore};
use nve_studio_workspace::create_surface_for_viewport;
use nve_web_html::{DataAttrList, HtmlDataAttrs};
use nve_web_html_web::register_web_html_systems;

pub fn register_studio_tables() {
    nve_components::tables::register_core_tables();
    let _ = <nve_canvas::Canvas as nve_store::NveTable>::slot();
    nve_web_html::register_web_html_tables();
}

pub fn find_in_store(store: &NodeStore, test_id: &str) -> Option<NodeId> {
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if id != NodeId::ROOT
            && let Some(dbg) = store.try_row::<DebugId>(id)
            && dbg.test_id() == test_id
        {
            return Some(id);
        }
        stack.extend_from_slice(store.children(id));
    }
    None
}

pub fn data_attr_in_store(store: &NodeStore, id: NodeId, key: &str) -> Option<String> {
    let res = store.try_row::<HtmlDataAttrs>(id)?.entries();
    let list = store.resource::<DataAttrList>(res)?;
    let stripped = key.strip_prefix("data-").unwrap_or(key);
    list.iter()
        .find(|(k, _)| *k == key || *k == stripped)
        .map(|(_, v)| v.to_owned())
}

pub fn live_node_ids(store: &NodeStore) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend_from_slice(store.children(id));
    }
    out
}

pub struct MultiBackendHarness {
    pub surface: DioxusSurface,
    pub headless: HeadlessBackend,
    pub skia: SkiaBackend,
    pub web: WebBackend,
    pub viewport: Viewport,
    pub next_frame: u64,
}

impl MultiBackendHarness {
    pub fn new(width: f32, height: f32) -> Self {
        register_studio_tables();
        let viewport = Viewport {
            width,
            height,
            scale_factor: 1.0,
        };
        let mut headless = HeadlessBackend::new();
        let mut skia = SkiaBackend::new();
        skia.register_table_system(CanvasSkiaSystem);
        let mut web = WebBackend::new();
        register_web_html_systems(web.table_registry_mut());
        web.register_table_system(CanvasWebSystem);
        headless.set_viewport(SurfaceId::ROOT, viewport);
        skia.set_viewport(SurfaceId::ROOT, viewport);
        web.set_viewport(SurfaceId::ROOT, viewport);

        let mut h = Self {
            surface: create_surface_for_viewport(SurfaceId::ROOT, width, height),
            headless,
            skia,
            web,
            viewport,
            next_frame: 1,
        };
        h.render_next();
        h
    }

    pub fn render_next(&mut self) {
        let fid = self.next_frame;
        self.next_frame += 1;
        self.surface.reconcile(FrameId(fid));
        let store = self.surface.store();
        let changes = self.surface.last_changes();
        self.headless.commit(store, changes).expect("headless commit");
        self.skia.commit(store, changes).expect("skia commit");
        self.web.commit(store, changes).expect("web commit");
    }

    pub fn find_by_test_id(&self, test_id: &str) -> Option<NodeId> {
        find_in_store(self.surface.store(), test_id)
    }

    pub fn data_attr(&self, id: NodeId, key: &str) -> Option<String> {
        data_attr_in_store(self.surface.store(), id, key)
    }

    pub fn press(&mut self, test_id: &str) {
        let node_id = self.find_by_test_id(test_id);
        assert!(
            node_id.is_some(),
            "expected node with testID='{test_id}'\n{}",
            self.ascii()
        );
        let node_id = node_id.expect("checked above");
        let rect = self.headless.layout_of(node_id);
        assert!(rect.is_some(), "missing layout rect for testID='{test_id}'");
        let rect = rect.expect("checked above");
        assert!(
            rect.width > 0.0 && rect.height > 0.0,
            "zero-sized control '{test_id}': {rect:?}"
        );
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        let hit = self.headless.hit_test(self.surface.store(), cx, cy);
        assert!(
            hit.is_some(),
            "hit_test missed '{test_id}' at ({cx}, {cy}) rect={rect:?}"
        );
        let hit = hit.expect("checked above");
        assert_eq!(
            hit, node_id,
            "hit_test at center of '{test_id}' hit Node#{hit} instead of Node#{node_id}"
        );

        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id: hit,
            timestamp_us: self.next_frame * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: cx, y: cy },
        }]);
        assert!(handled, "press on '{test_id}' was not handled");
        self.render_next();
    }

    pub fn type_text(&mut self, test_id: &str, text: &str) {
        let node_id = self.find_by_test_id(test_id);
        assert!(
            node_id.is_some(),
            "expected textinput with testID='{test_id}'\n{}",
            self.ascii()
        );
        let node_id = node_id.expect("checked above");
        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: SurfaceId::ROOT,
            node_id,
            timestamp_us: self.next_frame * 16_000,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(text.to_owned()),
        }]);
        assert!(handled, "text change on '{test_id}' was not handled");
        self.render_next();
    }

    pub fn ascii(&self) -> String {
        self.headless.render_ascii_tree(self.surface.store())
    }

    pub fn html(&self) -> String {
        self.web.render_html(self.surface.store())
    }

    pub fn rasterize(&self) -> PixelBuffer {
        let mut buf = PixelBuffer::new(self.viewport.width as u32, self.viewport.height as u32);
        self.skia
            .rasterize(&mut buf, ColorRgba::rgb(11, 17, 32));
        buf
    }

    pub fn assert_no_horizontal_overflow(&self, label: &str) {
        let vp_w = self.viewport.width;
        let store = self.surface.store();
        for id in live_node_ids(store) {
            if id == NodeId::ROOT {
                continue;
            }
            let Some(r) = self.headless.layout_of(id) else {
                continue;
            };
            if r.width <= 0.0 || r.height <= 0.0 {
                continue;
            }
            let text = store
                .try_row::<TextContent>(id)
                .map(|tc| tc.text());
            assert!(
                r.x >= -0.5,
                "[{label}] node #{} (text={text:?}) has negative x: {r:?}",
                id.raw()
            );
            assert!(
                r.x + r.width <= vp_w + 1.0,
                "[{label}] node #{} (text={text:?}) overflows right edge ({vp_w}): {r:?}",
                id.raw()
            );
        }
    }
}

pub fn write_ppm(path: &Path, buf: &PixelBuffer) {
    let w = buf.width();
    let h = buf.height();
    let mut out = Vec::with_capacity((w * h * 3) as usize + 32);
    out.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = buf.get_rgb(x, y).unwrap_or((0, 0, 0));
            out.push(r);
            out.push(g);
            out.push(b);
        }
    }
    std::fs::write(path, out).expect("write ppm screenshot");
}

pub fn unique_test_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "nve-studio-wave10-{tag}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
