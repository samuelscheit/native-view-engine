use std::borrow::Cow;
use std::collections::HashMap;

use accesskit::{
    Action, ActionData, ActionRequest, Affine, AutoComplete, Invalid, Live, Orientation, Point,
    Rect, Role, ScrollUnit, TextPosition, TextSelection, Toggled, TreeId,
};
use nve_abi::InteractionEvent;
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_tables::tables::{
    Accessibility, AriaAutoComplete, AriaLive, AriaOrientation, AriaRelations, AriaRole,
    AriaTriState, DebugId, Display, Image, InputKind, Interaction, Layout, Link, Position, Scroll,
    ScrollAxes, TextContent, TextInput, Transform, Visual,
};

use crate::{
    aria_role_to_accesskit, byte_offset_to_char_index, char_index_to_byte_offset,
    encode_store_node_id, encode_text_run_id, A11yAction, A11yGeometryProvider,
    AccessKitTreeBridge, ResolvedA11yAction, StoreEpoch,
};

#[derive(Default)]
struct MapGeo {
    scale: f32,
    rects: HashMap<NodeId, LayoutRect>,
    scroll_offsets: HashMap<NodeId, (f32, f32)>,
    scroll_contents: HashMap<NodeId, (f32, f32)>,
    sticky_deltas: HashMap<NodeId, (f32, f32)>,
    focused: Option<NodeId>,
    values: HashMap<NodeId, String>,
    selections: HashMap<NodeId, (usize, usize)>,
    display_none: HashMap<NodeId, bool>,
    display_contents: HashMap<NodeId, bool>,
}

impl MapGeo {
    fn new(scale: f32) -> Self {
        Self {
            scale,
            ..Default::default()
        }
    }
}

impl A11yGeometryProvider for MapGeo {
    fn scale_factor(&self) -> f32 {
        self.scale
    }
    fn layout_rect(&self, id: NodeId) -> Option<LayoutRect> {
        self.rects.get(&id).copied()
    }
    fn scroll_offset(&self, id: NodeId) -> (f32, f32) {
        self.scroll_offsets.get(&id).copied().unwrap_or((0.0, 0.0))
    }
    fn scroll_content_size(&self, id: NodeId) -> Option<(f32, f32)> {
        self.scroll_contents.get(&id).copied()
    }
    fn sticky_delta(&self, id: NodeId) -> (f32, f32) {
        self.sticky_deltas.get(&id).copied().unwrap_or((0.0, 0.0))
    }
    fn focused_node(&self) -> Option<NodeId> {
        self.focused
    }
    fn text_input_value(&self, id: NodeId) -> Option<Cow<'_, str>> {
        self.values.get(&id).map(|s| Cow::Borrowed(s.as_str()))
    }
    fn text_selection_bytes(&self, id: NodeId) -> Option<(usize, usize)> {
        self.selections.get(&id).copied()
    }
    fn is_display_none(&self, store: &NodeStore, id: NodeId) -> bool {
        self.display_none
            .get(&id)
            .copied()
            .unwrap_or_else(|| nve_tables::tables::is_node_display_none(store, id))
    }
    fn is_display_contents(&self, store: &NodeStore, id: NodeId) -> bool {
        self.display_contents
            .get(&id)
            .copied()
            .unwrap_or_else(|| nve_tables::tables::is_display_contents(store, id))
    }
}

#[test]
fn all_48_aria_roles_map_exhaustively_and_deterministically() {
    let all_roles = [
        (AriaRole::Button, Role::Button),
        (AriaRole::Link, Role::Link),
        (AriaRole::Heading, Role::Heading),
        (AriaRole::Text, Role::Label),
        (AriaRole::Image, Role::Image),
        (AriaRole::TextBox, Role::TextInput),
        (AriaRole::SearchBox, Role::SearchInput),
        (AriaRole::ScrollArea, Role::ScrollView),
        (AriaRole::Checkbox, Role::CheckBox),
        (AriaRole::Switch, Role::Switch),
        (AriaRole::Radio, Role::RadioButton),
        (AriaRole::Slider, Role::Slider),
        (AriaRole::Tab, Role::Tab),
        (AriaRole::TabList, Role::TabList),
        (AriaRole::TabPanel, Role::TabPanel),
        (AriaRole::Dialog, Role::Dialog),
        (AriaRole::AlertDialog, Role::AlertDialog),
        (AriaRole::Alert, Role::Alert),
        (AriaRole::Status, Role::Status),
        (AriaRole::Tooltip, Role::Tooltip),
        (AriaRole::Navigation, Role::Navigation),
        (AriaRole::Main, Role::Main),
        (AriaRole::Article, Role::Article),
        (AriaRole::Complementary, Role::Complementary),
        (AriaRole::Banner, Role::Banner),
        (AriaRole::ContentInfo, Role::ContentInfo),
        (AriaRole::Form, Role::Form),
        (AriaRole::Region, Role::Region),
        (AriaRole::Group, Role::Group),
        (AriaRole::List, Role::List),
        (AriaRole::ListItem, Role::ListItem),
        (AriaRole::ListBox, Role::ListBox),
        (AriaRole::Option, Role::ListBoxOption),
        (AriaRole::Menu, Role::Menu),
        (AriaRole::MenuItem, Role::MenuItem),
        (AriaRole::Separator, Role::Splitter),
        (AriaRole::Presentation, Role::GenericContainer),
        (AriaRole::Table, Role::Table),
        (AriaRole::Row, Role::Row),
        (AriaRole::Cell, Role::Cell),
        (AriaRole::ColumnHeader, Role::ColumnHeader),
        (AriaRole::RowHeader, Role::RowHeader),
        (AriaRole::Code, Role::Code),
        (AriaRole::CodeBlock, Role::Code),
        (AriaRole::ProgressBar, Role::ProgressIndicator),
        (AriaRole::ComboBox, Role::ComboBox),
        (AriaRole::Grid, Role::Grid),
        (AriaRole::GridCell, Role::GridCell),
    ];
    assert_eq!(all_roles.len(), 48);
    for (aria, expected) in all_roles {
        assert_eq!(aria_role_to_accesskit(aria), expected);
    }
}

