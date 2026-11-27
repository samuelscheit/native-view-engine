//! Arena-backed iterative HTML & Declarative Shadow DOM tree parser + slot promotion for
//! headless native hydration simulation ([`NativeSsrTree`]).

use nve_abi::hash::HashMap;
use nve_abi::{NodeId, ROOT_NODE_ID};

use crate::hydrate::{ssr_select_base_value, SsrSelectOption};

/// A parsed `<select>`: its list of options, resolved to the markup base value after parsing.
struct SelectScratch {
    idx: usize,
    multiple: bool,
    size: Option<String>,
    options: Vec<SsrSelectOption>,
    /// Arena index of the open `<option>` collecting text into `options.last()`.
    open_option: Option<usize>,
}

#[inline]
fn is_form_control_tag(tag: &str) -> bool {
    matches!(tag, "input" | "textarea" | "select")
}

/// One parsed HTML element inside [`NativeSsrTree`].
#[derive(Debug, Clone)]
pub(crate) struct NativeHtmlNode {
    pub(crate) tag: String,
    pub(crate) ssr_id: Option<NodeId>,
    pub(crate) instance_id: u64,
    pub(crate) base_control_value: Option<String>,
    pub(crate) live_control_value: Option<String>,
    pub(crate) slot_name: Option<String>,
    pub(crate) data_nve_slot: Option<String>,
    pub(crate) data_nve_dsd_host: bool,
    pub(crate) is_shadow_template: bool,
    pub(crate) is_app_container: bool,
    pub(crate) ssr_hash: Option<String>,
    pub(crate) closed: bool,
    pub(crate) children: Vec<usize>,
}

/// Arena-backed element tree parsed from SSR or streaming DSD HTML for headless native hydration.
#[derive(Debug, Clone, Default)]
pub(crate) struct NativeSsrTree {
    pub(crate) arena: Vec<NativeHtmlNode>,
    pub(crate) root_idx: Option<usize>,
    pub(crate) root_hash: Option<String>,
    pub(crate) has_base_style: bool,
    pub(crate) seeded_hashes: Vec<String>,
    pub(crate) next_instance_id: u64,
}

impl NativeSsrTree {
    #[inline]
    pub(crate) fn is_ignored_node(node: &NativeHtmlNode) -> bool {
        matches!(node.tag.as_str(), "style" | "script" | "template") || node.data_nve_dsd_host
    }

    /// Finds the first reachable node under `root_idx` whose `data-nve-id` equals `ssr_id`.
    pub(crate) fn find_by_ssr_id(&self, ssr_id: NodeId) -> Option<usize> {
        let root = self.root_idx?;
        let mut stack = vec![root];
        while let Some(idx) = stack.pop() {
            let node = self.arena.get(idx)?;
            if node.ssr_id == Some(ssr_id) {
                return Some(idx);
            }
            for &cid in node.children.iter().rev() {
                stack.push(cid);
            }
        }
        None
    }

    /// Returns the ordered UI element children of `dom_idx`, skipping non-UI siblings
    /// (`<style>`, `<script>`, `<template>`, `[data-nve-dsd-host]`).
    pub(crate) fn ui_children_of(&self, dom_idx: usize) -> Vec<usize> {
        self.arena.get(dom_idx).map_or_else(Vec::new, |node| {
            node.children
                .iter()
                .copied()
                .filter(|&cid| self.arena.get(cid).is_some_and(|c| !Self::is_ignored_node(c)))
                .collect()
        })
    }
}

#[inline]
fn is_html_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn unescape_html_text(val: &str) -> String {
    if !val.contains('&') {
        return val.to_owned();
    }
    const ENTITIES: [(&str, char); 5] = [
        ("&quot;", '"'),
        ("&#39;", '\''),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&amp;", '&'),
    ];
    let (mut out, mut rest) = (String::with_capacity(val.len()), val);
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        if let Some((after, ch)) = ENTITIES
            .iter()
            .find_map(|&(p, c)| rest.strip_prefix(p).map(|a| (a, c)))
        {
            out.push(ch);
            rest = after;
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn parse_html_attrs(attrs: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let bytes = attrs.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "owned arena attribute name, normalized once at parse (cold headless simulator)"
        )]
        let name = attrs[name_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let v_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                let raw = &attrs[v_start..i];
                if i < bytes.len() {
                    i += 1;
                }
                Some(unescape_html_text(raw))
            } else {
                let v_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                Some(unescape_html_text(&attrs[v_start..i]))
            }
        } else {
            None
        };
        if !name.is_empty() {
            out.push((name, value));
        }
    }
    out
}

