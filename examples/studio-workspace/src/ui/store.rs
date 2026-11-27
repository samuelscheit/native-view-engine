//! Fine-grained reactive Studio store: one Dioxus `Signal` per UI domain projected from the
//! authoritative `Arc<Mutex<WorkspaceState>>` (shared with the `NveApp` module for persistence,
//! host events, and time travel).
//!
//! [`StudioStore::update`] mutates the backing state, then republishes only the domains whose
//! projection changed. Change detection never allocates and never walks an unchanged
//! collection: collections are copy-on-write [`Shared`] handles compared by pointer first, typing
//! a draft only touches its draft domain, and derived views (route path, workbench preview,
//! palette matches) are computed by the components that render them, not on every update.

use std::sync::{Arc, Mutex};

use dioxus_core::{consume_context, provide_root_context, try_consume_context};
use dioxus_signals::{CopyValue, ReadableExt, Signal, WritableExt};
use nve_dioxus::traits::ReactiveTraitStore;

use crate::routes::StudioRoute;
use crate::sdk::StudioHostServices;
use crate::state::{
    ActiveView, BoardFilter, ChatMessage, DocBlockKind, Shared, Stream100kState, StudioConfig,
    SyncTelemetryState, TaskItem, WorkbenchState, WorkspaceDoc, WorkspaceState,
};
use crate::traits::sync_reactive_trait_store;
use crate::ui_islands::IslandsSnapshot;
use crate::ui_palette::CommandPaletteState;

/// The authoritative workspace state shared with `StudioWorkspaceModule`.
pub type SharedWorkspace = Arc<Mutex<WorkspaceState>>;

/// Sidebar badge counts (projected separately so the sidebar ignores content edits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NavCounts {
    pub tasks: usize,
    pub eng_messages: usize,
    pub design_messages: usize,
    pub docs: usize,
    pub island_cards: usize,
    pub stress_rows: usize,
}

/// Miniature-preview summary for the Workbench viewport frames.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkbenchPreview {
    pub issue_count: usize,
    pub doc_count: usize,
    pub top_task_key: String,
    pub active_doc_slug: String,
}

fn nav_counts(st: &WorkspaceState) -> NavCounts {
    let c = &st.current;
    NavCounts {
        tasks: c.tasks.len(),
        eng_messages: c.eng_messages.len(),
        design_messages: c.design_messages.len(),
        docs: c.docs.len(),
        island_cards: c.islands.cards.len(),
        stress_rows: c.stress_rows,
    }
}

/// Declares every UI domain once: its projection struct, its [`DomainMask`] bit, its field in
/// [`DomainValues`] / [`StudioStore`], and its change detection. Each field projection is an
/// expression yielding `&T` from the state, compared before it is cloned.
macro_rules! studio_domains {
    ($(
        $(#[$doc:meta])*
        $field:ident: $Domain:ident [$Bit:ident] {
            $( $f:ident: $t:ty = |$st:ident| $e:expr ),+ $(,)?
        }
    )+) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, PartialEq)]
            pub struct $Domain { $(pub $f: $t),+ }

            impl $Domain {
                pub fn project(state: &WorkspaceState) -> Self {
                    Self { $($f: { let $st = state; Clone::clone($e) }),+ }
                }

                /// `true` when any projected field differs from `state` (no cloning).
                pub fn differs(&self, state: &WorkspaceState) -> bool {
                    false $( || { let $st = state; &self.$f != $e } )+
                }
            }
        )+

        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u8)]
        enum DomainIndex { $($Bit),+ }

        /// Set of domains changed by one update.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct DomainMask(pub u32);

        impl DomainMask {
            $( pub const $Bit: Self = Self(1 << DomainIndex::$Bit as u32); )+
            pub const NONE: Self = Self(0);

            pub const fn union(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }

            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }
        }

        /// Plain (non-reactive) projection of every domain; [`Self::changed`] is the pure
        /// change-detection core of [`StudioStore::update`].
        #[derive(Debug, Clone, PartialEq)]
        pub struct DomainValues { $(pub $field: $Domain),+ }

        impl DomainValues {
            pub fn project(state: &WorkspaceState) -> Self {
                Self { $($field: $Domain::project(state)),+ }
            }

            pub fn changed(&self, state: &WorkspaceState) -> DomainMask {
                let mut mask = DomainMask::NONE;
                $( if self.$field.differs(state) { mask = mask.union(DomainMask::$Bit); } )+
                mask
            }
        }

        /// Context-provided reactive store: `Copy` handle of per-domain signals.
        #[derive(Clone, Copy, PartialEq)]
        pub struct StudioStore {
            backing: CopyValue<SharedWorkspace>,
            services: CopyValue<Arc<StudioHostServices>>,
            traits: CopyValue<ReactiveTraitStore>,
            $(pub $field: Signal<$Domain>),+
        }

        impl StudioStore {
            fn create(backing: SharedWorkspace, services: Arc<StudioHostServices>, traits: ReactiveTraitStore) -> Self {
                let values = {
                    let guard = backing.lock().unwrap_or_else(|e| e.into_inner());
                    DomainValues::project(&guard)
                };
                Self {
                    backing: CopyValue::new(backing),
                    services: CopyValue::new(services),
                    traits: CopyValue::new(traits),
                    $($field: Signal::new(values.$field)),+
                }
            }

            fn publish(&self, state: &WorkspaceState) -> DomainMask {
                let mut mask = DomainMask::NONE;
                $(
                    if self.$field.peek().differs(state) {
                        let mut sig = self.$field;
                        sig.set($Domain::project(state));
                        mask = mask.union(DomainMask::$Bit);
                    }
                )+
                mask
            }
        }
    };
}

