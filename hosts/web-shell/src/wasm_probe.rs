//! Dev/E2E-only (`dev` feature) semantic HTML5 + typed table hydration probe: mounts a
//! hand-built `NodeStore` frame as SSR HTML into `#<mount_id>` and hydrates it in place.

use std::cell::RefCell;
use std::fmt::Write as _;

use nve_abi::{EventBatchBuf, EventSet, InteractionEvent, ScrollEvent};
use nve_backend_web::WebBackend;
use nve_canvas::Canvas;
use nve_tables::tables::{
    register_core_tables, Accessibility, AriaRole, BoxShadow, BoxShadows, DebugId, FlexDirection,
    FlexWrap, Interaction, Layout, Link, Overlay, OverlayFlags, OverlayLayer, OverlayPlacement,
    PointerCaptureMode, Position, Scroll, TextAlign, TextContent, TextInput, Typography, Visual,
};
use nve_engine::StoreBackend;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_web_html::{
    register_web_html_tables, DataAttrList, HtmlDataAttrs, HtmlSemantics, PopoverMode,
    SemanticElement,
};
use wasm_bindgen::prelude::*;

fn new_probe_store() -> NodeStore {
    register_core_tables();
    register_web_html_tables();
    NodeStore::new()
}

thread_local! {
    /// Keeps the most recent probe `WebBackend` alive while E2E inspects its DOM &
    /// `adoptedStyleSheets` (replacing it drops the previous backend cleanly).
    static LAST_PROBE_BACKEND: RefCell<Option<WebBackend>> = const { RefCell::new(None) };
}

fn new_probe_web(mut web: WebBackend) -> WebBackend {
    crate::register_dev_web_systems(&mut web);
    web
}

/// Hydrates a semantic HTML5 + typed table (`box_shadows`/`font_weight`/`text_align`)
/// `NodeStore` into `#<mount_id>`, returning the mismatch count (0 on clean hydration).
#[wasm_bindgen]
pub fn run_semantic_style_hydration_probe(mount_id: &str) -> Result<usize, JsValue> {
    let doc = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("missing document"))?;
    let container = doc
        .get_element_by_id(mount_id)
        .ok_or_else(|| JsValue::from_str(&format!("mount '#{mount_id}' not found")))?;

    let mut store = new_probe_store();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let root = NodeId::ROOT;
        let card = NodeId::new(2, 0);
        let heading = NodeId::new(3, 0);
        let link = NodeId::new(4, 0);
        let _ = tx.create_at(root);
        let _ = tx.create_at(card);
        let _ = tx.create_at(heading);
        let _ = tx.create_at(link);

        tx.set_str(card, DebugId::TEST_ID, "probe-card");
        tx.set(card, Accessibility::ROLE, Some(AriaRole::Article));
        let p = Length::points(12.0);
        tx.set(card, Layout::PADDING_TOP, p);
        tx.set(card, Layout::PADDING_RIGHT, p);
        tx.set(card, Layout::PADDING_BOTTOM, p);
        tx.set(card, Layout::PADDING_LEFT, p);
        tx.set(card, Visual::BACKGROUND, Rgba::rgb(17, 24, 39));
        tx.set(card, Visual::BORDER_RADIUS, 10.0);
        tx.set_resource(
            card,
            Visual::BOX_SHADOWS,
            Some(BoxShadows::new([BoxShadow {
                offset_x: 0.0,
                offset_y: 8.0,
                blur_radius: 24.0,
                spread_radius: 0.0,
                color: Rgba::new(0, 0, 0, 115),
                inset: false,
            }])),
        );

        tx.set_str(heading, DebugId::TEST_ID, "probe-heading");
        tx.set(heading, Accessibility::ROLE, Some(AriaRole::Heading));
        tx.set(heading, Accessibility::LEVEL, Some(1u8));
        tx.set(heading, Typography::COLOR, Rgba::WHITE);
        tx.set(heading, Typography::FONT_SIZE, 20.0);
        tx.set(heading, Typography::FONT_WEIGHT, 700u16);
        tx.set(heading, Typography::TEXT_ALIGN, TextAlign::Center);
        tx.set_str(heading, TextContent::TEXT, "Semantic SSR Heading");

        tx.set_str(link, DebugId::TEST_ID, "probe-link");
        tx.set(link, Accessibility::ROLE, Some(AriaRole::Link));
        tx.set_str(link, Link::HREF, "/docs/ssr");
        tx.edit::<Interaction>(link).listeners(EventSet::one(InteractionEvent::Press));
        tx.set_str(link, TextContent::TEXT, "Docs Link");

        tx.set_children(card, &[heading, link]);
        tx.set_children(root, &[card]);
        tx.commit_into(&mut changes);
    }

    // 1. Server render SSR HTML + styles into container if empty.
    if container
        .query_selector("[data-nve-ssr-hash]")
        .ok()
        .flatten()
        .is_none()
    {
        let mut server = new_probe_web(WebBackend::new());
        server
            .commit(&store, &changes)
            .map_err(|e| JsValue::from_str(&format!("probe server render failed: {e}")))?;
        let ssr = server.render_ssr(&store);
        if let Ok(style_el) = doc.create_element("style") {
            let _ = style_el.set_attribute("data-nve-ssr", "");
            let _ = style_el.set_attribute("data-nve-hashes", &ssr.rule_hashes.join(","));
            style_el.set_text_content(Some(&ssr.style_css));
            if let Some(head) = doc.query_selector("head").ok().flatten() {
                let _ = head.append_child(&style_el);
            }
        }
        container.set_inner_html(&ssr.html);
    }

    // 2. Hydrate in-place and return mismatch count.
    let mut client = new_probe_web(WebBackend::hydrate_into(container));
    client
        .commit(&store, &changes)
        .map_err(|e| JsValue::from_str(&format!("probe hydrate failed: {e}")))?;
    let mismatches = client.hydration_report().mismatches.len();
    LAST_PROBE_BACKEND.with(|slot| *slot.borrow_mut() = Some(client));
    Ok(mismatches)
}