#[test]
fn full_tree_emits_aria_attrs_relations_password_masking_and_omits_hidden_subtrees() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (heading, label_node, desc_node, slider, password, hidden_a11y, hidden_display, link) = {
        let mut tx = store.tx();
        let heading = tx.create();
        tx.edit::<Accessibility>(heading)
            .role(Some(AriaRole::Heading))
            .level(Some(2));
        tx.edit::<TextContent>(heading).text("Settings");

        let label_node = tx.create();
        tx.edit::<DebugId>(label_node).id("vol-label");
        tx.edit::<TextContent>(label_node).text("Volume");

        let desc_node = tx.create();
        tx.edit::<DebugId>(desc_node).id("vol-desc");
        tx.edit::<TextContent>(desc_node).text("Adjust output level");

        let slider = tx.create();
        tx.edit::<Accessibility>(slider)
            .role(Some(AriaRole::Slider))
            .orientation(Some(AriaOrientation::Horizontal))
            .live(Some(AriaLive::Polite))
            .autocomplete(Some(AriaAutoComplete::List))
            .checked(Some(AriaTriState::Mixed))
            .selected(Some(AriaTriState::True))
            .expanded(Some(AriaTriState::False))
            .invalid(true)
            .busy(true)
            .multiselectable(true)
            .modal(true)
            .value_now(Some(42.0))
            .value_min(Some(0.0))
            .value_max(Some(100.0))
            .hint("Drag or use arrows")
            .relations(Some(AriaRelations {
                controls: "vol-desc".into(),
                labelled_by: "vol-label".into(),
                described_by: "vol-desc".into(),
                active_descendant: "vol-label".into(),
            }));
        tx.edit::<Interaction>(slider).focusable(true);

        let password = tx.create();
        tx.edit::<TextInput>(password)
            .secure(true)
            .placeholder("Enter secret")
            .required(true)
            .value("s3cr€t");

        let hidden_a11y = tx.create();
        let hidden_a11y_child = tx.create();
        tx.edit::<Accessibility>(hidden_a11y).hidden(true);
        tx.edit::<TextContent>(hidden_a11y_child).text("Secret aria-hidden");
        tx.set_children(hidden_a11y, &[hidden_a11y_child]);

        let hidden_display = tx.create();
        let hidden_display_child = tx.create();
        tx.edit::<Layout>(hidden_display).display(Display::None);
        tx.edit::<TextContent>(hidden_display_child).text("Secret display-none");
        tx.set_children(hidden_display, &[hidden_display_child]);

        let link = tx.create();
        tx.edit::<Link>(link).href("https://example.com");
        tx.edit::<Image>(link).alt("Docs icon");

        tx.set_children(
            NodeId::ROOT,
            &[
                heading,
                label_node,
                desc_node,
                slider,
                password,
                hidden_a11y,
                hidden_display,
                link,
            ],
        );
        tx.commit_into(&mut changes);
        (
            heading,
            label_node,
            desc_node,
            slider,
            password,
            hidden_a11y,
            hidden_display,
            link,
        )
    };

    let mut geo = MapGeo::new(2.0);
    geo.rects.insert(
        NodeId::ROOT,
        LayoutRect { x: 0.0, y: 0.0, width: 400.0, height: 300.0 },
    );
    geo.rects.insert(
        password,
        LayoutRect { x: 10.0, y: 50.0, width: 200.0, height: 32.0 },
    );
    geo.focused = Some(password);
    // Select `"3cr€"` (bytes 1..7 -> char indices 1..5 in `"s3cr€t"`).
    geo.selections.insert(password, (1, 7));

    let mut bridge = AccessKitTreeBridge::new();
    let update = bridge.build_full_tree_update(&store, &geo, "Prefs Window");
    assert!(update.tree.is_some());
    assert_eq!(update.tree_id, TreeId::ROOT);
    assert_eq!(update.focus, bridge.encode_store_node_id(password));

    let by_id: HashMap<_, _> = update.nodes.into_iter().collect();
    let root = &by_id[&bridge.encode_store_node_id(NodeId::ROOT)];
    assert_eq!(root.role(), Role::Window);
    assert_eq!(root.label(), Some("Prefs Window"));
    assert_eq!(root.transform(), Some(&Affine::scale(2.0)));
    assert!(!root.children().contains(&bridge.encode_store_node_id(hidden_a11y)));
    assert!(!root.children().contains(&bridge.encode_store_node_id(hidden_display)));
    assert!(!by_id.contains_key(&bridge.encode_store_node_id(hidden_a11y)));
    assert!(!by_id.contains_key(&bridge.encode_store_node_id(hidden_display)));

    // Heading: aria-level=2 -> AccessKit 0-based level=1.
    let h = &by_id[&bridge.encode_store_node_id(heading)];
    assert_eq!(h.role(), Role::Heading);
    assert_eq!(h.level(), Some(1));
    assert_eq!(h.label(), Some("Settings"));

    // Label: text content goes in `value()`, NOT `label()`.
    let lbl = &by_id[&bridge.encode_store_node_id(label_node)];
    assert_eq!(lbl.role(), Role::Label);
    assert_eq!(lbl.value(), Some("Volume"));
    assert_eq!(lbl.label(), None);

    // Slider: all ARIA attributes & resolved AriaRelations.
    let sl = &by_id[&bridge.encode_store_node_id(slider)];
    assert_eq!(sl.role(), Role::Slider);
    assert_eq!(sl.orientation(), Some(Orientation::Horizontal));
    assert_eq!(sl.live(), Some(Live::Polite));
    assert_eq!(sl.auto_complete(), Some(AutoComplete::List));
    assert_eq!(sl.toggled(), Some(Toggled::Mixed));
    assert_eq!(sl.is_selected(), Some(true));
    assert_eq!(sl.is_expanded(), Some(false));
    assert_eq!(sl.invalid(), Some(Invalid::True));
    assert!(sl.is_busy());
    assert!(sl.is_multiselectable());
    assert!(sl.is_modal());
    assert_eq!(sl.numeric_value(), Some(42.0));
    assert_eq!(sl.min_numeric_value(), Some(0.0));
    assert_eq!(sl.max_numeric_value(), Some(100.0));
    assert_eq!(sl.description(), Some("Drag or use arrows"));
    assert_eq!(sl.labelled_by(), &[bridge.encode_store_node_id(label_node)]);
    assert_eq!(sl.described_by(), &[bridge.encode_store_node_id(desc_node)]);
    assert_eq!(sl.controls(), &[bridge.encode_store_node_id(desc_node)]);
    assert_eq!(sl.active_descendant(), Some(bridge.encode_store_node_id(label_node)));
    assert!(sl.supports_action(Action::Increment));
    assert!(sl.supports_action(Action::Decrement));

    // PasswordInput: NEVER leaks plaintext; emits bullet mask + synthetic TextRun + selection!
    let pw = &by_id[&bridge.encode_store_node_id(password)];
    assert_eq!(pw.role(), Role::PasswordInput);
    assert_eq!(pw.value(), Some("••••••"));
    assert_eq!(pw.placeholder(), Some("Enter secret"));
    assert!(pw.is_required());
    assert!(pw.supports_action(Action::SetValue));
    assert!(pw.supports_action(Action::ReplaceSelectedText));
    let run_id = encode_text_run_id(bridge.epoch(), password);
    assert_eq!(pw.children(), &[run_id]);
    let run = &by_id[&run_id];
    assert_eq!(run.role(), Role::TextRun);
    assert_eq!(run.value(), Some("••••••"));
    assert_eq!(run.character_lengths(), &[3, 3, 3, 3, 3, 3]);
    assert_eq!(
        run.character_lengths().iter().map(|&n| usize::from(n)).sum::<usize>(),
        run.value().expect("text run value").len()
    );
    assert_eq!(
        pw.text_selection(),
        Some(&TextSelection {
            anchor: TextPosition { node: run_id, character_index: 1 },
            focus: TextPosition { node: run_id, character_index: 5 },
        })
    );

    // Link with alt fallback.
    let lnk = &by_id[&bridge.encode_store_node_id(link)];
    assert_eq!(lnk.role(), Role::Link);
    assert_eq!(lnk.url(), Some("https://example.com"));
    assert_eq!(lnk.label(), Some("Docs icon"));
    assert!(lnk.supports_action(Action::Click));
}

