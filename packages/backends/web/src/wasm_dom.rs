//! Live `wasm32` DOM mounting, deferred element creation (adopting SSR elements staged by
//! positional hydration), `COMPOSITE`-only 120 Hz fast lane, and `WebTableRegistry` plugin DOM
//! synchronization.

use crate::css::Keyword;
use nve_abi::table::effect;
use nve_tables::tables::{
    DebugId, Image, ImageSrc, Interaction, Link, LinkRel, LinkTarget, Overlay, OverlayFlags,
    PointerCaptureMode, Scroll, StreamBoundary, TextContent, TextInput, NVE_STREAM_SLOT_ATTR,
    NVE_STREAM_SLOT_ROLE_ATTR, NVE_SUSPENSE_ATTR,
};
use nve_store::{ChangeSet, NodeId, NodeStore, NveTable, TableStorage};
use wasm_bindgen::JsCast;

use crate::css::write_compositor_css;
use crate::events::RootEventDelegator;
use crate::scroll_dom::apply_compositor_properties_live;
use crate::table_system::is_safe_web_url;
use crate::WebBackend;

impl WebBackend {
    /// Mounts a new live [`WebBackend`] into `container` on `wasm32`, clearing any static loading
    /// placeholder and installing the single root event delegator.
    pub fn mount_into(container: web_sys::Element) -> Self {
        Self::attach_into(container, true)
    }

    pub(crate) fn attach_into(container: web_sys::Element, clear_existing: bool) -> Self {
        if clear_existing {
            container.set_inner_html("");
        }
        let mut backend = Self::new();
        let delegator = RootEventDelegator::attach(container.clone(), &backend);
        backend.delegated_events = Some(delegator);
        backend.mount_root = Some(container);
        backend.attach_visual_viewport_listener();
        backend
    }

    pub(crate) fn clear_live_dom(&mut self) {
        if let Some(ref root) = self.mount_root {
            root.set_inner_html("");
        }
        self.live_elements.clear();
        self.last_requested_scroll_seq.clear();
        self.pending_scroll_sync.clear();
        self.programmatic_scroll_guard.borrow_mut().clear();
        self.overlay_anchors.clear();
        if let Some(ref mut delegator) = self.delegated_events {
            delegator.clear_pointer_state();
        }
    }

    pub(crate) fn commit_live_dom(&mut self, store: &NodeStore, changes: &ChangeSet) -> u32 {
        let Some(mount_root) = self.mount_root.clone() else {
            return 0;
        };
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return 0;
        };
        if self.native_event_bridges.is_stale(self.renderer.registry()) {
            let sinks = self.event_sinks();
            self.native_event_bridges
                .refresh(self.renderer.registry(), &mount_root, &sinks);
        }

        // 1. Notify plugins BEFORE removing destroyed DOM elements so `remove_node` sees `live_element: Some(el)`.
        self.renderer
            .remove_plugins(changes, Some(&self.live_elements));
        for &id in &changes.removed {
            self.clear_scroll_seq(id);
            self.overlay_anchors.remove(id);
            if let Some(el) = self.live_elements.remove(&id) {
                el.remove();
            }
        }
        if let Some(tc) = changes.table(Scroll::slot()) {
            for &id in &tc.detached {
                self.clear_scroll_seq(id);
            }
        }
        if let Some(tc) = changes.table(Overlay::slot()) {
            for &id in &tc.detached {
                self.overlay_anchors.remove(id);
            }
        }

