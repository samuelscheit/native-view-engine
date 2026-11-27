//! Notion-class block document domain model, Markdown exporter, and deterministic
//! time-travel actions for `ActiveView::Docs`.

use super::WorkspaceState;

/// Structural block kinds supported in Aether Workspace Docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocBlockKind {
    Heading1,
    Heading2,
    Paragraph,
    Checklist { checked: bool },
    Code,
    Callout,
}

impl DocBlockKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Heading1 => "H1",
            Self::Heading2 => "H2",
            Self::Paragraph => "Text",
            Self::Checklist { .. } => "Todo",
            Self::Code => "Code",
            Self::Callout => "Callout",
        }
    }

    pub const fn next_kind(self) -> Self {
        match self {
            Self::Heading1 => Self::Heading2,
            Self::Heading2 => Self::Paragraph,
            Self::Paragraph => Self::Checklist { checked: false },
            Self::Checklist { .. } => Self::Code,
            Self::Code => Self::Callout,
            Self::Callout => Self::Heading1,
        }
    }

    pub const fn is_checklist(self) -> bool {
        matches!(self, Self::Checklist { .. })
    }

    pub const fn is_checked(self) -> bool {
        matches!(self, Self::Checklist { checked: true })
    }
}

/// One typed block inside a [`WorkspaceDoc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocBlock {
    pub id: u32,
    pub kind: DocBlockKind,
    pub text: String,
}

/// A rich local-first workspace engineering document with typed blocks and Markdown export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDoc {
    pub id: u32,
    pub slug: String,
    pub title: String,
    pub updated_at: String,
    pub blocks: Vec<DocBlock>,
}

impl WorkspaceDoc {
    /// Exports this document to clean GitHub-Flavored Markdown.
    pub fn to_markdown(&self) -> String {
        let mut out = String::with_capacity(self.blocks.len() * 64 + 64);
        let has_leading_h1 = self
            .blocks
            .first()
            .is_some_and(|b| matches!(b.kind, DocBlockKind::Heading1));
        if !has_leading_h1 && !self.title.is_empty() {
            out.push_str("# ");
            out.push_str(&self.title);
            if !self.blocks.is_empty() {
                out.push_str("\n\n");
            }
        }
        for (idx, block) in self.blocks.iter().enumerate() {
            if idx > 0 {
                let prev_is_check = self.blocks[idx - 1].kind.is_checklist();
                let cur_is_check = block.kind.is_checklist();
                if prev_is_check && cur_is_check {
                    out.push('\n');
                } else {
                    out.push_str("\n\n");
                }
            }
            match block.kind {
                DocBlockKind::Heading1 => {
                    out.push_str("# ");
                    out.push_str(&block.text);
                }
                DocBlockKind::Heading2 => {
                    out.push_str("## ");
                    out.push_str(&block.text);
                }
                DocBlockKind::Paragraph => {
                    out.push_str(&block.text);
                }
                DocBlockKind::Checklist { checked } => {
                    if checked {
                        out.push_str("- [x] ");
                    } else {
                        out.push_str("- [ ] ");
                    }
                    out.push_str(&block.text);
                }
                DocBlockKind::Code => {
                    out.push_str("```\n");
                    out.push_str(&block.text);
                    out.push_str("\n```");
                }
                DocBlockKind::Callout => {
                    out.push_str("> ");
                    out.push_str(&block.text);
                }
            }
        }
        out
    }

    /// Returns `(completed, total)` checklist item counts in this document.
    pub fn checklist_progress(&self) -> (usize, usize) {
        let mut done = 0usize;
        let mut total = 0usize;
        for block in &self.blocks {
            if let DocBlockKind::Checklist { checked } = block.kind {
                total += 1;
                if checked {
                    done += 1;
                }
            }
        }
        (done, total)
    }
}