fn mount_probe_container(mount_id: &str) -> Result<web_sys::Element, JsValue> {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(mount_id))
        .ok_or_else(|| JsValue::from_str(&format!("mount '#{mount_id}' not found")))
}

/// Mounts a live `WebBackend` into `#<mount_id>` and applies hostile transactions the live DOM
/// path must survive: reserved attributes, non-secure inputs, empty draw lists, child
/// reparenting/moves, and text set on a node with children.
#[wasm_bindgen]
pub fn run_hostile_frame_probe(mount_id: &str) -> Result<(), JsValue> {
    let mut web = new_probe_web(WebBackend::mount_into(mount_probe_container(mount_id)?));
    let mut store = new_probe_store();
    let mut changes = ChangeSet::default();

    {
        let mut tx = store.tx();
        let root = NodeId::ROOT;
        let n2 = NodeId::new(2, 0);
        let n3 = NodeId::new(3, 0);
        let n4 = NodeId::new(4, 0);
        let n5 = NodeId::new(5, 0);
        let n6 = NodeId::new(6, 0);
        let n7 = NodeId::new(7, 0);
        let n8 = NodeId::new(8, 0);
        for id in [root, n2, n3, n4, n5, n6, n7, n8] {
            let _ = tx.create_at(id);
        }

        tx.set_str(n2, TextContent::TEXT, "parent text");
        tx.set(n2, Layout::FLEX_WRAP, FlexWrap::Wrap);
        tx.set(n2, Typography::FONT_WEIGHT, 700u16);
        tx.set_resource(
            n2,
            HtmlDataAttrs::ENTRIES,
            Some(DataAttrList::from_pairs(vec![
                ("style".into(), "color: rgb(255, 0, 0)".into()),
                ("class".into(), "evil".into()),
                ("data-nve-id".into(), "99".into()),
            ])),
        );

        tx.attach::<TextInput>(n3);
        tx.set_str(n3, DebugId::TEST_ID, "probe-input");

        tx.attach::<Canvas>(n4);

        tx.set_str(n5, TextContent::TEXT, "child of two");
        tx.set_str(n6, TextContent::TEXT, "own text");
        tx.set_str(n7, TextContent::TEXT, "kept child");

        tx.set_children(root, &[n2, n3, n4, n6, n8]);
        tx.set_children(n2, &[n5]);
        tx.set_children(n6, &[n7]);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes)
        .map_err(|e| JsValue::from_str(&format!("hostile probe frame 1 failed: {e}")))?;

    {
        let mut tx = store.tx();
        tx.set(NodeId::ROOT, Accessibility::ROLE, Some(AriaRole::Navigation));
        tx.set_resource(
            NodeId::ROOT,
            HtmlDataAttrs::ENTRIES,
            Some(DataAttrList::from_pairs(vec![(
                "data-nve-id".into(),
                "7".into(),
            )])),
        );
        tx.set_children(NodeId::new(8, 0), &[NodeId::new(6, 0)]);
        tx.set(NodeId::new(3, 0), TextInput::SECURE, true);
        tx.set(NodeId::new(3, 0), TextInput::SECURE, false);
        tx.set_str(NodeId::new(6, 0), TextContent::TEXT, "replaced own text");
        tx.set_str(NodeId::new(2, 0), TextContent::TEXT, "restored text");
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes)
        .map_err(|e| JsValue::from_str(&format!("hostile probe frame 2 failed: {e}")))?;

    {
        let mut tx = store.tx();
        tx.set_children(NodeId::new(2, 0), &[]);
        tx.commit_into(&mut changes);
    }
    web.commit(&store, &changes)
        .map_err(|e| JsValue::from_str(&format!("hostile probe frame 3 failed: {e}")))?;
    LAST_PROBE_BACKEND.with(|slot| *slot.borrow_mut() = Some(web));
    Ok(())
}

