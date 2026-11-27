//! Live `wasm32` DOM mounting, zero-flicker hydration adoption, deferred element creation,
//! `COMPOSITE`-only 120 Hz fast lane, and `WebTableRegistry` plugin DOM synchronization.

use nve_abi::table::effect;
use nve_components::tables::{
    Accessibility, DebugId, Image, ImageSrc, Interaction, Link, LinkTarget, TextContent,
    TextInput, Transform, Visual,
};
use nve_store::{ChangeSet, NodeId, NodeStore};
use wasm_bindgen::JsCast;

use crate::css::write_compositor_css;
use crate::events::RootEventDelegator;
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
        let delegator = RootEventDelegator::attach(
            container.clone(),
            std::rc::Rc::clone(&backend.surface_id),
            std::rc::Rc::clone(&backend.pending_events),
            std::rc::Rc::clone(&backend.node_event_meta),
            std::rc::Rc::clone(&backend.input_sync),
            std::rc::Rc::clone(&backend.ime_guard),
            std::rc::Rc::clone(&backend.on_event),
        );
        backend.delegated_events = Some(delegator);
        backend.mount_root = Some(container);
        backend.attach_visual_viewport_listener();
        backend
    }

    /// Scans an SSR-rendered `container` for existing `[data-nve-id]` elements and stages them
    /// into `live_elements` for tag-verified adoption during the first hydrated commit.
    pub(crate) fn adopt_existing_dom(&mut self, container: &web_sys::Element) {
        if let Ok(list) = container.query_selector_all("[data-nve-id]") {
            let len = list.length();
            for i in 0..len {
                if let Some(node) = list.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                    && let Some(raw) = el.get_attribute("data-nve-id")
                    && let Ok(raw_id) = raw.parse::<u32>()
                {
                    let id = NodeId(raw_id);
                    self.live_elements.insert(id, el);
                    self.unverified_ssr_ids.insert(id);
                }
            }
        }
        if let Some(doc) = web_sys::window().and_then(|w| w.document())
            && let Ok(Some(style_el)) = doc.query_selector("style[data-nve-ssr]")
            && let Some(hashes) = style_el.get_attribute("data-nve-hashes")
        {
            let before = self.stylesheet.rule_count();
            self.stylesheet.seed_ssr_hashes(hashes.split(','));
            #[cfg(feature = "hydrate")]
            {
                self.hydration_report.reused_css_rules =
                    self.stylesheet.rule_count().saturating_sub(before) as u32;
            }
            let _ = before;
        }
    }

    pub(crate) fn clear_live_dom(&mut self) {
        if let Some(ref root) = self.mount_root {
            root.set_inner_html("");
        }
        self.live_elements.clear();
        self.unverified_ssr_ids.clear();
    }

    pub(crate) fn commit_live_dom(&mut self, store: &NodeStore, changes: &ChangeSet) -> u32 {
        let Some(mount_root) = self.mount_root.clone() else {
            return 0;
        };
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return 0;
        };

        // 1. Notify plugins BEFORE removing destroyed DOM elements so `remove_node` sees `live_element: Some(el)`.
        self.renderer
            .remove_plugins(changes, Some(&self.live_elements));
        for &id in &changes.removed {
            self.unverified_ssr_ids.remove(&id);
            if let Some(el) = self.live_elements.remove(&id) {
                el.remove();
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
            let was_ssr_candidate = self.unverified_ssr_ids.remove(&id);
            let el = if let Some(adopted) = self.live_elements.get(&id).cloned() {
                if adopted.tag_name().eq_ignore_ascii_case(tag) {
                    #[cfg(feature = "hydrate")]
                    if was_ssr_candidate {
                        self.hydration_report.adopted_nodes += 1;
                    }
                    adopted
                } else {
                    let fresh = create_dom_element(&doc, id, tag, ns);
                    move_dom_children(&adopted, &fresh);
                    let _ = adopted.replace_with_with_node_1(&fresh);
                    self.live_elements.insert(id, fresh.clone());
                    retagged.push(id);
                    #[cfg(feature = "hydrate")]
                    if was_ssr_candidate {
                        self.hydration_report.replaced_nodes += 1;
                        let mut ssr_tag = adopted.tag_name();
                        ssr_tag.make_ascii_lowercase();
                        self.hydration_report.mismatches.push(crate::HydrationMismatch {
                            node_id: id,
                            reason: format!("SSR <{ssr_tag}> != live <{tag}>"),
                        });
                    }
                    fresh
                }
            } else {
                let fresh = create_dom_element(&doc, id, tag, ns);
                self.live_elements.insert(id, fresh.clone());
                fresh
            };
            self.sync_full_element(store, id, &el);
        }

        // Ensure ROOT is mounted into `mount_root` and verify ROOT hydration if present.
        if self.unverified_ssr_ids.remove(&NodeId::ROOT) {
            #[cfg(feature = "hydrate")]
            if let Some(root_el) = self.live_elements.get(&NodeId::ROOT) {
                if root_el.tag_name().eq_ignore_ascii_case("div") {
                    self.hydration_report.adopted_nodes += 1;
                } else {
                    self.hydration_report.replaced_nodes += 1;
                }
            }
        }
        if let Some(root_el) = self.live_elements.get(&NodeId::ROOT)
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
                if let Some(el) = self.live_elements.get(&id) {
                    apply_compositor_properties_live(store, id, el);
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
                    move_dom_children(&existing, &fresh);
                    let _ = existing.replace_with_with_node_1(&fresh);
                    self.live_elements.insert(id, fresh.clone());
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

        // 3b. Prune any dead SSR DOM nodes left unvisited after the first hydrated commit.
        if !self.unverified_ssr_ids.is_empty() {
            let dead_ids: Vec<NodeId> = self.unverified_ssr_ids.drain().collect();
            for node_id in dead_ids {
                if !store.is_live(node_id) {
                    if let Some(stale_el) = self.live_elements.remove(&node_id) {
                        stale_el.remove();
                        #[cfg(feature = "hydrate")]
                        {
                            self.hydration_report.replaced_nodes += 1;
                            let mut ssr_tag = stale_el.tag_name();
                            ssr_tag.make_ascii_lowercase();
                            self.hydration_report.mismatches.push(crate::HydrationMismatch {
                                node_id,
                                reason: format!("SSR <{ssr_tag}> missing from live NodeStore"),
                            });
                        }
                    }
                } else {
                    #[cfg(feature = "hydrate")]
                    {
                        self.hydration_report.adopted_nodes += 1;
                    }
                }
            }
        }

        // 4. Sync plugin live elements (`WebTableRegistry`), commands, and retagged nodes ONCE.
        let plugin_muts = self.renderer.commit_plugins_active(
            store,
            changes,
            Some(&self.live_elements),
            &retagged,
        );
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
                    let _ = parent_el.insert_before(child_el, ref_node);
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

        // 6. Flush newly compiled CSS rules into adoptedStyleSheets / <style>.
        self.stylesheet.flush_to_dom(&doc);
        plugin_muts
    }

    pub(crate) fn sync_full_element(&mut self, store: &NodeStore, id: NodeId, el: &web_sys::Element) {
        let resolved = self.stylesheet.resolve_node(store, id);
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

        if let Some(dbg) = store.try_row::<DebugId>(id) {
            set_or_remove_attr(el, "id", dbg.id());
            set_or_remove_attr(el, "data-testid", dbg.test_id());
            set_or_remove_attr(el, "slot", dbg.slot());
        } else {
            let _ = el.remove_attribute("id");
            let _ = el.remove_attribute("data-testid");
            let _ = el.remove_attribute("slot");
        }

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
            let rel_bits = if is_blank {
                link.rel() | 1 | 2
            } else {
                link.rel()
            };
            if rel_bits != 0 {
                let mut rel = String::new();
                for (bit, kw) in [(1u8, "noopener"), (2u8, "noreferrer"), (4u8, "nofollow")] {
                    if rel_bits & bit != 0 {
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
                && let Some(ImageSrc::Url(url)) = store.resource::<ImageSrc>(img.src())
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
            self.input_sync.borrow_mut().on_backend_value_sync(id, val);
            if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
                if inp.value() != val {
                    inp.set_value(val);
                }
                if ti.secure() {
                    inp.set_type("password");
                } else if ti.input_kind() == nve_components::tables::InputKind::Text {
                    let _ = el.remove_attribute("type");
                } else {
                    inp.set_type(ti.input_kind().as_html_type());
                }
            } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>()
                && ta.value() != val
            {
                ta.set_value(val);
            }
            set_or_remove_attr(el, "placeholder", ti.placeholder());
            if let Some(ac) = ti.autofill().get() {
                let _ = el.set_attribute("autocomplete", ac.as_html_autocomplete());
            } else {
                let _ = el.remove_attribute("autocomplete");
            }
            if ti.input_mode() != nve_components::InputMode::Text {
                let _ = el.set_attribute("inputmode", ti.input_mode().as_html_inputmode());
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
            if ti.max_length() > 0 && ti.max_length() < u16::MAX {
                let _ = el.set_attribute("maxlength", &ti.max_length().to_string());
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
            if inter.tab_index_set() {
                let _ = el.set_attribute("tabindex", &inter.tab_index().to_string());
            } else {
                let _ = el.remove_attribute("tabindex");
            }
        } else {
            let _ = el.remove_attribute("disabled");
            let _ = el.remove_attribute("aria-disabled");
            let _ = el.remove_attribute("tabindex");
        }
        if id != NodeId::ROOT {
            if let Some(a11y) = store.try_row::<Accessibility>(id) {
                set_or_remove_attr(el, "role", a11y.role().as_aria_str());
                set_or_remove_attr(el, "aria-label", a11y.label());
                set_or_remove_attr(el, "aria-live", a11y.live().as_aria_str().unwrap_or(""));
                if a11y.hidden() {
                    let _ = el.set_attribute("aria-hidden", "true");
                } else {
                    let _ = el.remove_attribute("aria-hidden");
                }
            } else {
                let _ = el.remove_attribute("role");
                let _ = el.remove_attribute("aria-label");
                let _ = el.remove_attribute("aria-live");
                let _ = el.remove_attribute("aria-hidden");
            }
        }
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

/// Applies `effect::COMPOSITE` properties (`opacity`, `z-index`, `transform`, `transform-origin`)
/// directly via CSSOM `style.setProperty`/`removeProperty` so `inline_fallback`, plugin inline CSS
/// (e.g. `backdrop-filter`), and `patch_css` are never clobbered on the 120 Hz fast lane.
fn apply_compositor_properties_live(store: &NodeStore, id: NodeId, el: &web_sys::Element) {
    let style = if let Some(html_el) = el.dyn_ref::<web_sys::HtmlElement>() {
        html_el.style()
    } else if let Some(svg_el) = el.dyn_ref::<web_sys::SvgElement>() {
        svg_el.style()
    } else {
        return;
    };
    if let Some(v) = store.try_row::<Visual>(id) {
        if (v.opacity() - 1.0).abs() > 1e-4 {
            let mut buf = String::with_capacity(8);
            crate::css::write_compact_num(v.opacity().clamp(0.0, 1.0), &mut buf);
            let _ = style.set_property("opacity", &buf);
        } else {
            let _ = style.remove_property("opacity");
        }
        if v.z_index() != 0 {
            let _ = style.set_property("z-index", &v.z_index().to_string());
        } else {
            let _ = style.remove_property("z-index");
        }
    } else {
        let _ = style.remove_property("opacity");
        let _ = style.remove_property("z-index");
    }

    if let Some(tf) = store.try_row::<Transform>(id) {
        let (tx, ty) = (tf.translate_x(), tf.translate_y());
        let (sx, sy) = (tf.scale_x(), tf.scale_y());
        let rot = tf.rotate_deg();
        if tx != 0.0
            || ty != 0.0
            || (sx - 1.0).abs() > 1e-4
            || (sy - 1.0).abs() > 1e-4
            || rot != 0.0
        {
            let mut val = String::with_capacity(48);
            let mut first = true;
            if tx != 0.0 || ty != 0.0 {
                val.push_str("translate(");
                crate::css::write_compact_px(tx, &mut val);
                val.push(',');
                crate::css::write_compact_px(ty, &mut val);
                val.push(')');
                first = false;
            }
            if (sx - 1.0).abs() > 1e-4 || (sy - 1.0).abs() > 1e-4 {
                if !first {
                    val.push(' ');
                }
                val.push_str("scale(");
                crate::css::write_compact_num(sx, &mut val);
                val.push(',');
                crate::css::write_compact_num(sy, &mut val);
                val.push(')');
                first = false;
            }
            if rot != 0.0 {
                if !first {
                    val.push(' ');
                }
                val.push_str("rotate(");
                crate::css::write_compact_num(rot, &mut val);
                val.push_str("deg)");
            }
            let _ = style.set_property("transform", &val);
        } else {
            let _ = style.remove_property("transform");
        }
        if (tf.origin_x() - 0.5).abs() > 1e-4 || (tf.origin_y() - 0.5).abs() > 1e-4 {
            let mut orig = String::with_capacity(20);
            crate::css::write_compact_num(tf.origin_x() * 100.0, &mut orig);
            orig.push_str("% ");
            crate::css::write_compact_num(tf.origin_y() * 100.0, &mut orig);
            orig.push('%');
            let _ = style.set_property("transform-origin", &orig);
        } else {
            let _ = style.remove_property("transform-origin");
        }
    } else {
        let _ = style.remove_property("transform");
        let _ = style.remove_property("transform-origin");
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

fn move_dom_children(from: &web_sys::Element, to: &web_sys::Element) {
    while let Some(child) = from.first_child() {
        let _ = to.append_child(&child);
    }
}