#[test]
fn incremental_updates_emit_minimal_diffs_and_scroll_mutates_only_scroll_container() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (scroll, btn_a, btn_b) = {
        let mut tx = store.tx();
        let scroll = tx.create();
        tx.edit::<Scroll>(scroll).axes(ScrollAxes::Vertical);
        let btn_a = tx.create();
        tx.edit::<Interaction>(btn_a).listen(InteractionEvent::Press);
        tx.edit::<Accessibility>(btn_a).label("First");
        tx.edit::<Visual>(btn_a).opacity(1.0);
        let btn_b = tx.create();
        tx.edit::<Interaction>(btn_b).listen(InteractionEvent::Press);
        tx.edit::<Accessibility>(btn_b).label("Second");
        tx.set_children(scroll, &[btn_a, btn_b]);
        tx.set_children(NodeId::ROOT, &[scroll]);
        tx.commit_into(&mut changes);
        (scroll, btn_a, btn_b)
    };

    let mut geo = MapGeo::new(1.0);
    geo.rects.insert(NodeId::ROOT, LayoutRect { x: 0.0, y: 0.0, width: 300.0, height: 200.0 });
    geo.rects.insert(scroll, LayoutRect { x: 0.0, y: 0.0, width: 300.0, height: 200.0 });
    geo.rects.insert(btn_a, LayoutRect { x: 0.0, y: 0.0, width: 200.0, height: 40.0 });
    geo.rects.insert(btn_b, LayoutRect { x: 0.0, y: 50.0, width: 200.0, height: 40.0 });
    geo.scroll_contents.insert(scroll, (300.0, 600.0));

    let mut bridge = AccessKitTreeBridge::new();
    // Inactive bridge produces None.
    assert!(bridge.sync_changes(&store, &changes, &geo, "App", false).is_none());

    bridge.activate();
    let full = bridge.build_full_tree_update(&store, &geo, "App");
    assert_eq!(full.nodes.len(), 4);

    // 1a. Visual::opacity-only COMPOSITE edit -> 0-alloc None!
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(btn_a).opacity(0.6);
        tx.commit_into(&mut changes);
    }
    assert!(bridge.sync_changes(&store, &changes, &geo, "App", false).is_none());
    assert_eq!(bridge.last_sync_nodes_built(), 0);

    // 1b. Static Transform-only edit -> O(1) targeted transform update!
    {
        let mut tx = store.tx();
        tx.edit::<Transform>(btn_a).translate_x(4.0);
        tx.commit_into(&mut changes);
    }
    let tf_upd = bridge
        .sync_changes(&store, &changes, &geo, "App", false)
        .expect("Transform-only edit must emit targeted update");
    assert_eq!(bridge.last_sync_nodes_built(), 1);
    assert_eq!(tf_upd.nodes.len(), 1);
    assert_eq!(tf_upd.nodes[0].0, bridge.encode_store_node_id(btn_a));
    assert_eq!(tf_upd.nodes[0].1.transform(), Some(&Affine::translate((4.0, 0.0))));

    // 1c. Resetting Transform back to identity clears transform() in O(1)!
    {
        let mut tx = store.tx();
        tx.edit::<Transform>(btn_a).translate_x(0.0);
        tx.commit_into(&mut changes);
    }
    let tf_clear = bridge
        .sync_changes(&store, &changes, &geo, "App", false)
        .expect("Transform reset to identity must emit targeted update");
    assert_eq!(bridge.last_sync_nodes_built(), 1);
    assert_eq!(tf_clear.nodes.len(), 1);
    assert_eq!(tf_clear.nodes[0].0, bridge.encode_store_node_id(btn_a));
    assert_eq!(tf_clear.nodes[0].1.transform(), None);

    // 2. Single label change -> emits ONLY `btn_a` (1 node)!
    {
        let mut tx = store.tx();
        tx.edit::<Accessibility>(btn_a).label("Updated");
        tx.commit_into(&mut changes);
    }
    let inc = bridge.sync_changes(&store, &changes, &geo, "App", false).expect("incremental");
    assert!(inc.tree.is_none());
    assert_eq!(inc.nodes.len(), 1);
    assert_eq!(inc.nodes[0].0, bridge.encode_store_node_id(btn_a));
    assert_eq!(inc.nodes[0].1.label(), Some("Updated"));

    // 3. Scroll offset change -> emits ONLY the `scroll` container (O(1), zero children re-emitted)!
    geo.scroll_offsets.insert(scroll, (0.0, 120.0));
    let scroll_upd = bridge
        .sync_changes(&store, &ChangeSet::default(), &geo, "App", true)
        .expect("scroll update");
    assert_eq!(scroll_upd.nodes.len(), 1);
    assert_eq!(scroll_upd.nodes[0].0, bridge.encode_store_node_id(scroll));
    assert_eq!(scroll_upd.nodes[0].1.scroll_y(), Some(120.0));
    assert_eq!(scroll_upd.nodes[0].1.scroll_y_max(), Some(400.0));
    assert_eq!(scroll_upd.nodes[0].1.transform(), Some(&Affine::translate((0.0, -120.0))));
    assert_eq!(
        scroll_upd.nodes[0].1.bounds(),
        Some(Rect { x0: 0.0, y0: 120.0, x1: 300.0, y1: 320.0 })
    );

    // 4. Removal of `btn_b` -> emits ONLY parent `scroll` with updated children (never removed node!).
    {
        let mut tx = store.tx();
        tx.set_children(scroll, &[btn_a]);
        tx.remove(btn_b);
        tx.commit_into(&mut changes);
    }
    let rem = bridge.sync_changes(&store, &changes, &geo, "App", false).expect("remove update");
    assert_eq!(rem.nodes.len(), 1);
    assert_eq!(rem.nodes[0].0, bridge.encode_store_node_id(scroll));
    assert_eq!(rem.nodes[0].1.children(), &[bridge.encode_store_node_id(btn_a)]);

    // 5. Focus on live `btn_a` updates focus, then focus on removed `btn_b` falls back to ROOT.
    geo.focused = Some(btn_a);
    let focus_a = bridge
        .sync_changes(&store, &ChangeSet::default(), &geo, "App", false)
        .expect("focus on btn_a");
    assert_eq!(focus_a.focus, bridge.encode_store_node_id(btn_a));

    geo.focused = Some(btn_b);
    let focus_upd = bridge
        .sync_changes(&store, &ChangeSet::default(), &geo, "App", false)
        .expect("focus fallback");
    assert_eq!(focus_upd.focus, bridge.encode_store_node_id(NodeId::ROOT));

    // 6. All InputKind variants map to their native AccessKit roles.
    let kinds = [
        (InputKind::Text, Role::TextInput),
        (InputKind::Email, Role::EmailInput),
        (InputKind::Password, Role::PasswordInput),
        (InputKind::Tel, Role::PhoneNumberInput),
        (InputKind::Url, Role::UrlInput),
        (InputKind::Number, Role::NumberInput),
        (InputKind::Search, Role::SearchInput),
        (InputKind::Date, Role::DateInput),
        (InputKind::Time, Role::TimeInput),
    ];
    for (kind, expected) in kinds {
        let mut s = NodeStore::new();
        let n = {
            let mut tx = s.tx();
            let id = tx.create();
            tx.edit::<TextInput>(id).input_kind(kind);
            tx.set_children(NodeId::ROOT, &[id]);
            tx.commit_into(&mut changes);
            id
        };
        assert_eq!(crate::resolve_node_role(&s, n), expected);
    }
}