fn attr_val<'a>(attrs: &'a [(String, Option<String>)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_deref().unwrap_or(""))
}

fn has_attr(attrs: &[(String, Option<String>)], name: &str) -> bool {
    attrs.iter().any(|(k, _)| k == name)
}

fn find_close_raw_tag(html: &str, from: usize, close_tag_lower: &[u8]) -> usize {
    let bytes = html.as_bytes();
    let nlen = close_tag_lower.len();
    let mut i = from;
    while i + nlen <= bytes.len() {
        if bytes[i] == b'<'
            && bytes[i + 1] == b'/'
            && bytes[i..i + nlen].eq_ignore_ascii_case(close_tag_lower)
        {
            let mut end = i + nlen;
            while end < bytes.len() && bytes[end] != b'>' {
                end += 1;
            }
            return if end < bytes.len() { end + 1 } else { bytes.len() };
        }
        i += 1;
    }
    bytes.len()
}

/// Parses an SSR fragment OR full/partial streaming DSD HTML document into a [`NativeSsrTree`]
/// and runs simulated Declarative Shadow DOM slot flattening (`promote_dsd_in_native_tree`).
pub(crate) fn parse_ssr_html_tree(html: &str) -> NativeSsrTree {
    let mut tree = NativeSsrTree {
        next_instance_id: 1,
        ..Default::default()
    };
    let mut top_level: Vec<usize> = Vec::new();
    let mut open_stack: Vec<usize> = Vec::new();
    let mut selects: Vec<SelectScratch> = Vec::new();
    let mut disabled_groups: Vec<usize> = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            let text_start = i;
            while i < bytes.len() && bytes[i] != b'<' {
                i += 1;
            }
            if let Some(&top) = open_stack.last() {
                let tag = tree.arena[top].tag.as_str();
                if tag == "textarea" {
                    let decoded = unescape_html_text(&html[text_start..i]);
                    if let Some(ref mut base) = tree.arena[top].base_control_value {
                        base.push_str(&decoded);
                    }
                    if let Some(ref mut live) = tree.arena[top].live_control_value {
                        live.push_str(&decoded);
                    }
                } else if tag == "option"
                    && let Some(sel) = selects
                        .iter_mut()
                        .rev()
                        .find(|s| s.open_option == Some(top))
                    && let Some(opt) = sel.options.last_mut()
                {
                    opt.text.push_str(&unescape_html_text(&html[text_start..i]));
                }
            }
            continue;
        }

        if html[i..].starts_with("<!--") {
            if let Some(rel) = html[i + 4..].find("-->") {
                i = i + 4 + rel + 3;
            } else {
                break;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i + 1] == b'!' {
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            continue;
        }
        if i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let start = i + 2;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'>' {
                end += 1;
            }
            if end >= bytes.len() {
                break;
            }
            let close_name = html[start..end].trim();
            if let Some(pos) = open_stack
                .iter()
                .rposition(|&idx| tree.arena[idx].tag.eq_ignore_ascii_case(close_name))
            {
                let closed_idx = open_stack[pos];
                tree.arena[closed_idx].closed = true;
                for sel in &mut selects {
                    if sel.open_option == Some(closed_idx) {
                        sel.open_option = None;
                    }
                }
                open_stack.truncate(pos);
            }
            i = end + 1;
            continue;
        }

        let start = i + 1;
        let mut end = start;
        let mut quote: Option<u8> = None;
        while end < bytes.len() {
            let b = bytes[end];
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
            } else if b == b'"' || b == b'\'' {
                quote = Some(b);
            } else if b == b'>' {
                break;
            }
            end += 1;
        }
        if end >= bytes.len() {
            break;
        }

        let raw_inner = html[start..end].trim();
        let self_closing = raw_inner.ends_with('/');
        let inner = if self_closing {
            raw_inner[..raw_inner.len() - 1].trim_end()
        } else {
            raw_inner
        };
        let mut parts = inner.splitn(2, char::is_whitespace);
        #[expect(
            clippy::disallowed_methods,
            reason = "owned arena tag, normalized once at parse (cold headless simulator)"
        )]
        let tag = parts.next().unwrap_or("").to_ascii_lowercase();
        let attrs = parse_html_attrs(parts.next().unwrap_or(""));
        i = end + 1;

        if tag.is_empty() {
            continue;
        }

        if tag == "style" || tag == "script" {
            if tag == "style" {
                if has_attr(&attrs, "data-nve-base") {
                    tree.has_base_style = true;
                }
                if (has_attr(&attrs, "data-nve-ssr") || has_attr(&attrs, "data-nve-ssr-chunk"))
                    && let Some(csv) = attr_val(&attrs, "data-nve-hashes")
                {
                    for tok in csv.split(',') {
                        let t = tok.trim();
                        if !t.is_empty() {
                            tree.seeded_hashes.push(t.to_owned());
                        }
                    }
                }
            }
            if !self_closing {
                let close_pat: &[u8] = if tag == "style" { b"</style" } else { b"</script" };
                i = find_close_raw_tag(html, i, close_pat);
            }
            continue;
        }

        let ssr_id = attr_val(&attrs, "data-nve-id").and_then(|s| s.parse::<NodeId>().ok());
        let ssr_hash = attr_val(&attrs, "data-nve-ssr-hash").map(str::to_owned);
        let base_control_value = if is_form_control_tag(&tag) {
            Some(if tag == "input" {
                attr_val(&attrs, "value").unwrap_or("").to_owned()
            } else {
                String::new()
            })
        } else {
            None
        };
        let live_control_value = base_control_value.clone();
        let is_void = self_closing || is_html_void_tag(&tag);
        let instance_id = tree.next_instance_id;
        tree.next_instance_id += 1;

        let idx = tree.arena.len();
        tree.arena.push(NativeHtmlNode {
            tag: tag.clone(),
            ssr_id,
            instance_id,
            base_control_value,
            live_control_value,
            slot_name: (tag == "slot")
                .then(|| attr_val(&attrs, "name").map(str::to_owned))
                .flatten(),
            data_nve_slot: attr_val(&attrs, "data-nve-slot").map(str::to_owned),
            data_nve_dsd_host: has_attr(&attrs, "data-nve-dsd-host"),
            is_shadow_template: tag == "template" && has_attr(&attrs, "shadowrootmode"),
            is_app_container: attr_val(&attrs, "id") == Some("app"),
            ssr_hash,
            closed: is_void,
            children: Vec::new(),
        });

        if tag == "select" {
            selects.push(SelectScratch {
                idx,
                multiple: has_attr(&attrs, "multiple"),
                size: attr_val(&attrs, "size").map(str::to_owned),
                options: Vec::new(),
                open_option: None,
            });
        }
        if let Some(&parent_idx) = open_stack.last() {
            tree.arena[parent_idx].children.push(idx);
            let parent_tag = tree.arena[parent_idx].tag.as_str();
            let grandparent = open_stack.iter().rev().nth(1).copied();
            if tag == "optgroup" && parent_tag == "select" && has_attr(&attrs, "disabled") {
                disabled_groups.push(idx);
            }
            // The list of options: `select > option` and `select > optgroup > option`.
            let owner = match parent_tag {
                "select" if tag == "option" => Some((parent_idx, false)),
                "optgroup" if tag == "option" => grandparent
                    .filter(|&g| tree.arena[g].tag == "select")
                    .map(|g| (g, disabled_groups.contains(&parent_idx))),
                _ => None,
            };
            if let Some((select_idx, group_disabled)) = owner
                && let Some(sel) = selects.iter_mut().rev().find(|s| s.idx == select_idx)
            {
                sel.options.push(SsrSelectOption {
                    selected: has_attr(&attrs, "selected"),
                    disabled: group_disabled || has_attr(&attrs, "disabled"),
                    value: attr_val(&attrs, "value").map(str::to_owned),
                    text: String::new(),
                });
                sel.open_option = (!is_void).then_some(idx);
            }
        } else {
            top_level.push(idx);
        }

        if !is_void {
            open_stack.push(idx);
        }
    }

    for sel in selects {
        let base = ssr_select_base_value(&sel.options, sel.multiple, sel.size.as_deref());
        tree.arena[sel.idx].live_control_value = Some(base.clone());
        tree.arena[sel.idx].base_control_value = Some(base);
    }
    promote_dsd_in_native_tree(&mut tree, &top_level);
    tree
}

