#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nve_abi::{
    ColorRgba, ComponentKind, EventPayload, NativeEvent, NativeEventKind, NodeId, RawPropValue,
};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::{PixelBuffer, SkiaBackend};
use nve_backend_web::WebBackend;
use nve_dioxus::DioxusSurface;
use nve_engine::{RenderBackend, Viewport};
use nve_studio_workspace::create_surface_for_viewport;

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
        let viewport = Viewport {
            width,
            height,
            scale_factor: 1.0,
        };
        let mut headless = HeadlessBackend::new();
        let mut skia = SkiaBackend::new();
        let mut web = WebBackend::new();
        headless.set_viewport(1, viewport);
        skia.set_viewport(1, viewport);
        web.set_viewport(1, viewport);

        let mut h = Self {
            surface: create_surface_for_viewport(1, width, height),
            headless,
            skia,
            web,
            viewport,
            next_frame: 1,
        };
        h.render_next();
        h
    }

    pub fn render_next(&mut self) -> Vec<u8> {
        let fid = self.next_frame;
        self.next_frame += 1;
        let (_, _, bytes) = self.surface.render_frame(fid);
        self.headless.submit_frame(&bytes).expect("headless submit");
        self.skia.submit_frame(&bytes).expect("skia submit");
        self.web.submit_frame(&bytes).expect("web submit");
        bytes
    }

    pub fn find_by_test_id(&self, test_id: &str) -> Option<NodeId> {
        self.headless
            .nodes()
            .values()
            .find(|n| {
                matches!(
                    n.raw_props.get("testID"),
                    Some(RawPropValue::String(s)) if s == test_id
                )
            })
            .map(|n| n.id)
    }

    pub fn press(&mut self, test_id: &str) {
        let node_id = self
            .find_by_test_id(test_id)
            .unwrap_or_else(|| panic!("expected node with testID='{test_id}'\n{}", self.ascii()));
        let rect = self
            .headless
            .layout_of(node_id)
            .unwrap_or_else(|| panic!("missing layout rect for testID='{test_id}'"));
        assert!(
            rect.width > 0.0 && rect.height > 0.0,
            "zero-sized control '{test_id}': {rect:?}"
        );
        let cx = rect.x + rect.width * 0.5;
        let cy = rect.y + rect.height * 0.5;
        let hit = self
            .headless
            .hit_test(cx, cy, true)
            .unwrap_or_else(|| panic!("hit_test missed '{test_id}' at ({cx}, {cy}) rect={rect:?}"));
        assert_eq!(
            hit, node_id,
            "hit_test at center of '{test_id}' hit Node#{hit} instead of Node#{node_id}"
        );

        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: hit,
            timestamp_us: self.next_frame * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: cx, y: cy },
        }]);
        assert!(handled, "press on '{test_id}' was not handled");
        self.render_next();
    }

    pub fn type_text(&mut self, test_id: &str, text: &str) {
        let node_id = self
            .find_by_test_id(test_id)
            .unwrap_or_else(|| panic!("expected textinput with testID='{test_id}'\n{}", self.ascii()));
        let handled = self.surface.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id,
            timestamp_us: self.next_frame * 16_000,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(text.to_owned()),
        }]);
        assert!(handled, "text change on '{test_id}' was not handled");
        self.render_next();
    }

    pub fn ascii(&self) -> String {
        self.headless.render_ascii_tree()
    }

    pub fn html(&self) -> String {
        self.web.render_html()
    }

    pub fn rasterize(&self) -> PixelBuffer {
        let mut buf = PixelBuffer::new(self.viewport.width as u32, self.viewport.height as u32);
        self.skia
            .rasterize(&mut buf, ColorRgba::rgb(11, 17, 32));
        buf
    }

    pub fn assert_no_horizontal_overflow(&self, label: &str) {
        let vp_w = self.viewport.width;
        for node in self.headless.nodes().values() {
            if node.kind == ComponentKind::Root {
                continue;
            }
            let r = node.computed_rect;
            if r.width <= 0.0 || r.height <= 0.0 {
                continue;
            }
            assert!(
                r.x >= -0.5,
                "[{label}] node #{id} ({kind:?}, text={text:?}) has negative x: {r:?}",
                id = node.id,
                kind = node.kind,
                text = node.text
            );
            assert!(
                r.x + r.width <= vp_w + 1.0,
                "[{label}] node #{id} ({kind:?}, text={text:?}) overflows right edge ({vp_w}): {r:?}",
                id = node.id,
                kind = node.kind,
                text = node.text
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