#[test]
fn epoch_advancement_rejects_stale_actions_and_translates_all_action_types() {
    let mut bridge = AccessKitTreeBridge::new();
    let node = NodeId::new(42, 0);
    let epoch1 = bridge.epoch();
    assert_eq!(epoch1, StoreEpoch::INITIAL);
    let old_id = encode_store_node_id(epoch1, node);
    let old_run = encode_text_run_id(epoch1, node);

    // Translate Click, Focus, Blur, SetValue, ReplaceSelectedText, SetTextSelection, Scroll*.
    let click = bridge.translate_action(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: old_id,
        data: None,
    });
    assert_eq!(click, Some(ResolvedA11yAction { target: node, action: A11yAction::Click }));

    let set_val = bridge.translate_action(&ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node: old_id,
        data: Some(ActionData::Value("hello".into())),
    });
    assert_eq!(
        set_val,
        Some(ResolvedA11yAction {
            target: node,
            action: A11yAction::SetValue("hello".into()),
        })
    );

    let sel = bridge.translate_action(&ActionRequest {
        action: Action::SetTextSelection,
        target_tree: TreeId::ROOT,
        target_node: old_run,
        data: Some(ActionData::SetTextSelection(TextSelection {
            anchor: TextPosition { node: old_run, character_index: 1 },
            focus: TextPosition { node: old_run, character_index: 4 },
        })),
    });
    assert_eq!(
        sel,
        Some(ResolvedA11yAction {
            target: node,
            action: A11yAction::SetTextSelection { anchor_char: 1, focus_char: 4 },
        })
    );

    let scroll_pg = bridge.translate_action(&ActionRequest {
        action: Action::ScrollDown,
        target_tree: TreeId::ROOT,
        target_node: old_id,
        data: Some(ActionData::ScrollUnit(ScrollUnit::Page)),
    });
    assert_eq!(
        scroll_pg,
        Some(ResolvedA11yAction {
            target: node,
            action: A11yAction::ScrollBy { dx: 0.0, dy: 240.0 },
        })
    );

    let scroll_abs = bridge.translate_action(&ActionRequest {
        action: Action::SetScrollOffset,
        target_tree: TreeId::ROOT,
        target_node: old_id,
        data: Some(ActionData::SetScrollOffset(Point { x: 10.0, y: 88.5 })),
    });
    assert_eq!(
        scroll_abs,
        Some(ResolvedA11yAction {
            target: node,
            action: A11yAction::SetScrollOffset { x: 10.0, y: 88.5 },
        })
    );

    // Non-finite scroll offset is rejected.
    assert!(bridge
        .translate_action(&ActionRequest {
            action: Action::SetScrollOffset,
            target_tree: TreeId::ROOT,
            target_node: old_id,
            data: Some(ActionData::SetScrollOffset(Point { x: f64::NAN, y: 0.0 })),
        })
        .is_none());

    // Advance epoch (Tier-2 DylibReload): ALL old-epoch requests MUST return None!
    bridge.advance_epoch();
    assert_ne!(bridge.epoch(), epoch1);
    assert!(bridge
        .translate_action(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: old_id,
            data: None,
        })
        .is_none());

    // UTF-8 byte <-> char index helpers handle multibyte characters & out-of-range offsets safely.
    let s = "a€🚀b";
    assert_eq!(byte_offset_to_char_index(s, 0), 0);
    assert_eq!(byte_offset_to_char_index(s, 1), 1);
    assert_eq!(byte_offset_to_char_index(s, 4), 2);
    assert_eq!(byte_offset_to_char_index(s, 8), 3);
    assert_eq!(byte_offset_to_char_index(s, 999), 4);
    assert_eq!(char_index_to_byte_offset(s, 2), 4);
    assert_eq!(char_index_to_byte_offset(s, 3), 8);
    assert_eq!(char_index_to_byte_offset(s, 99), s.len());
}