fn find_root_child(arena: &[NativeHtmlNode], candidates: &[usize]) -> Option<usize> {
    candidates
        .iter()
        .copied()
        .find(|&c| arena[c].ssr_id == Some(ROOT_NODE_ID))
        .or_else(|| {
            candidates
                .iter()
                .copied()
                .find(|&c| !NativeSsrTree::is_ignored_node(&arena[c]))
        })
}

fn promote_dsd_in_native_tree(tree: &mut NativeSsrTree, top_level: &[usize]) {
    let host_idx = tree
        .arena
        .iter()
        .position(|n| n.data_nve_dsd_host)
        .or_else(|| {
            tree.arena.iter().position(|n| {
                n.is_app_container
                    && n.children
                        .iter()
                        .any(|&c| tree.arena[c].is_shadow_template)
            })
        });

    if let Some(h_idx) = host_idx {
        let shadow_idx = tree.arena[h_idx]
            .children
            .iter()
            .copied()
            .find(|&c| tree.arena[c].is_shadow_template);

        // Collect complete Light-DOM `[data-nve-slot="{id}"]` chunks (last wins).
        let mut slot_chunks: HashMap<String, usize> = HashMap::default();
        for &cid in &tree.arena[h_idx].children {
            let child = &tree.arena[cid];
            if child.closed
                && let Some(ref slot_id) = child.data_nve_slot
            {
                slot_chunks.insert(slot_id.clone(), cid);
            }
        }

        if let Some(s_idx) = shadow_idx {
            let mut worklist = vec![s_idx];
            while let Some(p_idx) = worklist.pop() {
                let old_children = tree.arena[p_idx].children.clone();
                let mut new_children = Vec::with_capacity(old_children.len());
                let mut changed = false;
                for cid in old_children {
                    if tree.arena[cid].tag == "slot"
                        && let Some(slot_id) = tree.arena[cid]
                            .slot_name
                            .as_deref()
                            .and_then(|n| n.strip_prefix("nve-slot-"))
                            .map(str::to_owned)
                    {
                        changed = true;
                        let replacement = if let Some(&chunk_idx) = slot_chunks.get(&slot_id) {
                            tree.arena[chunk_idx].children.clone()
                        } else {
                            tree.arena[cid].children.clone()
                        };
                        for gc in replacement {
                            new_children.push(gc);
                            worklist.push(gc);
                        }
                    } else {
                        new_children.push(cid);
                        worklist.push(cid);
                    }
                }
                if changed {
                    tree.arena[p_idx].children = new_children;
                }
            }
            let shadow_kids = tree.arena[s_idx].children.clone();
            tree.root_idx = find_root_child(&tree.arena, &shadow_kids);
        }
    } else if let Some(app_idx) = tree.arena.iter().position(|n| n.is_app_container) {
        tree.root_idx = if tree.arena[app_idx].ssr_id == Some(ROOT_NODE_ID) {
            Some(app_idx)
        } else {
            let kids = tree.arena[app_idx].children.clone();
            find_root_child(&tree.arena, &kids)
        };
    } else {
        tree.root_idx = find_root_child(&tree.arena, top_level);
    }

    if let Some(root) = tree.root_idx {
        tree.root_hash = tree.arena[root].ssr_hash.clone();
    }
}