/// Returns live host-service Rust & JS closure/listener slab counts for E2E leak verification:
/// `[pending_tasks, wake_trampolines, js_pending_microtasks, clip_pending_reads,
///   clip_dom_listeners, js_clip_pending_reads, js_clip_listeners, sysenv_rust_watchers,
///   sysenv_js_watchers]`.
#[wasm_bindgen]
pub fn host_services_slab_counts() -> Vec<u32> {
    let (tasks, trampolines, js_tasks) = crate::host_services::wasm_drain::debug_drain_counts();
    let clip = nve_clipboard_web::WebClipboard::debug_slab_counts();
    let sysenv = nve_system_env_web::WebThemeWatcher::debug_slab_counts();
    vec![
        tasks as u32,
        trampolines as u32,
        js_tasks as u32,
        clip.pending_reads as u32,
        clip.dom_listeners as u32,
        clip.js_pending_reads as u32,
        clip.js_dom_listeners as u32,
        sysenv.rust_watchers as u32,
        sysenv.js_watchers as u32,
    ]
}

/// Returns the cached `scale_factor_x100` held by `WebSystemEnv` (only updated when the live
/// re-arming `(resolution: <dpr>dppx)` listener fires), proving live DPR watcher reactivity.
#[wasm_bindgen]
pub fn host_services_cached_dpr_x100(surface_id: u32) -> u16 {
    nve_abi::SurfaceId::from_raw(surface_id)
        .and_then(|sid| crate::wasm_api::with_active_host_guard(sid, |g| g.cached_scale_factor_x100()))
        .unwrap_or(0)
}

/// Returns how many times the live re-arming `matchMedia("(resolution: <dpr>dppx)")` change
/// listener (excluding `resize` fallbacks) has fired.
#[wasm_bindgen]
pub fn host_services_dpr_mql_fires() -> u32 {
    nve_system_env_web::WebThemeWatcher::debug_dpr_mql_fire_count()
}

/// Starts a real browser `navigator.clipboard.readText()` async `Pending` call on `surface_id`,
/// returning its `request_id` (`0` if completed synchronously or surface missing).
#[wasm_bindgen]
pub fn host_services_start_clipboard_read(surface_id: u32) -> u64 {
    nve_abi::SurfaceId::from_raw(surface_id)
        .and_then(|sid| {
            crate::wasm_api::with_active_host_guard(sid, |g| g.debug_start_clipboard_read())
        })
        .unwrap_or(0)
}