/// Seeds the workspace with 2 realistic engineering specs (`RFC-042` and `Release Checklist: v1.0`).
/// Returns `(docs, active_doc_id, next_doc_id, next_block_id)`.
pub fn default_seed_docs() -> (Vec<WorkspaceDoc>, u32, u32, u32) {
    let rfc = WorkspaceDoc {
        id: 1,
        slug: "rfc-042".into(),
        title: "RFC-042: Zero-Copy NveSceneView".into(),
        updated_at: "10:14".into(),
        blocks: vec![
            DocBlock {
                id: 1,
                kind: DocBlockKind::Heading1,
                text: "RFC-042: Zero-Copy NveSceneView".into(),
            },
            DocBlock {
                id: 2,
                kind: DocBlockKind::Callout,
                text: "Status: Accepted | Single CommandBuffer + NVE_DRAW1 across 9 host shells."
                    .into(),
            },
            DocBlock {
                id: 3,
                kind: DocBlockKind::Heading2,
                text: "Binary ABI Invariants".into(),
            },
            DocBlock {
                id: 4,
                kind: DocBlockKind::Paragraph,
                text: "Every frame crosses the Rust/host boundary as one flat binary CommandBuffer with direct binary FFI or per-prop FFI calls."
                    .into(),
            },
            DocBlock {
                id: 5,
                kind: DocBlockKind::Code,
                text: "nve_get_interface(NVE_IFACE_SURFACE) -> submit_frame(&[u8])".into(),
            },
            DocBlock {
                id: 6,
                kind: DocBlockKind::Checklist { checked: true },
                text: "Eliminate JSON scene descriptors across SwiftUI, Flutter, Qt 6, and GTK4"
                    .into(),
            },
            DocBlock {
                id: 7,
                kind: DocBlockKind::Checklist { checked: false },
                text: "Enable crash-safe NVERELOC + MmapOffsetArena local persistence in Studio"
                    .into(),
            },
        ],
    };

    let release = WorkspaceDoc {
        id: 2,
        slug: "release-v1".into(),
        title: "Release Checklist: v1.0".into(),
        updated_at: "10:28".into(),
        blocks: vec![
            DocBlock {
                id: 8,
                kind: DocBlockKind::Heading1,
                text: "Release Checklist: v1.0".into(),
            },
            DocBlock {
                id: 9,
                kind: DocBlockKind::Callout,
                text:
                    "Target: v1.0 GA | All 9 native & web hosts passing parity and latency budgets."
                        .into(),
            },
            DocBlock {
                id: 10,
                kind: DocBlockKind::Heading2,
                text: "Pre-Flight Gates".into(),
            },
            DocBlock {
                id: 11,
                kind: DocBlockKind::Checklist { checked: true },
                text: "Verify p99 layout diff holds under 0.5ms at 1,000 nodes".into(),
            },
            DocBlock {
                id: 12,
                kind: DocBlockKind::Checklist { checked: true },
                text: "Verify Web WASM E2E passes Desktop 1280x800 and Mobile 393x852".into(),
            },
            DocBlock {
                id: 13,
                kind: DocBlockKind::Checklist { checked: false },
                text: "Cut signed v1.0 release bundles for Apple, Android, Linux, Windows, and Web"
                    .into(),
            },
            DocBlock {
                id: 14,
                kind: DocBlockKind::Paragraph,
                text: "Refresh quiet-machine cargo nve bench baselines prior to tagging main."
                    .into(),
            },
            DocBlock {
                id: 15,
                kind: DocBlockKind::Code,
                text:
                    "RUSTFLAGS=\"-D warnings\" cargo test --workspace && ./hosts/web-shell/build.sh"
                        .into(),
            },
        ],
    };

    (vec![rfc, release], 1, 3, 16)
}

impl WorkspaceState {
    /// Returns the currently selected document (falling back to the first document if any).
    pub fn active_doc(&self) -> Option<&WorkspaceDoc> {
        self.current
            .docs
            .iter()
            .find(|d| d.id == self.current.active_doc_id)
            .or_else(|| self.current.docs.first())
    }

    /// Exports the currently selected document as Markdown.
    pub fn active_doc_markdown(&self) -> String {
        self.active_doc()
            .map(WorkspaceDoc::to_markdown)
            .unwrap_or_default()
    }

    pub fn select_doc(&mut self, doc_id: u32) {
        let Some(title) = self
            .current
            .docs
            .iter()
            .find(|d| d.id == doc_id)
            .map(|d| d.title.clone())
        else {
            return;
        };
        if self.current.active_doc_id == doc_id {
            return;
        }
        let label = format!("Doc -> {title}");
        self.commit_action(label, |s| {
            s.active_doc_id = doc_id;
            s.doc_editing_block_id = None;
            s.doc_edit_draft.clear();
        });
        if self.current.active_view == super::ActiveView::Docs {
            self.router
                .push(crate::routes::StudioRoute::Doc { id: doc_id });
        }
    }