studio_domains! {
    /// Active view and drawer/overlay toggles.
    nav: NavDomain [NAV] {
        active_view: ActiveView = |s| &s.current.active_view,
        mobile_nav_open: bool = |s| &s.current.mobile_nav_open,
        mobile_studio_open: bool = |s| &s.current.mobile_studio_open,
        palette_open: bool = |s| &s.palette.open,
    }
    /// Router position (typed; components render its path) and the clipboard notice.
    route: RouteDomain [ROUTE] {
        route: StudioRoute = |s| s.current_route(),
        can_back: bool = |s| &s.can_navigate_back(),
        can_forward: bool = |s| &s.can_navigate_forward(),
        clipboard_notice: Option<String> = |s| &s.last_clipboard_notice,
    }
    /// Sidebar badge counts.
    counts: CountsDomain [COUNTS] {
        counts: NavCounts = |s| &nav_counts(s),
    }
    /// Issue board: tasks, filter, search, deep-link focus.
    board: BoardDomain [BOARD] {
        tasks: Shared<TaskItem> = |s| &s.current.tasks,
        board_filter: BoardFilter = |s| &s.current.board_filter,
        board_search_query: String = |s| &s.current.board_search_query,
        focused_task_id: Option<u32> = |s| &s.focused_task_id,
    }
    /// The board's new-task composer text.
    board_draft: BoardDraftDomain [BOARD_DRAFT] {
        new_task_draft: String = |s| &s.current.new_task_draft,
    }
    /// Messaging channels.
    channels: ChannelsDomain [CHANNELS] {
        eng_messages: Shared<ChatMessage> = |s| &s.current.eng_messages,
        design_messages: Shared<ChatMessage> = |s| &s.current.design_messages,
    }
    /// Channel composer texts.
    channel_drafts: ChannelDraftsDomain [CHANNEL_DRAFTS] {
        eng_draft: String = |s| &s.current.eng_draft,
        design_draft: String = |s| &s.current.design_draft,
    }
    /// Block-doc editor.
    docs: DocsDomain [DOCS] {
        docs: Shared<WorkspaceDoc> = |s| &s.current.docs,
        active_doc_id: u32 = |s| &s.current.active_doc_id,
        doc_editing_block_id: Option<u32> = |s| &s.current.doc_editing_block_id,
        doc_markdown_preview_open: bool = |s| &s.current.doc_markdown_preview_open,
    }
    /// Doc new-block composer: the block kind and text.
    doc_drafts: DocDraftsDomain [DOC_DRAFTS] {
        doc_new_block_kind: DocBlockKind = |s| &s.current.doc_new_block_kind,
        doc_new_block_draft: String = |s| &s.current.doc_new_block_draft,
    }
    /// Text of the block open in the in-place editor.
    doc_edit_draft: DocEditDraftDomain [DOC_EDIT_DRAFT] {
        text: String = |s| &s.current.doc_edit_draft,
    }
    /// 100k virtualized stream window.
    stream: StreamDomain [STREAM] {
        state: Stream100kState = |s| &s.current.stream,
    }
    /// Multi-host island sandboxes.
    islands: IslandsDomain [ISLANDS] {
        snapshot: IslandsSnapshot = |s| &s.current.islands,
    }
    /// Live metrics view.
    metrics: MetricsDomain [METRICS] {
        stress_rows: usize = |s| &s.current.stress_rows,
        telemetry_samples_us: Vec<u32> = |s| &s.current.telemetry_samples_us,
        active_tasks: usize = |s| &s.active_task_count(),
        total_messages: usize = |s| &s.total_message_count(),
    }
    /// Meta workbench presets and host selection, plus the content its previews summarize.
    workbench: WorkbenchDomain [WORKBENCH] {
        state: WorkbenchState = |s| &s.current.workbench,
        tasks: Shared<TaskItem> = |s| &s.current.tasks,
        docs: Shared<WorkspaceDoc> = |s| &s.current.docs,
        active_doc_id: u32 = |s| &s.current.active_doc_id,
    }
    /// Workspace-wide Studio self-edits: accent, radius, density, X-Ray, layout engine.
    style: StyleDomain [STYLE] {
        config: StudioConfig = |s| &s.current.studio.styling(),
    }
    /// Docked Studio panel visibility.
    chrome: ChromeDomain [CHROME] {
        panel_open: bool = |s| &s.current.studio.panel_open,
    }
    /// Inspector: frame counter, time-travel cursor, action label.
    inspector: InspectorDomain [INSPECTOR] {
        frame_count: u64 = |s| &s.frame_count,
        snapshot_index: usize = |s| &s.snapshot_index_1based(),
        history_len: usize = |s| &s.history_len(),
        can_undo: bool = |s| &s.can_undo(),
        can_redo: bool = |s| &s.can_redo(),
        action_label: String = |s| &s.current.action_label,
    }
    /// Live CRDT sync transport telemetry.
    sync: SyncDomain [SYNC] {
        telemetry: SyncTelemetryState = |s| &s.sync_telemetry,
    }
    /// Cmd+K command palette state plus the stamps of what its items are built from (tasks and
    /// history); the matches are filtered while it renders.
    palette: PaletteDomain [PALETTE] {
        state: CommandPaletteState = |s| &s.palette,
        tasks: Shared<TaskItem> = |s| &s.current.tasks,
        commit_seq: u64 = |s| &s.commit_seq,
    }
}