/// Arms `surface_id`'s `CompletionSink::on_ready` callback to synchronously unmount and tear down
/// `surface_id` re-entrantly from inside an active JS Promise settlement callback (E2E WASM UAF
/// and automatic deferred-reap regression probe).
#[wasm_bindgen]
pub fn host_services_arm_reentrant_unmount(surface_id: u32) -> bool {
    nve_abi::SurfaceId::from_raw(surface_id)
        .and_then(|sid| {
            crate::wasm_api::with_active_host_guard(sid, |g| {
                g.debug_arm_reentrant_unmount_on_ready(sid);
            })
        })
        .is_some()
}

/// Seeds `WebClipboard`'s synchronous shadow buffer on `surface_id` (E2E probe for `NotAllowedError`
/// and missing `navigator.clipboard` fallback behavior).
#[wasm_bindgen]
pub fn host_services_set_clipboard_shadow(surface_id: u32, text: Option<String>) -> bool {
    nve_abi::SurfaceId::from_raw(surface_id)
        .and_then(|sid| {
            crate::wasm_api::with_active_host_guard(sid, |g| {
                g.debug_set_clipboard_shadow(text);
            })
        })
        .is_some()
}

/// Invokes `SERVICE_CLIPBOARD::METHOD_READ_TEXT` on `surface_id`, returning `"sync:<text>"`,
/// `"sync-none"`, `"pending:<request_id>"`, or `"err:<status_i32>"`.
#[wasm_bindgen]
pub fn host_services_clipboard_read_probe(surface_id: u32) -> String {
    nve_abi::SurfaceId::from_raw(surface_id)
        .and_then(|sid| {
            crate::wasm_api::with_active_host_guard(sid, |g| g.debug_clipboard_read_probe())
        })
        .unwrap_or_else(|| "missing-surface".to_owned())
}

/// Consumes a settled clipboard probe response from `CompletionSink`, returning `"ok:<text>"`,
/// `"ok-none"`, `"err:<status_i32>"`, or `"none"`.
#[wasm_bindgen]
pub fn host_services_take_clipboard_result(surface_id: u32, request_id: u64) -> String {
    nve_abi::SurfaceId::from_raw(surface_id)
        .and_then(|sid| {
            crate::wasm_api::with_active_host_guard(sid, |g| {
                g.debug_take_clipboard_result(request_id)
            })
        })
        .unwrap_or_else(|| "missing-surface".to_owned())
}

/// Panics with `message`, so the E2E can check that a release bundle prints the panic message and
/// its `file:line`. Dev bundle only: production bundles carry no trigger.
#[wasm_bindgen]
#[expect(
    clippy::panic,
    reason = "intentional E2E panic_probe trigger in dev-only wasm_probe"
)]
pub fn panic_probe(message: &str) {
    panic!("{message}");
}

/// Stateful live-DOM probe exercising Track 2A web fixes inside Chromium CDP:
/// post-flush `requested_scroll_seq` (incl. `1 -> 0 -> 1`), `Overlay`-driven `<dialog>`/`:modal`
/// and `[popover]`/`:popover-open`, `OverlayAnchorFallbackManager` inline cleanup + stale
/// `anchor-name` stripping, disabled capture exclusion, and mid-drag captured node removal.
#[wasm_bindgen]
pub struct Track2aProbe {
    web: WebBackend,
    store: NodeStore,
    changes: ChangeSet,
    scroller: NodeId,
    dialog: NodeId,
    popover: NodeId,
    anchor: NodeId,
    anchored_ov: NodeId,
    disabled_cap: NodeId,
    live_cap: NodeId,
}