    pub fn create_new_doc(&mut self) -> u32 {
        let doc_id = self.current.next_doc_id;
        let b1 = self.current.next_block_id;
        let b2 = b1.saturating_add(1);
        let title = format!("Design Spec #{doc_id:02}");
        let label = format!("New {title}");
        self.commit_action(label, |s| {
            s.docs.make_mut().push(WorkspaceDoc {
                id: doc_id,
                slug: format!("spec-{doc_id}"),
                title: title.clone(),
                updated_at: "Now".into(),
                blocks: vec![
                    DocBlock {
                        id: b1,
                        kind: DocBlockKind::Heading1,
                        text: title,
                    },
                    DocBlock {
                        id: b2,
                        kind: DocBlockKind::Checklist { checked: false },
                        text: "Capture invariants and verification criteria".into(),
                    },
                ],
            });
            s.active_doc_id = doc_id;
            s.next_doc_id = doc_id.saturating_add(1);
            s.next_block_id = b2.saturating_add(1);
            s.doc_editing_block_id = None;
            s.doc_edit_draft.clear();
        });
        doc_id
    }

    pub fn toggle_doc_checklist(&mut self, block_id: u32) -> bool {
        let active_id = self.current.active_doc_id;
        let Some(next_checked) = self
            .current
            .docs
            .iter()
            .find(|d| d.id == active_id)
            .and_then(|d| d.blocks.iter().find(|b| b.id == block_id))
            .and_then(|b| match b.kind {
                DocBlockKind::Checklist { checked } => Some(!checked),
                _ => None,
            })
        else {
            return false;
        };
        let label = if next_checked {
            format!("Check #{block_id} [x]")
        } else {
            format!("Uncheck #{block_id} [ ]")
        };
        self.commit_action(label, |s| {
            if let Some(doc) = s.docs.make_mut().iter_mut().find(|d| d.id == active_id)
                && let Some(block) = doc.blocks.iter_mut().find(|b| b.id == block_id)
            {
                block.kind = DocBlockKind::Checklist {
                    checked: next_checked,
                };
            }
        });
        true
    }

    pub fn cycle_doc_block_kind(&mut self, block_id: u32) -> bool {
        let active_id = self.current.active_doc_id;
        let Some(next_kind) = self
            .current
            .docs
            .iter()
            .find(|d| d.id == active_id)
            .and_then(|d| d.blocks.iter().find(|b| b.id == block_id))
            .map(|b| b.kind.next_kind())
        else {
            return false;
        };
        let label = format!("Block #{block_id} -> {}", next_kind.label());
        self.commit_action(label, |s| {
            if let Some(doc) = s.docs.make_mut().iter_mut().find(|d| d.id == active_id)
                && let Some(block) = doc.blocks.iter_mut().find(|b| b.id == block_id)
            {
                block.kind = next_kind;
            }
        });
        true
    }

    pub fn move_doc_block(&mut self, block_id: u32, delta: isize) -> bool {
        let active_id = self.current.active_doc_id;
        let Some((idx, len)) = self
            .current
            .docs
            .iter()
            .find(|d| d.id == active_id)
            .and_then(|d| {
                d.blocks
                    .iter()
                    .position(|b| b.id == block_id)
                    .map(|p| (p, d.blocks.len()))
            })
        else {
            return false;
        };
        let target_isize = idx as isize + delta;
        if target_isize < 0 || target_isize >= len as isize {
            return false;
        }
        let target = target_isize as usize;
        if target == idx {
            return false;
        }
        let dir = if delta < 0 { "Up" } else { "Down" };
        let label = format!("Move Block #{block_id} {dir}");
        self.commit_action(label, |s| {
            if let Some(doc) = s.docs.make_mut().iter_mut().find(|d| d.id == active_id)
                && idx < doc.blocks.len()
                && target < doc.blocks.len()
            {
                doc.blocks.swap(idx, target);
            }
        });
        true
    }

    pub fn delete_doc_block(&mut self, block_id: u32) -> bool {
        let active_id = self.current.active_doc_id;
        let exists = self
            .current
            .docs
            .iter()
            .find(|d| d.id == active_id)
            .is_some_and(|d| d.blocks.iter().any(|b| b.id == block_id));
        if !exists {
            return false;
        }
        let label = format!("Delete Block #{block_id}");
        self.commit_action(label, |s| {
            if let Some(doc) = s.docs.make_mut().iter_mut().find(|d| d.id == active_id) {
                doc.blocks.retain(|b| b.id != block_id);
            }
            if s.doc_editing_block_id == Some(block_id) {
                s.doc_editing_block_id = None;
                s.doc_edit_draft.clear();
            }
        });
        true
    }

