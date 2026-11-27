//! Tier-1 Web DOM hot-reload template patching (`HotPatchMsg::TemplatePatch`) over `NodeStore`.

use core::fmt::Write as _;

use nve_components::tables::{DebugId, TextContent};
use nve_engine::HotPatchMsg;
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::WebBackend;

impl WebBackend {
    /// Applies a Tier-1 [`HotPatchMsg::TemplatePatch`] directly onto `store` and `self.patch_css`,
    /// returning the number of patched nodes.
    pub fn apply_template_patch(&mut self, store: &mut NodeStore, patch: &HotPatchMsg) -> u32 {
        let mut changes = ChangeSet::default();
        self.apply_template_patch_into(store, patch, &mut changes)
    }

    /// Applies a Tier-1 [`HotPatchMsg::TemplatePatch`] directly onto `store` and `self.patch_css`,
    /// recording the resulting store delta in `out_changes` and returning the number of patched
    /// nodes.
    pub fn apply_template_patch_into(
        &mut self,
        store: &mut NodeStore,
        patch: &HotPatchMsg,
        out_changes: &mut ChangeSet,
    ) -> u32 {
        let HotPatchMsg::TemplatePatch {
            template_id,
            attr_updates,
            text_updates,
            target_test_id,
            text_replacements,
            ..
        } = patch
        else {
            return 0;
        };

        let mut live_ids: Vec<NodeId> = Vec::new();
        let mut stack = vec![NodeId::ROOT];
        while let Some(id) = stack.pop() {
            if !store.is_live(id) {
                continue;
            }
            live_ids.push(id);
            for &cid in store.children(id).iter().rev() {
                stack.push(cid);
            }
        }

        let target_tid = target_test_id.as_deref().filter(|s| !s.is_empty());

        let matching_ids: Vec<NodeId> = if let Some(tid) = target_tid {
            live_ids
                .iter()
                .copied()
                .filter(|&id| {
                    store
                        .try_row::<DebugId>(id)
                        .is_some_and(|d| d.test_id() == tid)
                })
                .collect()
        } else {
            live_ids.clone()
        };
        let _ = template_id;

        let mut mutations = 0u32;
        let mut tx = store.tx();

        for (old_text, new_text) in text_replacements {
            for &id in &live_ids {
                let matches = tx
                    .store()
                    .try_row::<TextContent>(id)
                    .is_some_and(|t| t.text() == old_text);
                if matches {
                    tx.edit::<TextContent>(id).text(new_text);
                    mutations += 1;
                }
            }
        }

        if !text_updates.is_empty() {
            let text_nodes: Vec<NodeId> = matching_ids
                .iter()
                .copied()
                .filter(|&id| {
                    tx.store().children(id).is_empty()
                        && tx
                            .store()
                            .try_row::<TextContent>(id)
                            .is_some_and(|t| !t.text().is_empty())
                })
                .collect();
            for (idx, new_text) in text_updates {
                if let Some(&id) = text_nodes.get(*idx) {
                    tx.edit::<TextContent>(id).text(new_text);
                    mutations += 1;
                }
            }
        }

        tx.commit_into(out_changes);

        if !attr_updates.is_empty() {
            let mut css = String::new();
            for (k, v) in attr_updates {
                if k == "style" {
                    css.push_str(v.trim());
                    if !css.ends_with(';') {
                        css.push(';');
                    }
                } else {
                    let _ = write!(&mut css, "{k}:{v};");
                }
            }
            for &id in &matching_ids {
                if id != NodeId::ROOT {
                    self.patch_css.entry(id).or_default().push_str(&css);
                    mutations += 1;
                }
            }
        }

        if !out_changes.touched.is_empty() {
            let _ = nve_engine::StoreBackend::commit(self, store, out_changes);
        }
        #[cfg(target_arch = "wasm32")]
        if !attr_updates.is_empty() {
            for &id in &matching_ids {
                if id != NodeId::ROOT
                    && let Some(el) = self.live_elements.get(&id).cloned()
                {
                    self.sync_full_element(store, id, &el);
                }
            }
        }
        mutations
    }
}