        // 2. Deferred DOM creation: create/adopt elements ONCE after all initial tables attach.
        let mut created_set = std::mem::take(&mut self.created_scratch);
        created_set.clear();
        let mut retagged = std::mem::take(&mut self.retagged_scratch);
        retagged.clear();
        for &id in &changes.created {
            if !store.is_live(id) {
                continue;
            }
            created_set.insert(id);
            let (tag, ns) = self.renderer.registry().resolve_tag(store, id);
            let el = if let Some(adopted) = self.live_elements.get(&id).cloned() {
                if adopted.tag_name().eq_ignore_ascii_case(tag) {
                    adopted
                } else {
                    let fresh = create_dom_element(&doc, id, tag, ns);
                    self.retag_live(store, id, &adopted, &fresh);
                    retagged.push(id);
                    fresh
                }
            } else {
                let fresh = create_dom_element(&doc, id, tag, ns);
                self.live_elements.insert(id, fresh.clone());
                fresh
            };
            self.sync_full_element(store, id, &el);
        }

        // Ensure ROOT is mounted into `mount_root` (unless the mount container IS the adopted root).
        if let Some(root_el) = self.live_elements.get(&NodeId::ROOT)
            && root_el != &mount_root
            && root_el.parent_element().as_ref() != Some(&mount_root)
        {
            let _ = mount_root.append_child(root_el);
        }

        // 3. Touched nodes: non-destructive COMPOSITE-only fast lane vs full sync.
        for (&id, &fx) in changes.touched.iter().zip(changes.effects.iter()) {
            if !store.is_live(id) || created_set.contains(&id) {
                continue;
            }
            if fx == effect::COMPOSITE {
                if let Some(el) = self.live_elements.get(&id).cloned() {
                    apply_compositor_properties_live(store, id, &el);
                    self.sync_requested_scroll(store, id, &el, false);
                }
                continue;
            }
            let (tag, ns) = if id == NodeId::ROOT {
                ("div", None)
            } else {
                self.renderer.registry().resolve_tag(store, id)
            };
            let el = if let Some(existing) = self.live_elements.get(&id).cloned() {
                if id != NodeId::ROOT
                    && fx & effect::REALIZE != 0
                    && !existing.tag_name().eq_ignore_ascii_case(tag)
                {
                    let fresh = create_dom_element(&doc, id, tag, ns);
                    self.retag_live(store, id, &existing, &fresh);
                    retagged.push(id);
                    fresh
                } else {
                    existing
                }
            } else {
                let fresh = create_dom_element(&doc, id, tag, ns);
                self.live_elements.insert(id, fresh.clone());
                fresh
            };
            self.sync_full_element(store, id, &el);
        }

        // 4. Sync plugin live elements (`WebTableRegistry`), commands, and retagged nodes ONCE.
        let plugin_muts = self.renderer.commit_plugins_active(
            store,
            changes,
            Some(&self.live_elements),
            &retagged,
        );
        // A retagged element is a fresh element: imperative plugin state must be re-applied.
        for &id in &retagged {
            self.renderer.queue_connected_sync_node(store, id);
        }
        self.created_scratch = created_set;
        self.retagged_scratch = retagged;

        // 5. Reconcile DOM children order (skipping plugin-owned inner HTML and removing stray Text nodes).
        for &parent in &changes.children {
            if self.renderer.plugin_inner_html(parent).is_some() {
                continue;
            }
            let Some(parent_el) = self.live_elements.get(&parent).cloned() else {
                continue;
            };
            let wanted = store.children(parent);
            if !wanted.is_empty() {
                // Strip any orphaned non-element Text nodes left over from an earlier leaf TextContent.
                let mut child_node = parent_el.first_child();
                while let Some(cn) = child_node {
                    child_node = cn.next_sibling();
                    if cn.node_type() != web_sys::Node::ELEMENT_NODE {
                        let _ = parent_el.remove_child(&cn);
                    }
                }
            }
            let mut cursor = parent_el.first_element_child();
            for &cid in wanted {
                let Some(child_el) = self.live_elements.get(&cid) else {
                    continue;
                };
                if cursor.as_ref() == Some(child_el) {
                    cursor = cursor.and_then(|c| c.next_element_sibling());
                } else {
                    let ref_node: Option<&web_sys::Node> = cursor.as_deref().map(AsRef::as_ref);
                    // A fallback move (removes/reinserts subtree -> top layer exits) or attaching
                    // a previously unparented subtree into a connected parent needs Step-6 sync
                    // across the whole subtree so unchanged open overlays enter the top layer.
                    // Childless newly created leaves already committed in Step 4 and are skipped.
                    if !crate::dom_move::place_child(&parent_el, child_el, ref_node)
                        && parent_el.is_connected()
                        && crate::table_system::needs_connected_sync_on_place(
                            &self.created_scratch,
                            store,
                            cid,
                        )
                    {
                        let _ = self.renderer.queue_connected_sync_subtree(store, cid);
                    }
                }
            }
            while let Some(extra) = cursor {
                cursor = extra.next_element_sibling();
                extra.remove();
            }
            if wanted.is_empty()
                && !store.has::<TextInput>(parent)
                && let Some(tc) = store.try_row::<TextContent>(parent)
            {
                let txt = tc.text();
                if parent_el.text_content().as_deref() != Some(txt) {
                    parent_el.set_text_content(Some(txt));
                }
            }
        }