#[test]
fn secure_text_input_never_leaks_plaintext_with_explicit_non_textbox_aria_roles() {
    let explicit_roles = [
        AriaRole::SearchBox,
        AriaRole::ComboBox,
        AriaRole::Group,
        AriaRole::Presentation,
    ];
    let secret = "S3cr3t!Pass";
    let expected_mask = "•".repeat(secret.chars().count());

    for &explicit_role in &explicit_roles {
        for use_input_kind in [false, true] {
            let mut store = NodeStore::new();
            let mut changes = ChangeSet::default();
            let pw = {
                let mut tx = store.tx();
                let id = tx.create();
                tx.edit::<Accessibility>(id).role(Some(explicit_role));
                let ti = tx.edit::<TextInput>(id);
                if use_input_kind {
                    ti.input_kind(InputKind::Password).value(secret);
                } else {
                    ti.secure(true).value(secret);
                }
                tx.set_children(NodeId::ROOT, &[id]);
                tx.commit_into(&mut changes);
                id
            };

            assert_eq!(
                crate::resolve_node_role(&store, pw),
                Role::PasswordInput,
                "secure TextInput must keep Role::PasswordInput even with {explicit_role:?}"
            );

            let mut geo = MapGeo::new(1.0);
            geo.rects.insert(
                pw,
                LayoutRect { x: 0.0, y: 0.0, width: 160.0, height: 32.0 },
            );
            geo.values.insert(pw, secret.to_owned());

            let mut bridge = AccessKitTreeBridge::new();
            let update = bridge.build_full_tree_update(&store, &geo, "Login");
            let by_id: HashMap<_, _> = update.nodes.into_iter().collect();
            let node = &by_id[&bridge.encode_store_node_id(pw)];
            assert_eq!(node.role(), Role::PasswordInput);
            assert_eq!(node.value(), Some(expected_mask.as_str()));
            assert_ne!(node.value(), Some(secret));

            let run_id = encode_text_run_id(bridge.epoch(), pw);
            let run = &by_id[&run_id];
            assert_eq!(run.role(), Role::TextRun);
            assert_eq!(run.value(), Some(expected_mask.as_str()));
            assert_ne!(run.value(), Some(secret));
        }
    }
}