#[wasm_bindgen]
impl Track2aProbe {
    #[wasm_bindgen(constructor)]
    pub fn new(mount_id: &str) -> Result<Self, JsValue> {
        let mut web = new_probe_web(WebBackend::mount_into(mount_probe_container(mount_id)?));
        let mut store = new_probe_store();
        let mut changes = ChangeSet::default();
        let (scroller, dialog, popover, anchor, anchored_ov, disabled_cap, live_cap) = {
            let mut tx = store.tx();
            let root = NodeId::ROOT;
            let scroller = NodeId::new(2, 0);
            let sc_child = NodeId::new(3, 0);
            let dialog = NodeId::new(4, 0);
            let popover = NodeId::new(5, 0);
            let anchor = NodeId::new(6, 0);
            let anchored_ov = NodeId::new(7, 0);
            let disabled_cap = NodeId::new(8, 0);
            let live_cap = NodeId::new(9, 0);
            for id in [
                root,
                scroller,
                sc_child,
                dialog,
                popover,
                anchor,
                anchored_ov,
                disabled_cap,
                live_cap,
            ] {
                let _ = tx.create_at(id);
            }

            tx.edit::<Layout>(scroller)
                .flex_direction(FlexDirection::Column)
                .width(Length::points(200.0))
                .height(Length::points(100.0));
            tx.edit::<DebugId>(scroller).test_id("t2a-scroller");
            tx.edit::<Scroll>(scroller)
                .requested_scroll_y(Some(140.0))
                .requested_scroll_seq(1)
                .listeners(EventSet::one(ScrollEvent::Scroll));
            tx.edit::<Layout>(sc_child)
                .flex_shrink(0.0)
                .width(Length::points(200.0))
                .height(Length::points(600.0));

            tx.edit::<DebugId>(dialog).test_id("t2a-dialog");
            tx.edit::<HtmlSemantics>(dialog)
                .element(Some(SemanticElement::Dialog))
                .open(false);
            tx.edit::<Overlay>(dialog)
                .layer(OverlayLayer::Modal)
                .options(OverlayFlags::NONE);

            tx.edit::<DebugId>(popover).test_id("t2a-popover");
            tx.edit::<HtmlSemantics>(popover)
                .popover(Some(PopoverMode::Manual))
                .open(false);
            tx.edit::<Overlay>(popover)
                .layer(OverlayLayer::Popover)
                .options(OverlayFlags::NONE);

            tx.edit::<Layout>(anchor)
                .position(Position::Absolute)
                .inset_left(Length::points(20.0))
                .inset_top(Length::points(20.0))
                .width(Length::points(80.0))
                .height(Length::points(30.0));
            tx.edit::<DebugId>(anchor)
                .id("t2a-anchor")
                .test_id("t2a-anchor");

            tx.edit::<Layout>(anchored_ov)
                .position(Position::Fixed)
                .width(Length::points(120.0))
                .height(Length::points(60.0));
            tx.edit::<DebugId>(anchored_ov).test_id("t2a-anchored-ov");
            tx.edit::<Overlay>(anchored_ov)
                .layer(OverlayLayer::Popover)
                .anchor_id("t2a-anchor")
                .placement(OverlayPlacement::BottomStart)
                .offset_px(6.0)
                .options(
                    OverlayFlags::OPEN
                        | OverlayFlags::AUTO_SIZE
                        | OverlayFlags::FLIP
                        | OverlayFlags::SHIFT,
                );

            tx.edit::<Layout>(disabled_cap)
                .position(Position::Fixed)
                .inset_left(Length::points(20.0))
                .inset_top(Length::points(160.0))
                .width(Length::points(80.0))
                .height(Length::points(40.0));
            tx.edit::<DebugId>(disabled_cap).test_id("t2a-disabled-cap");
            tx.edit::<Interaction>(disabled_cap)
                .capture_mode(PointerCaptureMode::OnPrimaryDown)
                .disabled(true)
                .listeners(
                    EventSet::one(InteractionEvent::PointerDown)
                        .with(InteractionEvent::PointerMove)
                        .with(InteractionEvent::PointerUp),
                );

            tx.edit::<Layout>(live_cap)
                .position(Position::Fixed)
                .inset_left(Length::points(140.0))
                .inset_top(Length::points(160.0))
                .width(Length::points(80.0))
                .height(Length::points(40.0));
            tx.edit::<DebugId>(live_cap).test_id("t2a-live-cap");
            tx.edit::<Interaction>(live_cap)
                .capture_mode(PointerCaptureMode::OnPrimaryDown)
                .listeners(
                    EventSet::one(InteractionEvent::PointerDown)
                        .with(InteractionEvent::PointerMove)
                        .with(InteractionEvent::PointerUp)
                        .with(InteractionEvent::PointerCancel),
                );

            tx.set_children(scroller, &[sc_child]);
            tx.set_children(
                root,
                &[
                    scroller,
                    dialog,
                    popover,
                    anchor,
                    anchored_ov,
                    disabled_cap,
                    live_cap,
                ],
            );
            tx.commit_into(&mut changes);
            (
                scroller,
                dialog,
                popover,
                anchor,
                anchored_ov,
                disabled_cap,
                live_cap,
            )
        };

        web.commit(&store, &changes)
            .map_err(|e| JsValue::from_str(&format!("track2a frame 1 failed: {e}")))?;
        Ok(Self {
            web,
            store,
            changes,
            scroller,
            dialog,
            popover,
            anchor,
            anchored_ov,
            disabled_cap,
            live_cap,
        })
    }