        // 6. Imperative state needing connected elements (`showModal()`, popovers, anchored
        //    overlays, and deferred `requested_scroll_seq` sync after children mount).
        self.renderer.sync_connected_plugins(store, &self.live_elements);
        if let Some(tc) = changes.table(Overlay::slot()) {
            // Ensure anchor elements whose `Overlay` attached on a later frame receive `anchor-name`.
            if let Some(TableStorage { owners, row_count, .. }) = store.table_storage(DebugId::slot()) {
                for &oid in &tc.nodes {
                    if let Some(ov) = store.try_row::<Overlay>(oid) {
                        let aid = ov.anchor_id();
                        if !aid.is_empty() {
                            for &did in owners.iter().take(row_count) {
                                if !did.is_null()
                                    && store.is_live(did)
                                    && store.try_row::<DebugId>(did).is_some_and(|d| d.id() == aid)
                                    && let Some(ael) = self.live_elements.get(&did).cloned()
                                {
                                    self.sync_full_element(store, did, &ael);
                                }
                            }
                        }
                    }
                }
            }
        }
        self.overlay_anchors.reposition_all();
        let pending_scroll = std::mem::take(&mut self.pending_scroll_sync);
        for id in pending_scroll {
            if store.is_live(id)
                && let Some(el) = self.live_elements.get(&id).cloned()
                && el.is_connected()
            {
                self.sync_requested_scroll(store, id, &el, false);
            }
        }