impl WorkbenchDomain {
    pub fn preview(&self) -> WorkbenchPreview {
        WorkbenchPreview {
            issue_count: self.tasks.len(),
            doc_count: self.docs.len(),
            top_task_key: self
                .tasks
                .first()
                .map_or_else(|| "NONE".into(), |t| t.key.clone()),
            active_doc_slug: self
                .docs
                .iter()
                .find(|d| d.id == self.active_doc_id)
                .or_else(|| self.docs.first())
                .map_or_else(|| "docs".into(), |d| d.slug.clone()),
        }
    }
}

impl StudioStore {
    /// Creates the store and provides it at the surface's root scope (to every view, and to the
    /// owning module via [`Self::from_root`]): syncs the host's live appearance/locale first
    /// (authoritative from frame one) and seeds the reactive trait store from it.
    pub fn mount(backing: SharedWorkspace, services: Arc<StudioHostServices>) -> Self {
        let traits = {
            let mut guard = backing.lock().unwrap_or_else(|e| e.into_inner());
            guard.sync_system_env(&services);
            ReactiveTraitStore::new(guard.trait_store().clone())
        };
        provide_root_context(Self::create(backing, services, traits))
    }

    /// The store of the surface whose root scope is current (`DioxusSurface::in_root_scope`),
    /// once mounted.
    pub fn from_root() -> Option<Self> {
        try_consume_context::<Self>()
    }

    /// Mutates the authoritative state, syncs the reactive trait store, and republishes only the
    /// changed domains. Returns `f`'s result.
    pub fn update<R>(&self, f: impl FnOnce(&mut WorkspaceState) -> R) -> R {
        self.update_masked(f).0
    }

    /// [`Self::update`] that also reports which domains were republished.
    pub fn update_masked<R>(&self, f: impl FnOnce(&mut WorkspaceState) -> R) -> (R, DomainMask) {
        let backing = self.backing.cloned();
        let mut guard = backing.lock().unwrap_or_else(|e| e.into_inner());
        let out = f(&mut guard);
        let _ = sync_reactive_trait_store(&self.traits.read(), &guard.current, &guard.system_env);
        let mask = self.publish(&guard);
        (out, mask)
    }

    /// Read-only access to the authoritative state (e.g. Markdown export).
    pub fn with<R>(&self, f: impl FnOnce(&WorkspaceState) -> R) -> R {
        let backing = self.backing.cloned();
        let guard = backing.lock().unwrap_or_else(|e| e.into_inner());
        f(&guard)
    }

    /// Host capability services (`nve-clipboard`, `nve-system-env`).
    pub fn services(&self) -> Arc<StudioHostServices> {
        self.services.cloned()
    }

    /// The reactive environment trait store (`TraitProvider`).
    pub fn traits(&self) -> ReactiveTraitStore {
        self.traits.cloned()
    }
}

/// The [`StudioStore`] provided at the surface root (every Studio view renders inside it).
pub fn use_studio() -> StudioStore {
    consume_context::<StudioStore>()
}