#[test]
fn display_contents_subtrees_flatten_transparently_and_respect_aria_hidden() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (outer_wrap, inner_wrap, btn_a, btn_b, hidden_wrap, hidden_btn) = {
        let mut tx = store.tx();
        let outer = tx.create();
        tx.edit::<Layout>(outer).display(Display::Contents);
        let inner = tx.create();
        tx.edit::<Layout>(inner).display(Display::Contents);
        let a = tx.create();
        tx.edit::<Accessibility>(a).role(Some(AriaRole::Button)).label("Visible A");
        let b = tx.create();
        tx.edit::<Accessibility>(b).role(Some(AriaRole::Button)).label("Visible B");
        tx.set_children(inner, &[a]);
        tx.set_children(outer, &[inner, b]);

        let h_wrap = tx.create();
        tx.edit::<Layout>(h_wrap).display(Display::Contents);
        tx.edit::<Accessibility>(h_wrap).hidden(true);
        let h_btn = tx.create();
        tx.edit::<Accessibility>(h_btn).role(Some(AriaRole::Button)).label("Hidden");
        tx.set_children(h_wrap, &[h_btn]);

        tx.set_children(NodeId::ROOT, &[outer, h_wrap]);
        tx.commit_into(&mut changes);
        (outer, inner, a, b, h_wrap, h_btn)
    };

    let mut geo = MapGeo::new(1.0);
    geo.rects.insert(NodeId::ROOT, LayoutRect { x: 0.0, y: 0.0, width: 300.0, height: 200.0 });
    geo.rects.insert(btn_a, LayoutRect { x: 10.0, y: 10.0, width: 80.0, height: 32.0 });
    geo.rects.insert(btn_b, LayoutRect { x: 100.0, y: 10.0, width: 80.0, height: 32.0 });
    geo.display_contents.insert(outer_wrap, true);
    geo.display_contents.insert(inner_wrap, true);
    geo.display_contents.insert(hidden_wrap, true);

    let mut bridge = AccessKitTreeBridge::new();
    let full = bridge.build_full_tree_update(&store, &geo, "Contents Demo");
    let by_id: HashMap<_, _> = full.nodes.into_iter().collect();
    let root = &by_id[&bridge.encode_store_node_id(NodeId::ROOT)];
    assert_eq!(
        root.children(),
        &[bridge.encode_store_node_id(btn_a), bridge.encode_store_node_id(btn_b)]
    );
    assert!(!by_id.contains_key(&bridge.encode_store_node_id(outer_wrap)));
    assert!(!by_id.contains_key(&bridge.encode_store_node_id(inner_wrap)));
    assert!(!by_id.contains_key(&bridge.encode_store_node_id(hidden_wrap)));
    assert!(!by_id.contains_key(&bridge.encode_store_node_id(hidden_btn)));

    // Toggling aria-hidden on the transparent display:contents wrapper removes flattened children!
    {
        let mut tx = store.tx();
        tx.edit::<Accessibility>(outer_wrap).hidden(true);
        tx.commit_into(&mut changes);
    }
    let inc = bridge
        .sync_changes(&store, &changes, &geo, "Contents Demo", false)
        .expect("aria-hidden toggle on display:contents wrapper must update root");
    let root_upd = inc
        .nodes
        .iter()
        .find(|(id, _)| *id == bridge.encode_store_node_id(NodeId::ROOT))
        .expect("root updated");
    assert!(root_upd.1.children().is_empty());
}

#[test]
fn aria_relations_exclude_detached_and_ancestor_hidden_debug_ids() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (target, visible_dup) = {
        let mut tx = store.tx();
        // Detached node (never attached to ROOT).
        let detached = tx.create();
        tx.edit::<DebugId>(detached).id("detached-id");

        // Ancestor aria-hidden subtree.
        let hidden_a11y = tx.create();
        tx.edit::<Accessibility>(hidden_a11y).hidden(true);
        let hidden_a11y_child = tx.create();
        tx.edit::<DebugId>(hidden_a11y_child).id("hidden-a11y-id");
        let hidden_dup = tx.create();
        tx.edit::<DebugId>(hidden_dup).id("shared-label");
        tx.set_children(hidden_a11y, &[hidden_a11y_child, hidden_dup]);

        // Ancestor display:none subtree.
        let hidden_disp = tx.create();
        tx.edit::<Layout>(hidden_disp).display(Display::None);
        let hidden_disp_child = tx.create();
        tx.edit::<DebugId>(hidden_disp_child).id("hidden-disp-id");
        tx.set_children(hidden_disp, &[hidden_disp_child]);

        // Visible node sharing the same DebugId as `hidden_dup` MUST win!
        let vis = tx.create();
        tx.edit::<DebugId>(vis).id("shared-label");
        tx.edit::<TextContent>(vis).text("Real Label");

        let tgt = tx.create();
        tx.edit::<Accessibility>(tgt)
            .role(Some(AriaRole::Button))
            .relations(Some(AriaRelations {
                labelled_by: "detached-id shared-label hidden-a11y-id".into(),
                described_by: "hidden-a11y-id".into(),
                controls: "hidden-disp-id".into(),
                active_descendant: "detached-id".into(),
            }));

        tx.set_children(NodeId::ROOT, &[hidden_a11y, hidden_disp, vis, tgt]);
        tx.commit_into(&mut changes);
        (tgt, vis)
    };

    let geo = MapGeo::new(1.0);
    let mut bridge = AccessKitTreeBridge::new();
    let full = bridge.build_full_tree_update(&store, &geo, "Relations");
    let by_id: HashMap<_, _> = full.nodes.into_iter().collect();
    let t = &by_id[&bridge.encode_store_node_id(target)];
    assert_eq!(t.labelled_by(), &[bridge.encode_store_node_id(visible_dup)]);
    assert!(t.described_by().is_empty());
    assert!(t.controls().is_empty());
    assert_eq!(t.active_descendant(), None);
}