        // 7. Flush newly compiled CSS rules into adoptedStyleSheets / <style>.
        self.stylesheet.flush_to_dom(&doc);
        plugin_muts
    }

    /// Swaps `id`'s live element for `fresh` (a tag change). The retagged node itself is
    /// re-synced after plugin commit; a fallback (non-`moveBefore`) child move also ran the
    /// removing steps on every descendant, so their subtrees re-sync too.
    fn retag_live(&mut self, store: &NodeStore, id: NodeId, old: &web_sys::Element, fresh: &web_sys::Element) {
        if !crate::dom_move::retag_element(old, fresh) {
            for &child in store.children(id) {
                let _ = self.renderer.queue_connected_sync_subtree(store, child);
            }
        }
        self.live_elements.insert(id, fresh.clone());
    }

    pub(crate) fn sync_full_element(&mut self, store: &NodeStore, id: NodeId, el: &web_sys::Element) {
        let resolved = self.stylesheet.resolve_node(store, id, self.renderer.registry());
        if el.class_name() != resolved.classes {
            el.set_class_name(&resolved.classes);
        }
        let mut inline = String::new();
        if !resolved.inline_fallback.is_empty() {
            inline.push_str(&resolved.inline_fallback);
        }
        write_compositor_css(store, id, &mut inline);
        for sys in self.renderer.registry().iter() {
            if store.has_slot(id, sys.slot()) {
                sys.write_inline_css(store, id, &mut inline);
            }
        }
        if let Some(patch) = self.patch_css.get(&id) {
            inline.push_str(patch);
        }
        if inline.is_empty() {
            let _ = el.remove_attribute("style");
        } else {
            let _ = el.set_attribute("style", &inline);
        }

        let dbg = store.try_row::<DebugId>(id);
        let dom_id = dbg.map_or("", |d| d.id());
        // An adopted mount container (`<div id="app" data-nve-id="1">`) keeps its host `id`.
        if !dom_id.is_empty() || id != NodeId::ROOT || self.mount_root.as_ref() != Some(el) {
            set_or_remove_attr(el, "id", dom_id);
        }
        set_or_remove_attr(el, "data-testid", dbg.map_or("", |d| d.test_id()));
        set_or_remove_attr(el, "slot", dbg.map_or("", |d| d.slot()));
        let boundary = store.try_row::<StreamBoundary>(id);
        set_or_remove_attr(el, NVE_STREAM_SLOT_ATTR, boundary.map_or("", |b| b.stream_slot()));
        set_or_remove_attr(el, NVE_SUSPENSE_ATTR, boundary.map_or("", |b| b.suspense()));
        set_or_remove_attr(
            el,
            NVE_STREAM_SLOT_ROLE_ATTR,
            boundary.and_then(|b| b.slot_role()).map_or("", |r| r.as_attr()),
        );

        if let Some(link) = store.try_row::<Link>(id) {
            let href = link.href();
            if is_safe_web_url(href) {
                let _ = el.set_attribute("href", href);
            } else {
                let _ = el.remove_attribute("href");
            }
            let is_blank = link.target() == LinkTarget::Blank;
            match link.target() {
                LinkTarget::Default | LinkTarget::Self_ => {
                    let _ = el.remove_attribute("target");
                }
                LinkTarget::Blank => {
                    let _ = el.set_attribute("target", "_blank");
                }
                LinkTarget::Parent => {
                    let _ = el.set_attribute("target", "_parent");
                }
            }
            let rel_flags = if is_blank {
                link.rel() | LinkRel::NOOPENER | LinkRel::NOREFERRER
            } else {
                link.rel()
            };
            if !rel_flags.is_empty() {
                let mut rel = String::new();
                for (flag, kw) in [
                    (LinkRel::NOOPENER, "noopener"),
                    (LinkRel::NOREFERRER, "noreferrer"),
                    (LinkRel::NOFOLLOW, "nofollow"),
                ] {
                    if rel_flags.contains(flag) {
                        if !rel.is_empty() {
                            rel.push(' ');
                        }
                        rel.push_str(kw);
                    }
                }
                set_or_remove_attr(el, "rel", &rel);
            } else {
                let _ = el.remove_attribute("rel");
            }
        } else {
            let _ = el.remove_attribute("href");
            let _ = el.remove_attribute("target");
            let _ = el.remove_attribute("rel");
        }

        if let Some(img) = store.try_row::<Image>(id) {
            if !img.src().is_none()
                && let Some(nve_tables::tables::ImageSrcView::Url(url)) =
                    store.resource::<ImageSrc>(img.src())
                && is_safe_web_url(url)
            {
                let _ = el.set_attribute("src", url);
            } else {
                let _ = el.remove_attribute("src");
            }
            set_or_remove_attr(el, "alt", img.alt());
        } else {
            let _ = el.remove_attribute("alt");
        }

        if let Some(ti) = store.try_row::<TextInput>(id) {
            let val = ti.value();
            // Adopted SSR controls keep their pre-hydration `.value` until 3-way reconciliation.
            #[cfg(feature = "hydrate")]
            let defer_value = self.deferred_controls.contains(&id);
            #[cfg(not(feature = "hydrate"))]
            let defer_value = false;
            let write_value = self
                .input_sync
                .borrow_mut()
                .on_backend_value_commit(id, val, defer_value);
            if write_value {
                crate::input_attrs::write_dom_control_value(el, val);
            }
            if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
                if ti.secure() {
                    inp.set_type("password");
                } else if ti.input_kind() == nve_tables::tables::InputKind::Text {
                    let _ = el.remove_attribute("type");
                } else {
                    inp.set_type(ti.input_kind().keyword());
                }
            }
            set_or_remove_attr(el, "placeholder", ti.placeholder());
            if let Some(ac) = ti.autofill() {
                let _ = el.set_attribute("autocomplete", ac.whatwg_token());
            } else {
                let _ = el.remove_attribute("autocomplete");
            }
            if ti.input_mode() != nve_tables::InputMode::Text {
                let _ = el.set_attribute("inputmode", ti.input_mode().whatwg_token());
            } else {
                let _ = el.remove_attribute("inputmode");
            }
            if ti.required() {
                let _ = el.set_attribute("required", "");
            } else {
                let _ = el.remove_attribute("required");
            }
            if ti.read_only() {
                let _ = el.set_attribute("readonly", "");
            } else {
                let _ = el.remove_attribute("readonly");
            }
            if let Some(ml) = ti.max_length() {
                let _ = el.set_attribute("maxlength", &ml.to_string());
            } else {
                let _ = el.remove_attribute("maxlength");
            }
        }
        if let Some(inter) = store.try_row::<Interaction>(id) {
            if inter.disabled() {
                let _ = el.set_attribute("disabled", "");
                let _ = el.set_attribute("aria-disabled", "true");
            } else {
                let _ = el.remove_attribute("disabled");
                let _ = el.remove_attribute("aria-disabled");
            }
            if let Some(ti_idx) = inter.tab_index() {
                let _ = el.set_attribute("tabindex", &ti_idx.to_string());
            } else {
                let _ = el.remove_attribute("tabindex");
            }
            if inter.capture_mode() == PointerCaptureMode::OnPrimaryDown {
                let _ = el.set_attribute("data-nve-capture", "primary");
            } else {
                let _ = el.remove_attribute("data-nve-capture");
            }
        } else {
            let _ = el.remove_attribute("disabled");
            let _ = el.remove_attribute("aria-disabled");
            let _ = el.remove_attribute("tabindex");
            let _ = el.remove_attribute("data-nve-capture");
        }
        if let Some(ov) = store.try_row::<Overlay>(id) {
            let state = if ov.options().contains(OverlayFlags::OPEN) {
                "open"
            } else {
                "closed"
            };
            let _ = el.set_attribute("data-nve-overlay", state);
            self.overlay_anchors.sync_node(store, id, el);
        } else {
            let _ = el.remove_attribute("data-nve-overlay");
            self.overlay_anchors.remove(id);
        }
        if id != NodeId::ROOT {
            crate::aria::sync_accessibility_dom_attrs(store, id, el);
        }
        self.sync_requested_scroll(store, id, el, true);
        if store.children(id).is_empty()
            && !store.has::<TextInput>(id)
            && self.renderer.plugin_inner_html(id).is_none()
            && let Some(tc) = store.try_row::<TextContent>(id)
        {
            let txt = tc.text();
            if el.text_content().as_deref() != Some(txt) {
                el.set_text_content(Some(txt));
            }
        }
    }
}

#[inline]
fn set_or_remove_attr(el: &web_sys::Element, name: &str, value: &str) {
    if value.is_empty() {
        let _ = el.remove_attribute(name);
    } else {
        let _ = el.set_attribute(name, value);
    }
}

fn create_dom_element(
    doc: &web_sys::Document,
    id: NodeId,
    tag: &str,
    ns: Option<&'static str>,
) -> web_sys::Element {
    let el = if let Some(ns_uri) = ns {
        doc.create_element_ns(Some(ns_uri), tag)
            .unwrap_or_else(|_| doc.create_element("div").expect("create fallback div"))
    } else {
        doc.create_element(tag)
            .unwrap_or_else(|_| doc.create_element("div").expect("create fallback div"))
    };
    let _ = el.set_attribute("data-nve-id", &id.raw().to_string());
    el
}