    pub fn add_doc_block(&mut self, kind: DocBlockKind, text: &str) -> Option<u32> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        let active_id = self.current.active_doc_id;
        if !self.current.docs.iter().any(|d| d.id == active_id) {
            return None;
        }
        let block_id = self.current.next_block_id;
        let owned = trimmed.to_owned();
        let label = format!("Add {} Block", kind.label());
        self.commit_action(label, |s| {
            if let Some(doc) = s.docs.make_mut().iter_mut().find(|d| d.id == active_id) {
                doc.blocks.push(DocBlock {
                    id: block_id,
                    kind,
                    text: owned,
                });
            }
            s.next_block_id = block_id.saturating_add(1);
            s.doc_new_block_draft.clear();
        });
        Some(block_id)
    }

    pub fn set_doc_new_block_draft(&mut self, draft: impl Into<String>) {
        let next = draft.into();
        if self.current.doc_new_block_draft == next {
            return;
        }
        self.current.doc_new_block_draft = next;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn set_doc_new_block_kind(&mut self, kind: DocBlockKind) {
        if self.current.doc_new_block_kind == kind {
            return;
        }
        self.current.doc_new_block_kind = kind;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn cycle_doc_new_block_kind(&mut self) {
        let next = self.current.doc_new_block_kind.next_kind();
        self.set_doc_new_block_kind(next);
    }

    pub fn submit_new_doc_block_draft(&mut self) -> bool {
        let kind = self.current.doc_new_block_kind;
        let draft = self.current.doc_new_block_draft.clone();
        self.add_doc_block(kind, &draft).is_some()
    }

    pub fn begin_edit_doc_block(&mut self, block_id: u32) -> bool {
        let active_id = self.current.active_doc_id;
        let Some(existing) = self
            .current
            .docs
            .iter()
            .find(|d| d.id == active_id)
            .and_then(|d| d.blocks.iter().find(|b| b.id == block_id))
            .map(|b| b.text.clone())
        else {
            return false;
        };
        self.current.doc_editing_block_id = Some(block_id);
        self.current.doc_edit_draft = existing;
        self.frame_count = self.frame_count.saturating_add(1);
        true
    }

    pub fn cancel_edit_doc_block(&mut self) {
        if self.current.doc_editing_block_id.is_none() {
            return;
        }
        self.current.doc_editing_block_id = None;
        self.current.doc_edit_draft.clear();
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn set_doc_edit_draft(&mut self, text: impl Into<String>) {
        let next = text.into();
        if self.current.doc_edit_draft == next {
            return;
        }
        self.current.doc_edit_draft = next;
        self.frame_count = self.frame_count.saturating_add(1);
    }

    pub fn commit_doc_block_edit(&mut self) -> bool {
        let Some(block_id) = self.current.doc_editing_block_id else {
            return false;
        };
        let draft = self.current.doc_edit_draft.clone();
        self.update_doc_block_text(block_id, draft)
    }

    pub fn update_doc_block_text(&mut self, block_id: u32, text: impl Into<String>) -> bool {
        let trimmed = text.into().trim().to_owned();
        if trimmed.is_empty() {
            return false;
        }
        let active_id = self.current.active_doc_id;
        let exists = self
            .current
            .docs
            .iter()
            .find(|d| d.id == active_id)
            .is_some_and(|d| d.blocks.iter().any(|b| b.id == block_id));
        if !exists {
            return false;
        }
        let label = format!("Edit Block #{block_id}");
        self.commit_action(label, |s| {
            if let Some(doc) = s.docs.make_mut().iter_mut().find(|d| d.id == active_id)
                && let Some(block) = doc.blocks.iter_mut().find(|b| b.id == block_id)
            {
                block.text = trimmed.clone();
                if matches!(block.kind, DocBlockKind::Heading1)
                    && doc.blocks.first().is_some_and(|first| first.id == block_id)
                {
                    doc.title = trimmed;
                }
            }
            s.doc_editing_block_id = None;
            s.doc_edit_draft.clear();
        });
        true
    }

    pub fn toggle_doc_markdown_preview(&mut self) {
        let next = !self.current.doc_markdown_preview_open;
        let label = if next {
            "Docs -> Export MD"
        } else {
            "Docs -> Hide MD"
        };
        self.commit_action(label, |s| s.doc_markdown_preview_open = next);
    }
}