#[test]
fn targeted_dirty_sync_updates_scroll_and_text_input_in_o1_on_large_tree() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (scroll, input) = {
        let mut tx = store.tx();
        let scroll = tx.create();
        tx.edit::<Scroll>(scroll).axes(ScrollAxes::Vertical);
        let mut rows = Vec::with_capacity(280);
        for i in 0..280u32 {
            let row = tx.create();
            tx.edit::<TextContent>(row).text(&format!("Item {i}"));
            rows.push(row);
        }
        tx.set_children(scroll, &rows);
        let input = tx.create();
        tx.edit::<TextInput>(input).value("initial");
        tx.set_children(NodeId::ROOT, &[scroll, input]);
        tx.commit_into(&mut changes);
        (scroll, input)
    };

    let mut geo = MapGeo::new(1.0);
    geo.rects.insert(NodeId::ROOT, LayoutRect { x: 0.0, y: 0.0, width: 400.0, height: 300.0 });
    geo.rects.insert(scroll, LayoutRect { x: 0.0, y: 0.0, width: 400.0, height: 240.0 });
    geo.rects.insert(input, LayoutRect { x: 0.0, y: 250.0, width: 200.0, height: 32.0 });
    geo.scroll_contents.insert(scroll, (400.0, 2800.0));
    geo.values.insert(input, "initial".into());

    let mut bridge = AccessKitTreeBridge::new();
    let full = bridge.build_full_tree_update(&store, &geo, "Large");
    assert!(full.nodes.len() > 280);

    geo.scroll_offsets.insert(scroll, (0.0, 144.0));
    let scroll_upd = bridge
        .sync_scroll_node(&store, &geo, "Large", scroll)
        .expect("targeted scroll update");
    assert_eq!(bridge.last_sync_nodes_built(), 1, "scroll sync must be O(1)");
    assert_eq!(scroll_upd.nodes.len(), 1);
    assert_eq!(scroll_upd.nodes[0].1.scroll_y(), Some(144.0));

    geo.focused = Some(input);
    geo.values.insert(input, "edited".into());
    geo.selections.insert(input, (2, 6));
    let text_upd = bridge
        .sync_text_input_node(&store, &geo, "Large", input)
        .expect("targeted text input update");
    assert_eq!(bridge.last_sync_nodes_built(), 1, "text input sync must be O(1)");
    let by_id: HashMap<_, _> = text_upd.nodes.into_iter().collect();
    assert_eq!(by_id[&bridge.encode_store_node_id(input)].value(), Some("edited"));
}

#[test]
fn virtualized_scroll_and_sticky_descendant_transforms_align_coordinates() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (scroll, sticky_hdr, sticky_btn) = {
        let mut tx = store.tx();
        let scroll = tx.create();
        tx.edit::<Scroll>(scroll)
            .axes(ScrollAxes::Vertical)
            .virtual_top_spacer(Some(960.0));
        let hdr = tx.create();
        tx.edit::<Layout>(hdr).position(Position::Sticky);
        let btn = tx.create();
        tx.edit::<Accessibility>(btn).role(Some(AriaRole::Button)).label("Pin");
        tx.set_children(hdr, &[btn]);

        let mut children = vec![hdr];
        for i in 0..100u32 {
            let row = tx.create();
            tx.edit::<TextContent>(row).text(&format!("Row {i}"));
            children.push(row);
        }
        tx.set_children(scroll, &children);
        tx.set_children(NodeId::ROOT, &[scroll]);
        tx.commit_into(&mut changes);
        (scroll, hdr, btn)
    };

    let mut geo = MapGeo::new(1.0);
    geo.rects.insert(NodeId::ROOT, LayoutRect { x: 0.0, y: 0.0, width: 320.0, height: 240.0 });
    geo.rects.insert(scroll, LayoutRect { x: 0.0, y: 0.0, width: 320.0, height: 200.0 });
    geo.rects.insert(sticky_hdr, LayoutRect { x: 0.0, y: 100.0, width: 320.0, height: 32.0 });
    geo.rects.insert(sticky_btn, LayoutRect { x: 12.0, y: 106.0, width: 64.0, height: 20.0 });
    geo.scroll_contents.insert(scroll, (320.0, 4000.0));
    geo.scroll_offsets.insert(scroll, (0.0, 1000.0));
    geo.sticky_deltas.insert(sticky_hdr, (0.0, 40.0));

    let mut bridge = AccessKitTreeBridge::new();
    let full = bridge.build_full_tree_update(&store, &geo, "VirtSticky");
    let by_id: HashMap<_, _> = full.nodes.into_iter().collect();
    let sc = &by_id[&bridge.encode_store_node_id(scroll)];
    // Absolute scroll_y reported as 1000.0, while visual shift is 1000.0 - 960.0 = 40.0!
    assert_eq!(sc.scroll_y(), Some(1000.0));
    assert_eq!(sc.transform(), Some(&Affine::translate((0.0, -40.0))));
    assert_eq!(sc.bounds(), Some(Rect { x0: 0.0, y0: 40.0, x1: 320.0, y1: 240.0 }));

    // Sticky delta lives in transform(), leaving sticky header bounds unmutated so child inherits it.
    let hdr_node = &by_id[&bridge.encode_store_node_id(sticky_hdr)];
    assert_eq!(hdr_node.bounds(), Some(Rect { x0: 0.0, y0: 100.0, x1: 320.0, y1: 132.0 }));
    assert_eq!(hdr_node.transform(), Some(&Affine::translate((0.0, 40.0))));

    // Advancing scroll updates BOTH scroll container and sticky header in O(1 + sticky_count).
    geo.scroll_offsets.insert(scroll, (0.0, 1020.0));
    geo.sticky_deltas.insert(sticky_hdr, (0.0, 60.0));
    let inc = bridge
        .sync_scroll_node(&store, &geo, "VirtSticky", scroll)
        .expect("scroll + sticky update");
    assert_eq!(bridge.last_sync_nodes_built(), 2, "must rebuild only scroll + sticky node");
    let inc_map: HashMap<_, _> = inc.nodes.into_iter().collect();
    assert_eq!(
        inc_map[&bridge.encode_store_node_id(scroll)].transform(),
        Some(&Affine::translate((0.0, -60.0)))
    );
    assert_eq!(
        inc_map[&bridge.encode_store_node_id(sticky_hdr)].transform(),
        Some(&Affine::translate((0.0, 60.0)))
    );
}