    /// Opens `<dialog>` and `[popover]` strictly via `Overlay::options` (zero `HtmlSemantics`
    /// writes) and resets `requested_scroll_seq` to `0`.
    pub fn step_open_overlays_and_reset_scroll_zero(&mut self) -> Result<(), JsValue> {
        {
            let mut tx = self.store.tx();
            tx.edit::<Overlay>(self.dialog).options(OverlayFlags::OPEN);
            tx.edit::<Overlay>(self.popover).options(OverlayFlags::OPEN);
            tx.edit::<Scroll>(self.scroller).requested_scroll_seq(0);
            tx.commit_into(&mut self.changes);
        }
        self.web
            .commit(&self.store, &self.changes)
            .map_err(|e| JsValue::from_str(&format!("track2a step 2 failed: {e}")))?;
        Ok(())
    }

    /// Re-triggers `requested_scroll_seq = 1` (`1 -> 0 -> 1`), closes `<dialog>` & `[popover]`
    /// via `Overlay::options = NONE`, and detaches `Overlay` from `anchored_ov`.
    pub fn step_scroll_seq_one_and_close_overlays(&mut self) -> Result<(), JsValue> {
        {
            let mut tx = self.store.tx();
            tx.edit::<Scroll>(self.scroller)
                .requested_scroll_y(Some(140.0))
                .requested_scroll_seq(1);
            tx.edit::<Overlay>(self.dialog).options(OverlayFlags::NONE);
            tx.edit::<Overlay>(self.popover).options(OverlayFlags::NONE);
            tx.detach::<Overlay>(self.anchored_ov);
            tx.commit_into(&mut self.changes);
        }
        self.web
            .commit(&self.store, &self.changes)
            .map_err(|e| JsValue::from_str(&format!("track2a step 3 failed: {e}")))?;
        Ok(())
    }

    /// Removes `live_cap` from the live DOM mid-drag to verify `prune_pointer_state`.
    pub fn step_remove_live_cap(&mut self) -> Result<(), JsValue> {
        {
            let mut tx = self.store.tx();
            tx.set_children(
                NodeId::ROOT,
                &[
                    self.scroller,
                    self.dialog,
                    self.popover,
                    self.anchor,
                    self.anchored_ov,
                    self.disabled_cap,
                ],
            );
            tx.remove(self.live_cap);
            tx.commit_into(&mut self.changes);
        }
        self.web
            .commit(&self.store, &self.changes)
            .map_err(|e| JsValue::from_str(&format!("track2a step 4 failed: {e}")))?;
        Ok(())
    }

    /// Drains queued events as `"<node_index>:<kind>:cap=<0|1>;"` tokens (zero `serde_json`).
    pub fn drain_events_summary(&mut self) -> String {
        let mut batch = EventBatchBuf::new();
        self.web.drain_events(&mut batch);
        let mut out = String::new();
        for ev in batch.as_validated().iter() {
            let kind = match ev.interaction() {
                Some(InteractionEvent::PointerDown) => "down",
                Some(InteractionEvent::PointerMove) => "move",
                Some(InteractionEvent::PointerUp) => "up",
                Some(InteractionEvent::PointerCancel) => "cancel",
                Some(InteractionEvent::Press) => "press",
                _ => continue,
            };
            let _ = write!(
                out,
                "{}:{}:cap={};",
                ev.node().index(),
                kind,
                u8::from(ev.is_capture())
            );
        }
        out
    }
}