#[test]
fn set_text_selection_rejects_mismatched_text_run_owner() {
    let bridge = AccessKitTreeBridge::new();
    let node_a = NodeId::new(10, 0);
    let node_b = NodeId::new(11, 0);
    let run_a = encode_text_run_id(bridge.epoch(), node_a);
    let run_b = encode_text_run_id(bridge.epoch(), node_b);
    let req = ActionRequest {
        action: Action::SetTextSelection,
        target_tree: TreeId::ROOT,
        target_node: run_a,
        data: Some(ActionData::SetTextSelection(TextSelection {
            anchor: TextPosition { node: run_b, character_index: 0 },
            focus: TextPosition { node: run_b, character_index: 3 },
        })),
    };
    assert_eq!(
        bridge.translate_action(&req),
        None,
        "SetTextSelection targeting TextRun(A) with positions on TextRun(B) must be rejected"
    );
}

#[test]
fn effective_display_provider_overrides_static_display_none_and_contents() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (restored_none, restored_contents, inner_btn) = {
        let mut tx = store.tx();
        let none_node = tx.create();
        tx.edit::<Layout>(none_node).display(Display::None);
        tx.edit::<Accessibility>(none_node)
            .role(Some(AriaRole::Button))
            .label("Restored From None");

        let contents_node = tx.create();
        tx.edit::<Layout>(contents_node).display(Display::Contents);
        tx.edit::<Accessibility>(contents_node)
            .role(Some(AriaRole::Group))
            .label("Restored From Contents");
        let inner = tx.create();
        tx.edit::<Accessibility>(inner)
            .role(Some(AriaRole::Button))
            .label("Inner Button");
        tx.set_children(contents_node, &[inner]);

        tx.set_children(NodeId::ROOT, &[none_node, contents_node]);
        tx.commit_into(&mut changes);
        (none_node, contents_node, inner)
    };

    let mut geo = MapGeo::new(1.0);
    geo.rects.insert(NodeId::ROOT, LayoutRect { x: 0.0, y: 0.0, width: 320.0, height: 240.0 });
    geo.rects.insert(restored_none, LayoutRect { x: 0.0, y: 0.0, width: 120.0, height: 32.0 });
    geo.rects.insert(restored_contents, LayoutRect { x: 0.0, y: 40.0, width: 200.0, height: 64.0 });
    geo.rects.insert(inner_btn, LayoutRect { x: 8.0, y: 48.0, width: 100.0, height: 32.0 });

    // 1. Without provider overrides, static Layout::Display rules apply.
    let mut bridge = AccessKitTreeBridge::new();
    let static_full = bridge.build_full_tree_update(&store, &geo, "Display Overrides");
    let static_map: HashMap<_, _> = static_full.nodes.into_iter().collect();
    assert!(!static_map.contains_key(&bridge.encode_store_node_id(restored_none)));
    assert!(!static_map.contains_key(&bridge.encode_store_node_id(restored_contents)));
    assert_eq!(
        static_map[&bridge.encode_store_node_id(NodeId::ROOT)].children(),
        &[bridge.encode_store_node_id(inner_btn)]
    );

    // 2. Host StateRules restore Display::None -> Display::Flex and Display::Contents -> Display::Block.
    geo.display_none.insert(restored_none, false);
    geo.display_contents.insert(restored_contents, false);
    let restored_upd = bridge
        .sync_changes(&store, &ChangeSet::default(), &geo, "Display Overrides", true)
        .expect("restoring display via effective provider must update tree");
    let restored_map: HashMap<_, _> = restored_upd.nodes.into_iter().collect();

    let root = &restored_map[&bridge.encode_store_node_id(NodeId::ROOT)];
    assert_eq!(
        root.children(),
        &[
            bridge.encode_store_node_id(restored_none),
            bridge.encode_store_node_id(restored_contents),
        ]
    );
    let btn = &restored_map[&bridge.encode_store_node_id(restored_none)];
    assert_eq!(btn.role(), Role::Button);
    assert_eq!(btn.label(), Some("Restored From None"));

    let grp = &restored_map[&bridge.encode_store_node_id(restored_contents)];
    assert_eq!(grp.role(), Role::Group);
    assert_eq!(grp.label(), Some("Restored From Contents"));
    assert_eq!(grp.children(), &[bridge.encode_store_node_id(inner_btn)]);
}
